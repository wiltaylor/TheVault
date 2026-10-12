//! `vault-server`: serves the HTTP API for one vault folder.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Parser;
use scheduler::{Scheduler, TestClock};
use state::{Clock, SecretKey, SystemClock};

mod web;

#[derive(Parser)]
#[command(name = "vault-server", about = "Serve a TheVault vault over HTTP")]
struct Args {
    /// The vault folder to serve.
    #[arg(long)]
    vault: PathBuf,

    /// The address to listen on. Port 0 picks a free port.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,

    /// A folder holding the built web client to serve outside `/api`, such as `web/dist`.
    #[arg(long)]
    web: Option<PathBuf>,

    /// Start a clock that only `/api/test/clock` moves, at this RFC 3339 time or count of
    /// milliseconds since the epoch. Only test builds accept it (S7).
    #[arg(long, value_name = "TIME")]
    test_clock: Option<String>,
}

/// Script worker threads for scheduled tasks.
const SCRIPT_WORKERS: usize = 4;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let test_clock = args.test_clock.as_deref().map(test_clock).transpose()?;
    if !args.vault.is_dir() {
        bail!("vault folder {} does not exist", args.vault.display());
    }
    if let Some(web) = &args.web
        && !web.join("index.html").is_file()
    {
        bail!("web folder {} has no index.html", web.display());
    }
    let vault = store::Vault::init(&args.vault)
        .with_context(|| format!("opening vault {}", args.vault.display()))?;
    let clock: Arc<dyn Clock> = match &test_clock {
        Some(c) => c.clone(),
        None => Arc::new(SystemClock),
    };
    let state =
        state::State::open_with_clock(&vault.server_dir().join("state.db"), None, clock.clone())
            .context("opening state.db")?;
    let key = secret_key(&state.vault_id().context("reading the vault id")?)?;
    let state = state.with_key(key);
    let handlers = script::Handlers::new(script::Limits::default());
    let writer = Arc::new(
        writer::Writer::open(vault, state)
            .context("loading the vault")?
            .with_clock(clock.clone())
            .with_hooks(handlers.hooks()),
    );
    handlers.start(&writer);
    // The script hosts' HTTP clients block, so they are made outside the async runtime. Tasks
    // change notes through the writer, so a task's changes run handlers too.
    let host = Arc::new(script::Host::new(
        writer.clone(),
        SCRIPT_WORKERS,
        script::Limits::default(),
    ));
    let scheduler =
        Scheduler::new(writer.clone(), host, clock).context("loading the scheduled tasks")?;
    if test_clock.is_none() {
        scheduler.start();
    }
    let app = api::router(writer, scheduler, test_clock);

    tokio::runtime::Runtime::new()
        .context("starting the async runtime")?
        .block_on(serve(args.listen, args.web, app))
}

/// The clock `--test-clock` asks for. A release build refuses it.
fn test_clock(at: &str) -> anyhow::Result<Arc<TestClock>> {
    if !cfg!(debug_assertions) {
        bail!("--test-clock is accepted only by test builds");
    }
    let ms = at
        .parse::<i64>()
        .ok()
        .or_else(|| scheduler::parse_time(at))
        .with_context(|| format!("--test-clock {at:?} is not an RFC 3339 time"))?;
    Ok(Arc::new(TestClock::new(ms)))
}

/// The key that encrypts secrets (OQ-S2): the file `VAULT_SECRET_KEY_FILE` names, which must
/// exist, or else `$XDG_CONFIG_HOME/thevault/<vault-id>.key` when `vault init` wrote one. Without
/// a key, secrets can be listed but not written or read.
fn secret_key(vault_id: &str) -> anyhow::Result<Option<SecretKey>> {
    let named = std::env::var_os("VAULT_SECRET_KEY_FILE").filter(|p| !p.is_empty());
    let required = named.is_some();
    let Some(path) = state::key_file_path(
        vault_id,
        named.map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    ) else {
        return Ok(None);
    };
    if !required && !path.exists() {
        return Ok(None);
    }
    SecretKey::load_file(&path)
        .map(Some)
        .with_context(|| format!("reading the secret key {}", path.display()))
}

async fn serve(
    listen: SocketAddr,
    web: Option<PathBuf>,
    mut app: axum::Router,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
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

    if let Some(web) = web {
        let root = Arc::new(web);
        app = app.fallback(move |req| web::serve(root.clone(), req));
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
