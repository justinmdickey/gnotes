use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use axum::{Json, Router, routing::get};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

struct Config {
    data_dir: PathBuf,
    bind: SocketAddr,
    web_dir: PathBuf,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            data_dir: env::var("GNOTES_DATA_DIR").unwrap_or_else(|_| "./data".into()).into(),
            bind: env::var("GNOTES_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8080".into())
                .parse()
                .context("GNOTES_BIND must be host:port")?,
            web_dir: env::var("GNOTES_WEB_DIR").unwrap_or_else(|_| "./web/dist".into()).into(),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env()?;
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

    // Unknown paths fall back to index.html so the PWA's client-side routes load.
    let web = ServeDir::new(&config.web_dir)
        .fallback(ServeFile::new(config.web_dir.join("index.html")));

    let app = Router::new()
        .route("/api/health", get(|| async { Json(json!({ "ok": true })) }))
        .fallback_service(web)
        .layer(TraceLayer::new_for_http())
        .with_state(db);

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!("listening on http://{}", config.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
