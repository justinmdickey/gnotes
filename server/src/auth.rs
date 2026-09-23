use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, header::USER_AGENT, request::Parts},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    AppState,
    error::{ApiResult, AppError},
    util::{new_id, now_ms},
};

pub const SESSION_COOKIE: &str = "gnotes_session";
const SESSION_DAYS: i64 = 90;

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing password: {e}"))
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

fn hash_token(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

pub fn valid_username(username: &str) -> bool {
    (1..=32).contains(&username.len())
        && username.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
}

pub async fn create_user(
    db: &sqlx::SqlitePool,
    username: &str,
    display_name: &str,
    password: &str,
    is_admin: bool,
) -> ApiResult<User> {
    if !valid_username(username) {
        return Err(AppError::BadRequest(
            "Usernames are 1-32 characters of a-z, 0-9, '.', '_' or '-'".into(),
        ));
    }
    if password.len() < 8 {
        return Err(AppError::BadRequest("Passwords need at least 8 characters".into()));
    }
    let display_name = if display_name.trim().is_empty() { username } else { display_name.trim() };
    let user = User {
        id: new_id(),
        username: username.into(),
        display_name: display_name.into(),
        is_admin,
    };
    let result = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_admin, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.username)
    .bind(&user.display_name)
    .bind(hash_password(password)?)
    .bind(is_admin)
    .bind(now_ms())
    .execute(db)
    .await;
    match result {
        Ok(_) => Ok(user),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(AppError::Conflict(format!("User '{username}' already exists")))
        }
        Err(e) => Err(e.into()),
    }
}

/// The logged-in user, taken from the session cookie.
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(SESSION_COOKIE).ok_or(AppError::Unauthorized)?.value().to_owned();
        let user = sqlx::query_as::<_, User>(
            "SELECT u.id, u.username, u.display_name, u.is_admin
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.expires_at > ?",
        )
        .bind(hash_token(&token))
        .bind(now_ms())
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;
        Ok(CurrentUser(user))
    }
}

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> ApiResult<(CookieJar, Json<User>)> {
    let row = sqlx::query_as::<_, (String, String, String, bool, String)>(
        "SELECT id, username, display_name, is_admin, password_hash FROM users WHERE username = ?",
    )
    .bind(body.username.trim().to_lowercase())
    .fetch_optional(&state.db)
    .await?;

    let Some((id, username, display_name, is_admin, password_hash)) = row else {
        return Err(AppError::Unauthorized);
    };
    let password = body.password;
    let ok = tokio::task::spawn_blocking(move || verify_password(&password, &password_hash)).await?;
    if !ok {
        return Err(AppError::Unauthorized);
    }

    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    let token = URL_SAFE_NO_PAD.encode(raw);
    let now = now_ms();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, user_agent) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(hash_token(&token))
    .bind(&id)
    .bind(now)
    .bind(now + SESSION_DAYS * 24 * 60 * 60 * 1000)
    .bind(headers.get(USER_AGENT).and_then(|v| v.to_str().ok()))
    .execute(&state.db)
    .await?;

    let cookie = Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.config.secure_cookies())
        .path("/")
        .max_age(time::Duration::days(SESSION_DAYS))
        .build();
    Ok((jar.add(cookie), Json(User { id, username, display_name, is_admin })))
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> ApiResult<(CookieJar, Json<Value>)> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(hash_token(c.value()))
            .execute(&state.db)
            .await?;
    }
    let jar = jar.remove(Cookie::build(SESSION_COOKIE).path("/"));
    Ok((jar, Json(json!({ "ok": true }))))
}

pub async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}

#[derive(Deserialize)]
pub struct NewUserBody {
    username: String,
    #[serde(default)]
    display_name: String,
    password: String,
    #[serde(default)]
    is_admin: bool,
}

pub async fn admin_create_user(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<NewUserBody>,
) -> ApiResult<Json<User>> {
    if !me.is_admin {
        return Err(AppError::Forbidden);
    }
    let user = create_user(
        &state.db,
        &body.username.trim().to_lowercase(),
        &body.display_name,
        &body.password,
        body.is_admin,
    )
    .await?;
    Ok(Json(user))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct UserSummary {
    id: String,
    username: String,
    display_name: String,
}

/// Everyone on the server, for picking who to share with.
pub async fn list_users(State(state): State<AppState>, _: CurrentUser) -> ApiResult<Json<Vec<UserSummary>>> {
    let users = sqlx::query_as::<_, UserSummary>(
        "SELECT id, username, display_name FROM users ORDER BY display_name COLLATE NOCASE",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}
