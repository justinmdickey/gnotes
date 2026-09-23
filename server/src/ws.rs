//! One websocket per client, multiplexing every note it has open.

use std::{collections::HashSet, time::Duration};

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{
        HeaderMap,
        header::{HOST, ORIGIN},
    },
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use loro::VersionVector;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{CurrentUser, User},
    error::AppError,
    perms::note_role,
    rooms::{self, KIND_PRESENCE, KIND_UPDATE, Tx, control},
};

const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const PING_EVERY: Duration = Duration::from_secs(25);

/// Browsers always send Origin; it must match the configured public URL or this host.
fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if let Some(public) = &state.config.public_url {
        return origin == public.trim_end_matches('/');
    }
    let host = headers.get(HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    origin.split_once("://").is_some_and(|(_, rest)| rest == host)
}

pub async fn handler(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if !origin_allowed(&state, &headers) {
        return Err(AppError::Forbidden);
    }
    Ok(ws.max_message_size(MAX_MESSAGE).on_upgrade(move |socket| run(state, user, socket)))
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum ClientMsg {
    Join { note: Uuid, version: Option<String> },
    Leave { note: Uuid },
}

async fn run(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let conn = state.hub.register(tx.clone());

    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        loop {
            let msg = tokio::select! {
                msg = rx.recv() => match msg { Some(m) => m, None => break },
                _ = ping.tick() => Message::Ping(Default::default()),
            };
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut joined: HashSet<Uuid> = HashSet::new();
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
                Ok(ClientMsg::Join { note, version }) => {
                    if joined.contains(&note) {
                        rooms::leave(&state, &note, conn).await;
                    }
                    if handle_join(&state, &user, conn, &tx, note, version).await {
                        joined.insert(note);
                    } else {
                        joined.remove(&note);
                    }
                }
                Ok(ClientMsg::Leave { note }) => {
                    if joined.remove(&note) {
                        rooms::leave(&state, &note, conn).await;
                    }
                }
                Err(_) => {
                    let _ = tx.send(control(json!({ "t": "error", "code": "bad_message" })));
                }
            },
            Message::Binary(data) => handle_frame(&state, conn, &tx, &joined, &data).await,
            Message::Close(_) => break,
            _ => {}
        }
    }

    for note in &joined {
        rooms::leave(&state, note, conn).await;
    }
    state.hub.unregister(conn);
    writer.abort();
}

async fn handle_join(state: &AppState, user: &User, conn: u64, tx: &Tx, note: Uuid, version: Option<String>) -> bool {
    let error = |code: &str| {
        let _ = tx.send(control(json!({ "t": "error", "note": note, "code": code })));
        false
    };
    let role = match note_role(&state.db, &user.id, &note.to_string()).await {
        Ok(Some(role)) => role,
        Ok(None) => return error("not_found"),
        Err(e) => {
            tracing::error!("role lookup for {note}: {e:#}");
            return error("internal");
        }
    };
    let since = match version {
        None => None,
        Some(v) => match STANDARD.decode(v).ok().and_then(|b| VersionVector::decode(&b).ok()) {
            Some(vv) => Some(vv),
            None => return error("bad_version"),
        },
    };
    match rooms::join(state, note, conn, &user.id, role, tx, since).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("joining {note}: {e:#}");
            error("internal")
        }
    }
}

async fn handle_frame(state: &AppState, conn: u64, tx: &Tx, joined: &HashSet<Uuid>, data: &[u8]) {
    if data.len() < 17 {
        return;
    }
    let Ok(note) = Uuid::from_slice(&data[1..17]) else { return };
    let payload = &data[17..];
    let result = match state.rooms.get(&note).await {
        Some(room) if joined.contains(&note) => match data[0] {
            KIND_UPDATE => room.apply_update(state, conn, payload).await,
            KIND_PRESENCE => room.apply_presence(conn, payload).await,
            _ => return,
        },
        _ => Err(rooms::RoomError::Forbidden),
    };
    if let Err(e) = result {
        let _ = tx.send(control(json!({ "t": "error", "note": note, "code": e.code() })));
    }
}
