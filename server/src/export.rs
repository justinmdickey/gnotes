//! Plain Markdown copies of every note under `data/export`. See "Plain Markdown copies" in docs/DESIGN.md.
//!
//! One background task keeps the folder in step with the database. Edits and tree changes wake it;
//! it waits a moment so a burst of typing becomes one write, then compares what should exist with
//! what does. A copy's modified time is set to its note's `updated_at`, so a copy whose time differs
//! is stale. Anything at a path no live note wants (old titles, moves, trash) is removed.

use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use tokio::sync::Notify;
use uuid::Uuid;

use crate::{AppState, util::new_id};

/// How long after a change the copies are brought up to date.
const DELAY: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct Exporter {
    wake: Notify,
}

impl Exporter {
    /// Asks for the copies to be brought up to date shortly. Cheap; call it after any change.
    pub fn changed(&self) {
        self.wake.notify_one();
    }
}

/// Runs the export task: once at startup, so existing notes get copies, then after each change.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = sync(&state).await {
                tracing::error!("writing Markdown copies: {e:#}");
            }
            state.export.wake.notified().await;
            tokio::time::sleep(DELAY).await;
        }
    });
}

pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("export")
}

/// A title or name as a file name: no path separators or characters Windows refuses, no leading dot,
/// and short enough for any file system.
fn safe_name(name: &str, fallback: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { ' ' } else { c })
        .collect();
    let joined = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for c in joined.trim_matches(['.', ' ']).chars() {
        if out.len() + c.len_utf8() > 100 {
            break;
        }
        out.push(c);
    }
    let out = out.trim_end_matches(['.', ' ']);
    if out.is_empty() { fallback.into() } else { out.into() }
}

/// The end of an id, which is its random part for UUIDv7, so it tells apart notes made in the same minute.
fn short_id(id: &str) -> &str {
    &id[id.len().saturating_sub(8)..]
}

fn note_file(title: &str, id: &str) -> String {
    format!("{} ({}).md", safe_name(title, "Untitled"), short_id(id))
}

fn attachment_file(filename: &str, id: &str) -> String {
    let name = safe_name(filename, "attachment");
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains(' ') => format!("{stem} ({}).{ext}", short_id(id)),
        _ => format!("{name} ({})", short_id(id)),
    }
}

/// `to` relative to the folder `from`, with `/` separators.
fn relative(from: &Path, to: &Path) -> String {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); from.len() - common];
    parts.extend(to[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

/// Escapes what would end or break a Markdown link destination.
fn link_path(path: &str) -> String {
    let mut out = String::new();
    for c in path.chars() {
        match c {
            ' ' | '(' | ')' | '<' | '>' | '[' | ']' | '%' | '#' | '?' => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Points `](att:<id>)` links at the copied files, relative to the note's folder.
fn rewrite_links(body: &str, dir: &Path, attachments: &HashMap<String, PathBuf>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find("](att:") {
        let (before, after) = rest.split_at(at + 2);
        out.push_str(before);
        let id = after.get(4..40).unwrap_or("");
        match attachments.get(id) {
            Some(path) => {
                out.push_str(&link_path(&relative(dir, path)));
                rest = &after[40..];
            }
            None => rest = after,
        }
    }
    out.push_str(rest);
    out
}

fn mtime_ms(meta: &std::fs::Metadata) -> Option<i64> {
    Some(meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// Writes through a temporary file and a rename, so a reader never sees half a copy.
fn write_atomic(path: &Path, bytes: &[u8], modified: Option<i64>) -> std::io::Result<()> {
    let dir = path.parent().expect("export paths have a folder");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", new_id()));
    let written = (|| {
        std::fs::write(&tmp, bytes)?;
        if let Some(ms) = modified {
            let file = std::fs::File::options().write(true).open(&tmp)?;
            file.set_modified(UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64))?;
        }
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Every file under the export folder with its modified time, plus the folders, deepest first.
fn walk(root: &Path) -> std::io::Result<(HashMap<PathBuf, Option<i64>>, Vec<PathBuf>)> {
    let mut files = HashMap::new();
    let mut dirs = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                todo.push(entry.path());
                dirs.push(entry.path());
            } else if meta.is_file() {
                files.insert(entry.path(), mtime_ms(&meta));
            }
        }
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    Ok((files, dirs))
}

/// Removes copies no live note wants, and folders left empty. Files we didn't write are left alone.
fn clean(files: &HashMap<PathBuf, Option<i64>>, dirs: &[PathBuf], keep: &HashSet<PathBuf>) {
    for path in files.keys() {
        if keep.contains(path) {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let in_attachments = path.parent().and_then(Path::file_name).is_some_and(|d| d == "attachments");
        let ours = name.ends_with(".md") || in_attachments || (name.starts_with('.') && name.ends_with(".tmp"));
        if ours && let Err(e) = std::fs::remove_file(path) {
            tracing::error!("removing old copy {}: {e}", path.display());
        }
    }
    for dir in dirs {
        // Fails when the folder isn't empty, which is fine.
        let _ = std::fs::remove_dir(dir);
    }
}

struct Note {
    id: String,
    owner: String,
    notebook_id: Option<String>,
    title: String,
    updated_at: i64,
}

/// Each live notebook's folder, relative to its owner's folder.
fn notebook_dirs(notebooks: &[(String, Option<String>, String)]) -> HashMap<String, PathBuf> {
    let by_id: HashMap<&str, (Option<&str>, &str)> =
        notebooks.iter().map(|(id, parent, name)| (id.as_str(), (parent.as_deref(), name.as_str()))).collect();
    let mut dirs = HashMap::new();
    for (id, _, _) in notebooks {
        let mut names = Vec::new();
        let mut at = Some(id.as_str());
        // The depth limit only guards against a cycle in bad data.
        while let Some(nb) = at
            && names.len() < 64
        {
            let Some((parent, name)) = by_id.get(nb) else { break };
            names.push(safe_name(name, "Notebook"));
            at = *parent;
        }
        dirs.insert(id.clone(), names.into_iter().rev().collect());
    }
    dirs
}

/// Brings `data/export` in line with the database.
pub async fn sync(state: &AppState) -> anyhow::Result<()> {
    let root = root(&state.config.data_dir);
    let notes: Vec<(String, String, Option<String>, String, i64)> = sqlx::query_as(
        "SELECT n.id, u.username, n.notebook_id, n.title, n.updated_at
         FROM notes n JOIN users u ON u.id = n.owner_id WHERE n.deleted_at IS NULL",
    )
    .fetch_all(&state.db)
    .await?;
    let notebooks: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT id, parent_id, name FROM notebooks WHERE deleted_at IS NULL")
            .fetch_all(&state.db)
            .await?;
    let attachments: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT a.id, a.note_id, a.filename, a.sha256
         FROM attachments a JOIN notes n ON n.id = a.note_id WHERE n.deleted_at IS NULL",
    )
    .fetch_all(&state.db)
    .await?;

    let notebook_dirs = notebook_dirs(&notebooks);
    let notes: Vec<Note> = notes
        .into_iter()
        .map(|(id, owner, notebook_id, title, updated_at)| Note { id, owner, notebook_id, title, updated_at })
        .collect();
    let paths: HashMap<&str, PathBuf> = notes
        .iter()
        .map(|n| {
            let mut dir = root.join(safe_name(&n.owner, "user"));
            if let Some(nb) = n.notebook_id.as_ref().and_then(|nb| notebook_dirs.get(nb)) {
                dir.push(nb);
            }
            (n.id.as_str(), dir.join(note_file(&n.title, &n.id)))
        })
        .collect();
    let attachment_paths: HashMap<String, PathBuf> = attachments
        .iter()
        .filter_map(|(id, note, filename, _)| {
            let dir = paths.get(note.as_str())?.parent()?;
            Some((id.clone(), dir.join("attachments").join(attachment_file(filename, id))))
        })
        .collect();

    let walk_root = root.clone();
    let (files, dirs) = tokio::task::spawn_blocking(move || walk(&walk_root)).await??;

    let mut keep = HashSet::new();
    let mut kept_notes = HashSet::new();
    for note in &notes {
        let path = &paths[note.id.as_str()];
        if files.get(path).copied().flatten() == Some(note.updated_at) {
            keep.insert(path.clone());
            kept_notes.insert(note.id.as_str());
            continue;
        }
        let written = async {
            let body = crate::rooms::note_body(state, Uuid::parse_str(&note.id)?).await?;
            if body.trim().is_empty() {
                return anyhow::Ok(false);
            }
            let text = rewrite_links(&body, path.parent().unwrap(), &attachment_paths);
            let (path, at) = (path.clone(), note.updated_at);
            tokio::task::spawn_blocking(move || write_atomic(&path, text.as_bytes(), Some(at))).await??;
            Ok(true)
        }
        .await;
        match written {
            Ok(true) => {
                keep.insert(path.clone());
                kept_notes.insert(note.id.as_str());
            }
            // A blank note has no copy until something is typed.
            Ok(false) => {}
            Err(e) => {
                // Keep the old copy rather than losing it to a failed write.
                keep.insert(path.clone());
                tracing::error!("writing copy of note {}: {e:#}", note.id);
            }
        }
    }

    for (id, note, _, sha) in &attachments {
        if !kept_notes.contains(note.as_str()) {
            continue;
        }
        let Some(path) = attachment_paths.get(id) else { continue };
        keep.insert(path.clone());
        if files.contains_key(path) {
            continue;
        }
        let blob = crate::attachments::blob_path(&state.config.data_dir, sha);
        let path = path.clone();
        let copied = tokio::task::spawn_blocking(move || write_atomic(&path, &std::fs::read(&blob)?, None)).await;
        match copied {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("copying attachment {id}: {e}"),
            Err(e) => tracing::error!("copying attachment {id}: {e}"),
        }
    }

    tokio::task::spawn_blocking(move || clean(&files, &dirs, &keep)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_files() {
        assert_eq!(safe_name("a/b: c?", "x"), "a b c");
        assert_eq!(safe_name("  ..hidden. ", "x"), "hidden");
        assert_eq!(safe_name("", "Untitled"), "Untitled");
        assert!(safe_name(&"é".repeat(200), "x").len() <= 100);
        let id = "01a0d056-667e-733b-a280-1d118b647773";
        assert_eq!(note_file("Groceries", id), "Groceries (8b647773).md");
        assert_eq!(attachment_file("photo.png", id), "photo (8b647773).png");
        assert_eq!(attachment_file("memo", id), "memo (8b647773)");
    }

    #[test]
    fn links_point_at_copies() {
        let id = "01a0d056-667e-733b-a280-1d118b647773";
        let dir = Path::new("/x/ann/Home");
        let atts = HashMap::from([(id.to_owned(), PathBuf::from("/x/ann/Home/attachments/my photo (8b647773).png"))]);
        let body = format!("# Trip\n![Beach](att:{id})\n![Gone](att:01a0d056-667e-733b-a280-000000000000)");
        assert_eq!(
            rewrite_links(&body, dir, &atts),
            "# Trip\n![Beach](attachments/my%20photo%20%288b647773%29.png)\n![Gone](att:01a0d056-667e-733b-a280-000000000000)"
        );
        assert_eq!(relative(Path::new("/x/ann/Home/Kitchen"), Path::new("/x/ann/Work/a.png")), "../../Work/a.png");
    }
}
