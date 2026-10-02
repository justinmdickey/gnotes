//! Live transcripts while recording: the app streams microphone audio here and the server relays it to
//! the configured realtime speech-to-text endpoint. The browser can't reach that endpoint itself; it is
//! usually a plain ws:// address on the server's network.
//!
//! App -> server: binary frames of 16 kHz mono 16-bit PCM, and `{"t":"commit"}` when recording stops.
//! Server -> app: `{"t":"ready"}`, `{"t":"delta","text"}` as words arrive, `{"t":"final","text"}` for
//! each finished stretch of speech (it replaces that stretch's deltas), and `{"t":"error","message"}`.
//!
//! Upstream it speaks the OpenAI realtime transcription events (`input_audio_buffer.append`,
//! `conversation.item.input_audio_transcription.delta` / `.completed`) with a flat `session.update`.

use std::time::Duration;

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{self, client::IntoClientRequest},
};

use crate::{AppState, WhisperConfig, auth::CurrentUser, error::AppError, ws::origin_allowed};

pub const SAMPLE_RATE: u32 = 16_000;
type Upstream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens the realtime endpoint and starts a transcription session.
async fn connect(w: &WhisperConfig) -> anyhow::Result<Upstream> {
    let url = w.realtime_url.as_deref().ok_or_else(|| anyhow::anyhow!("no live URL"))?;
    let mut req = url.into_client_request()?;
    if let Some(key) = &w.key {
        req.headers_mut().insert("authorization", format!("Bearer {key}").parse()?);
    }
    let (mut up, _) = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(req)).await??;
    let session = json!({
        "type": "session.update",
        "session": { "type": "transcription", "model": w.model, "input_audio_format": "pcm16", "sample_rate": SAMPLE_RATE },
    });
    up.send(tungstenite::Message::Text(session.to_string().into())).await?;
    Ok(up)
}

/// For Settings › Test: connects and waits for the service to accept the session.
pub async fn check(w: &WhisperConfig) -> Result<(), String> {
    let mut up = connect(w).await.map_err(|_| "couldn't connect to the live URL".to_owned())?;
    let answer = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(msg)) = up.next().await {
            let tungstenite::Message::Text(text) = msg else { continue };
            let event: Value = serde_json::from_str(&text).unwrap_or_default();
            match event["type"].as_str() {
                Some("session.created" | "session.updated") => return Ok(()),
                Some("error") => {
                    return Err(event["error"]["message"].as_str().unwrap_or("the service refused the session").to_owned());
                }
                _ => {}
            }
        }
        Err("the service closed the connection".to_owned())
    })
    .await
    .unwrap_or_else(|_| Err("no answer within 5 seconds".to_owned()));
    let _ = up.close(None).await;
    answer
}

/// `GET /transcribe/live`: a websocket for one recording.
pub async fn handler(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if !origin_allowed(&state, &headers) {
        return Err(AppError::Forbidden);
    }
    let Some(w) = state.whisper.read().await.clone().filter(|w| w.realtime_url.is_some()) else {
        return Err(AppError::Conflict("Live transcription isn't set up on this server".into()));
    };
    Ok(ws.on_upgrade(move |socket| relay(socket, w)))
}

fn say(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

async fn relay(mut app: WebSocket, w: WhisperConfig) {
    let up = match connect(&w).await {
        Ok(up) => up,
        Err(e) => {
            tracing::warn!("live transcription: connecting upstream failed: {e:#}");
            let _ = app.send(say(json!({ "t": "error", "message": "Couldn't reach the live transcription service" }))).await;
            return;
        }
    };
    let _ = app.send(say(json!({ "t": "ready" }))).await;
    let (mut up_tx, mut up_rx) = up.split();
    let (mut app_tx, mut app_rx) = app.split();
    loop {
        tokio::select! {
            msg = app_rx.next() => {
                let event = match msg {
                    Some(Ok(Message::Binary(pcm))) => json!({ "type": "input_audio_buffer.append", "audio": STANDARD.encode(&pcm) }),
                    Some(Ok(Message::Text(text))) if text.contains("commit") => json!({ "type": "input_audio_buffer.commit" }),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                if up_tx.send(tungstenite::Message::Text(event.to_string().into())).await.is_err() {
                    let _ = app_tx.send(say(json!({ "t": "error", "message": "The live transcription service went away" }))).await;
                    break;
                }
            }
            msg = up_rx.next() => {
                let text = match msg {
                    Some(Ok(tungstenite::Message::Text(text))) => text,
                    Some(Ok(_)) => continue,
                    _ => {
                        let _ = app_tx.send(say(json!({ "t": "closed" }))).await;
                        break;
                    }
                };
                let event: Value = serde_json::from_str(&text).unwrap_or_default();
                let out = match event["type"].as_str().unwrap_or_default() {
                    "conversation.item.input_audio_transcription.delta" => match event["delta"].as_str() {
                        Some(d) if !d.is_empty() => json!({ "t": "delta", "text": d }),
                        _ => continue,
                    },
                    "conversation.item.input_audio_transcription.completed" => {
                        json!({ "t": "final", "text": event["transcript"].as_str().unwrap_or_default() })
                    }
                    "error" => json!({ "t": "error", "message": event["error"]["message"].as_str().unwrap_or("Live transcription failed") }),
                    _ => continue,
                };
                if app_tx.send(say(out)).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = up_tx.send(tungstenite::Message::Close(None)).await;
}
