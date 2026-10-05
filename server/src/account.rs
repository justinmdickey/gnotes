//! The Settings screen: your own account, plus admin management of other accounts.

use axum::{
    Json,
    extract::{Path, State},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::{CurrentUser, SESSION_COOKIE, User, hash_password, hash_token, verify_password},
    error::{ApiResult, AppError},
    util::now_ms,
};

fn clean_display_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(AppError::BadRequest("Names need 1-60 characters".into()));
    }
    Ok(name.to_owned())
}

fn check_new_password(password: &str) -> ApiResult<()> {
    if password.len() < 8 {
        return Err(AppError::BadRequest("Passwords need at least 8 characters".into()));
    }
    Ok(())
}

/// The current session's token hash, so "log out other devices" can keep this one.
fn current_session(jar: &CookieJar) -> Option<String> {
    jar.get(SESSION_COOKIE).map(|c| hash_token(c.value()))
}

async fn drop_sessions(state: &AppState, user_id: &str, keep: Option<&str>) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash IS NOT ?")
        .bind(user_id)
        .bind(keep)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ProfilePatch {
    display_name: String,
}

pub async fn update_me(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<ProfilePatch>,
) -> ApiResult<Json<User>> {
    let display_name = clean_display_name(&body.display_name)?;
    sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
        .bind(&display_name)
        .bind(&me.id)
        .execute(&state.db)
        .await?;
    // Owner names show in other people's note lists.
    state.hub.tree_changed();
    Ok(Json(User { display_name, ..me }))
}

#[derive(Deserialize)]
pub struct PasswordChange {
    current: String,
    new: String,
}

/// Changes your password and signs out every other device.
pub async fn change_password(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    jar: CookieJar,
    Json(body): Json<PasswordChange>,
) -> ApiResult<Json<Value>> {
    check_new_password(&body.new)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&me.id)
        .fetch_one(&state.db)
        .await?;
    if !verify_password(body.current, hash).await? {
        return Err(AppError::BadRequest("Current password is wrong".into()));
    }
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash_password(&body.new).await?)
        .bind(&me.id)
        .execute(&state.db)
        .await?;
    drop_sessions(&state, &me.id, current_session(&jar).as_deref()).await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn logout_others(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    jar: CookieJar,
) -> ApiResult<Json<Value>> {
    drop_sessions(&state, &me.id, current_session(&jar).as_deref()).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AdminUser {
    id: String,
    username: String,
    display_name: String,
    is_admin: bool,
    disabled: bool,
    created_at: i64,
}

pub(crate) fn require_admin(me: &User) -> ApiResult<()> {
    if me.is_admin { Ok(()) } else { Err(AppError::Forbidden) }
}

pub async fn admin_list_users(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Vec<AdminUser>>> {
    require_admin(&me)?;
    let users = sqlx::query_as::<_, AdminUser>(
        "SELECT id, username, display_name, is_admin, disabled_at IS NOT NULL AS disabled, created_at
         FROM users ORDER BY display_name COLLATE NOCASE",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct AdminUserPatch {
    is_admin: Option<bool>,
    disabled: Option<bool>,
}

pub async fn admin_update_user(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<AdminUserPatch>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    if id == me.id && (body.is_admin == Some(false) || body.disabled == Some(true)) {
        // Keeps the server from ending up with nobody able to manage it.
        return Err(AppError::BadRequest("You can't remove your own admin access or disable yourself".into()));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?").bind(&id).fetch_optional(&state.db).await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }
    if let Some(is_admin) = body.is_admin {
        sqlx::query("UPDATE users SET is_admin = ? WHERE id = ?").bind(is_admin).bind(&id).execute(&state.db).await?;
    }
    if let Some(disabled) = body.disabled {
        sqlx::query("UPDATE users SET disabled_at = ? WHERE id = ?")
            .bind(disabled.then(now_ms))
            .bind(&id)
            .execute(&state.db)
            .await?;
        if disabled {
            drop_sessions(&state, &id, None).await?;
            state.hub.disconnect_user(&id);
        }
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PasswordReset {
    password: String,
}

/// Sets someone's password (they forgot it) and signs them out everywhere.
pub async fn admin_reset_password(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<PasswordReset>,
) -> ApiResult<Json<Value>> {
    require_admin(&me)?;
    check_new_password(&body.password)?;
    let result = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash_password(&body.password).await?)
        .bind(&id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    drop_sessions(&state, &id, None).await?;
    state.hub.disconnect_user(&id);
    Ok(Json(json!({ "ok": true })))
}
