//! `browser-agent`, the browser box's exec service (browser B2; the routes are in lib.rs). Its
//! settings come from the environment:
//!
//! - `BROWSER_TOKEN` (required, at least 16 characters): the bearer token requests carry.
//! - `BROWSER_LISTEN`: the address to listen on, default `0.0.0.0:7700`.
//! - `BROWSER_WORK_DIR`, default `/work`, and `BROWSER_HOME_DIR`, default `/home/agent`.
//! - `DISPLAY`: the desktop commands draw on, default `:1`.
//! - `PLAYWRIGHT_CLI_VERSION`: the installed `@playwright/cli`, which `/health` reports.

use std::path::PathBuf;
use std::process::ExitCode;

use browser_agent::Config;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn main() -> ExitCode {
    let token = std::env::var("BROWSER_TOKEN").unwrap_or_default();
    if token.trim().len() < 16 {
        eprintln!(
            "browser-agent: BROWSER_TOKEN must be set to at least 16 characters; generate one with: openssl rand -hex 32"
        );
        return ExitCode::from(2);
    }
    let config = Config {
        token: token.trim().to_string(),
        work_dir: PathBuf::from(env_or("BROWSER_WORK_DIR", "/work")),
        home_dir: PathBuf::from(env_or("BROWSER_HOME_DIR", "/home/agent")),
        display: env_or("DISPLAY", ":1"),
        playwright_cli: std::env::var("PLAYWRIGHT_CLI_VERSION")
            .ok()
            .filter(|v| !v.is_empty()),
    };
    let listen = env_or("BROWSER_LISTEN", "0.0.0.0:7700");
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("browser-agent: could not start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&listen)
            .await
            .map_err(|e| format!("could not listen on {listen}: {e}"))?;
        eprintln!("browser-agent: listening on {listen}");
        browser_agent::serve(listener, config, shutdown())
            .await
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("browser-agent: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves on SIGTERM or SIGINT.
async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}
