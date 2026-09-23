use std::io::{BufRead, Write};

use anyhow::Context;
use clap::{Parser, Subcommand};
use gnotes_server::{Config, auth, build, open_db, serve};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "gnotes-server", about = "Self-hosted Gnotes server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (default).
    Serve,
    /// Create an account. Reads the password from GNOTES_PASSWORD or stdin.
    CreateUser {
        username: String,
        #[arg(long, default_value = "")]
        display_name: String,
        #[arg(long)]
        admin: bool,
    },
}

fn read_password() -> anyhow::Result<String> {
    if let Ok(p) = std::env::var("GNOTES_PASSWORD") {
        return Ok(p);
    }
    eprint!("Password: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cli = Cli::parse();
    let config = Config::from_env()?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            let listener = tokio::net::TcpListener::bind(config.bind)
                .await
                .with_context(|| format!("binding {}", config.bind))?;
            tracing::info!("listening on http://{}", config.bind);
            serve(build(config).await?, listener).await
        }
        Command::CreateUser { username, display_name, admin } => {
            let db = open_db(&config).await?;
            let password = read_password()?;
            let user = auth::create_user(&db, &username.to_lowercase(), &display_name, &password, admin)
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            println!("Created {} ({})", user.username, if admin { "admin" } else { "user" });
            Ok(())
        }
    }
}
