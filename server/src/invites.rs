//! One-time signup links. An admin makes one (optionally sharing a note or notebook),
//! texts it to someone, and they pick their own username and password.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::{CurrentUser, User, create_user, hash_token, random_token, start_session},
    error::{ApiResult, AppError},
    perms::{Role, note_role, notebook_role},
    util::{new_id, now_ms},
};

const INVITE_DAYS: i64 = 7;

#[derive(Deserialize)]
pub struct NewInvite {
    resource_type: Option<String>,
    resource_id: Option<String>,
    #[serde(default = "editor")]
    role: String,
}

fn editor() -> String {
    "editor".into()
}

#[derive(Serialize)]
pub struct CreatedInvite {
    id: String,
    /// Shown once; the server keeps only its hash.
    token: String,
    expires_at: i64,
}

pub async fn create_invite(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<NewInvite>,
) -> ApiResult<Json<CreatedInvite>> {
    if !me.is_admin {
        return Err(AppError::Forbidden);
    }
    if !matches!(body.role.as_str(), "editor" | "viewer") {
        return Err(AppError::BadRequest("role must be 'editor' or 'viewer'".into()));
    }
    match (body.resource_type.as_deref(), body.resource_id.as_deref()) {
        (None, None) => {}
        (Some(kind), Some(id)) => {
            let role = match kind {
                "note" => note_role(&state.db, &me.id, id).await?,
                "notebook" => notebook_role(&state.db, &me.id, id).await?,
                _ => return Err(AppError::BadRequest("resource_type must be 'note' or 'notebook'".into())),
            };
            match role {
                None => return Err(AppError::NotFound),
                Some(Role::Owner) => {}
                Some(_) => return Err(AppError::Forbidden),
            }
        }
        _ => return Err(AppError::BadRequest("resource_type and resource_id go together".into())),
    }

    let token = random_token()?;
    let id = new_id();
    let now = now_ms();
    let expires_at = now + INVITE_DAYS * 24 * 60 * 60 * 1000;
    sqlx::query(
        "INSERT INTO invites (id, token_hash, created_by, resource_type, resource_id, role, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(hash_token(&token))
    .bind(&me.id)
    .bind(&body.resource_type)
    .bind(&body.resource_id)
    .bind(&body.role)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await?;
    Ok(Json(CreatedInvite { id, token, expires_at }))
}

#[derive(Serialize)]
pub struct InvitePreview {
    inviter: String,
    /// Name of the shared note or notebook, if any.
    shared: Option<String>,
    expires_at: i64,
}

/// Public: what the join page shows. 404 once used or expired.
pub async fn preview_invite(State(state): State<AppState>, Path(token): Path<String>) -> ApiResult<Json<InvitePreview>> {
    let row: Option<(String, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT u.display_name, i.resource_type, i.resource_id, i.expires_at
         FROM invites i JOIN users u ON u.id = i.created_by
         WHERE i.token_hash = ? AND i.used_at IS NULL AND i.expires_at > ?",
    )
    .bind(hash_token(&token))
    .bind(now_ms())
    .fetch_optional(&state.db)
    .await?;
    let (inviter, kind, id, expires_at) = row.ok_or(AppError::NotFound)?;
    let shared = match (kind.as_deref(), id) {
        (Some("note"), Some(id)) => sqlx::query_scalar("SELECT title FROM notes WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .map(|t: String| if t.is_empty() { "a note".into() } else { t }),
        (Some("notebook"), Some(id)) => {
            sqlx::query_scalar("SELECT name FROM notebooks WHERE id = ? AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&state.db)
                .await?
        }
        _ => None,
    };
    Ok(Json(InvitePreview { inviter, shared, expires_at }))
}

#[derive(Deserialize)]
pub struct AcceptInvite {
    username: String,
    #[serde(default)]
    display_name: String,
    password: String,
}

/// Public: creates the account, applies the invite's share, and logs the new user in.
pub async fn accept_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<AcceptInvite>,
) -> ApiResult<(CookieJar, Json<User>)> {
    let now = now_ms();
    // Claim the invite first so two people racing on one link can't both sign up.
    let claimed: Option<(String, String, Option<String>, Option<String>, String)> = sqlx::query_as(
        "UPDATE invites SET used_at = ?1 WHERE token_hash = ?2 AND used_at IS NULL AND expires_at > ?1
         RETURNING id, created_by, resource_type, resource_id, role",
    )
    .bind(now)
    .bind(hash_token(&token))
    .fetch_optional(&state.db)
    .await?;
    let (invite_id, inviter, kind, resource_id, role) = claimed.ok_or(AppError::NotFound)?;

    let user = match create_user(&state.db, &body.username.trim().to_lowercase(), &body.display_name, &body.password, false).await
    {
        Ok(user) => user,
        Err(e) => {
            // Give the link back so they can retry with a different username or password.
            sqlx::query("UPDATE invites SET used_at = NULL WHERE id = ?").bind(&invite_id).execute(&state.db).await?;
            return Err(e);
        }
    };
    sqlx::query("UPDATE invites SET used_by = ? WHERE id = ?").bind(&user.id).bind(&invite_id).execute(&state.db).await?;

    if let (Some(kind), Some(resource_id)) = (kind, resource_id) {
        sqlx::query(
            "INSERT INTO shares (id, resource_type, resource_id, user_id, role, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(new_id())
        .bind(&kind)
        .bind(&resource_id)
        .bind(&user.id)
        .bind(&role)
        .bind(&inviter)
        .bind(now)
        .execute(&state.db)
        .await?;
    }
    state.hub.tree_changed();
    let jar = start_session(&state, &user.id, &headers, jar).await?;
    Ok((jar, Json(user)))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PendingInvite {
    id: String,
    resource_type: Option<String>,
    resource_id: Option<String>,
    role: String,
    created_at: i64,
    expires_at: i64,
}

/// Admin: invites that haven't been used and haven't expired.
pub async fn list_invites(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Vec<PendingInvite>>> {
    if !me.is_admin {
        return Err(AppError::Forbidden);
    }
    let invites = sqlx::query_as::<_, PendingInvite>(
        "SELECT id, resource_type, resource_id, role, created_at, expires_at FROM invites
         WHERE used_at IS NULL AND expires_at > ? ORDER BY created_at DESC",
    )
    .bind(now_ms())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(invites))
}

pub async fn delete_invite(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !me.is_admin {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM invites WHERE id = ? AND used_at IS NULL").bind(&id).execute(&state.db).await?;
    Ok(Json(json!({ "ok": true })))
}
