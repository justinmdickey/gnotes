//! The agent API, /api/v1: what an agent or script needs to find, read and write notes as a user,
//! with an API key (api_keys.rs). Its shapes are a contract, documented in docs/API.md and kept
//! apart from the app's own routes so those can change freely. Add to it; don't change what's here.
//!
//! Every handler takes a `KeyUser`, which also stops read-only keys from writing. Access is the
//! key's user's, checked the same way as in the app (tree.rs, perms.rs).

use axum::{
    Json, Router,
    extract::{FromRequest, FromRequestParts, Path, Request, State},
    http::request::Parts,
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    AppState,
    api_keys::{KeyUser, Scope},
    error::{ApiResult, AppError},
    perms::{Role, note_role},
    rooms::{read_text, write_text},
    search::{SearchResult, by_words},
    semantic::by_meaning,
    tree::{add_note, add_notebook, load_tree, move_note, trash_note},
    util::{double_option, new_id, parse_id},
};

/// The most text one note may hold through the API, in bytes.
const MAX_TEXT: usize = 1_000_000;
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;
const MAX_SEARCH: usize = 50;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(me))
        .route("/notebooks", get(list_notebooks).post(create_notebook))
        .route("/notes", get(list_notes).post(create_note))
        .route("/notes/{id}", get(get_note).patch(update_note).delete(delete_note))
        .route("/notes/{id}/append", post(append))
        .route("/search", get(search))
        .fallback(|| async { AppError::BadRequest("No such endpoint; see docs/API.md".into()) })
}

// Request bodies and queries that don't parse answer 400 with what was wrong, not axum's plain text.

pub struct Body<T>(T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Body<T> {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, AppError> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(Body(v)),
            Err(e) => Err(AppError::BadRequest(e.body_text())),
        }
    }
}

pub struct Query<T>(T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for Query<T> {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, AppError> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(v)) => Ok(Query(v)),
            Err(e) => Err(AppError::BadRequest(e.body_text())),
        }
    }
}

/// Milliseconds since 1970 as an RFC 3339 time in UTC, e.g. `2026-10-06T14:03:51.123Z`.
fn timestamp(ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default()
}

fn note_id(id: &str) -> ApiResult<Uuid> {
    id.parse().map_err(|_| AppError::NotFound)
}

fn check_text(text: &str) -> ApiResult<()> {
    if text.len() > MAX_TEXT {
        return Err(AppError::BadRequest(format!("Notes hold at most {MAX_TEXT} bytes of text")));
    }
    Ok(())
}

#[derive(Serialize)]
struct Me {
    user: UserInfo,
    key: KeyInfo,
}

#[derive(Serialize)]
struct UserInfo {
    id: String,
    username: String,
    display_name: String,
}

#[derive(Serialize)]
struct KeyInfo {
    name: String,
    scope: Scope,
}

/// `GET /v1/me`: who the key acts as, and what it may do.
async fn me(key: KeyUser) -> Json<Me> {
    Json(Me {
        user: UserInfo { id: key.user.id, username: key.user.username, display_name: key.user.display_name },
        key: KeyInfo { name: key.key_name, scope: key.scope },
    })
}

#[derive(Serialize)]
struct Notebook {
    id: String,
    /// `null` at the top level, or when the parent isn't visible to you.
    parent_id: Option<String>,
    name: String,
    /// The owner's display name.
    owner: String,
    /// `owner`, `editor` or `viewer`.
    role: Role,
    updated_at: String,
}

/// `GET /v1/notebooks`: every notebook you can see, yours and shared with you, A–Z.
async fn list_notebooks(State(state): State<AppState>, key: KeyUser) -> ApiResult<Json<Vec<Notebook>>> {
    let tree = load_tree(&state, &key.user).await?;
    let ids: std::collections::HashSet<&str> = tree.notebooks.iter().map(|nb| nb.id.as_str()).collect();
    let notebooks = tree
        .notebooks
        .iter()
        .map(|nb| Notebook {
            id: nb.id.clone(),
            parent_id: nb.parent_id.clone().filter(|p| ids.contains(p.as_str())),
            name: nb.name.clone(),
            owner: nb.owner.clone(),
            role: nb.role,
            updated_at: timestamp(nb.updated_at),
        })
        .collect();
    Ok(Json(notebooks))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewNotebook {
    name: String,
    #[serde(default)]
    parent_id: Option<String>,
}

/// `POST /v1/notebooks`: makes a notebook, at the top level or inside `parent_id`.
async fn create_notebook(State(state): State<AppState>, key: KeyUser, Body(body): Body<NewNotebook>) -> ApiResult<Json<Notebook>> {
    let id = add_notebook(&state, &key.user.id, &body.name, body.parent_id.as_deref()).await?;
    let tree = load_tree(&state, &key.user).await?;
    let nb = tree.notebooks.iter().find(|nb| nb.id == id).ok_or(AppError::NotFound)?;
    Ok(Json(Notebook {
        id,
        parent_id: nb.parent_id.clone(),
        name: nb.name.clone(),
        owner: nb.owner.clone(),
        role: nb.role,
        updated_at: timestamp(nb.updated_at),
    }))
}

/// A note without its text, as lists show it.
#[derive(Serialize)]
struct NoteSummary {
    id: String,
    /// `null` for a note at the top level, or when its notebook isn't visible to you.
    notebook_id: Option<String>,
    /// The first line of the text, without Markdown marks.
    title: String,
    /// The line after the title.
    preview: String,
    owner: String,
    role: Role,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotesQuery {
    /// Only the notes directly in this notebook.
    notebook_id: Option<String>,
    limit: Option<usize>,
}

/// `GET /v1/notes`: notes you can see, most recently changed first.
async fn list_notes(State(state): State<AppState>, key: KeyUser, Query(q): Query<NotesQuery>) -> ApiResult<Json<Vec<NoteSummary>>> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let tree = load_tree(&state, &key.user).await?;
    let notes = tree
        .notes
        .iter()
        .filter(|n| q.notebook_id.is_none() || n.notebook_id == q.notebook_id)
        .take(limit)
        .map(|n| NoteSummary {
            id: n.id.clone(),
            notebook_id: n.notebook_id.clone().filter(|nb| tree.notebooks.iter().any(|x| &x.id == nb)),
            title: n.title.clone(),
            preview: n.preview.clone(),
            owner: n.owner.clone(),
            role: n.role,
            updated_at: timestamp(n.updated_at),
        })
        .collect();
    Ok(Json(notes))
}

/// A note with its Markdown text.
#[derive(Serialize)]
struct Note {
    id: String,
    notebook_id: Option<String>,
    title: String,
    owner: String,
    role: Role,
    updated_at: String,
    /// The text as of this read. Send it back with a change to merge with edits made since.
    version: String,
    text: String,
}

/// The note as `user` sees it now; not found when they can't see it.
async fn load_note(state: &AppState, user: &str, id: &str) -> ApiResult<Note> {
    let role = note_role(&state.db, user, id).await?.ok_or(AppError::NotFound)?;
    let (notebook_id, title, owner, updated_at): (Option<String>, String, String, i64) = sqlx::query_as(
        "SELECT n.notebook_id, n.title, u.display_name, n.updated_at FROM notes n JOIN users u ON u.id = n.owner_id WHERE n.id = ?",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    // A notebook the user can't see stays hidden, as in the lists.
    let notebook_id = match notebook_id {
        Some(nb) if crate::perms::notebook_role(&state.db, user, &nb).await?.is_some() => Some(nb),
        _ => None,
    };
    let text = read_text(state, note_id(id)?).await?;
    Ok(Note { id: id.to_owned(), notebook_id, title, owner, role, updated_at: timestamp(updated_at), version: text.version, text: text.text })
}

async fn require_editor(state: &AppState, user: &str, id: &str) -> ApiResult<()> {
    match note_role(&state.db, user, id).await? {
        None => Err(AppError::NotFound),
        Some(r) if r < Role::Editor => Err(AppError::NotAllowed("You can only view this note".into())),
        Some(_) => Ok(()),
    }
}

/// `GET /v1/notes/:id`: a note and its text.
async fn get_note(State(state): State<AppState>, key: KeyUser, Path(id): Path<String>) -> ApiResult<Json<Note>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    Ok(Json(load_note(&state, &key.user.id, &id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewNote {
    #[serde(default)]
    notebook_id: Option<String>,
    #[serde(default)]
    text: String,
}

/// `POST /v1/notes`: makes a note, at the top level or in `notebook_id`, with `text`.
async fn create_note(State(state): State<AppState>, key: KeyUser, Body(body): Body<NewNote>) -> ApiResult<Json<Note>> {
    check_text(&body.text)?;
    let id = new_id();
    add_note(&state, &key.user.id, &id, body.notebook_id.as_deref()).await?;
    if !body.text.is_empty() {
        write_text(&state, note_id(&id)?, &key.user.id, None, |_| body.text).await?;
    }
    Ok(Json(load_note(&state, &key.user.id, &id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotePatch {
    /// The note's whole new text.
    text: Option<String>,
    /// The `version` the text was based on. With it, edits made since are kept; without it, `text`
    /// replaces whatever the note holds now.
    version: Option<String>,
    /// Moves the note into this notebook; `null` moves it to the top level.
    #[serde(default, deserialize_with = "double_option")]
    notebook_id: Option<Option<String>>,
}

/// `PATCH /v1/notes/:id`: changes a note's text, moves it, or both.
async fn update_note(
    State(state): State<AppState>,
    key: KeyUser,
    Path(id): Path<String>,
    Body(body): Body<NotePatch>,
) -> ApiResult<Json<Note>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    if body.text.is_none() && body.notebook_id.is_none() {
        return Err(AppError::BadRequest("Send text, notebook_id or both".into()));
    }
    if body.version.is_some() && body.text.is_none() {
        return Err(AppError::BadRequest("version goes with text".into()));
    }
    require_editor(&state, &key.user.id, &id).await?;
    if let Some(text) = body.text {
        check_text(&text)?;
        write_text(&state, note_id(&id)?, &key.user.id, body.version.as_deref(), |_| text).await?;
    }
    if let Some(target) = body.notebook_id {
        move_note(&state, &key.user.id, &id, target.as_deref()).await?;
    }
    Ok(Json(load_note(&state, &key.user.id, &id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Append {
    text: String,
}

/// `POST /v1/notes/:id/append`: adds `text` at the end of the note, after a blank line. Nothing is
/// overwritten, so it needs no version: good for logs and journals.
async fn append(State(state): State<AppState>, key: KeyUser, Path(id): Path<String>, Body(body): Body<Append>) -> ApiResult<Json<Note>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    if body.text.trim().is_empty() {
        return Err(AppError::BadRequest("Send some text to append".into()));
    }
    require_editor(&state, &key.user.id, &id).await?;
    let mut too_long = false;
    write_text(&state, note_id(&id)?, &key.user.id, None, |current| {
        let gap = if current.is_empty() || current.ends_with("\n\n") {
            ""
        } else if current.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let text = format!("{current}{gap}{}", body.text);
        too_long = text.len() > MAX_TEXT;
        if too_long { current.to_owned() } else { text }
    })
    .await?;
    if too_long {
        return Err(AppError::BadRequest(format!("Notes hold at most {MAX_TEXT} bytes of text")));
    }
    Ok(Json(load_note(&state, &key.user.id, &id).await?))
}

/// `DELETE /v1/notes/:id`: moves a note you own to the trash, where it can be restored for 30 days.
async fn delete_note(State(state): State<AppState>, key: KeyUser, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    trash_note(&state, &key.user.id, &id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    /// Notes with every word, as prefixes.
    #[default]
    Words,
    /// Notes about what the query means, even without its words. Needs semantic search set up.
    Meaning,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    mode: Mode,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct Hit {
    id: String,
    title: String,
    /// Text around the match; for `words`, the matched words are wrapped in `**`.
    snippet: String,
}

/// `GET /v1/search?q=`: notes you can see that match, best first.
async fn search(State(state): State<AppState>, key: KeyUser, Query(q): Query<SearchQuery>) -> ApiResult<Json<Vec<Hit>>> {
    let limit = q.limit.unwrap_or(20).clamp(1, MAX_SEARCH);
    let found: Vec<SearchResult> = match q.mode {
        Mode::Words => by_words(&state, &key.user.id, &q.q, limit as i64).await?,
        Mode::Meaning => by_meaning(&state, &key.user.id, &q.q, limit)
            .await?
            .ok_or_else(|| AppError::Conflict("Search by meaning isn't set up on this server".into()))?,
    };
    let hits = found
        .into_iter()
        .map(|r| Hit {
            id: r.note,
            title: r.title,
            snippet: r.snippet.into_iter().map(|s| if s.hit { format!("**{}**", s.text) } else { s.text }).collect(),
        })
        .collect();
    Ok(Json(hits))
}
