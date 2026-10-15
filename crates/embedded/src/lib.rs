//! The server stack in the apps' own process (D18, D26).
//!
//! [`Embedded::open`] opens a local vault folder and builds the same `/api` router as
//! `vault-server`, from the same handlers. [`Embedded::handle`] answers one request, so a Tauri
//! custom URI scheme handler can serve the webview with no network port (D26). Requests act as the
//! owner's session: nothing outside the app's own webview reaches the scheme, so no login is
//! needed between the webview and its own server. Paths outside `/api` serve the built web client
//! from [`Config::web`], as `vault-server --web` does.
//!
//! The device is not the home server, so it runs neither the scheduler nor `after_*` handlers
//! (D17). Validation and `before_*` handlers run on every write here, and again on the home server
//! when the write arrives. `after_*` handlers are not queued at all, and a request to run a task
//! now answers 409.
//!
//! [`Embedded::handle`] takes and returns streaming bodies, so an upload is never held twice and a
//! `Range` request on an asset answers `206 Partial Content` with just that range (pdf.js). It
//! must be awaited inside a Tokio runtime, which Tauri's async runtime is.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::body::{Body, HttpBody};
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use axum::response::IntoResponse;
use schema::{Doc, Schema};
use state::{Author, SecretKey};
use tower::ServiceExt;
use writer::{Hooks, Migration, NoteChange, Saved, ScriptRole, Writer};

/// Workers for the script host the router's task routes take. Tasks never run on a device, so it
/// only ever checks task files.
const SCRIPT_WORKERS: usize = 1;

/// What [`Embedded::open`] serves.
pub struct Config {
    /// The local copy of the vault.
    pub vault: PathBuf,
    /// The built web client, such as `web/dist`, served outside `/api`. `None` serves only `/api`.
    pub web: Option<PathBuf>,
    /// The key that encrypts secrets (OQ-S2). Without one, secrets can be listed but not written
    /// or read.
    pub secret_key: Option<SecretKey>,
}

/// The in-process server of one local vault.
pub struct Embedded {
    /// Always `Some` until dropped.
    parts: Option<Parts>,
}

struct Parts {
    app: Router,
    writer: Arc<Writer>,
    /// Keeps the `before_*` handler host alive; it holds only a weak link to the writer.
    _handlers: script::Handlers,
}

impl Drop for Embedded {
    /// The script hosts hold blocking HTTP clients, which panic when dropped inside an async
    /// runtime, so the parts are dropped on a thread of their own.
    fn drop(&mut self) {
        if let Some(parts) = self.parts.take() {
            let _ = std::thread::spawn(move || drop(parts)).join();
        }
    }
}

impl Embedded {
    /// Opens the vault folder in `config`, creating its server files when they are missing. It
    /// blocks while it loads the vault. The script hosts' blocking HTTP clients cannot be made
    /// inside an async runtime, so the work runs on a thread of its own and `open` may be called
    /// from anywhere.
    pub fn open(config: Config) -> anyhow::Result<Embedded> {
        std::thread::scope(|s| s.spawn(|| Embedded::open_here(config)).join())
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    fn open_here(config: Config) -> anyhow::Result<Embedded> {
        if !config.vault.is_dir() {
            anyhow::bail!("vault folder {} does not exist", config.vault.display());
        }
        if let Some(web) = &config.web
            && !web.join("index.html").is_file()
        {
            anyhow::bail!("web folder {} has no index.html", web.display());
        }
        let vault = store::Vault::init(&config.vault)
            .with_context(|| format!("opening vault {}", config.vault.display()))?;
        let state = state::State::open(&vault.server_dir().join("state.db"), config.secret_key)
            .context("opening state.db")?;
        let handlers = script::Handlers::new(script::Limits::default());
        let writer = Arc::new(
            writer::Writer::open(vault, state)
                .context("loading the vault")?
                .with_hooks(Box::new(LocalHooks(handlers.hooks()))),
        );
        writer
            .ensure_built_in_types()
            .context("adding the built-in types")?;
        // Bound, never started: no queue thread runs.
        handlers.bind(&writer);
        let host = Arc::new(script::Host::new(
            writer.clone(),
            SCRIPT_WORKERS,
            script::Limits::default(),
        ));
        // Never started: no scheduler thread runs.
        let scheduler =
            scheduler::Scheduler::new(writer.clone(), host, Arc::new(state::SystemClock))
                .context("loading the scheduled tasks")?;
        let mut app = api::owner_router(writer.clone(), scheduler)
            .layer(axum::middleware::from_fn(refuse_task_runs));
        if let Some(web) = config.web {
            let root = Arc::new(web);
            app = app.fallback(move |req| api::web::serve(root.clone(), req));
        }
        Ok(Embedded {
            parts: Some(Parts {
                app,
                writer,
                _handlers: handlers,
            }),
        })
    }

    /// The local vault's writer, for in-process callers such as device scripts and sync.
    pub fn writer(&self) -> &Arc<Writer> {
        &self.parts().writer
    }

    fn parts(&self) -> &Parts {
        self.parts
            .as_ref()
            .expect("an open embedded server has its parts")
    }

    /// Answers one request as the owner. The body streams in and out.
    pub async fn handle<B>(&self, req: Request<B>) -> Response<Body>
    where
        B: HttpBody<Data = axum::body::Bytes> + Send + 'static,
        B::Error: Into<axum::BoxError>,
    {
        let req = req.map(Body::new);
        match self.parts().app.clone().oneshot(req).await {
            Ok(res) => res,
            Err(never) => match never {},
        }
    }
}

/// `POST /api/tasks/{name}/run` answers 409: tasks run only on the home server (D17).
async fn refuse_task_runs(req: Request<Body>, next: axum::middleware::Next) -> Response<Body> {
    let path = req.uri().path();
    let is_run = req.method() == Method::POST
        && path
            .strip_prefix("/api/tasks/")
            .and_then(|rest| rest.strip_suffix("/run"))
            .is_some_and(|name| !name.is_empty() && !name.contains('/'));
    if !is_run {
        return next.run(req).await;
    }
    let mut res = (
        StatusCode::CONFLICT,
        r#"{"error":"home_server_only","message":"tasks run only on the home server"}"#,
    )
        .into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}

/// The writer hooks of a device: the script handlers' with `after_*` dropped (D17).
struct LocalHooks(Box<dyn Hooks>);

impl Hooks for LocalHooks {
    fn before(&self, change: &mut NoteChange, author: &Author) -> Result<(), String> {
        self.0.before(change, author)
    }

    fn side_effects(&self, changes: &[NoteChange]) -> writer::Result<Vec<NoteChange>> {
        self.0.side_effects(changes)
    }

    fn after(&self, _saved: &[Saved], _author: &Author, _schema: &Schema) {}

    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> Result<Option<Doc>, String> {
        self.0.migrate(type_name, migration, id, doc)
    }

    fn check_script(&self, role: ScriptRole, name: &str, source: &str) -> Result<(), String> {
        self.0.check_script(role, name, source)
    }
}
