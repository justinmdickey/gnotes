pub mod account;
pub mod attachments;
pub mod auth;
pub mod error;
pub mod import;
pub mod invites;
pub mod live;
pub mod perms;
pub mod rooms;
pub mod settings;
pub mod shares;
pub mod tree;
pub mod util;
pub mod ws;

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    routing::{get, patch, post},
};
use serde_json::json;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

pub struct Config {
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub web_dir: PathBuf,
    /// e.g. `https://notes.example.com`. Enables Secure cookies and pins the websocket Origin.
    pub public_url: Option<String>,
    /// Default speech-to-text for voice notes, from env. An admin's saved setting overrides it.
    pub whisper: Option<WhisperConfig>,
    /// Default photo reading, from env. An admin's saved setting overrides it.
    pub vision: Option<VisionConfig>,
}

/// An OpenAI-compatible chat API with a vision model, for reading the text in photos, e.g. Ollama,
/// llama.cpp or vLLM on local hardware, or a hosted one.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct VisionConfig {
    /// Base URL including the version, e.g. `http://ollama:11434/v1`. `/chat/completions` is appended.
    pub url: String,
    pub model: String,
    pub key: Option<String>,
}

/// An OpenAI-compatible transcription API, e.g. faster-whisper-server or OpenAI itself.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WhisperConfig {
    /// Base URL including the version, e.g. `http://whisper:8000/v1`. `/audio/transcriptions` is appended.
    pub url: String,
    pub model: String,
    pub key: Option<String>,
    /// Optional realtime endpoint, e.g. `ws://whisper:8000/v1/realtime`, for live transcripts while recording.
    #[serde(default)]
    pub realtime_url: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            data_dir: env::var("GNOTES_DATA_DIR").unwrap_or_else(|_| "./data".into()).into(),
            bind: env::var("GNOTES_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8080".into())
                .parse()
                .context("GNOTES_BIND must be host:port")?,
            web_dir: env::var("GNOTES_WEB_DIR").unwrap_or_else(|_| "./web/dist".into()).into(),
            public_url: env::var("GNOTES_PUBLIC_URL").ok().filter(|s| !s.is_empty()),
            whisper: env::var("GNOTES_WHISPER_URL").ok().filter(|s| !s.is_empty()).map(|url| WhisperConfig {
                url,
                model: env::var("GNOTES_WHISPER_MODEL").unwrap_or_else(|_| "whisper-1".into()),
                key: env::var("GNOTES_WHISPER_KEY").ok().filter(|s| !s.is_empty()),
                realtime_url: env::var("GNOTES_WHISPER_REALTIME_URL").ok().filter(|s| !s.is_empty()),
            }),
            vision: env::var("GNOTES_VISION_URL").ok().filter(|s| !s.is_empty()).map(|url| VisionConfig {
                url,
                model: env::var("GNOTES_VISION_MODEL").unwrap_or_default(),
                key: env::var("GNOTES_VISION_KEY").ok().filter(|s| !s.is_empty()),
            }),
        })
    }

    pub fn secure_cookies(&self) -> bool {
        self.public_url.as_deref().is_some_and(|u| u.starts_with("https://"))
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub rooms: Arc<rooms::Rooms>,
    pub hub: Arc<rooms::Hub>,
    /// Outgoing HTTP, for transcription.
    pub http: reqwest::Client,
    /// The speech-to-text service in use right now. Admins can change it while running.
    pub whisper: Arc<tokio::sync::RwLock<Option<WhisperConfig>>>,
    /// The photo-reading service in use right now.
    pub vision: Arc<tokio::sync::RwLock<Option<VisionConfig>>>,
}

impl AppState {
    pub async fn recheck_access(&self) {
        rooms::recheck_access(self).await;
    }
}

pub async fn open_db(config: &Config) -> anyhow::Result<SqlitePool> {
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("creating {}", config.data_dir.display()))?;
    let db = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(config.data_dir.join("gnotes.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .foreign_keys(true),
        )
        .await?;
    sqlx::migrate!().run(&db).await?;
    Ok(db)
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me).patch(account::update_me))
        .route("/me/password", post(account::change_password))
        .route("/me/logout-others", post(account::logout_others))
        .route("/users", get(auth::list_users))
        .route("/admin/users", get(account::admin_list_users).post(auth::admin_create_user))
        .route("/admin/users/{id}", patch(account::admin_update_user))
        .route("/admin/users/{id}/password", post(account::admin_reset_password))
        .route("/invites", get(invites::list_invites).post(invites::create_invite))
        .route("/invites/{id}", axum::routing::delete(invites::delete_invite))
        .route("/join/{token}", get(invites::preview_invite).post(invites::accept_invite))
        .route("/tree", get(tree::get_tree))
        .route("/notebooks", post(tree::create_notebook))
        .route("/notebooks/{id}", patch(tree::update_notebook).delete(tree::delete_notebook))
        .route("/notebooks/{id}/shares", get(shares::list_notebook_shares))
        .route("/notes", post(tree::create_note))
        .route("/notes/{id}", patch(tree::update_note).delete(tree::delete_note))
        .route("/notes/{id}/shares", get(shares::list_note_shares))
        .route("/trash", get(tree::get_trash))
        .route("/trash/{kind}/{id}/restore", post(tree::restore))
        .route("/shares", post(shares::create_share))
        .route("/shares/{id}", patch(shares::update_share).delete(shares::delete_share))
        .route(
            "/attachments",
            post(attachments::upload).layer(DefaultBodyLimit::max(attachments::MAX_UPLOAD + 64 * 1024)),
        )
        .route("/attachments/{id}", get(attachments::download))
        .route("/attachments/{id}/meta", get(attachments::meta))
        .route("/attachments/{id}/transcribe", post(attachments::transcribe))
        .route("/import", post(import::import).layer(DefaultBodyLimit::max(import::MAX_IMPORT)))
        .route("/features", get(attachments::features))
        .route("/transcribe/live", get(live::handler))
        .route("/admin/settings", get(settings::get_settings))
        .route("/admin/settings/whisper", axum::routing::put(settings::put_whisper))
        .route("/admin/settings/whisper/test", post(settings::test_whisper))
        .route("/admin/settings/vision", axum::routing::put(settings::put_vision))
        .route("/admin/settings/vision/test", post(settings::test_vision))
        .route("/attachments/{id}/text", post(attachments::photo_text))
        .route("/ws", get(ws::handler))
        .fallback(|| async { error::AppError::NotFound });

    // Unknown paths fall back to index.html so the PWA's client-side routes load.
    let web = ServeDir::new(&state.config.web_dir)
        .fallback(ServeFile::new(state.config.web_dir.join("index.html")));

    Router::new()
        .nest("/api", api)
        .fallback_service(web)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn build(config: Config) -> anyhow::Result<AppState> {
    let db = open_db(&config).await?;
    let whisper = match settings::load_whisper(&db).await? {
        Some(saved) => saved,
        None => config.whisper.clone(),
    };
    let vision = match settings::load_vision(&db).await? {
        Some(saved) => saved,
        None => config.vision.clone(),
    };
    Ok(AppState {
        whisper: Arc::new(tokio::sync::RwLock::new(whisper)),
        vision: Arc::new(tokio::sync::RwLock::new(vision)),
        db,
        config: Arc::new(config),
        rooms: Default::default(),
        hub: Default::default(),
        http: reqwest::Client::builder().timeout(Duration::from_secs(300)).build()?,
    })
}

/// Runs until the listener fails. Also purges old trash hourly.
pub async fn serve(state: AppState, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    let db = state.db.clone();
    tokio::spawn(async move {
        let mut hourly = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            hourly.tick().await;
            if let Err(e) = tree::purge_trash(&db).await {
                tracing::error!("purging trash: {e:#}");
            }
        }
    });
    axum::serve(listener, router(state)).await?;
    Ok(())
}
