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
use base64::{Engine, engine::general_purpose::STANDARD};
use loro::{ExportMode, LoroDoc, VersionVector, awareness::EphemeralStore};
use serde_json::json;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use crate::{AppState, perms::Role, util::now_ms};

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
    l.replace("**", "").replace("~~", "").replace('`', "").trim().chars().take(120).collect()
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
        let status = st.doc.import(data).map_err(|_| RoomError::BadUpdate)?;
        if status.pending.is_some() {
            // Loro keeps pending changes and applies them once the gap is filled by the rejoin.
            return Err(RoomError::OutOfSync);
        }
        Self::broadcast(&st, conn, &frame(KIND_UPDATE, &self.id, data));

        st.last_seq += 1;
        st.unsnapshotted += 1;
        let key = self.id.to_string();
        let now = now_ms();
        let (title, preview) = summarize(&st.doc.get_text("body").to_string());
        let summary_changed = title != st.title || preview != st.preview;
        let persisted = async {
            sqlx::query("INSERT INTO note_updates (note_id, seq, data, user_id, created_at) VALUES (?, ?, ?, ?, ?)")
                .bind(&key)
                .bind(st.last_seq)
                .bind(data)
                .bind(&user_id)
                .bind(now)
                .execute(&state.db)
                .await?;
            sqlx::query("UPDATE notes SET title = ?, preview = ?, updated_at = ? WHERE id = ?")
                .bind(&title)
                .bind(&preview)
                .bind(now)
                .bind(&key)
                .execute(&state.db)
                .await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(e) = persisted {
            tracing::error!("saving update for note {key}: {e:#}");
        }
        if summary_changed {
            st.title = title;
            st.preview = preview;
            state.hub.tree_changed();
        }
        if st.unsnapshotted >= SNAPSHOT_EVERY
            && let Err(e) = write_snapshot(&state.db, &self.id, &mut st).await
        {
            tracing::error!("snapshot for note {key}: {e:#}");
        }
        Ok(())
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
