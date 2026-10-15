//! The HTTP API: routes, password session and token auth, scope checks and conflict responses.
//!
//! Notes are read and written as WCL text or JSON (D32). A request whose `Content-Type` is
//! `application/json` sends `{ "type"?, "base_version"?, "note" | "wcl" }`, where `note` is the
//! JSON form of [`writer::Input::Json`]; any other body is the note's WCL text, with the base
//! version in `If-Match`. A response is JSON unless `Accept` asks for `text/plain` or WCL, in
//! which case the body is the canonical WCL on disk and the version is in `ETag` (D21).
//!
//! Queries run at `/api/query` and full-text search at `/api/search`, a page at a time with a
//! cursor (D33). `/api/query/parse` and `/api/query/text` convert between text and the visual
//! builder's model (D8).
//!
//! `/api/batch` applies many note writes in one commit, or dry-runs them, and `/api/upsert` writes
//! a note matched by a key field (D32). Assets, templates, taxonomies, scripts and note canvases
//! each have their own routes, `/api/schema` describes every type and taxonomy, and
//! `/api/openapi.json` lists every endpoint.
//!
//! Notifications for the user, from scripts, agents and the server, live under
//! `/api/notifications`. `/api/events` streams notification and note changes to open clients as
//! they happen (D4).
//!
//! The desktop and Android apps pair under `/api/devices`, each getting a `write` token bound to
//! the device (D2, D3).
//!
//! Scheduled tasks live under `/api/tasks`, with their run history and a route to run one now;
//! the secrets they read are written, never read back, under `/api/secrets` (D10).
//!
//! Every route but health, login and device pairing needs a session cookie or a bearer token,
//! and the scope [`auth::required_scope`] names for it (D11). [`owner_router`] serves the same
//! routes to an in-process caller that is always the owner (D26).
//!
//! [`web::serve`] serves the built web client outside `/api` (D18).

mod assets;
mod auth;
mod batch;
mod defs;
mod devices;
mod error;
mod events;
mod inspect;
mod notes;
mod notifications;
mod openapi;
mod pins;
mod query;
mod tasks;
mod tokens;
mod types;
pub mod web;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::handler::Handler;
use axum::http::{Method, StatusCode};
use axum::routing::{MethodRouter, delete, get, patch, post, put};
use axum::{Json, Router, middleware};
use scheduler::{Scheduler, TestClock};
use serde_json::{Value, json};
use writer::Writer;

pub use auth::{Caller, OWNER_SESSION, SESSION_COOKIE, required_scope};
pub use error::ApiError;
pub use openapi::{ENDPOINTS, document as openapi_document};

/// The largest batch body accepted, in bytes.
const MAX_BATCH: usize = 64 * 1024 * 1024;

/// A router that records every method and path it registers, so the endpoint listing can be
/// checked against what is actually served.
struct Routes<S> {
    router: Router<S>,
    seen: Vec<(Method, &'static str)>,
}

impl<S: Clone + Send + Sync + 'static> Routes<S> {
    fn new() -> Self {
        Routes {
            router: Router::new(),
            seen: Vec::new(),
        }
    }

    fn add(mut self, method: Method, path: &'static str, route: MethodRouter<S>) -> Self {
        self.router = self.router.route(path, route);
        self.seen.push((method, path));
        self
    }

    fn get<H: Handler<T, S>, T: 'static>(self, path: &'static str, h: H) -> Self {
        self.add(Method::GET, path, get(h))
    }

    fn post<H: Handler<T, S>, T: 'static>(self, path: &'static str, h: H) -> Self {
        self.add(Method::POST, path, post(h))
    }

    fn put<H: Handler<T, S>, T: 'static>(self, path: &'static str, h: H) -> Self {
        self.add(Method::PUT, path, put(h))
    }

    fn delete<H: Handler<T, S>, T: 'static>(self, path: &'static str, h: H) -> Self {
        self.add(Method::DELETE, path, delete(h))
    }

    fn patch<H: Handler<T, S>, T: 'static>(self, path: &'static str, h: H) -> Self {
        self.add(Method::PATCH, path, patch(h))
    }

    /// A POST whose body may be up to `limit` bytes.
    fn post_up_to<H: Handler<T, S>, T: 'static>(
        self,
        path: &'static str,
        limit: usize,
        h: H,
    ) -> Self {
        self.add(
            Method::POST,
            path,
            post(h).layer(DefaultBodyLimit::max(limit)),
        )
    }
}

/// Every route, grouped by the state it takes.
struct Built {
    /// The routes that take the writer.
    api: Routes<Arc<Writer>>,
    /// The login routes, which take the session store.
    login: Routes<Arc<auth::Sessions>>,
    /// The task routes, which take the scheduler.
    tasks: Routes<Arc<Scheduler>>,
}

fn build() -> Built {
    let login = Routes::new()
        .post("/api/login", auth::login)
        .post("/api/logout", auth::logout);
    let api = Routes::new()
        .get("/api/health", health)
        .get("/api/openapi.json", openapi::serve)
        .get("/api/schema", inspect::schema)
        .get("/api/notes", notes::list)
        .post("/api/notes", notes::create)
        .get("/api/notes/{id}", notes::read)
        .put("/api/notes/{id}", notes::update)
        .delete("/api/notes/{id}", notes::delete)
        .get("/api/notes/{id}/history", notes::history)
        .get("/api/notes/{id}/graph", notes::graph)
        .get("/api/notes/{id}/history/{version}", notes::version)
        .post("/api/notes/{id}/restore", notes::restore_version)
        .post("/api/notes/{id}/archive", notes::archive)
        .post("/api/notes/{id}/unarchive", notes::unarchive)
        .get("/api/notes/{id}/canvas/{name}", defs::canvas)
        .put("/api/notes/{id}/canvas/{name}", defs::put_canvas)
        .post_up_to("/api/batch", MAX_BATCH, batch::run)
        .post("/api/upsert", batch::upsert)
        .get("/api/query", query::run_get)
        .post("/api/query", query::run_post)
        .post("/api/query/parse", query::parse_query)
        .post("/api/query/text", query::model_text)
        .get("/api/search", query::search)
        .get("/api/daily/{date}", notes::daily)
        .post("/api/daily/{date}", notes::open_daily)
        .get("/api/pins", pins::list)
        .put("/api/pins/{id}", pins::pin)
        .delete("/api/pins/{id}", pins::unpin)
        .post("/api/pins/folders", pins::create_folder)
        .put("/api/pins/folders/{id}", pins::rename_folder)
        .delete("/api/pins/folders/{id}", pins::delete_folder)
        .post("/api/pins/move", pins::move_entry)
        .get("/api/events", events::stream)
        .get("/api/notifications", notifications::list)
        .post("/api/notifications", notifications::create)
        .delete("/api/notifications", notifications::clear_read)
        .post("/api/notifications/read-all", notifications::read_all)
        .post("/api/notifications/{id}/read", notifications::read)
        .delete("/api/notifications/{id}", notifications::delete)
        .get("/api/trash", notes::trash)
        .post("/api/trash/{id}/restore", notes::restore_from_trash)
        .get("/api/types", types::list)
        .get("/api/types/{name}", types::read)
        .put("/api/types/{name}", types::put)
        .delete("/api/types/{name}", types::delete)
        .post("/api/types/{name}/change", types::change)
        .get("/api/flagged", types::flagged)
        .get("/api/types/{name}/templates", defs::templates)
        .get("/api/types/{name}/templates/{template}", defs::template)
        .put("/api/types/{name}/templates/{template}", defs::put_template)
        .delete(
            "/api/types/{name}/templates/{template}",
            defs::delete_template,
        )
        .get("/api/taxonomies", defs::taxonomies)
        .get("/api/taxonomies/{name}", defs::taxonomy)
        .put("/api/taxonomies/{name}", defs::put_taxonomy)
        .delete("/api/taxonomies/{name}", defs::delete_taxonomy)
        .get("/api/scripts", defs::scripts)
        .get("/api/scripts/{name}", defs::script)
        .put("/api/scripts/{name}", defs::put_script)
        .delete("/api/scripts/{name}", defs::delete_script)
        .get("/api/assets", assets::list)
        .post_up_to("/api/assets", assets::MAX_UPLOAD, assets::upload)
        .get("/api/assets/{id}", assets::download)
        .delete("/api/assets/{id}", assets::delete)
        .post("/api/devices/pair", devices::pair)
        .post("/api/devices/pair-codes", devices::create_pair_code)
        .get("/api/devices", devices::list)
        .patch("/api/devices/{id}", devices::update)
        .delete("/api/devices/{id}", devices::revoke)
        .put("/api/devices/{id}/push", devices::put_push)
        .get("/api/tokens", tokens::list)
        .post("/api/tokens", tokens::create)
        .delete("/api/tokens/{name}", tokens::revoke)
        .get("/api/secrets", tasks::secrets)
        .get("/api/secrets/{name}", tasks::secret)
        .put("/api/secrets/{name}", tasks::put_secret)
        .delete("/api/secrets/{name}", tasks::delete_secret);
    let tasks = Routes::new()
        .get("/api/tasks", tasks::list)
        .get("/api/tasks/{name}", tasks::read)
        .put("/api/tasks/{name}", tasks::put)
        .delete("/api/tasks/{name}", tasks::delete)
        .get("/api/cron", tasks::cron)
        .get("/api/tasks/{name}/runs", tasks::runs)
        .get("/api/tasks/{name}/runs/{id}", tasks::run)
        .post("/api/tasks/{name}/run", tasks::trigger);
    Built { api, login, tasks }
}

/// Every method and path the router registers, in registration order.
pub fn routes() -> Vec<(Method, &'static str)> {
    let Built { api, login, tasks } = build();
    api.seen
        .into_iter()
        .chain(login.seen)
        .chain(tasks.seen)
        .collect()
}

/// Builds the router for every `/api` route. `test_clock` is the clock `scheduler` reads when the
/// server runs with `--test-clock`; it adds `/api/test/clock`, which [`routes`] does not list (S7).
pub fn router(
    writer: Arc<Writer>,
    scheduler: Arc<Scheduler>,
    test_clock: Option<Arc<TestClock>>,
) -> Router {
    let sessions = Arc::new(auth::Sessions::new(writer.clone()));
    assemble(writer, scheduler, test_clock, sessions.clone())
        .layer(middleware::from_fn_with_state(sessions, auth::authenticate))
}

/// Builds the router for every `/api` route with no credentials checked: every request acts as
/// the owner's session. Only for a caller in the same process, such as an app's webview through
/// its custom URI scheme, which nothing else can reach (D26). Never serve it on a port.
pub fn owner_router(writer: Arc<Writer>, scheduler: Arc<Scheduler>) -> Router {
    let sessions = Arc::new(auth::Sessions::new(writer.clone()));
    assemble(writer, scheduler, None, sessions).layer(middleware::from_fn(auth::as_owner))
}

fn assemble(
    writer: Arc<Writer>,
    scheduler: Arc<Scheduler>,
    test_clock: Option<Arc<TestClock>>,
    sessions: Arc<auth::Sessions>,
) -> Router {
    let Built { api, login, tasks } = build();
    let mut app = api
        .router
        .with_state(writer)
        .merge(login.router.with_state(sessions))
        .merge(tasks.router.with_state(scheduler.clone()));
    if let Some(clock) = test_clock {
        app = app.merge(
            Router::new()
                .route("/api/test/clock", get(tasks::clock).post(tasks::move_clock))
                .with_state(tasks::TestTime { scheduler, clock }),
        );
    }
    app
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Run blocking work against the writer off the async runtime.
pub(crate) async fn blocking<T, E>(
    writer: Arc<Writer>,
    f: impl FnOnce(&Writer) -> Result<T, E> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    E: Into<ApiError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || f(&writer))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(Into::into)
}
