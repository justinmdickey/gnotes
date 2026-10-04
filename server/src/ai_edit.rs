//! Edit with AI and the AI Prompt block: the summary chat model changes a note as asked, or writes
//! new text into it. Like Tidy Up, nothing is saved here; the app shows or inserts the result and
//! writes it through the note's Loro doc. Unlike Tidy Up, an edit may remove things when asked.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState, ChatConfig,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::Role,
    rooms::note_body,
    tidy::{MAX_CHARS, reply_note},
    tree::require_note,
    util::parse_id,
};

/// The longest instruction or prompt, in characters.
pub const MAX_PROMPT: usize = 1000;

/// Where the written text goes, in the note sent with a `/write` prompt.
pub const MARKER: &str = "<<WRITE HERE>>";

const EDIT_PROMPT: &str = "Change the Markdown note below as the instruction says, and reply with the whole changed note.

Change only what the instruction asks for; keep everything else as it is, word for word. Keep attachment lines like ![Photo](att:…), links and code exactly as they are unless the instruction is about them.

Reply with only the note, starting with its first line: no explanation, and no ``` fence around the whole note.

The instruction: ";

const WRITE_PROMPT: &str = "Write Markdown for the note below, to go where it says <<WRITE HERE>>, following the request. Use the rest of the note as context: match its language, style and formatting, and don't repeat what it already says.

Reply with only the new Markdown to insert there: no explanation, not the rest of the note, no <<WRITE HERE>>, and no ``` fence around the whole reply.

The request: ";

#[derive(Deserialize)]
pub struct EditBody {
    instruction: String,
    /// The text to edit, as the app shows it; by default the note's text on the server.
    text: Option<String>,
}

#[derive(Deserialize)]
pub struct WriteBody {
    prompt: String,
    /// The note's text as the app shows it.
    text: String,
    /// Where the new text goes in `text`, in UTF-16 code units (an editor position).
    at: usize,
}

/// The chat model, after checking that `me` can edit the note.
async fn editor_chat(state: &AppState, me: &str, id: &str) -> Result<ChatConfig, AppError> {
    require_note(state, me, id, Role::Editor).await?;
    state.summary.read().await.clone().ok_or_else(|| AppError::Conflict("AI Summaries aren't set up on this server".into()))
}

/// A prompt the user typed: trimmed, not empty and not too long.
fn prompt_text(text: &str, what: &str) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest(format!("Say what to {what}")));
    }
    if text.chars().count() > MAX_PROMPT {
        return Err(AppError::BadRequest(format!("That's too long (over {MAX_PROMPT} characters)")));
    }
    Ok(text.to_owned())
}

/// `POST /notes/:id/edit`: the note's text changed by the chat model as the instruction says, for
/// the app to show and apply. Editors only.
pub async fn edit(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<EditBody>,
) -> ApiResult<Json<Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    let cfg = editor_chat(&state, &me.id, &id).await?;
    let instruction = prompt_text(&body.instruction, "change")?;
    let original = match body.text {
        Some(text) => text,
        None => note_body(&state, id.parse().map_err(|_| AppError::NotFound)?).await?,
    };
    if original.trim().is_empty() {
        return Err(AppError::BadRequest("There's nothing in this note to edit yet".into()));
    }
    if original.chars().count() > MAX_CHARS {
        return Err(AppError::BadRequest(format!("This note is too long to edit (over {} characters)", MAX_CHARS)));
    }
    let content = format!("{EDIT_PROMPT}{instruction}\n\nThe note:\n\n{original}");
    let reply = crate::chat::complete(&state, &cfg, json!(content), "AI").await?;
    let text = reply_note(&reply, &original);
    if text.trim().is_empty() {
        return Err(AppError::Conflict("The AI service sent back nothing".into()));
    }
    Ok(Json(json!({ "original": original, "text": text })))
}

/// `POST /notes/:id/write`: Markdown written by the chat model for one spot in the note, with the
/// rest of the note as context, for the app to insert there. Editors only.
pub async fn write(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<WriteBody>,
) -> ApiResult<Json<Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    let cfg = editor_chat(&state, &me.id, &id).await?;
    let prompt = prompt_text(&body.prompt, "write")?;
    if body.text.chars().count() > MAX_CHARS {
        return Err(AppError::BadRequest(format!("This note is too long to write in (over {} characters)", MAX_CHARS)));
    }
    let at = byte_index(&body.text, body.at);
    let note = format!("{}{MARKER}{}", &body.text[..at], &body.text[at..]);
    let content = format!("{WRITE_PROMPT}{prompt}\n\nThe note:\n\n{note}");
    let reply = crate::chat::complete(&state, &cfg, json!(content), "AI").await?;
    let text = reply_text(&reply);
    if text.trim().is_empty() {
        return Err(AppError::Conflict("The AI service sent back nothing".into()));
    }
    Ok(Json(json!({ "text": text })))
}

/// The byte index in `text` of a position in UTF-16 code units, clamped to the end and moved back
/// out of the middle of a character.
fn byte_index(text: &str, at: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= at {
            return i;
        }
        units += c.len_utf16();
        if units > at {
            return i;
        }
    }
    text.len()
}

/// The text in a model's reply to `/write`: without a leading `<think>…</think>`, the marker, or
/// a ```markdown fence around all of it. A fence around code it was asked to write stays.
pub fn reply_text(reply: &str) -> String {
    let mut text = reply.trim_start();
    if let Some(rest) = text.strip_prefix("<think>") {
        text = rest.split_once("</think>").map_or("", |(_, after)| after);
    }
    let text = text.replace(MARKER, "");
    let mut text = text.trim_matches('\n').trim_end();
    let first = text.lines().next().unwrap_or("");
    if matches!(first.trim(), "```" | "```markdown" | "```md") && text.len() > first.len() && text.ends_with("```") {
        let inner = &text[first.len()..];
        text = inner.strip_suffix("```").unwrap_or(inner).trim_matches('\n').trim_end();
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{byte_index, reply_text};

    #[test]
    fn positions_are_utf16_units() {
        assert_eq!(byte_index("abc", 1), 1);
        assert_eq!(byte_index("abc", 9), 3);
        // 🎉 is two UTF-16 units and four bytes; é is one unit and two bytes.
        assert_eq!(byte_index("🎉x", 2), 4);
        assert_eq!(byte_index("🎉x", 1), 0);
        assert_eq!(byte_index("éx", 1), 2);
    }

    #[test]
    fn written_text_loses_thinking_and_markdown_fences() {
        assert_eq!(reply_text("<think>hmm</think>\n\n- a\n- b\n"), "- a\n- b");
        assert_eq!(reply_text("```markdown\n## A\nb\n```"), "## A\nb");
        assert_eq!(reply_text("<<WRITE HERE>>Hello"), "Hello");
        // Code it was asked for keeps its fence.
        assert_eq!(reply_text("```python\nprint(1)\n```"), "```python\nprint(1)\n```");
    }
}
