//! Who wrote what: each Loro peer is one editing session, and the server notes whose session it was
//! as its edits arrive. The app maps the peer that wrote each character back to a person.

use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::{Map, Value, json};

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::Role,
    tree::require_note,
    util::parse_id,
};

/// `GET /notes/:id/authors`: `{ peers: { "<peer>": { user_id, name } } }` for every session that
/// has edited the note since authors were tracked. Older edits have no entry.
pub async fn list_authors(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    require_note(&state, &me.id, &id, Role::Viewer).await?;
    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.peer, p.user_id, u.display_name FROM note_peers p LEFT JOIN users u ON u.id = p.user_id WHERE p.note_id = ?",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    let peers: Map<String, Value> =
        rows.into_iter().map(|(peer, user_id, name)| (peer, json!({ "user_id": user_id, "name": name }))).collect();
    Ok(Json(json!({ "peers": peers })))
}
