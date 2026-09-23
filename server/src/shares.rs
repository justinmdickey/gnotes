use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::{Role, note_role, notebook_role},
    util::{new_id, now_ms},
};

async fn role_on(state: &AppState, user_id: &str, kind: &str, id: &str) -> ApiResult<Option<Role>> {
    Ok(match kind {
        "note" => note_role(&state.db, user_id, id).await?,
        "notebook" => notebook_role(&state.db, user_id, id).await?,
        _ => return Err(AppError::BadRequest("resource_type must be 'note' or 'notebook'".into())),
    })
}

async fn require_owner(state: &AppState, user_id: &str, kind: &str, id: &str) -> ApiResult<()> {
    match role_on(state, user_id, kind, id).await? {
        None => Err(AppError::NotFound),
        Some(Role::Owner) => Ok(()),
        Some(_) => Err(AppError::Forbidden),
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ShareInfo {
    id: String,
    username: String,
    display_name: String,
    role: String,
}

async fn list(state: AppState, me: &str, kind: &str, id: &str) -> ApiResult<Json<Vec<ShareInfo>>> {
    require_owner(&state, me, kind, id).await?;
    let shares = sqlx::query_as::<_, ShareInfo>(
        "SELECT s.id, u.username, u.display_name, s.role FROM shares s JOIN users u ON u.id = s.user_id
         WHERE s.resource_type = ? AND s.resource_id = ? ORDER BY u.display_name COLLATE NOCASE",
    )
    .bind(kind)
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(shares))
}

pub async fn list_note_shares(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<ShareInfo>>> {
    list(state, &me.id, "note", &id).await
}

pub async fn list_notebook_shares(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<ShareInfo>>> {
    list(state, &me.id, "notebook", &id).await
}

fn share_role(role: &str) -> ApiResult<&str> {
    match role {
        "editor" | "viewer" => Ok(role),
        _ => Err(AppError::BadRequest("role must be 'editor' or 'viewer'".into())),
    }
}

#[derive(Deserialize)]
pub struct NewShare {
    resource_type: String,
    resource_id: String,
    username: String,
    role: String,
}

/// Creates a share, or changes the role if this person already has one on the item.
pub async fn create_share(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<NewShare>,
) -> ApiResult<Json<Value>> {
    let role = share_role(&body.role)?;
    require_owner(&state, &me.id, &body.resource_type, &body.resource_id).await?;
    let target: String = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
        .bind(body.username.trim().to_lowercase())
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest(format!("No user named '{}'", body.username)))?;
    if target == me.id {
        return Err(AppError::BadRequest("You already own this".into()));
    }
    let id: String = sqlx::query_scalar(
        "INSERT INTO shares (id, resource_type, resource_id, user_id, role, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (resource_type, resource_id, user_id) DO UPDATE SET role = excluded.role
         RETURNING id",
    )
    .bind(new_id())
    .bind(&body.resource_type)
    .bind(&body.resource_id)
    .bind(&target)
    .bind(role)
    .bind(&me.id)
    .bind(now_ms())
    .fetch_one(&state.db)
    .await?;
    state.recheck_access().await;
    state.hub.tree_changed();
    Ok(Json(json!({ "id": id })))
}

struct ShareRow {
    resource_type: String,
    resource_id: String,
    user_id: String,
}

async fn load_share(state: &AppState, id: &str) -> ApiResult<ShareRow> {
    let (resource_type, resource_id, user_id): (String, String, String) =
        sqlx::query_as("SELECT resource_type, resource_id, user_id FROM shares WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    Ok(ShareRow { resource_type, resource_id, user_id })
}

#[derive(Deserialize)]
pub struct SharePatch {
    /// Owner only.
    role: Option<String>,
    /// Recipient only: hides the item from their "Shared with me" list.
    hidden: Option<bool>,
}

pub async fn update_share(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<SharePatch>,
) -> ApiResult<Json<Value>> {
    let share = load_share(&state, &id).await?;
    if let Some(role) = &body.role {
        let role = share_role(role)?;
        require_owner(&state, &me.id, &share.resource_type, &share.resource_id).await?;
        sqlx::query("UPDATE shares SET role = ? WHERE id = ?").bind(role).bind(&id).execute(&state.db).await?;
        state.recheck_access().await;
    }
    if let Some(hidden) = body.hidden {
        if share.user_id != me.id {
            return Err(AppError::Forbidden);
        }
        sqlx::query("UPDATE shares SET hidden = ? WHERE id = ?").bind(hidden).bind(&id).execute(&state.db).await?;
    }
    state.hub.tree_changed();
    Ok(Json(json!({ "ok": true })))
}

/// The owner can remove anyone's share; a recipient can remove their own (leave).
pub async fn delete_share(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let share = load_share(&state, &id).await?;
    if share.user_id != me.id {
        require_owner(&state, &me.id, &share.resource_type, &share.resource_id).await?;
    }
    sqlx::query("DELETE FROM shares WHERE id = ?").bind(&id).execute(&state.db).await?;
    state.recheck_access().await;
    state.hub.tree_changed();
    Ok(Json(json!({ "ok": true })))
}
