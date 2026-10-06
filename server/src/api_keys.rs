//! API keys: you make one in Account and give it to an agent or script, which sends it as
//! `Authorization: Bearer <key>` to reach /api/v1 as you. Keys are managed with a logged-in session
//! only, so a key can't make more keys. The agent API itself is in v1.rs; docs/API.md documents it.

use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{Method, header::AUTHORIZATION, request::Parts},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::{CurrentUser, User, hash_token, random_token},
    error::{ApiResult, AppError},
    util::{new_id, now_ms},
};

/// Starts every key, so it's easy to recognize, e.g. by secret scanners.
pub const KEY_PREFIX: &str = "gnk_";
const MAX_KEYS: i64 = 50;
/// How stale `last_used_at` may get, so a busy agent doesn't write to the database on every request.
const TOUCH_EVERY_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// GET requests only.
    Read,
    /// Everything in /api/v1.
    Write,
}

impl Scope {
    fn parse(s: &str) -> Scope {
        if s == "write" { Scope::Write } else { Scope::Read }
    }

    fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
        }
    }
}

#[derive(Serialize)]
pub struct ApiKey {
    id: String,
    name: String,
    scope: Scope,
    created_at: i64,
    last_used_at: Option<i64>,
}

/// `GET /me/keys`: your keys, newest first. Never the keys themselves, which aren't stored.
pub async fn list_keys(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Vec<ApiKey>>> {
    let rows: Vec<(String, String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, name, scope, created_at, last_used_at FROM api_keys WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(&me.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, scope, created_at, last_used_at)| ApiKey { id, name, scope: Scope::parse(&scope), created_at, last_used_at })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct NewKey {
    name: String,
    scope: Scope,
}

/// `POST /me/keys`: a new key. The response is the only time the key itself is shown.
pub async fn create_key(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<NewKey>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(AppError::BadRequest("Key names need 1-60 characters".into()));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE user_id = ?")
        .bind(&me.id)
        .fetch_one(&state.db)
        .await?;
    if count >= MAX_KEYS {
        return Err(AppError::Conflict(format!("You have {MAX_KEYS} keys already; revoke one first")));
    }
    let token = format!("{KEY_PREFIX}{}", random_token()?);
    let key = ApiKey { id: new_id(), name: name.to_owned(), scope: body.scope, created_at: now_ms(), last_used_at: None };
    sqlx::query("INSERT INTO api_keys (id, user_id, name, scope, token_hash, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&key.id)
        .bind(&me.id)
        .bind(&key.name)
        .bind(key.scope.as_str())
        .bind(hash_token(&token))
        .bind(key.created_at)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "key": key, "token": token })))
}

/// `DELETE /me/keys/:id`: revokes a key; it stops working at once.
pub async fn revoke_key(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM api_keys WHERE id = ? AND user_id = ?")
        .bind(&id)
        .bind(&me.id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "ok": true })))
}

/// The caller of an /api/v1 route: the user an API key acts as, and the key.
///
/// Extracting it also enforces the key's scope by HTTP method: a read key may only GET. Every v1
/// route takes a `KeyUser`, so new routes get the check without doing anything.
pub struct KeyUser {
    pub user: User,
    pub key_name: String,
    pub scope: Scope,
}

impl FromRequestParts<AppState> for KeyUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| t.starts_with(KEY_PREFIX))
            .ok_or(AppError::BadKey)?;
        let row: Option<(String, String, String, Option<i64>, String, String, String, bool)> = sqlx::query_as(
            "SELECT k.id, k.name, k.scope, k.last_used_at, u.id, u.username, u.display_name, u.is_admin
             FROM api_keys k JOIN users u ON u.id = k.user_id
             WHERE k.token_hash = ? AND u.disabled_at IS NULL",
        )
        .bind(hash_token(token))
        .fetch_optional(&state.db)
        .await?;
        let (key_id, key_name, scope, last_used_at, id, username, display_name, is_admin) = row.ok_or(AppError::BadKey)?;
        let scope = Scope::parse(&scope);
        if scope == Scope::Read && !matches!(parts.method, Method::GET | Method::HEAD) {
            return Err(AppError::NotAllowed("This key can only read; make a Read and write key to change notes".into()));
        }
        let now = now_ms();
        if last_used_at.is_none_or(|t| now - t > TOUCH_EVERY_MS) {
            sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?").bind(now).bind(&key_id).execute(&state.db).await?;
        }
        Ok(KeyUser { user: User { id, username, display_name, is_admin }, key_name, scope })
    }
}
