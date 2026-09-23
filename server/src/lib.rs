pub mod auth;
pub mod error;
pub mod invites;
pub mod perms;
pub mod rooms;
pub mod shares;
pub mod tree;
pub mod util;
pub mod ws;

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
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
        .route("/me", get(auth::me))
        .route("/users", get(auth::list_users))
        .route("/admin/users", post(auth::admin_create_user))
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
    Ok(AppState {
        db,
        config: Arc::new(config),
        rooms: Default::default(),
        hub: Default::default(),
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
