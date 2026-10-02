//! AI summaries of whole notes, made only when someone asks (opening a note's Summary tab).
//! Kept beside the note rather than in its text, with a hash of the text they cover, so the app can
//! say when the note has changed since.

use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::Role,
    rooms::note_body,
    tree::require_note,
    util::{now_ms, parse_id},
};

/// Enough for a long meeting transcript; beyond this the start of the note is summarized.
const MAX_CHARS: usize = 60_000;

const PROMPT: &str = "Summarize the note below. Reply in Markdown only, in the note's own language, with:
- first, one or two plain sentences on what the note is about;
- then a \"## Key points\" section of short bullet points;
- then, only if the note has tasks, decisions to follow up or things someone has to do, a \"## Action items\" section of checkboxes written as \"- [ ] ...\".
Use only what the note says. Don't add a title, a preamble or anything after the sections.

The note:
";

fn hash(body: &str) -> String {
    Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The note as the model should read it: embeds become their labels, e.g. "[Voice memo]".
fn readable(body: &str) -> String {
    let lines: Vec<String> = body
        .lines()
        .map(|line| match line.trim().strip_prefix("![").and_then(|r| r.split_once("](att:")) {
            Some((label, _)) => format!("[{}]", if label.is_empty() { "Attachment" } else { label }),
            None => line.to_owned(),
        })
        .collect();
    let text = lines.join("\n");
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let cut: String = text.chars().take(MAX_CHARS).collect();
    format!("{cut}\n\n(The note goes on; only this first part is shown.)")
}

/// A model's answer without code fences around it.
fn clean(answer: &str) -> String {
    let text = answer.trim();
    match text.strip_prefix("```") {
        Some(inner) => inner.split_once('\n').map_or("", |(_, rest)| rest).trim_end().trim_end_matches("```").trim().to_owned(),
        None => text.to_owned(),
    }
}

async fn note_text(state: &AppState, id: &str) -> ApiResult<String> {
    let uuid = id.parse().map_err(|_| AppError::NotFound)?;
    Ok(note_body(state, uuid).await?)
}

/// `GET /notes/:id/summary`: the saved summary, if any, and whether the note changed since.
pub async fn get_summary(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    require_note(&state, &me.id, &id, Role::Viewer).await?;
    let row: Option<(String, String, i64)> =
        sqlx::query_as("SELECT summary, body_hash, created_at FROM note_summaries WHERE note_id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await?;
    let Some((summary, body_hash, created_at)) = row else {
        return Ok(Json(json!({ "summary": null })));
    };
    let stale = hash(&note_text(&state, &id).await?) != body_hash;
    Ok(Json(json!({ "summary": summary, "stale": stale, "created_at": created_at })))
}

/// `POST /notes/:id/summary`: summarizes the note as it is now, replacing any older summary.
/// Anyone who can read the note can ask; the note itself isn't changed.
pub async fn summarize(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    require_note(&state, &me.id, &id, Role::Viewer).await?;
    let Some(cfg) = state.summary.read().await.clone() else {
        return Err(AppError::Conflict("Summaries aren't set up on this server".into()));
    };
    let body = note_text(&state, &id).await?;
    if body.trim().lines().count() < 2 {
        return Err(AppError::BadRequest("There isn't enough in this note to summarize yet".into()));
    }
    let content = json!(format!("{PROMPT}{}", readable(&body)));
    let summary = clean(&crate::chat::complete(&state, &cfg, content, "summary").await?);
    if summary.is_empty() {
        return Err(AppError::Conflict("The summary service sent back nothing".into()));
    }
    let now = now_ms();
    sqlx::query(
        "INSERT INTO note_summaries (note_id, summary, body_hash, model, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (note_id) DO UPDATE SET summary = excluded.summary, body_hash = excluded.body_hash,
           model = excluded.model, created_by = excluded.created_by, created_at = excluded.created_at",
    )
    .bind(&id)
    .bind(&summary)
    .bind(hash(&body))
    .bind(&cfg.model)
    .bind(&me.id)
    .bind(now)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "summary": summary, "stale": false, "created_at": now })))
}

#[cfg(test)]
mod tests {
    use super::{clean, readable};

    #[test]
    fn embeds_read_as_labels_and_fences_go() {
        assert_eq!(readable("Call\n![Voice memo](att:01a0d056-667e-733b-a280-1d118b647773)\nbuy milk"), "Call\n[Voice memo]\nbuy milk");
        assert_eq!(clean("```markdown\nHi\n## Key points\n- a\n```"), "Hi\n## Key points\n- a");
    }
}
