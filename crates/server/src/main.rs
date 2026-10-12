//! `vault-server`: serves the HTTP API for one vault folder.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::Parser;

#[derive(Parser)]
#[command(name = "vault-server", about = "Serve a TheVault vault over HTTP")]
struct Args {
    /// The vault folder to serve.
    #[arg(long)]
    vault: PathBuf,

    /// The address to listen on. Port 0 picks a free port.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if !args.vault.is_dir() {
        bail!("vault folder {} does not exist", args.vault.display());
    }

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("binding {}", args.listen))?;
    let addr = listener.local_addr()?;

    // One JSON line on stdout, so whoever started the server can read the port it got.
    let mut stdout = std::io::stdout().lock();
    writeln!(
        stdout,
        "{}",
        serde_json::json!({ "listen": addr.to_string() })
    )?;
    stdout.flush()?;
    drop(stdout);

    axum::serve(listener, api::router())
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
