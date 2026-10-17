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
//! when the write arrives. `after_*` handlers are not queued at all.
//!
//! Routes backed by state only the home server holds, such as the paired devices, the
//! notifications and the task runs, are not answered from the local `state.db`, which has none of
//! it: once [`Embedded::set_remote`] pairs the server, they go to the home server with the device
//! token, and answer 503 while it cannot be reached or the device is not paired (`home`). Running
//! a task now is one of them, since tasks run only on the home server.
//!
//! Every local note write is queued for the home server in [`sync::Queue`], beside the server
//! tables in the local `state.db`. [`Embedded::sync_engine`] builds the engine that pushes the
//! queue and pulls the server's changes (D19, D20); what it pulls is not queued back.
//!
//! [`Embedded::handle`] takes and returns streaming bodies, so an upload is never held twice and a
//! `Range` request on an asset answers `206 Partial Content` with just that range (pdf.js). It
//! must be awaited inside a Tokio runtime, which Tauri's async runtime is.
//!
//! [`Embedded::device`] adds the device event runtime of [`device`]: the durable queue of device
//! events, their handlers, and the commands server scripts send the device (D5, D6, D22).

pub mod device;
mod home;

pub use home::{NEEDS_WEB_APP_MESSAGE, NOT_PAIRED_MESSAGE, OFFLINE_MESSAGE};

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, PoisonError, RwLock, Weak};

use anyhow::Context;
use axum::Router;
use axum::body::{Body, HttpBody};
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use home::{Home, Route, Unavailable};
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

/// The app build an embedded server runs in, which `GET /api/app` reports for Settings > General.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AppBuild {
    /// The app's version, from its `tauri.conf.json`.
    pub version: String,
    /// How the app runs: `linux-appimage`, `linux-package` (a deb or rpm), `linux` (any other
    /// binary) or `android`.
    pub platform: String,
    /// `release` or `debug`.
    pub build: String,
}

/// The in-process server of one local vault.
pub struct Embedded {
    /// Always `Some` until dropped.
    parts: Option<Parts>,
}

struct Parts {
    app: Router,
    writer: Arc<Writer>,
    queue: Arc<sync::Queue>,
    home: Home,
    /// The app build, once the app says it ([`Embedded::set_app`]).
    build: RwLock<Option<AppBuild>>,
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
        let state_path = vault.server_dir().join("state.db");
        let state =
            state::State::open(&state_path, config.secret_key).context("opening state.db")?;
        let queue = Arc::new(sync::Queue::open(&state_path).context("opening the sync queue")?);
        let handlers = script::Handlers::new(script::Limits::default());
        let bound = Arc::new(OnceLock::new());
        let writer = Arc::new(
            writer::Writer::open(vault, state)
                .context("loading the vault")?
                .with_hooks(Box::new(LocalHooks {
                    inner: handlers.hooks(),
                    queue: queue.clone(),
                    writer: bound.clone(),
                })),
        );
        let _ = bound.set(Arc::downgrade(&writer));
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
        let mut app = api::owner_router(writer.clone(), scheduler);
        if let Some(web) = config.web {
            let root = Arc::new(web);
            app = app.fallback(move |req| api::web::serve(root.clone(), req));
        }
        Ok(Embedded {
            parts: Some(Parts {
                app,
                writer,
                queue,
                home: Home::new().context("building the home server's HTTP client")?,
                build: RwLock::new(None),
                _handlers: handlers,
            }),
        })
    }

    /// The local vault's writer, for in-process callers such as device scripts and sync.
    pub fn writer(&self) -> &Arc<Writer> {
        &self.parts().writer
    }

    /// The queue of local writes waiting for the home server.
    pub fn queue(&self) -> &Arc<sync::Queue> {
        &self.parts().queue
    }

    /// The sync engine of this vault with its home server (D19, D20). It blocks: run it on a
    /// thread of its own, not inside an async runtime.
    pub fn sync_engine(
        &self,
        remote: sync::Remote,
        assets: sync::AssetPolicy,
    ) -> sync::Result<sync::Engine> {
        sync::Engine::new(self.writer().clone(), self.queue().clone(), remote, assets)
    }

    /// The device event runtime of the device `config` names, over this vault. Open one per
    /// device; it holds the event queue in the local `state.db`.
    pub fn device(&self, config: device::DeviceConfig) -> anyhow::Result<device::DeviceRuntime> {
        let writer = self.writer().clone();
        let state_db = writer.vault().server_dir().join("state.db");
        device::DeviceRuntime::open(writer, &state_db, config)
    }

    fn parts(&self) -> &Parts {
        self.parts
            .as_ref()
            .expect("an open embedded server has its parts")
    }

    /// Pair the embedded server with its home server, or unpair it with `None`. The routes backed
    /// by the home server's own state go there with `remote`'s token ([`home`]).
    pub fn set_remote(&self, remote: Option<sync::Remote>) {
        self.parts().home.set_remote(remote);
    }

    /// Say which app build this server runs in, for `GET /api/app`.
    pub fn set_app(&self, build: AppBuild) {
        *self
            .parts()
            .build
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(build);
    }

    /// The app build [`Embedded::set_app`] gave, if any.
    pub fn app_build(&self) -> Option<AppBuild> {
        self.parts()
            .build
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The home server the embedded server forwards to, once paired.
    pub fn remote(&self) -> Option<sync::Remote> {
        self.parts().home.remote()
    }

    /// Answers one request as the owner. The body streams in and out. Routes backed by the home
    /// server's own state, such as the paired devices and the notifications, are answered by the
    /// home server (`docs/sync.md` lists them); the rest by the local vault.
    pub async fn handle<B>(&self, req: Request<B>) -> Response<Body>
    where
        B: HttpBody<Data = axum::body::Bytes> + Send + 'static,
        B::Error: Into<axum::BoxError>,
    {
        let req = req.map(Body::new);
        let home = &self.parts().home;
        match home::route(req.method(), req.uri().path()) {
            Route::Local => self.local(req).await,
            Route::Forward => home.forward(req).await,
            Route::ForwardOrLocal => {
                let (parts, _) = req.into_parts();
                match home.send(again(&parts, Body::empty())).await {
                    Ok(res) => home::translate(res).await,
                    Err(_) => self.local(again(&parts, Body::empty())).await,
                }
            }
            Route::History => self.history(req).await,
            Route::Version { id, version } => {
                if self.has_version(&id, &version).await {
                    self.local(req).await
                } else {
                    home.forward(req).await
                }
            }
            Route::Restore { id } => self.restore(&id, req).await,
            Route::Events => home.events(self.writer(), req).await,
            Route::App => {
                let remote = home.remote();
                let build = self.app_build();
                home::json_response(
                    StatusCode::OK,
                    &serde_json::json!({
                        "app": true,
                        "paired": remote.is_some(),
                        "device_id": remote.as_ref().map(|r| &r.device_id),
                        "device_name": remote.as_ref().map(|r| &r.device_name),
                        "server_url": remote.as_ref().map(|r| &r.url),
                        "version": build.as_ref().map(|b| &b.version),
                        "platform": build.as_ref().map(|b| &b.platform),
                        "build": build.as_ref().map(|b| &b.build),
                    }),
                )
            }
            Route::ServerVersion => home.server_version().await,
            Route::NeedsWebApp => home::error(
                StatusCode::FORBIDDEN,
                "needs_web_app",
                home::NEEDS_WEB_APP_MESSAGE,
            ),
        }
    }

    async fn local(&self, req: Request<Body>) -> Response<Body> {
        match self.parts().app.clone().oneshot(req).await {
            Ok(res) => res,
            Err(never) => match never {},
        }
    }

    /// Whether the local history holds `version` of note `id`.
    async fn has_version(&self, id: &str, version: &str) -> bool {
        let (writer, id, version) = (self.writer().clone(), id.to_owned(), version.to_owned());
        tokio::task::spawn_blocking(move || writer.version(&id, &version).is_ok())
            .await
            .unwrap_or(false)
    }

    /// `GET /api/notes/{id}/history`: the home server's history, which holds every version from
    /// everywhere, with `source: "server"`. Unpaired, offline, or for a note the server does not
    /// have yet, the versions this device recorded, with `source: "device"`.
    async fn history(&self, req: Request<Body>) -> Response<Body> {
        let (parts, _) = req.into_parts();
        match self.parts().home.send(again(&parts, Body::empty())).await {
            Ok(res) if res.status().is_success() => {
                home::with_key(home::translate(res).await, "source", "server").await
            }
            Ok(res) if res.status() != StatusCode::NOT_FOUND => home::translate(res).await,
            _ => {
                let res = self.local(again(&parts, Body::empty())).await;
                if res.status().is_success() {
                    home::with_key(res, "source", "device").await
                } else {
                    res
                }
            }
        }
    }

    /// `POST /api/notes/{id}/restore`. A version this device recorded is restored here. A version
    /// only the home server holds is fetched from it and written here as a new version of the
    /// note over the request's base, so it syncs like any local edit.
    async fn restore(&self, id: &str, req: Request<Body>) -> Response<Body> {
        #[derive(serde::Deserialize)]
        struct Restore {
            version: String,
            base_version: Option<String>,
        }
        let (parts, body) = req.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, 1024 * 1024).await else {
            return home::error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "the body is too large",
            );
        };
        let asked = serde_json::from_slice::<Restore>(&bytes).ok();
        let local = match &asked {
            None => true,
            Some(r) => self.has_version(id, &r.version).await,
        };
        let Some(asked) = asked.filter(|_| !local) else {
            return self.local(again(&parts, Body::from(bytes))).await;
        };
        let path = format!("/api/notes/{id}/history/{}", asked.version);
        let mut fetch = Request::get(path.as_str())
            .body(Body::empty())
            .unwrap_or_default();
        let (name, value) = home::accept_text();
        fetch.headers_mut().insert(name, value);
        let res = match self.parts().home.send(fetch).await {
            Ok(res) if res.status().is_success() => res,
            Ok(res) => return home::translate(res).await,
            Err(why) => return why.response(),
        };
        let Some(text) = home::read_limited(res)
            .await
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
        else {
            return Unavailable::Offline.response();
        };
        let mut body = serde_json::json!({ "wcl": text });
        if let Some(base) = asked.base_version {
            body["base_version"] = serde_json::json!(base);
        }
        let mut put = again(&parts, Body::from(body.to_string()));
        *put.method_mut() = Method::PUT;
        *put.uri_mut() = match format!("/api/notes/{id}").parse() {
            Ok(uri) => uri,
            Err(_) => return home::error(StatusCode::BAD_REQUEST, "bad_request", "bad note id"),
        };
        put.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        put.headers_mut().remove(header::CONTENT_LENGTH);
        self.local(put).await
    }
}

/// A request with the method, URI and headers of `parts`, and `body`.
fn again(parts: &axum::http::request::Parts, body: Body) -> Request<Body> {
    home::request(parts.method.clone(), &parts.uri, &parts.headers, body)
}

/// The writer hooks of a device: the script handlers' with `after_*` dropped (D17), and every
/// committed note change queued for the home server.
struct LocalHooks {
    inner: Box<dyn Hooks>,
    queue: Arc<sync::Queue>,
    /// The writer these hooks belong to, set once it exists, for the base texts of the queue.
    writer: Arc<OnceLock<Weak<Writer>>>,
}

impl Hooks for LocalHooks {
    fn before(&self, change: &mut NoteChange, author: &Author) -> Result<(), String> {
        self.inner.before(change, author)
    }

    fn side_effects(&self, changes: &[NoteChange]) -> writer::Result<Vec<NoteChange>> {
        self.inner.side_effects(changes)
    }

    /// No `after_*` handlers run on a device (D17); the change is queued for the home server
    /// instead. A queue that cannot be written loses nothing yet, since the note is saved, but
    /// would leave the change behind, so it is reported.
    fn after(&self, saved: &[Saved], _author: &Author, _schema: &Schema) {
        let Some(writer) = self.writer.get().and_then(Weak::upgrade) else {
            return;
        };
        if let Err(e) = self.queue.record(writer.state(), saved) {
            eprintln!("queueing a local change for sync failed: {e}");
        }
    }

    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> Result<Option<Doc>, String> {
        self.inner.migrate(type_name, migration, id, doc)
    }

    fn asset_known(&self, id: &str) -> bool {
        self.queue.asset_on_server(id)
    }

    fn check_script(&self, role: ScriptRole, name: &str, source: &str) -> Result<(), String> {
        self.inner.check_script(role, name, source)
    }
}
