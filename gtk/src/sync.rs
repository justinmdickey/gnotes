//! One websocket for every open note, like `web/src/lib/sync.ts`. Protocol: "Websocket protocol" in
//! docs/DESIGN.md. The socket lives on the tokio runtime; events come out on a channel the UI reads.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use loro::VersionVector;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

use crate::api::{Api, Role};

pub const KIND_UPDATE: u8 = 0x01;
pub const KIND_PRESENCE: u8 = 0x02;

/// The server pings every 25 seconds; this long without hearing anything means the link is dead.
const SILENT_LIMIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Online,
    Offline,
}

#[derive(Debug)]
pub enum Event {
    Status(Status),
    /// Notebooks or notes changed; refetch `/tree`.
    TreeChanged,
    /// Some note's checkboxes changed; refetch `/tasks` if they're showing.
    TasksChanged,
    Joined { note: String, role: Role, version: VersionVector },
    Update { note: String, data: Vec<u8> },
    Presence { note: String, data: Vec<u8> },
    Role { note: String, role: Role },
    /// Access was removed.
    Revoked { note: String },
    /// `not_found`, `forbidden`, `out_of_sync`, `bad_update`, ...
    Error { note: Option<String>, code: String },
    /// The session isn't valid any more.
    SignedOut,
}

enum Cmd {
    Send(Message),
    /// Skips the rest of a reconnect wait.
    Wake,
    Stop,
}

/// Commands go in here; events come out of the receiver `start` returns.
#[derive(Clone)]
pub struct SyncClient {
    cmd: mpsc::UnboundedSender<Cmd>,
    online: Arc<AtomicBool>,
}

pub fn frame(kind: u8, note: &str, payload: &[u8]) -> Vec<u8> {
    let id = Uuid::parse_str(note).unwrap_or_default();
    let mut buf = Vec::with_capacity(17 + payload.len());
    buf.push(kind);
    buf.extend_from_slice(id.as_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Splits a binary frame into its kind, note id and payload.
pub fn parse_frame(data: &[u8]) -> Option<(u8, String, &[u8])> {
    if data.len() < 17 {
        return None;
    }
    let id = Uuid::from_slice(&data[1..17]).ok()?;
    Some((data[0], id.to_string(), &data[17..]))
}

fn parse_role(v: &Value) -> Option<Role> {
    serde_json::from_value(v.clone()).ok()
}

fn parse_control(text: &str) -> Option<Event> {
    let msg: Value = serde_json::from_str(text).ok()?;
    let note = msg["note"].as_str().map(str::to_owned);
    Some(match msg["t"].as_str()? {
        "tree_changed" => Event::TreeChanged,
        "tasks_changed" => Event::TasksChanged,
        "joined" => {
            let bytes = STANDARD.decode(msg["version"].as_str()?).ok()?;
            Event::Joined { note: note?, role: parse_role(&msg["role"])?, version: VersionVector::decode(&bytes).ok()? }
        }
        "role" => Event::Role { note: note?, role: parse_role(&msg["role"])? },
        "revoked" => Event::Revoked { note: note? },
        "error" => Event::Error { note, code: msg["code"].as_str().unwrap_or("unknown").to_owned() },
        _ => return None,
    })
}

impl SyncClient {
    /// Connects, and keeps reconnecting with backoff until `stop`. Must be called inside the tokio runtime.
    pub fn start(api: Api) -> (SyncClient, async_channel::Receiver<Event>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = async_channel::unbounded();
        let online = Arc::new(AtomicBool::new(false));
        tokio::spawn(run(api, cmd_rx, ev_tx, online.clone()));
        (SyncClient { cmd: cmd_tx, online }, ev_rx)
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// `version` is the local copy's version vector, or None to get a full snapshot.
    pub fn join(&self, note: &str, version: Option<&VersionVector>) {
        let version = version.map(|v| STANDARD.encode(v.encode()));
        self.send_json(json!({ "t": "join", "note": note, "version": version }));
    }

    pub fn leave(&self, note: &str) {
        self.send_json(json!({ "t": "leave", "note": note }));
    }

    /// False when offline: the update is dropped, and the rejoin after reconnecting sends what the server lacks.
    pub fn send_update(&self, note: &str, data: &[u8]) -> bool {
        self.send_frame(KIND_UPDATE, note, data)
    }

    pub fn send_presence(&self, note: &str, data: &[u8]) {
        self.send_frame(KIND_PRESENCE, note, data);
    }

    /// Tries to reconnect right away instead of waiting out the backoff.
    pub fn wake(&self) {
        let _ = self.cmd.send(Cmd::Wake);
    }

    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }

    fn send_json(&self, msg: Value) {
        if self.is_online() {
            let _ = self.cmd.send(Cmd::Send(Message::text(msg.to_string())));
        }
    }

    fn send_frame(&self, kind: u8, note: &str, payload: &[u8]) -> bool {
        if !self.is_online() {
            return false;
        }
        self.cmd.send(Cmd::Send(Message::binary(frame(kind, note, payload)))).is_ok()
    }
}

async fn run(
    api: Api,
    mut cmd: mpsc::UnboundedReceiver<Cmd>,
    events: async_channel::Sender<Event>,
    online: Arc<AtomicBool>,
) {
    let mut retry = 0u32;
    let mut status = Status::Connecting;
    let set_status = |s: Status, status: &mut Status| {
        if *status != s {
            *status = s;
            let _ = events.try_send(Event::Status(s));
        }
    };
    let _ = events.try_send(Event::Status(Status::Connecting));
    loop {
        let mut req = match api.ws_url().into_client_request() {
            Ok(r) => r,
            Err(_) => return,
        };
        req.headers_mut().insert("cookie", api.cookie().parse().expect("cookie header"));
        let connected = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(req)).await;
        match connected {
            Ok(Ok((ws, _))) => {
                retry = 0;
                let (mut sink, mut stream) = ws.split();
                online.store(true, Ordering::Relaxed);
                set_status(Status::Online, &mut status);
                // Catch up on anything that changed while disconnected.
                let _ = events.try_send(Event::TreeChanged);
                loop {
                    tokio::select! {
                        c = cmd.recv() => match c {
                            Some(Cmd::Send(msg)) => if sink.send(msg).await.is_err() { break },
                            Some(Cmd::Wake) => {}
                            Some(Cmd::Stop) | None => {
                                online.store(false, Ordering::Relaxed);
                                let _ = sink.send(Message::Close(None)).await;
                                return;
                            }
                        },
                        msg = tokio::time::timeout(SILENT_LIMIT, stream.next()) => match msg {
                            Ok(Some(Ok(Message::Text(text)))) => {
                                if let Some(ev) = parse_control(&text) {
                                    let _ = events.send(ev).await;
                                }
                            }
                            Ok(Some(Ok(Message::Binary(data)))) => {
                                if let Some((kind, note, payload)) = parse_frame(&data) {
                                    let data = payload.to_vec();
                                    let ev = match kind {
                                        KIND_UPDATE => Event::Update { note, data },
                                        KIND_PRESENCE => Event::Presence { note, data },
                                        _ => continue,
                                    };
                                    let _ = events.send(ev).await;
                                }
                            }
                            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) | Err(_) => break,
                            Ok(Some(Ok(_))) => {}
                        },
                    }
                }
                online.store(false, Ordering::Relaxed);
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(res))) if res.status() == 401 => {
                online.store(false, Ordering::Relaxed);
                let _ = events.send(Event::SignedOut).await;
                return;
            }
            _ => {}
        }
        set_status(Status::Offline, &mut status);
        let delay = Duration::from_millis((500u64 << retry.min(5)).min(10_000));
        retry += 1;
        let wait = tokio::time::sleep(delay);
        tokio::pin!(wait);
        // Commands sent while offline are dropped; joins are re-sent once back online.
        loop {
            tokio::select! {
                _ = &mut wait => break,
                c = cmd.recv() => match c {
                    Some(Cmd::Wake) => break,
                    Some(Cmd::Send(_)) => {}
                    Some(Cmd::Stop) | None => return,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let id = "0192f0c4-5b7e-7a3c-9d2e-112233445566";
        let f = frame(KIND_UPDATE, id, b"abc");
        assert_eq!(f.len(), 20);
        let (kind, note, payload) = parse_frame(&f).unwrap();
        assert_eq!((kind, note.as_str(), payload), (KIND_UPDATE, id, &b"abc"[..]));
    }

    #[test]
    fn control_messages() {
        let v = STANDARD.encode(VersionVector::new().encode());
        let ev = parse_control(&format!(r#"{{"t":"joined","note":"n","role":"editor","version":"{v}"}}"#));
        assert!(matches!(ev, Some(Event::Joined { role: Role::Editor, .. })));
        assert!(matches!(parse_control(r#"{"t":"tree_changed"}"#), Some(Event::TreeChanged)));
        assert!(matches!(parse_control(r#"{"t":"tasks_changed"}"#), Some(Event::TasksChanged)));
        let ev = parse_control(r#"{"t":"error","note":"n","code":"out_of_sync"}"#);
        assert!(matches!(ev, Some(Event::Error { code, .. }) if code == "out_of_sync"));
    }
}
