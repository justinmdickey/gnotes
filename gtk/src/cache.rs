//! What the client keeps on disk: the signed-in session, and per account the last tree and a Loro
//! snapshot of each opened note, so notes open and edit with no server. Like web/src/lib/offline.ts.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::api::{Tree, User};

/// `GNOTES_GTK_DIR` puts everything in one folder (for tests and screenshots); otherwise the XDG
/// config and data folders.
fn base(xdg: &str, fallback: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("GNOTES_GTK_DIR") {
        return PathBuf::from(dir).join(fallback.rsplit('/').next().unwrap_or(fallback));
    }
    let root = std::env::var_os(xdg)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback));
    root.join("gnotes")
}

pub fn config_dir() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config")
}

pub fn data_dir() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share")
}

/// Writes through a temporary file, so a crash never leaves half a file.
fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(tmp, path)
}

/// The signed-in account, kept between runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub server: String,
    pub token: String,
    pub user: User,
}

impl Session {
    fn path() -> PathBuf {
        config_dir().join("session.json")
    }

    pub fn load() -> Option<Session> {
        serde_json::from_slice(&fs::read(Self::path()).ok()?).ok()
    }

    pub fn save(&self) -> std::io::Result<()> {
        write_atomic(&Self::path(), &serde_json::to_vec_pretty(self)?, true)
    }

    pub fn clear() {
        let _ = fs::remove_file(Self::path());
    }
}

/// A note made offline, waiting to be created on the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Queued {
    pub id: String,
    pub notebook_id: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Pending {
    /// Notes that may have edits the server lacks.
    dirty: Vec<String>,
    queued: Vec<Queued>,
}

/// One account's offline copies.
#[derive(Clone)]
pub struct Cache {
    dir: PathBuf,
}

/// Something safe as a folder name from a server URL, e.g. `notes.example.com_8080`.
fn server_folder(server: &str) -> String {
    let rest = server.split_once("://").map_or(server, |(_, r)| r);
    rest.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

impl Cache {
    pub fn new(dir: PathBuf) -> Cache {
        Cache { dir }
    }

    pub fn for_account(server: &str, user_id: &str) -> Cache {
        Cache::new(data_dir().join(server_folder(server)).join(user_id))
    }

    fn note_path(&self, id: &str) -> PathBuf {
        // Ids are UUIDs; anything else can't name a file outside the folder.
        let safe: String = id.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        self.dir.join("notes").join(format!("{safe}.loro"))
    }

    pub fn save_tree(&self, tree: &Tree) {
        if let Ok(bytes) = serde_json::to_vec(tree) {
            let _ = write_atomic(&self.dir.join("tree.json"), &bytes, true);
        }
    }

    pub fn tree(&self) -> Option<Tree> {
        serde_json::from_slice(&fs::read(self.dir.join("tree.json")).ok()?).ok()
    }

    pub fn save_note(&self, id: &str, snapshot: &[u8]) {
        let _ = write_atomic(&self.note_path(id), snapshot, true);
    }

    pub fn note(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.note_path(id)).ok()
    }

    /// Drops a note's copy, e.g. when it's gone from the server.
    pub fn forget(&self, id: &str) {
        let _ = fs::remove_file(self.note_path(id));
        self.edit_pending(|p| {
            p.dirty.retain(|d| d != id);
            p.queued.retain(|q| q.id != id);
        });
    }

    fn pending(&self) -> Pending {
        fs::read(self.dir.join("pending.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn edit_pending(&self, f: impl FnOnce(&mut Pending)) {
        let mut p = self.pending();
        f(&mut p);
        if let Ok(bytes) = serde_json::to_vec(&p) {
            let _ = write_atomic(&self.dir.join("pending.json"), &bytes, true);
        }
    }

    pub fn mark_dirty(&self, id: &str) {
        self.edit_pending(|p| {
            if !p.dirty.iter().any(|d| d == id) {
                p.dirty.push(id.to_owned());
            }
        });
    }

    pub fn clear_dirty(&self, id: &str) {
        self.edit_pending(|p| p.dirty.retain(|d| d != id));
    }

    pub fn dirty(&self) -> Vec<String> {
        self.pending().dirty
    }

    pub fn queue(&self, q: Queued) {
        self.edit_pending(|p| p.queued.push(q));
    }

    pub fn unqueue(&self, id: &str) {
        self.edit_pending(|p| p.queued.retain(|q| q.id != id));
    }

    pub fn queued(&self) -> Vec<Queued> {
        self.pending().queued
    }

    /// Everything this account kept on this computer, on Log Out.
    pub fn wipe(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_notes_and_pending_work() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().to_path_buf());
        assert!(cache.tree().is_none());
        cache.save_tree(&Tree::default());
        assert!(cache.tree().is_some());

        let id = "0192f0c4-5b7e-7a3c-9d2e-112233445566";
        cache.save_note(id, b"snap");
        assert_eq!(cache.note(id).as_deref(), Some(&b"snap"[..]));
        assert!(cache.note("../../etc/passwd").is_none());

        cache.mark_dirty(id);
        cache.mark_dirty(id);
        assert_eq!(cache.dirty(), vec![id.to_owned()]);
        cache.queue(Queued { id: id.into(), notebook_id: None });
        cache.forget(id);
        assert!(cache.dirty().is_empty() && cache.queued().is_empty() && cache.note(id).is_none());
    }

    #[test]
    fn server_folders_are_plain() {
        assert_eq!(server_folder("https://notes.example.com:8443"), "notes.example.com_8443");
    }
}
