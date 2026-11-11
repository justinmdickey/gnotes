//! Calls to an OpenAI-compatible chat API (`/chat/completions`): reading photos, summarizing notes,
//! and Ask's conversation.

use std::collections::BTreeMap;

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
    let res = send(state, cfg, &body, what, false).await?;
    let answer: Value = res.json().await?;
    Ok(reply_text(&answer))
}

/// Sends a conversation (`[{role, content}]`) with `stream: true` and calls `each` with every piece
/// of the reply as it arrives, or with "" for an event without any (a reasoning model thinking);
/// `each` returns false to stop early. Returns the whole reply.
/// A service that ignores `stream` and answers with one JSON body gives a single piece.
/// Only the reply's `content` counts; a reasoning model's separate `reasoning` is dropped.
pub async fn stream(
    state: &AppState,
    cfg: &ChatConfig,
    messages: Value,
    what: &str,
    mut each: impl FnMut(&str) -> bool,
) -> Result<String, AppError> {
    let body = json!({ "model": cfg.model, "temperature": 0, "stream": true, "messages": messages });
    let mut res = send(state, cfg, &body, what, false).await?;
    let sse = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/event-stream"));
    if !sse {
        let text = reply_text(&res.json().await?);
        each(&text);
        return Ok(text);
    }
    let lost = |e: reqwest::Error| {
        tracing::warn!("{what} stream broke off: {e:#}");
        AppError::Conflict(format!("Lost the connection to the {what} service"))
    };
    let mut full = String::new();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(lost)? {
        buf.extend_from_slice(&chunk);
        // Server-sent events, one `data: {…}` line each; a piece can end mid-line.
        while let Some(end) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                return Ok(full);
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
            let piece = event["choices"][0]["delta"]["content"].as_str().unwrap_or_default();
            full.push_str(piece);
            if !each(piece) {
                return Ok(full);
            }
        }
    }
    Ok(full)
}

/// `stream` with OpenAI-style `tools` offered. `on_text` gets the pieces of the reply's
/// `content` as they arrive. Returns the whole `content` and the `tool_calls` the reply asks
/// for, if any; with `None` tools the reply can't call tools. A service that rejects the tools
/// (400) is `Err(ToolsRejected)`; the caller falls back to an answer-only turn.
pub async fn stream_tools(
    state: &AppState,
    cfg: &ChatConfig,
    messages: Value,
    tools: Option<Value>,
    what: &str,
    mut each: impl FnMut(&str) -> bool,
) -> Result<(String, Vec<Value>), AppError> {
    let mut body = json!({ "model": cfg.model, "temperature": 0, "stream": true, "messages": messages });
    if let Some(tools) = &tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    let mut res = send(state, cfg, &body, what, tools.is_some()).await?;
    let sse = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/event-stream"));
    let lost = |e: reqwest::Error| {
        tracing::warn!("{what} stream broke off: {e:#}");
        AppError::Conflict(format!("Lost the connection to the {what} service"))
    };
    if !sse {
        let answer: Value = res.json().await.map_err(lost)?;
        let text = reply_text(&answer);
        let calls = answer["choices"][0]["message"]["tool_calls"].as_array().cloned().unwrap_or_default();
        each(&text);
        return Ok((text, calls));
    }
    let mut full = String::new();
    // A tool call's pieces arrive spread over the stream: its id and name once, its arguments
    // in fragments. Kept by the call's index, in order.
    let mut calls: BTreeMap<usize, (Option<String>, Option<String>, String)> = BTreeMap::new();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(lost)? {
        buf.extend_from_slice(&chunk);
        // Server-sent events, one `data: {…}` line each; a piece can end mid-line.
        while let Some(end) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                return Ok((full, tool_calls(&calls)));
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
            let delta = &event["choices"][0]["delta"];
            if let Some(piece) = delta["content"].as_str() {
                full.push_str(piece);
                if !each(piece) {
                    return Ok((full, tool_calls(&calls)));
                }
            }
            if let Some(list) = delta["tool_calls"].as_array() {
                for tc in list {
                    let i = tc["index"].as_u64().unwrap_or(u64::MAX) as usize;
                    let slot = calls.entry(i).or_insert((None, None, String::new()));
                    if let Some(id) = tc["id"].as_str() {
                        slot.0 = Some(id.to_owned());
                    }
                    if let Some(name) = tc["function"]["name"].as_str() {
                        slot.1 = Some(name.to_owned());
                    }
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        slot.2.push_str(args);
                    }
                }
            }
        }
    }
    Ok((full, tool_calls(&calls)))
}

/// A streamed tool call's accumulated pieces as the reply's `tool_calls` entry.
fn tool_calls(calls: &BTreeMap<usize, (Option<String>, Option<String>, String)>) -> Vec<Value> {
    calls
        .values()
        .map(|(id, name, args)| {
            json!({ "id": id, "type": "function", "function": { "name": name, "arguments": args } })
        })
        .collect()
}

async fn send(state: &AppState, cfg: &ChatConfig, body: &Value, what: &str, tools: bool) -> Result<reqwest::Response, AppError> {
    let mut req = state.http.post(format!("{}/chat/completions", cfg.url)).json(body);
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
        if tools && status.as_u16() == 400 {
            return Err(AppError::ToolsRejected);
        }
        return Err(AppError::Conflict(format!("The {what} service returned an error")));
    }
    Ok(res)
}

fn reply_text(answer: &Value) -> String {
    answer["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_owned()
}
