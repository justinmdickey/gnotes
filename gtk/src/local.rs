//! Notes on this computer, with no server to set up: the Gnotes server runs inside the app on
//! localhost, with its data in one folder and an account it makes for you. Everything works as it
//! does against a real server, because it is one; Move to a Server takes the notes elsewhere later.

use std::path::{Path, PathBuf};

use gnotes_server::{Config, auth, build, serve};
use serde::{Deserialize, Serialize};

use crate::api::{Api, User};

/// The account made for local notes, kept so the app can sign in to its own server at each start.
#[derive(Serialize, Deserialize)]
struct Account {
    username: String,
    password: String,
}

/// Where local notes live: `~/.local/share/gnotes/local` (or under `GNOTES_GTK_DIR`).
pub fn data_dir() -> PathBuf {
    crate::cache::data_dir().join("local")
}

/// Starts the built-in server on a free localhost port and signs in. `name` is the name to show on
/// a first start. Must run inside the tokio runtime; the server keeps running until the app exits.
pub async fn start(dir: &Path, name: &str) -> anyhow::Result<(Api, User)> {
    std::fs::create_dir_all(dir)?;
    let config = Config {
        data_dir: dir.to_path_buf(),
        bind: "127.0.0.1:0".parse()?,
        // No web app: the GTK app is the only client.
        web_dir: dir.join("no-web"),
        public_url: None,
        whisper: None,
        vision: None,
        summary: None,
        embed: None,
    };
    let state = build(config).await?;
    let account_file = dir.join("account.json");
    let account: Account = match std::fs::read(&account_file).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        Some(a) => a,
        None => {
            // Random, as the server makes session tokens; only ever used on this computer.
            let a = Account { username: "me".into(), password: auth::random_token()? };
            auth::create_user(&state.db, &a.username, name, &a.password, true)
                .await
                .map_err(|e| anyhow::anyhow!("making the local account: {e:?}"))?;
            write_private(&account_file, &serde_json::to_vec(&a)?)?;
            a
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    tokio::spawn(async move {
        if let Err(e) = serve(state, listener).await {
            eprintln!("local notes server stopped: {e:#}");
        }
    });
    let (api, user) = Api::login(&base, &account.username, &account.password).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok((api, user))
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(bytes)
}
