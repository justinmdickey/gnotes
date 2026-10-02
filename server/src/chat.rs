//! Calls to an OpenAI-compatible chat API (`/chat/completions`): reading photos, summarizing notes.

use serde_json::{Value, json};

use crate::{AppState, ChatConfig, error::AppError};

/// Sends one user message (text, or text and an image) and returns the model's reply.
/// `what` names the service in errors, e.g. "photo reading".
pub async fn complete(state: &AppState, cfg: &ChatConfig, content: Value, what: &str) -> Result<String, AppError> {
    let body = json!({
        "model": cfg.model,
        "temperature": 0,
        "messages": [{ "role": "user", "content": content }],
    });
    let mut req = state.http.post(format!("{}/chat/completions", cfg.url)).json(&body);
    if let Some(key) = &cfg.key {
        req = req.bearer_auth(key);
    }
    let res = req.send().await.map_err(|e| {
        tracing::warn!("{what} request failed: {e:#}");
        AppError::Conflict(format!("Couldn't reach the {what} service"))
    })?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        tracing::warn!("{what} failed with {status}: {body}");
        return Err(AppError::Conflict(format!("The {what} service returned an error")));
    }
    let answer: Value = res.json().await?;
    Ok(answer["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_owned())
}
