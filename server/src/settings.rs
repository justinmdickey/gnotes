//! Server-wide settings admins change from the app. Stored values override the env defaults.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState, ChatConfig, WhisperConfig,
    account::require_admin,
    auth::CurrentUser,
    error::{ApiResult, AppError},
};

const WHISPER_KEY: &str = "whisper";

/// A service as saved. A switched-off service keeps its URL, model and key so it can come back on.
#[derive(Serialize, Deserialize)]
struct Stored<T> {
    #[serde(flatten)]
    config: T,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    off: bool,
}

/// Loads a saved setting as stored. `None` means nothing is saved, so the env default applies;
/// `Some(None)` means it was cleared.
async fn load_stored<T: serde::de::DeserializeOwned>(db: &sqlx::SqlitePool, key: &str) -> anyhow::Result<Option<Option<Stored<T>>>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await?;
    Ok(match raw {
        None => None,
        Some(raw) => Some(serde_json::from_str(&raw)?),
    })
}

/// Loads a saved setting as it applies: `Some(None)` when it's cleared or switched off.
async fn load<T: serde::de::DeserializeOwned>(db: &sqlx::SqlitePool, key: &str) -> anyhow::Result<Option<Option<T>>> {
    Ok(load_stored::<T>(db, key).await?.map(|s| s.and_then(|s| (!s.off).then_some(s.config))))
}

/// The settings a switched-off service had, for showing and for turning it back on.
async fn load_off<T: serde::de::DeserializeOwned>(db: &sqlx::SqlitePool, key: &str) -> anyhow::Result<Option<T>> {
    Ok(load_stored::<T>(db, key).await?.flatten().and_then(|s| s.off.then_some(s.config)))
}

async fn save<T: serde::Serialize>(db: &sqlx::SqlitePool, key: &str, value: &Option<Stored<T>>) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(serde_json::to_string(value)?)
        .execute(db)
        .await?;
    Ok(())
}

/// Loads the saved speech-to-text setting.
pub async fn load_whisper(db: &sqlx::SqlitePool) -> anyhow::Result<Option<Option<WhisperConfig>>> {
    load(db, WHISPER_KEY).await
}

/// The OpenAI-compatible services set by a URL, model and key, each its own setting.
#[derive(Clone, Copy)]
pub enum Chat {
    /// Reads the text in photos.
    Vision,
    /// Summarizes notes, and writes Ask's answers.
    Summary,
    /// Embeddings for semantic search (`/embeddings`, not chat).
    Embed,
}

impl Chat {
    fn from_path(name: &str) -> ApiResult<Self> {
        match name {
            "vision" => Ok(Chat::Vision),
            "summary" => Ok(Chat::Summary),
            "embed" => Ok(Chat::Embed),
            _ => Err(AppError::NotFound),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Chat::Vision => "vision",
            Chat::Summary => "summary",
            Chat::Embed => "embed",
        }
    }

    fn slot(self, state: &AppState) -> &tokio::sync::RwLock<Option<ChatConfig>> {
        match self {
            Chat::Vision => &state.vision,
            Chat::Summary => &state.summary,
            Chat::Embed => &state.embed,
        }
    }

    fn example_model(self) -> &'static str {
        match self {
            Chat::Vision => "qwen2.5vl",
            Chat::Summary => "llama3.1",
            Chat::Embed => "nomic-embed-text",
        }
    }
}

/// Loads a saved chat-service setting.
pub async fn load_chat(db: &sqlx::SqlitePool, chat: Chat) -> anyhow::Result<Option<Option<ChatConfig>>> {
    load(db, chat.key()).await
}

/// A service URL as typed: trimmed, without a trailing slash, and http(s). Empty means off.
fn clean_url(url: &str) -> ApiResult<Option<String>> {
    let url = url.trim().trim_end_matches('/').to_owned();
    if url.is_empty() {
        return Ok(None);
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::BadRequest("The URL needs to start with http:// or https://".into()));
    }
    Ok(Some(url))
}

/// The key to store: the current one when none was sent, none when it was cleared.
fn resolve_key(sent: Option<Option<String>>, current: Option<&String>) -> Option<String> {
    match sent {
        None => current.cloned(),
        Some(k) => k.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()),
    }
}

/// Asks an OpenAI-compatible service for `/models`, for Settings › Test.
async fn check_models(state: &AppState, url: &str, key: Option<&String>, model: &str) -> (bool, String) {
    let mut req = state.http.get(format!("{url}/models")).timeout(std::time::Duration::from_secs(10));
    if let Some(key) = key {
        req = req.bearer_auth(key);
    }
    match req.send().await {
        Err(e) if e.is_timeout() => (false, "The service didn't answer within 10 seconds".to_owned()),
        Err(_) => (false, "Couldn't connect. Check the URL and that the server can reach it.".to_owned()),
        Ok(res) if res.status() == 401 || res.status() == 403 => (false, "The service refused the API key".to_owned()),
        Ok(res) if res.status().is_success() => {
            let models: Vec<String> = res
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v["data"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                .collect();
            if models.is_empty() || models.iter().any(|m| m == model) {
                (true, "Connected".to_owned())
            } else {
                (true, format!("Connected, but it doesn't list “{model}”. It has: {}", models.join(", ")))
            }
        }
        Ok(res) => (false, format!("The service answered with {}", res.status())),
    }
}

/// `on` is the running service; `off` what a switched-off one had, shown so it can come back on.
fn describe_chat(on: Option<&ChatConfig>, off: Option<&ChatConfig>, saved: bool) -> Value {
    let v = on.or(off);
    json!({
        "enabled": on.is_some(),
        "url": v.map(|v| v.url.as_str()).unwrap_or(""),
        "model": v.map(|v| v.model.as_str()).unwrap_or(""),
        "has_key": v.is_some_and(|v| v.key.is_some()),
        "from_env": !saved && on.is_some(),
    })
}

fn describe(on: Option<&WhisperConfig>, off: Option<&WhisperConfig>, saved: bool) -> Value {
    let w = on.or(off);
    json!({
        "enabled": on.is_some(),
        "url": w.map(|w| w.url.as_str()).unwrap_or(""),
        "model": w.map(|w| w.model.as_str()).unwrap_or("whisper-1"),
        // The key is never sent back; the app only learns whether one is set.
        "has_key": w.is_some_and(|w| w.key.is_some()),
        "realtime_url": w.and_then(|w| w.realtime_url.as_deref()).unwrap_or(""),
        "from_env": !saved && on.is_some(),
    })
}

/// `GET /admin/settings`
pub async fn get_settings(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let saved = load_whisper(&state.db).await?.is_some();
    let off = load_off::<WhisperConfig>(&state.db, WHISPER_KEY).await?;
    let mut out = json!({ "whisper": describe(state.whisper.read().await.as_ref(), off.as_ref(), saved) });
    for chat in [Chat::Vision, Chat::Summary, Chat::Embed] {
        let saved = load_chat(&state.db, chat).await?.is_some();
        let off = load_off::<ChatConfig>(&state.db, chat.key()).await?;
        out[chat.key()] = describe_chat(chat.slot(&state).read().await.as_ref(), off.as_ref(), saved);
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct WhisperBody {
    /// Empty turns transcription off.
    url: String,
    #[serde(default)]
    model: String,
    /// Absent keeps the saved key, null or "" clears it.
    #[serde(default, deserialize_with = "crate::util::double_option")]
    key: Option<Option<String>>,
    /// Empty turns live transcription off.
    #[serde(default)]
    realtime_url: String,
    /// False switches it off but keeps what's filled in.
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

impl WhisperBody {
    /// The config this body describes, filling in the current key when it wasn't sent.
    fn resolve(self, current: Option<&WhisperConfig>) -> ApiResult<Option<WhisperConfig>> {
        let Some(url) = clean_url(&self.url)? else { return Ok(None) };
        let model = match self.model.trim() {
            "" => "whisper-1".to_owned(),
            m => m.to_owned(),
        };
        let key = resolve_key(self.key, current.and_then(|c| c.key.as_ref()));
        let realtime_url = match self.realtime_url.trim().trim_end_matches('/') {
            "" => None,
            u if u.starts_with("ws://") || u.starts_with("wss://") => Some(u.to_owned()),
            _ => return Err(AppError::BadRequest("The live URL needs to start with ws:// or wss://".into())),
        };
        Ok(Some(WhisperConfig { url, model, key, realtime_url }))
    }
}

/// `PUT /admin/settings/whisper`
pub async fn put_whisper(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<WhisperBody>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let mut whisper = state.whisper.write().await;
    let off = load_off::<WhisperConfig>(&state.db, WHISPER_KEY).await?;
    let enabled = body.enabled;
    let next = body.resolve(whisper.as_ref().or(off.as_ref()))?;
    let stored = next.map(|config| Stored { config, off: !enabled });
    save(&state.db, WHISPER_KEY, &stored).await?;
    let (on, off) = match stored {
        Some(Stored { config, off: false }) => (Some(config), None),
        Some(Stored { config, off: true }) => (None, Some(config)),
        None => (None, None),
    };
    *whisper = on;
    Ok(Json(json!({ "whisper": describe(whisper.as_ref(), off.as_ref(), true) })))
}

/// `POST /admin/settings/whisper/test`: checks the service answers, before saving it.
pub async fn test_whisper(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<WhisperBody>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let current = match state.whisper.read().await.clone() {
        Some(w) => Some(w),
        None => load_off(&state.db, WHISPER_KEY).await?,
    };
    let Some(w) = body.resolve(current.as_ref())? else {
        return Err(AppError::BadRequest("Enter the service URL first".into()));
    };
    let (ok, message) = check_models(&state, &w.url, w.key.as_ref(), &w.model).await;
    let (ok, message) = match (&w.realtime_url, ok) {
        (Some(_), true) => match crate::live::check(&w).await {
            Ok(()) => (true, format!("{message} · live transcription works")),
            Err(why) => (false, format!("{message}, but live transcription didn't: {why}")),
        },
        _ => (ok, message),
    };
    Ok(Json(json!({ "ok": ok, "message": message })))
}

#[derive(Deserialize)]
pub struct ChatBody {
    /// Empty turns the service off.
    url: String,
    #[serde(default)]
    model: String,
    /// Absent keeps the saved key, null or "" clears it.
    #[serde(default, deserialize_with = "crate::util::double_option")]
    key: Option<Option<String>>,
    /// False switches it off but keeps what's filled in.
    #[serde(default = "yes")]
    enabled: bool,
}

impl ChatBody {
    fn resolve(self, chat: Chat, current: Option<&ChatConfig>) -> ApiResult<Option<ChatConfig>> {
        let Some(url) = clean_url(&self.url)? else { return Ok(None) };
        // Local servers have no sensible default model, so it has to be named.
        let model = self.model.trim().to_owned();
        if model.is_empty() {
            return Err(AppError::BadRequest(format!("Enter the model's name, e.g. {}", chat.example_model())));
        }
        let key = resolve_key(self.key, current.and_then(|c| c.key.as_ref()));
        Ok(Some(ChatConfig { url, model, key }))
    }
}

/// `PUT /admin/settings/{vision|summary|embed}`
pub async fn put_chat(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<ChatBody>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let chat = Chat::from_path(&name)?;
    let mut slot = chat.slot(&state).write().await;
    let off = load_off::<ChatConfig>(&state.db, chat.key()).await?;
    let enabled = body.enabled;
    let next = body.resolve(chat, slot.as_ref().or(off.as_ref()))?;
    let stored = next.map(|config| Stored { config, off: !enabled });
    save(&state.db, chat.key(), &stored).await?;
    let (on, off) = match stored {
        Some(Stored { config, off: false }) => (Some(config), None),
        Some(Stored { config, off: true }) => (None, Some(config)),
        None => (None, None),
    };
    *slot = on;
    if let Chat::Embed = chat {
        // A new service or model: embed whatever it hasn't seen yet.
        state.embedder.refresh_all();
    }
    Ok(Json(json!({ chat.key(): describe_chat(slot.as_ref(), off.as_ref(), true) })))
}

/// `POST /admin/settings/{vision|summary|embed}/test`
pub async fn test_chat(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<ChatBody>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let chat = Chat::from_path(&name)?;
    let current = match chat.slot(&state).read().await.clone() {
        Some(c) => Some(c),
        None => load_off(&state.db, chat.key()).await?,
    };
    let Some(c) = body.resolve(chat, current.as_ref())? else {
        return Err(AppError::BadRequest("Enter the service URL first".into()));
    };
    let (ok, message) = check_models(&state, &c.url, c.key.as_ref(), &c.model).await;
    Ok(Json(json!({ "ok": ok, "message": message })))
}

/// `POST /admin/semantic/reindex`: throws away every note's embeddings and makes them again, for when
/// the model behind the same name changed. Meaning search is thin until the background run catches up.
pub async fn reindex_embed(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    if state.embed.read().await.is_none() {
        return Err(AppError::Conflict("Semantic search isn't set up on this server".into()));
    }
    sqlx::query("DELETE FROM note_chunks").execute(&state.db).await?;
    state.embedder.refresh_all();
    let notes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL").fetch_one(&state.db).await?;
    Ok(Json(json!({ "notes": notes })))
}
