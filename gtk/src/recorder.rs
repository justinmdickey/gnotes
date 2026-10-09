//! Voice memos: GStreamer records the microphone as Opus in WebM, like web/src/Recorder.svelte, and
//! when live transcripts are on, also streams 16 kHz mono 16-bit PCM over `/api/transcribe/live`
//! (protocol in server/src/live.rs) and passes back the words as they're heard. No GTK here.

use std::{path::PathBuf, time::Duration};

use futures_util::{SinkExt, StreamExt};
use gst::prelude::*;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

use crate::api::Api;

/// What the live transcript sends back.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveEvent {
    /// More words for the stretch of speech being heard.
    Delta(String),
    /// The finished text for the stretch so far; it replaces that stretch's deltas.
    Final(String),
    /// The service failed or went away; the recording itself carries on.
    Error(String),
}

/// Streams PCM chunks from `pcm` to the server and sends what comes back to `events`. When `pcm`
/// closes (recording stopped), asks for the last words and waits a little for them.
pub async fn live(api: Api, mut pcm: mpsc::UnboundedReceiver<Vec<u8>>, events: async_channel::Sender<LiveEvent>) {
    let url = api.ws_url().replace("/api/ws", "/api/transcribe/live");
    let mut req = match url.into_client_request() {
        Ok(r) => r,
        Err(e) => {
            let _ = events.send(LiveEvent::Error(e.to_string())).await;
            return;
        }
    };
    req.headers_mut().insert("cookie", api.cookie().parse().expect("cookie"));
    let ws = match tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(req)).await {
        Ok(Ok((ws, _))) => ws,
        _ => {
            let _ = events.send(LiveEvent::Error("Couldn't start live transcription".into())).await;
            return;
        }
    };
    let (mut sink, mut stream) = ws.split();
    let reader = {
        let events = events.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).unwrap_or_default();
                let text = v["text"].as_str().unwrap_or_default().to_owned();
                let ev = match v["t"].as_str() {
                    Some("delta") => LiveEvent::Delta(text),
                    Some("final") => LiveEvent::Final(text),
                    Some("error") => LiveEvent::Error(v["message"].as_str().unwrap_or("Live transcription stopped").to_owned()),
                    _ => continue,
                };
                if events.send(ev).await.is_err() {
                    break;
                }
            }
        })
    };
    while let Some(chunk) = pcm.recv().await {
        if sink.send(Message::binary(chunk)).await.is_err() {
            break;
        }
    }
    let _ = sink.send(Message::text(r#"{"t":"commit"}"#)).await;
    // The last stretch's final comes after the commit.
    let _ = tokio::time::timeout(Duration::from_secs(8), reader).await;
}

/// A recording in progress.
pub struct Recording {
    pipeline: gst::Pipeline,
    path: PathBuf,
}

/// The microphone, or `GNOTES_AUDIO_SRC` (a GStreamer source, e.g. a test tone for e2e).
fn source() -> String {
    std::env::var("GNOTES_AUDIO_SRC").unwrap_or_else(|_| "autoaudiosrc".into())
}

impl Recording {
    /// Starts recording into a file in `dir`. With `pcm`, also sends 16 kHz mono PCM there, in chunks.
    pub fn start(dir: &std::path::Path, pcm: Option<mpsc::UnboundedSender<Vec<u8>>>) -> Result<Recording, String> {
        gst::init().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("memo-{}.webm", uuid::Uuid::now_v7()));
        let live = if pcm.is_some() {
            " t. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=16000,channels=1 ! appsink name=pcm sync=false"
        } else {
            ""
        };
        let desc = format!(
            "{} ! audioconvert ! audioresample ! tee name=t \
             t. ! queue ! audioconvert ! opusenc ! webmmux ! filesink location=\"{}\"{live}",
            source(),
            path.display(),
        );
        let pipeline = gst::parse::launch(&desc)
            .map_err(|e| format!("Couldn't start recording: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "Couldn't start recording".to_owned())?;
        if let Some(tx) = pcm
            && let Some(sink) = pipeline.by_name("pcm").and_then(|s| s.downcast::<gst_app::AppSink>().ok())
        {
            sink.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        if let Some(buf) = sample.buffer()
                            && let Ok(map) = buf.map_readable()
                        {
                            let _ = tx.send(map.as_slice().to_vec());
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
        }
        pipeline.set_state(gst::State::Playing).map_err(|_| "Couldn't open the microphone".to_owned())?;
        Ok(Recording { pipeline, path })
    }

    /// Stops recording; the recording's bytes (WebM with Opus).
    pub async fn stop(self) -> Result<Vec<u8>, String> {
        self.pipeline.send_event(gst::event::Eos::new());
        let bus = self.pipeline.bus().expect("bus");
        let mut messages = bus.stream();
        // Awaited on the GTK thread, so glib's timeout rather than tokio's.
        let finished = gst::glib::future_with_timeout(Duration::from_secs(5), async {
            while let Some(msg) = messages.next().await {
                match msg.view() {
                    gst::MessageView::Eos(_) => return Ok(()),
                    gst::MessageView::Error(e) => return Err(e.error().to_string()),
                    _ => {}
                }
            }
            Ok(())
        })
        .await;
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Ok(Err(e)) = finished {
            return Err(format!("The recording failed: {e}"));
        }
        let bytes = std::fs::read(&self.path).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&self.path);
        bytes
    }

    /// Throws the recording away.
    pub fn cancel(self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = std::fs::remove_file(&self.path);
    }
}
