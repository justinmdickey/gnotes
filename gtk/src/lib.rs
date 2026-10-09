//! The Gnotes GTK client. `api`, `sync`, `note` and `cache` are the parts without GTK, tested against
//! a real server in `tests/`; `ui` is the app.

pub mod api;
pub mod cache;
pub mod local;
pub mod migrate;
pub mod note;
pub mod recorder;
pub mod sync;
pub mod ui;

use std::{future::Future, sync::OnceLock};

/// HTTP and the websocket run here, off the GTK thread.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Runs `f` on the runtime; await the result from the GTK thread.
pub async fn bg<F>(f: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(f).await.expect("background task")
}
