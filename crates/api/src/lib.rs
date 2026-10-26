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
//! a note matched by a key field (D32). Assets, templates, taxonomies, scripts, script tools
//! (`/api/tools`, script-tools T2) and note canvases each have their own routes, `/api/schema`
//! describes every type and taxonomy, and `/api/openapi.json` lists every endpoint.
//!
//! Notifications for the user, from scripts, agents and the server, live under
//! `/api/notifications`. `/api/activity` pages the activity log, admin only (activity Y3). `/api/events` streams notification and note changes to open clients as
//! they happen (D4).
//!
//! The desktop and Android apps pair under `/api/devices`, each getting a `write` token bound to
//! the device (D2, D3). An AI agent with a shell connects through a one-time install code from
//! `/api/agent-codes` and the script `GET /agent/install.sh` serves, the one route outside `/api`
//! (D11, D32).
//!
//! The chat panel talks to the user's AI provider through `/api/ai/chat`, a stream of the
//! answer with the server running read-only note tools (ai D14, D15), or through the turns form
//! under `/api/ai/chat/turns`, which the apps use; `/api/ai/apply` applies a proposed change and
//! `/api/ai/accounts` lists the AI accounts set up (ai D23, accounts A7).
//!
//! Agent Skills live under `/api/skills`: read by any token, written, imported from a zip and
//! deleted with `admin`, and exported as a zip (skills S3).
//!
//! Settings > Integrations > Google signs in with the user's own OAuth client under
//! `/api/integrations/google` (google-sync D1, D18).
//!
//! Settings > Integrations > Brave Search stores the key the chat's `web_search` tool uses under
//! `/api/integrations/brave` (ai D26).
//!
//! Settings > Integrations > Browser box reads the browser box's health at
//! `/api/integrations/browser`, and `/browser/` shows its desktop to a password login (browser
//! B4).
//!
//! Settings > AI keeps the named AI accounts, their keys and their ChatGPT sign-ins under
//! `/api/integrations/ai`, and `/api/ai/models` lists an account's models (ai D6, D8, D15,
//! accounts A7).
//!
//! Scheduled tasks live under `/api/tasks`, with their run history and a route to run one now;
//! the secrets they read are written, never read back, under `/api/secrets` (D10).
//!
//! Devices sync through `/api/sync`: an unauthenticated version handshake and a change feed of
//! every note and definition file, and record the base versions they hold under
//! `/api/devices/{id}/bases` (D20, D27, D29).
//!
//! `/api/clients` lists the desktop and Android app builds and the `vault` CLI builds the server
//! hands out, and
//! `/api/clients/{filename}` downloads one; both are public, so a phone can fetch the app before
//! it pairs.
//!
//! Every route but health, login, device pairing, agent install codes' redemption and script, the sync handshake and the app downloads needs a session cookie or a bearer token,
//! and the scope [`auth::required_scope`] names for it (D11). [`owner_router`] serves the same
//! routes to an in-process caller that is always the owner (D26).
//!
//! [`web::serve`] serves the built web client outside `/api` (D18).

mod activity;
mod agent_files;
mod agent_memory;
mod agent_runs;
mod agent_threads;
mod agents;
mod ai_activity;
mod ai_agents;
mod ai_assets;
mod ai_browser;
mod ai_chat;
mod ai_outbox;
mod ai_saved;
mod ai_scripts;
mod ai_settings;
mod ai_skill_changes;
mod ai_skills;
mod ai_subagents;
mod ai_taxonomies;
mod ai_turns;
mod ai_web;
mod apps;
mod assets;
mod auth;
mod batch;
mod brave;
mod browser;
mod defs;
mod devices;
mod error;
mod events;
mod google;
mod inspect;
mod notes;
mod notifications;
mod openapi;
mod pins;
mod query;
mod script_tools;
mod skills;
mod sync;
mod tasks;
mod tokens;
mod types;
pub mod web;

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::handler::Handler;
use axum::http::{Method, StatusCode};
use axum::routing::{MethodRouter, delete, get, patch, post, put};
use axum::{Json, Router, middleware};
use scheduler::{Scheduler, TestClock};
use serde_json::{Value, json};
use writer::Writer;

pub use apps::watch as watch_clients;
pub use auth::{
    Caller, OWNER_SESSION, SESSION_COOKIE, is_agent_file_write, is_human_only, required_scope,
};
pub use devices::DEFAULT_SKIP_PATTERN;
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

    /// A PUT whose body may be up to `limit` bytes.
    fn put_up_to<H: Handler<T, S>, T: 'static>(
        self,
        path: &'static str,
        limit: usize,
        h: H,
    ) -> Self {
        self.add(
            Method::PUT,
            path,
            put(h).layer(DefaultBodyLimit::max(limit)),
        )
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
    /// The app downloads, which take the clients folder.
    apps: Routes<Arc<apps::Folder>>,
    /// The Google sign-in routes, which take the sign-ins started.
    google: Routes<Arc<google::Google>>,
    /// The Settings > AI routes, which take the ChatGPT sign-ins started.
    ai: Routes<Arc<ai_settings::Ai>>,
    /// The browser box's status, which takes where the box is.
    browser: Routes<Arc<browser::Config>>,
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
        .get("/api/planner/{period}/{date}", notes::planner)
        .post("/api/planner/{period}/{date}", notes::open_planner)
        .get("/api/pins", pins::list)
        .put("/api/pins/{id}", pins::pin)
        .delete("/api/pins/{id}", pins::unpin)
        .post("/api/pins/folders", pins::create_folder)
        .put("/api/pins/folders/{id}", pins::rename_folder)
        .delete("/api/pins/folders/{id}", pins::delete_folder)
        .post("/api/pins/move", pins::move_entry)
        .get("/api/events", events::stream)
        .get("/api/activity", activity::list)
        .post("/api/ai/chat", ai_chat::chat)
        .get("/api/notifications", notifications::list)
        .post("/api/notifications", notifications::create)
        .delete("/api/notifications", notifications::clear_read)
        .post("/api/notifications/read-all", notifications::read_all)
        .post("/api/notifications/{id}/read", notifications::read)
        .delete("/api/notifications/{id}", notifications::delete)
        .get("/api/trash", notes::trash)
        .post("/api/trash/{id}/restore", notes::restore_from_trash)
        .post("/api/trash/empty", notes::empty_trash)
        .delete("/api/trash/{id}", notes::purge_from_trash)
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
        .get("/api/tools", script_tools::list)
        .get("/api/tools/{name}", script_tools::read)
        .put("/api/tools/{name}", script_tools::put)
        .delete("/api/tools/{name}", script_tools::delete)
        .post("/api/tools/{name}/run", script_tools::run_route)
        .get("/api/agents", agent_files::list)
        .get("/api/agents/{name}", agent_files::read)
        .put("/api/agents/{name}", agent_files::put)
        .delete("/api/agents/{name}", agent_files::delete)
        .get("/api/skills", skills::list)
        .post_up_to("/api/skills/import", skills::MAX_IMPORT, skills::import)
        .get("/api/skills/{name}", skills::read)
        .put_up_to("/api/skills/{name}", skills::MAX_SKILL_MD, skills::put)
        .delete("/api/skills/{name}", skills::delete)
        .get("/api/skills/{name}/export", skills::export)
        .get("/api/skills/{name}/files/{*path}", skills::file)
        .put_up_to(
            "/api/skills/{name}/files/{*path}",
            skills::MAX_FILE_UPLOAD,
            skills::put_file,
        )
        .delete("/api/skills/{name}/files/{*path}", skills::delete_file)
        .get("/api/agents/calls/{call}", agent_runs::call)
        .get("/api/agents/{name}/runs", agent_runs::list)
        .get("/api/agents/{name}/threads", agent_threads::list)
        .post("/api/agents/{name}/threads", agent_threads::start)
        .get("/api/agents/{name}/threads/{id}", agent_threads::read)
        .patch("/api/agents/{name}/threads/{id}", agent_threads::rename)
        .delete("/api/agents/{name}/threads/{id}", agent_threads::delete)
        .post(
            "/api/agents/{name}/threads/{id}/truncate",
            agent_threads::truncate,
        )
        .post("/api/agents/{name}/threads/{id}/fork", agent_threads::fork)
        .get("/api/agents/{name}/memories", agent_memory::list)
        .post("/api/agents/{name}/memories", agent_memory::create_route)
        .patch(
            "/api/agents/{name}/memories/{id}",
            agent_memory::update_route,
        )
        .delete(
            "/api/agents/{name}/memories/{id}",
            agent_memory::delete_route,
        )
        .get("/api/assets", assets::list)
        .post_up_to("/api/assets", assets::MAX_UPLOAD, assets::upload)
        .post("/api/assets/cleanup", assets::cleanup)
        .get("/api/assets/trash", assets::trash)
        .post("/api/assets/trash/restore", assets::restore)
        .get("/api/assets/{id}/usage", assets::usage)
        .patch("/api/assets/{id}", assets::rename)
        .get("/api/assets/{id}", assets::download)
        .put_up_to("/api/assets/{id}", assets::MAX_UPLOAD, assets::put)
        .delete("/api/assets/{id}", assets::delete)
        .post("/api/devices/pair", devices::pair)
        .post("/api/devices/pair-codes", devices::create_pair_code)
        .post("/api/agent-codes", agents::create)
        .post("/api/agent-codes/redeem", agents::redeem)
        .get("/agent/install.sh", agents::install_script)
        .get("/api/devices", devices::list)
        .patch("/api/devices/{id}", devices::update)
        .delete("/api/devices/{id}", devices::revoke)
        .put("/api/devices/{id}/push", devices::put_push)
        .put("/api/devices/{id}/health", devices::put_health)
        .put("/api/devices/{id}/bases", sync::put_bases)
        .get("/api/sync/info", sync::info)
        .get("/api/version", sync::version)
        .get("/api/sync/changes", sync::changes)
        .get("/api/devices/{id}/commands", devices::commands)
        .post("/api/devices/{id}/commands", devices::queue_command)
        .post(
            "/api/devices/{id}/commands/{cid}/result",
            devices::command_result,
        )
        .post("/api/devices/{id}/event-failures", devices::event_failure)
        .post("/api/devices/{id}/sync-rounds", devices::sync_round)
        .get("/api/device-files", devices::files)
        .get("/api/device-files/{platform}", devices::file)
        .put("/api/device-files/{platform}", devices::put_file)
        .delete("/api/device-files/{platform}", devices::delete_file)
        .get("/api/tokens", tokens::list)
        .post("/api/tokens", tokens::create)
        .delete("/api/tokens/{name}", tokens::revoke)
        .get("/api/secrets", tasks::secrets)
        .get("/api/secrets/{name}", tasks::secret)
        .put("/api/secrets/{name}", tasks::put_secret)
        .delete("/api/secrets/{name}", tasks::delete_secret)
        .get("/api/integrations/brave", brave::status)
        .put("/api/integrations/brave/key", brave::put_key)
        .post("/api/integrations/brave/test", brave::test);
    let tasks = Routes::new()
        .get("/api/tasks", tasks::list)
        .get("/api/tasks/{name}", tasks::read)
        .put("/api/tasks/{name}", tasks::put)
        .delete("/api/tasks/{name}", tasks::delete)
        .get("/api/cron", tasks::cron)
        .get("/api/tasks/{name}/runs", tasks::runs)
        .get("/api/tasks/{name}/runs/{id}", tasks::run)
        .post("/api/tasks/{name}/run", tasks::trigger);
    let apps = Routes::new()
        .get("/api/clients", apps::list)
        .get("/api/clients/{filename}", apps::download)
        .get("/api/clients/latest/{kind}", apps::latest)
        .get("/api/clients/cli/{target}", apps::cli)
        .get(
            "/api/clients/update/{target}/{arch}/{current_version}",
            apps::update,
        );
    let google = Routes::new()
        .get("/api/integrations/google", google::status)
        .put("/api/integrations/google/settings", google::put_settings)
        .post("/api/integrations/google/start", google::start)
        .post("/api/integrations/google/finish", google::finish)
        .post("/api/integrations/google/disconnect", google::disconnect);
    let ai = Routes::new()
        .get("/api/integrations/ai", ai_settings::status)
        .put("/api/integrations/ai/settings", ai_settings::put_settings)
        .put("/api/integrations/ai/default", ai_settings::put_default)
        .post("/api/integrations/ai/accounts", ai_settings::create_account)
        .put(
            "/api/integrations/ai/accounts/order",
            ai_settings::put_order,
        )
        .put(
            "/api/integrations/ai/accounts/{id}",
            ai_settings::update_account,
        )
        .delete(
            "/api/integrations/ai/accounts/{id}",
            ai_settings::delete_account,
        )
        .put(
            "/api/integrations/ai/accounts/{id}/key",
            ai_settings::put_key,
        )
        .post("/api/integrations/ai/accounts/{id}/test", ai_settings::test)
        .post(
            "/api/integrations/ai/accounts/{id}/chatgpt/start",
            ai_settings::chatgpt_start,
        )
        .post(
            "/api/integrations/ai/accounts/{id}/chatgpt/finish",
            ai_settings::chatgpt_finish,
        )
        .post(
            "/api/integrations/ai/accounts/{id}/chatgpt/disconnect",
            ai_settings::chatgpt_disconnect,
        )
        .get("/api/ai/models", ai_settings::models)
        .get("/api/ai/accounts", ai_settings::accounts)
        .post("/api/ai/chat/turns", ai_turns::start)
        .get("/api/ai/chat/turns/{turn}/events", ai_turns::events)
        .delete("/api/ai/chat/turns/{turn}", ai_turns::stop)
        .post("/api/ai/apply", ai_turns::apply)
        .post("/api/ai/discard", ai_turns::discard);
    let browser = Routes::new().get("/api/integrations/browser", browser::status);
    Built {
        api,
        login,
        tasks,
        apps,
        google,
        ai,
        browser,
    }
}

/// Every method and path the router registers, in registration order.
pub fn routes() -> Vec<(Method, &'static str)> {
    let Built {
        api,
        login,
        tasks,
        apps,
        google,
        ai,
        browser,
    } = build();
    api.seen
        .into_iter()
        .chain(login.seen)
        .chain(tasks.seen)
        .chain(apps.seen)
        .chain(google.seen)
        .chain(ai.seen)
        .chain(browser.seen)
        .collect()
}

/// Builds the router for every `/api` route. `test_clock` is the clock `scheduler` reads when the
/// server runs with `--test-clock`; it adds `/api/test/clock`, which [`routes`] does not list (S7).
/// `clients` is the folder of app builds `/api/clients` hands out, or `None` for none.
pub fn router(
    writer: Arc<Writer>,
    scheduler: Arc<Scheduler>,
    test_clock: Option<Arc<TestClock>>,
    clients: Option<PathBuf>,
) -> Router {
    let sessions = Arc::new(auth::Sessions::new(writer.clone()));
    // Only the server runs agent triggers (agents G10): the embedded app servers come through
    // `owner_router`, which installs no runner.
    scheduler.set_agent_runner(Arc::new(agent_runs::TriggerRunner::new(writer.clone())));
    // Only the server runs agents' scripts (script-tools T1), on the host the tasks run on.
    script_tools::register(&writer, &scheduler);
    let browser = browser::Config::from_env();
    let mut app = assemble(
        writer,
        scheduler,
        test_clock,
        sessions.clone(),
        clients,
        browser.clone(),
    );
    // The desktop is the signed-in user's alone (browser B4), so the embedded app servers, which
    // answer as the owner, never get it.
    if let Some(desktop) = browser::desktop_router(&browser) {
        app = app.merge(desktop);
    }
    app.layer(middleware::from_fn_with_state(sessions, auth::authenticate))
}

/// Builds the router for every `/api` route with no credentials checked: every request acts as
/// the owner's session. Only for a caller in the same process, such as an app's webview through
/// its custom URI scheme, which nothing else can reach (D26). Never serve it on a port.
pub fn owner_router(writer: Arc<Writer>, scheduler: Arc<Scheduler>) -> Router {
    let sessions = Arc::new(auth::Sessions::new(writer.clone()));
    assemble(
        writer,
        scheduler,
        None,
        sessions,
        None,
        browser::Config::default(),
    )
    .layer(middleware::from_fn(auth::as_owner))
}

fn assemble(
    writer: Arc<Writer>,
    scheduler: Arc<Scheduler>,
    test_clock: Option<Arc<TestClock>>,
    sessions: Arc<auth::Sessions>,
    clients: Option<PathBuf>,
    browser_config: browser::Config,
) -> Router {
    let Built {
        api,
        login,
        tasks,
        apps,
        google,
        ai,
        browser,
    } = build();
    let writer_for_ai = writer.clone();
    let mut app = api
        .router
        .with_state(writer)
        .merge(login.router.with_state(sessions))
        .merge(tasks.router.with_state(scheduler.clone()))
        .merge(apps.router.with_state(Arc::new(apps::Folder(clients))))
        .merge(
            google
                .router
                .with_state(Arc::new(google::Google::new(scheduler.clone()))),
        )
        .merge(
            ai.router
                .with_state(Arc::new(ai_settings::Ai::new(writer_for_ai))),
        )
        .merge(browser.router.with_state(Arc::new(browser_config)));
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
