//! Server-wide settings admins change from the app. Stored values override the env defaults.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState, WhisperConfig,
    account::require_admin,
    auth::CurrentUser,
    error::{ApiResult, AppError},
};

const WHISPER_KEY: &str = "whisper";

/// Loads the saved speech-to-text setting. `None` means nothing is saved, so the env default applies.
pub async fn load_whisper(db: &sqlx::SqlitePool) -> anyhow::Result<Option<Option<WhisperConfig>>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(WHISPER_KEY)
        .fetch_optional(db)
        .await?;
    Ok(match raw {
        None => None,
        // Saved as null when an admin turned it off.
        Some(raw) => Some(serde_json::from_str(&raw)?),
    })
}

fn describe(w: Option<&WhisperConfig>, saved: bool) -> Value {
    json!({
        "enabled": w.is_some(),
        "url": w.map(|w| w.url.as_str()).unwrap_or(""),
        "model": w.map(|w| w.model.as_str()).unwrap_or("whisper-1"),
        // The key is never sent back; the app only learns whether one is set.
        "has_key": w.is_some_and(|w| w.key.is_some()),
        "realtime_url": w.and_then(|w| w.realtime_url.as_deref()).unwrap_or(""),
        "from_env": !saved && w.is_some(),
    })
}

/// `GET /admin/settings`
pub async fn get_settings(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let saved = load_whisper(&state.db).await?.is_some();
    let whisper = state.whisper.read().await;
    Ok(Json(json!({ "whisper": describe(whisper.as_ref(), saved) })))
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
}

impl WhisperBody {
    /// The config this body describes, filling in the current key when it wasn't sent.
    fn resolve(self, current: Option<&WhisperConfig>) -> ApiResult<Option<WhisperConfig>> {
        let url = self.url.trim().trim_end_matches('/').to_owned();
        if url.is_empty() {
            return Ok(None);
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(AppError::BadRequest("The URL needs to start with http:// or https://".into()));
        }
        let model = match self.model.trim() {
            "" => "whisper-1".to_owned(),
            m => m.to_owned(),
        };
        let key = match self.key {
            None => current.and_then(|c| c.key.clone()),
            Some(k) => k.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()),
        };
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
    let next = body.resolve(whisper.as_ref())?;
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(WHISPER_KEY)
        .bind(serde_json::to_string(&next)?)
        .execute(&state.db)
        .await?;
    *whisper = next;
    Ok(Json(json!({ "whisper": describe(whisper.as_ref(), true) })))
}

/// `POST /admin/settings/whisper/test`: checks the service answers, before saving it.
pub async fn test_whisper(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<WhisperBody>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    let current = state.whisper.read().await.clone();
    let Some(w) = body.resolve(current.as_ref())? else {
        return Err(AppError::BadRequest("Enter the service URL first".into()));
    };
    let mut req = state.http.get(format!("{}/models", w.url)).timeout(std::time::Duration::from_secs(10));
    if let Some(key) = &w.key {
        req = req.bearer_auth(key);
    }
    let (ok, message) = match req.send().await {
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
            if models.is_empty() || models.iter().any(|m| m == &w.model) {
                (true, "Connected".to_owned())
            } else {
                (true, format!("Connected, but it doesn't list “{}”. It has: {}", w.model, models.join(", ")))
            }
        }
        Ok(res) => (false, format!("The service answered with {}", res.status())),
    };
    let (ok, message) = match (&w.realtime_url, ok) {
        (Some(_), true) => match crate::live::check(&w).await {
            Ok(()) => (true, format!("{message} · live transcription works")),
            Err(why) => (false, format!("{message}, but live transcription didn't: {why}")),
        },
        _ => (ok, message),
    };
    Ok(Json(json!({ "ok": ok, "message": message })))
}
