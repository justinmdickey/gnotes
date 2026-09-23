//! Images and voice recordings attached to notes. See "Attachments" in docs/DESIGN.md.

use std::path::{Path, PathBuf};

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path as UrlPath, Request, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeFile;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::Role,
    tree::require_note,
    util::{new_id, now_ms, parse_id},
};

/// Big enough for a long voice memo or an unscaled phone photo.
pub const MAX_UPLOAD: usize = 50 * 1024 * 1024;

#[derive(Serialize, sqlx::FromRow)]
pub struct Attachment {
    id: String,
    note_id: String,
    filename: String,
    mime: String,
    size: i64,
    #[serde(skip)]
    sha256: String,
}

fn blob_path(data_dir: &Path, sha: &str) -> PathBuf {
    data_dir.join("blobs").join(&sha[..2]).join(sha)
}

/// Only types the editor knows how to show; anything else would just be a download.
fn allowed(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml" || mime.starts_with("audio/")
}

fn clean_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    if cleaned.is_empty() { "attachment".into() } else { cleaned }
}

/// Looks up an attachment the caller can at least read.
async fn find(state: &AppState, user_id: &str, id: &str, min: Role) -> ApiResult<Attachment> {
    let id = parse_id(id).ok_or(AppError::NotFound)?;
    let att: Attachment =
        sqlx::query_as("SELECT id, note_id, filename, mime, size, sha256 FROM attachments WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    require_note(state, user_id, &att.note_id, min).await?;
    Ok(att)
}

/// `POST /attachments`, multipart with `note_id` then `file`. Stores the file by content hash.
pub async fn upload(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    mut form: Multipart,
) -> ApiResult<Json<Attachment>> {
    let bad = |m: &str| AppError::BadRequest(m.into());
    let mut note_id = None;
    while let Some(mut field) = form.next_field().await.map_err(|e| bad(&e.body_text()))? {
        match field.name() {
            Some("note_id") => {
                let text = field.text().await.map_err(|e| bad(&e.body_text()))?;
                let id = parse_id(&text).ok_or_else(|| bad("Unknown note"))?;
                require_note(&state, &me.id, &id, Role::Editor).await?;
                note_id = Some(id);
            }
            Some("file") => {
                // note_id comes first so nothing is written for a note the caller can't edit.
                let note_id = note_id.take().ok_or_else(|| bad("Send note_id before the file"))?;
                let mime = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                if !allowed(&mime) {
                    return Err(bad("Only images and audio can be attached"));
                }
                let filename = clean_filename(field.file_name().unwrap_or(""));

                let tmp_dir = state.config.data_dir.join("blobs").join("tmp");
                tokio::fs::create_dir_all(&tmp_dir).await?;
                let tmp = tmp_dir.join(new_id());
                let mut out = tokio::fs::File::create(&tmp).await?;
                let mut hasher = Sha256::new();
                let mut size = 0usize;
                let written: ApiResult<()> = async {
                    while let Some(chunk) = field.chunk().await.map_err(|e| bad(&e.body_text()))? {
                        size += chunk.len();
                        if size > MAX_UPLOAD {
                            return Err(bad("Attachments can be up to 50 MB"));
                        }
                        hasher.update(&chunk);
                        out.write_all(&chunk).await?;
                    }
                    out.flush().await?;
                    Ok(())
                }
                .await;
                drop(out);
                if let Err(e) = written {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(e);
                }
                if size == 0 {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(bad("The file is empty"));
                }

                let sha: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
                let dest = blob_path(&state.config.data_dir, &sha);
                tokio::fs::create_dir_all(dest.parent().unwrap()).await?;
                if tokio::fs::try_exists(&dest).await? {
                    tokio::fs::remove_file(&tmp).await?;
                } else {
                    tokio::fs::rename(&tmp, &dest).await?;
                }

                let att = Attachment { id: new_id(), note_id, filename, mime, size: size as i64, sha256: sha };
                sqlx::query(
                    "INSERT INTO attachments (id, note_id, uploader_id, filename, mime, size, sha256, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&att.id)
                .bind(&att.note_id)
                .bind(&me.id)
                .bind(&att.filename)
                .bind(&att.mime)
                .bind(att.size)
                .bind(&att.sha256)
                .bind(now_ms())
                .execute(&state.db)
                .await?;
                return Ok(Json(att));
            }
            _ => {}
        }
    }
    Err(bad("Missing file"))
}

/// `GET /attachments/:id/meta`: what kind of file it is, so the editor picks image or audio.
pub async fn meta(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Attachment>> {
    Ok(Json(find(&state, &me.id, &id, Role::Viewer).await?))
}

/// `GET /attachments/:id`: the file itself. Supports Range requests, which Safari needs for audio.
pub async fn download(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    UrlPath(id): UrlPath<String>,
    req: Request,
) -> ApiResult<Response> {
    let att = find(&state, &me.id, &id, Role::Viewer).await?;
    let mime: mime::Mime = att.mime.parse().unwrap_or(mime::APPLICATION_OCTET_STREAM);
    let path = blob_path(&state.config.data_dir, &att.sha256);
    let mut res = ServeFile::new_with_mime(path, &mime)
        .try_call(req)
        .await
        .map_err(anyhow::Error::from)?
        .map(Body::new);
    let headers = res.headers_mut();
    // An attachment id always points at the same bytes.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    Ok(res.into_response())
}

/// `POST /attachments/:id/transcribe`: sends a recording to the configured speech-to-text endpoint.
pub async fn transcribe(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let att = find(&state, &me.id, &id, Role::Editor).await?;
    let Some(whisper) = state.whisper.read().await.clone() else {
        return Err(AppError::Conflict("Transcription isn't set up on this server".into()));
    };
    if !att.mime.starts_with("audio/") {
        return Err(AppError::BadRequest("Only recordings can be transcribed".into()));
    }
    let bytes = tokio::fs::read(blob_path(&state.config.data_dir, &att.sha256)).await?;
    let part = reqwest::multipart::Part::bytes(bytes).file_name(att.filename.clone()).mime_str(&att.mime)?;
    let form = reqwest::multipart::Form::new()
        .text("model", whisper.model.clone())
        .text("response_format", "json")
        .part("file", part);
    let mut req = state
        .http
        .post(format!("{}/audio/transcriptions", whisper.url.trim_end_matches('/')))
        .multipart(form);
    if let Some(key) = &whisper.key {
        req = req.bearer_auth(key);
    }
    let res = req.send().await.map_err(|e| {
        tracing::warn!("transcription request failed: {e:#}");
        AppError::Conflict("Couldn't reach the transcription service".into())
    })?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        tracing::warn!("transcription failed with {status}: {body}");
        return Err(AppError::Conflict("The transcription service returned an error".into()));
    }
    let body: Value = res.json().await?;
    let text = body.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    Ok(Json(json!({ "text": text })))
}

/// `GET /features`: what this server can do beyond the basics.
pub async fn features(State(state): State<AppState>, CurrentUser(_): CurrentUser) -> Json<Value> {
    Json(json!({ "transcription": state.whisper.read().await.is_some(), "max_upload": MAX_UPLOAD }))
}
