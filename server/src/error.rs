use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type ApiResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized,
    Forbidden,
    /// Also used when the caller has no access, so hidden items don't leak.
    NotFound,
    Conflict(String),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "Not logged in".into()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Not allowed".into()),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not_found", "Not found".into()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m),
            AppError::Internal(e) => {
                tracing::error!("internal error: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", "Internal error".into())
            }
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError::Internal(e.into())
    }
}
