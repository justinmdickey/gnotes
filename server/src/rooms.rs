//! In-memory rooms: one per open note, holding its LoroDoc and connected clients.
//! Wire format and rules are under "Websocket protocol" in docs/DESIGN.md.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::extract::ws::Message;
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use loro::{ExportMode, Frontiers, LoroDoc, UpdateOptions, VersionVector, awareness::EphemeralStore};
use serde_json::json;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiResult, AppError},
    perms::Role,
    util::now_ms,
};

pub const KIND_UPDATE: u8 = 0x01;
pub const KIND_PRESENCE: u8 = 0x02;

const PRESENCE_TIMEOUT_MS: i64 = 30_000;
const SNAPSHOT_EVERY: usize = 500;
const IDLE_CLOSE: Duration = Duration::from_secs(60);

pub type Tx = mpsc::UnboundedSender<Message>;

pub fn frame(kind: u8, note: &Uuid, payload: &[u8]) -> Message {
    let mut buf = Vec::with_capacity(17 + payload.len());
    buf.push(kind);
    buf.extend_from_slice(note.as_bytes());
    buf.extend_from_slice(payload);
    Message::Binary(buf.into())
}

pub fn control(value: serde_json::Value) -> Message {
    Message::Text(value.to_string().into())
}

/// Tracks every open websocket so tree changes can be announced.
#[derive(Default)]
pub struct Hub {
    next_id: AtomicU64,
    conns: std::sync::Mutex<HashMap<u64, (String, Tx)>>,
}

impl Hub {
    pub fn register(&self, user_id: &str, tx: Tx) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.conns.lock().unwrap().insert(id, (user_id.to_owned(), tx));
        id
    }

    /// Closes every live connection a user has, e.g. after disabling them or resetting their password.
    pub fn disconnect_user(&self, user_id: &str) {
        for (owner, tx) in self.conns.lock().unwrap().values() {
            if owner == user_id {
                let _ = tx.send(Message::Close(None));
            }
        }
    }

    pub fn unregister(&self, id: u64) {
        self.conns.lock().unwrap().remove(&id);
    }

    /// Tells every client to refetch /tree. It carries no content, so it goes to everyone.
    pub fn tree_changed(&self) {
        let msg = control(json!({ "t": "tree_changed" }));
        for (_, tx) in self.conns.lock().unwrap().values() {
            let _ = tx.send(msg.clone());
        }
    }
}

struct Client {
    user_id: String,
    role: Role,
    tx: Tx,
    presence_keys: HashSet<String>,
}

struct RoomState {
    doc: LoroDoc,
    presence: EphemeralStore,
    clients: HashMap<u64, Client>,
    last_seq: i64,
    unsnapshotted: usize,
    title: String,
    preview: String,
    /// Bumped on every join so a pending idle close can tell someone came back.
    generation: u64,
    closed: bool,
}

pub struct Room {
    pub id: Uuid,
    state: Mutex<RoomState>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RoomError {
    Forbidden,
    BadUpdate,
    /// The update depends on changes the server never got. The client should rejoin with its version.
    OutOfSync,
}

impl RoomError {
    pub fn code(&self) -> &'static str {
        match self {
            RoomError::Forbidden => "forbidden",
            RoomError::BadUpdate => "bad_update",
            RoomError::OutOfSync => "out_of_sync",
        }
    }
}

/// A line without its Markdown block marks (headings, bullets, checkboxes, quotes).
fn plain_line(line: &str) -> String {
    let mut l = line.trim();
    l = l.trim_start_matches('#').trim_start();
    l = l.trim_start_matches('>').trim_start();
    for marker in ["- [ ] ", "- [x] ", "- [X] ", "* ", "- ", "+ "] {
        if let Some(rest) = l.strip_prefix(marker) {
            l = rest;
            break;
        }
    }
    if let Some((num, rest)) = l.split_once(". ")
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        l = rest;
    }
    // An embedded photo or recording reads as its label, e.g. "Voice memo".
    if let Some(rest) = l.strip_prefix("![")
        && let Some((alt, link)) = rest.split_once("](att:")
        && link.ends_with(')')
    {
        return alt.trim().chars().take(120).collect();
    }
    l.replace("**", "").replace("~~", "").replace('`', "").replace("[[", "").replace("]]", "").trim().chars().take(120).collect()
}

/// Title (first non-empty line) and preview (the next one), as shown in note lists.
pub fn summarize(body: &str) -> (String, String) {
    let mut lines = body.lines().map(plain_line).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or_default();
    let preview = lines.next().unwrap_or_default();
    (title, preview)
}

#[cfg(test)]
mod tests {
    use super::summarize;

    #[test]
    fn summary_skips_markdown_marks() {
        assert_eq!(summarize("\n# Groceries\n\n- [ ] **milk**\n- eggs"), ("Groceries".into(), "milk".into()));
        assert_eq!(summarize("1. first\n> quoted"), ("first".into(), "quoted".into()));
        assert_eq!(summarize(""), (String::new(), String::new()));
        let memo = "![Voice memo](att:01a0d056-667e-733b-a280-1d118b647773)\nbuy milk";
        assert_eq!(summarize(memo), ("Voice memo".into(), "buy milk".into()));
        let marked = "Trip\n# ![Photo](att:01a0d056-667e-733b-a280-1d118b647773)";
        assert_eq!(summarize(marked), ("Trip".into(), "Photo".into()));
        // A wiki link reads as its title, without the brackets.
        assert_eq!(summarize("[[Groceries]]\nbuy milk"), ("Groceries".into(), "buy milk".into()));
        assert_eq!(summarize("See [[Groceries]] for the list"), ("See Groceries for the list".into(), String::new()));
    }
}

#[derive(Default)]
pub struct Rooms {
    map: Mutex<HashMap<Uuid, Arc<Room>>>,
}

impl Rooms {
    pub async fn get(&self, id: &Uuid) -> Option<Arc<Room>> {
        self.map.lock().await.get(id).cloned()
    }

    async fn get_or_load(&self, db: &sqlx::SqlitePool, id: Uuid) -> anyhow::Result<Arc<Room>> {
        let mut map = self.map.lock().await;
        if let Some(room) = map.get(&id) {
            return Ok(room.clone());
        }
        let room = Arc::new(load(db, id).await?);
        map.insert(id, room.clone());
        Ok(room)
    }

    pub async fn all(&self) -> Vec<Arc<Room>> {
        self.map.lock().await.values().cloned().collect()
    }
}

async fn load(db: &sqlx::SqlitePool, id: Uuid) -> anyhow::Result<Room> {
    let key = id.to_string();
    let doc = LoroDoc::new();
    let snapshot: Option<Vec<u8>> = sqlx::query_scalar("SELECT snapshot FROM note_snapshots WHERE note_id = ?")
        .bind(&key)
        .fetch_optional(db)
        .await?;
    if let Some(snapshot) = snapshot {
        doc.import(&snapshot)?;
    }
    let updates: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT seq, data FROM note_updates WHERE note_id = ? ORDER BY seq")
            .bind(&key)
            .fetch_all(db)
            .await?;
    let mut last_seq = 0;
    for (seq, data) in &updates {
        doc.import(data)?;
        last_seq = *seq;
    }
    let (title, preview): (String, String) = sqlx::query_as("SELECT title, preview FROM notes WHERE id = ?")
        .bind(&key)
        .fetch_optional(db)
        .await?
        .unwrap_or_default();
    Ok(Room {
        id,
        state: Mutex::new(RoomState {
            doc,
            presence: EphemeralStore::new(PRESENCE_TIMEOUT_MS),
            clients: HashMap::new(),
            last_seq,
            unsnapshotted: updates.len(),
            title,
            preview,
            generation: 0,
            closed: false,
        }),
    })
}

/// Writes a snapshot and drops the update log it covers.
async fn write_snapshot(db: &sqlx::SqlitePool, id: &Uuid, st: &mut RoomState) -> anyhow::Result<()> {
    let snapshot = st.doc.export(ExportMode::Snapshot)?;
    let version = st.doc.oplog_vv().encode();
    let key = id.to_string();
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO note_snapshots (note_id, snapshot, version, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (note_id) DO UPDATE SET snapshot = excluded.snapshot, version = excluded.version,
           updated_at = excluded.updated_at",
    )
    .bind(&key)
    .bind(snapshot)
    .bind(version)
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM note_updates WHERE note_id = ? AND seq <= ?")
        .bind(&key)
        .bind(st.last_seq)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    st.unsnapshotted = 0;
    Ok(())
}

impl Room {
    /// Adds a client and sends `joined`, the missing document data and current presence.
    /// Returns false if the room closed meanwhile; the caller should load it again.
    async fn join(&self, conn: u64, user_id: &str, role: Role, tx: &Tx, since: Option<VersionVector>) -> anyhow::Result<bool> {
        let mut st = self.state.lock().await;
        if st.closed {
            return Ok(false);
        }
        st.generation += 1;
        let version = STANDARD.encode(st.doc.oplog_vv().encode());
        let data = match &since {
            Some(vv) => st.doc.export(ExportMode::updates(vv))?,
            None => st.doc.export(ExportMode::Snapshot)?,
        };
        let _ = tx.send(control(json!({
            "t": "joined", "note": self.id, "role": role.as_str(), "version": version,
        })));
        let _ = tx.send(frame(KIND_UPDATE, &self.id, &data));
        st.presence.remove_outdated();
        if !st.presence.keys().is_empty() {
            let _ = tx.send(frame(KIND_PRESENCE, &self.id, &st.presence.encode_all()));
        }
        st.clients.insert(
            conn,
            Client { user_id: user_id.into(), role, tx: tx.clone(), presence_keys: HashSet::new() },
        );
        Ok(true)
    }

    fn broadcast(st: &RoomState, except: u64, msg: &Message) {
        for (id, client) in &st.clients {
            if *id != except {
                let _ = client.tx.send(msg.clone());
            }
        }
    }

    /// Clears a departing client's cursors for everyone else.
    fn drop_client(&self, st: &mut RoomState, conn: u64) -> Option<Client> {
        let client = st.clients.remove(&conn)?;
        for key in &client.presence_keys {
            st.presence.delete(key);
            let msg = frame(KIND_PRESENCE, &self.id, &st.presence.encode(key));
            Self::broadcast(st, conn, &msg);
        }
        Some(client)
    }

    pub async fn apply_update(&self, state: &AppState, conn: u64, data: &[u8]) -> Result<(), RoomError> {
        let mut st = self.state.lock().await;
        let client = st.clients.get(&conn).ok_or(RoomError::Forbidden)?;
        if client.role < Role::Editor {
            return Err(RoomError::Forbidden);
        }
        let user_id = client.user_id.clone();
        let before = st.doc.oplog_vv();
        let status = st.doc.import(data).map_err(|_| RoomError::BadUpdate)?;
        if status.pending.is_some() {
            // Loro keeps pending changes and applies them once the gap is filled by the rejoin.
            return Err(RoomError::OutOfSync);
        }
        self.record(state, &mut st, conn, data, &user_id, &before).await;
        Ok(())
    }

    /// After `data` was imported into the doc: sends it to every client but `from`, and saves it,
    /// the note's title, preview and search text, and which sessions (Loro peers) were `user_id`'s.
    async fn record(&self, state: &AppState, st: &mut RoomState, from: u64, data: &[u8], user_id: &str, before: &VersionVector) {
        Self::broadcast(st, from, &frame(KIND_UPDATE, &self.id, data));
        // Sessions with new edits in this update are this user's, for showing who wrote what.
        let peers: Vec<String> =
            st.doc.oplog_vv().iter().filter(|(p, c)| before.get(p).is_none_or(|b| b < c)).map(|(p, _)| p.to_string()).collect();

        st.last_seq += 1;
        st.unsnapshotted += 1;
        let key = self.id.to_string();
        let now = now_ms();
        let body = st.doc.get_text("body").to_string();
        let (title, preview) = summarize(&body);
        let summary_changed = title != st.title || preview != st.preview;
        let persisted = async {
            sqlx::query("INSERT INTO note_updates (note_id, seq, data, user_id, created_at) VALUES (?, ?, ?, ?, ?)")
                .bind(&key)
                .bind(st.last_seq)
                .bind(data)
                .bind(user_id)
                .bind(now)
                .execute(&state.db)
                .await?;
            for peer in &peers {
                sqlx::query("INSERT OR IGNORE INTO note_peers (note_id, peer, user_id, first_seen) VALUES (?, ?, ?, ?)")
                    .bind(&key)
                    .bind(peer)
                    .bind(user_id)
                    .bind(now)
                    .execute(&state.db)
                    .await?;
            }
            sqlx::query("UPDATE notes SET title = ?, preview = ?, updated_at = ? WHERE id = ?")
                .bind(&title)
                .bind(&preview)
                .bind(now)
                .bind(&key)
                .execute(&state.db)
                .await?;
            crate::search::index(&state.db, &key, &body).await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(e) = persisted {
            tracing::error!("saving update for note {key}: {e:#}");
        }
        state.export.changed();
        state.embedder.changed(&key);
        if summary_changed {
            st.title = title;
            st.preview = preview;
            state.hub.tree_changed();
        }
        if st.unsnapshotted >= SNAPSHOT_EVERY
            && let Err(e) = write_snapshot(&state.db, &self.id, st).await
        {
            tracing::error!("snapshot for note {key}: {e:#}");
        }
    }

    pub async fn apply_presence(&self, conn: u64, data: &[u8]) -> Result<(), RoomError> {
        let mut st = self.state.lock().await;
        if !st.clients.contains_key(&conn) {
            return Err(RoomError::Forbidden);
        }
        // Decode separately to learn which keys this client owns, so they can be cleared when it leaves.
        let incoming = EphemeralStore::new(PRESENCE_TIMEOUT_MS);
        incoming.apply(data).map_err(|_| RoomError::BadUpdate)?;
        st.presence.apply(data).map_err(|_| RoomError::BadUpdate)?;
        if let Some(client) = st.clients.get_mut(&conn) {
            client.presence_keys.extend(incoming.keys());
        }
        Self::broadcast(&st, conn, &frame(KIND_PRESENCE, &self.id, data));
        Ok(())
    }
}

/// The note's current Markdown, from its open room or from storage.
pub async fn note_body(state: &AppState, note: Uuid) -> anyhow::Result<String> {
    let room = match state.rooms.get(&note).await {
        Some(room) => room,
        None => Arc::new(load(&state.db, note).await?),
    };
    let st = room.state.lock().await;
    Ok(st.doc.get_text("body").to_string())
}

/// A note's text and the version it was read at.
pub struct NoteText {
    pub text: String,
    /// The doc's frontiers, URL-safe base64. A writer hands it back so its change merges with
    /// anything written since, instead of undoing it.
    pub version: String,
}

/// No websocket: the conn id a server-side write records its update under, so every client gets it.
const SERVER: u64 = u64::MAX;

fn version_of(doc: &LoroDoc) -> String {
    URL_SAFE_NO_PAD.encode(doc.oplog_frontiers().encode())
}

/// The note's current text and version, from its open room or from storage.
pub async fn read_text(state: &AppState, note: Uuid) -> anyhow::Result<NoteText> {
    let room = match state.rooms.get(&note).await {
        Some(room) => room,
        None => Arc::new(load(&state.db, note).await?),
    };
    let st = room.state.lock().await;
    Ok(NoteText { text: st.doc.get_text("body").to_string(), version: version_of(&st.doc) })
}

/// Changes a note's text from the server as `user_id`: an agent's write through /api/v1. It goes
/// through the note's room like a client's edit, so open editors see it live and it's saved,
/// indexed and credited to `user_id` the same way.
///
/// `change` gets the text as of `base` (a version from `read_text`; the latest when `None`) and
/// returns the new text. The difference is applied to the doc at `base` and merged, so edits made
/// since `base` stay. Returns the text and version after the merge.
pub async fn write_text(
    state: &AppState,
    note: Uuid,
    user_id: &str,
    base: Option<&str>,
    change: impl FnOnce(&str) -> String,
) -> ApiResult<NoteText> {
    let base = match base {
        Some(v) => Some(
            URL_SAFE_NO_PAD
                .decode(v)
                .ok()
                .and_then(|b| Frontiers::decode(&b).ok())
                .ok_or_else(|| AppError::BadRequest("That version isn't one this note has had".into()))?,
        ),
        None => None,
    };
    let mut change = Some(change);
    loop {
        let room = state.rooms.get_or_load(&state.db, note).await?;
        let mut st = room.state.lock().await;
        if st.closed {
            // It closed while we waited for the lock; load it again.
            continue;
        }
        let at = base.clone().unwrap_or_else(|| st.doc.oplog_frontiers());
        let known = st.doc.oplog_vv();
        if !at.iter().all(|id| known.includes_id(id)) {
            return Err(AppError::BadRequest("That version isn't one this note has had".into()));
        }
        let fork = st
            .doc
            .fork_at(&at)
            .map_err(|_| AppError::BadRequest("That version isn't one this note has had".into()))?;
        // A peer of its own, so who-wrote-what can tell this write from the room's other writers.
        fork.set_peer_id(getrandom::u64().map_err(|e| anyhow::anyhow!("getrandom: {e}"))?)?;
        let from = fork.oplog_vv();
        let body = fork.get_text("body");
        let new = change.take().expect("runs once")(&body.to_string());
        body.update(&new, UpdateOptions::default()).map_err(|e| anyhow::anyhow!("diffing text: {e:?}"))?;
        fork.commit();
        if fork.oplog_vv() != from {
            let data = fork.export(ExportMode::updates(&from))?;
            let before = st.doc.oplog_vv();
            st.doc.import(&data)?;
            room.record(state, &mut st, SERVER, &data, user_id, &before).await;
        }
        let written = NoteText { text: st.doc.get_text("body").to_string(), version: version_of(&st.doc) };
        // Nobody has it open: the room snapshots and closes like one a client left.
        if st.clients.is_empty() {
            schedule_close(state.clone(), room.clone(), st.generation);
        }
        return Ok(written);
    }
}

/// Gives a note that has never been opened its first content, e.g. from an import.
pub async fn seed<'c>(
    db: impl sqlx::SqliteExecutor<'c>,
    note: &str,
    body: &str,
) -> anyhow::Result<()> {
    let doc = LoroDoc::new();
    doc.get_text("body").insert(0, body)?;
    doc.commit();
    sqlx::query("INSERT INTO note_snapshots (note_id, snapshot, version, updated_at) VALUES (?, ?, ?, ?)")
        .bind(note)
        .bind(doc.export(ExportMode::Snapshot)?)
        .bind(doc.oplog_vv().encode())
        .bind(now_ms())
        .execute(db)
        .await?;
    Ok(())
}

/// Joins `conn` to a note's room, loading it if needed.
pub async fn join(
    state: &AppState,
    note: Uuid,
    conn: u64,
    user_id: &str,
    role: Role,
    tx: &Tx,
    since: Option<VersionVector>,
) -> anyhow::Result<()> {
    loop {
        let room = state.rooms.get_or_load(&state.db, note).await?;
        if room.join(conn, user_id, role, tx, since.clone()).await? {
            return Ok(());
        }
    }
}

pub async fn leave(state: &AppState, note: &Uuid, conn: u64) {
    let Some(room) = state.rooms.get(note).await else { return };
    let mut st = room.state.lock().await;
    room.drop_client(&mut st, conn);
    if st.clients.is_empty() {
        schedule_close(state.clone(), room.clone(), st.generation);
    }
}

/// Closes a room that stays empty for IDLE_CLOSE, writing a final snapshot.
fn schedule_close(state: AppState, room: Arc<Room>, generation: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(IDLE_CLOSE).await;
        let mut map = state.rooms.map.lock().await;
        let mut st = room.state.lock().await;
        if st.closed || !st.clients.is_empty() || st.generation != generation {
            return;
        }
        if st.unsnapshotted > 0
            && let Err(e) = write_snapshot(&state.db, &room.id, &mut st).await
        {
            tracing::error!("snapshot for note {}: {e:#}", room.id);
        }
        st.closed = true;
        map.remove(&room.id);
    });
}

/// Re-resolves every connected client's role after shares, moves or deletes.
/// Clients that lost access get `revoked`; changed roles get a `role` message.
pub async fn recheck_access(state: &AppState) {
    for room in state.rooms.all().await {
        let mut st = room.state.lock().await;
        let had_clients = !st.clients.is_empty();
        let note = room.id.to_string();
        let clients: Vec<(u64, String, Role)> =
            st.clients.iter().map(|(id, c)| (*id, c.user_id.clone(), c.role)).collect();
        for (conn, user_id, old) in clients {
            let role = match crate::perms::note_role(&state.db, &user_id, &note).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("rechecking access to {note}: {e:#}");
                    continue;
                }
            };
            match role {
                None => {
                    if let Some(client) = room.drop_client(&mut st, conn) {
                        let _ = client.tx.send(control(json!({ "t": "revoked", "note": room.id })));
                    }
                }
                Some(role) if role != old => {
                    if let Some(client) = st.clients.get_mut(&conn) {
                        client.role = role;
                        let _ = client.tx.send(control(json!({ "t": "role", "note": room.id, "role": role.as_str() })));
                    }
                }
                Some(_) => {}
            }
        }
        if had_clients && st.clients.is_empty() {
            schedule_close(state.clone(), room.clone(), st.generation);
        }
    }
}
