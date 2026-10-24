//! Who is calling, and what they may do (D11).
//!
//! The browser logs in with the vault's password and gets a session cookie; a session is the
//! owner and may do everything. Sessions are kept in `state.db`, so a restart keeps the browser
//! signed in; one lasts 30 days from the last visit, and changing the password ends them all. Agents send `Authorization: Bearer <secret>` for an API token,
//! which carries one [`Scope`]. [`authenticate`] runs before every route: it looks up what the
//! route needs with [`required_scope`], answers 401 when the request carries no valid credentials
//! and 403 when they do not reach that scope, and otherwise hands the handler a [`Caller`].

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use state::{Author, Scope};
use writer::Writer;

use crate::{ApiError, blocking};

/// The name every browser session is recorded under (D13). There is one user.
pub const OWNER_SESSION: &str = "owner";

/// The cookie holding a browser session's id.
pub const SESSION_COOKIE: &str = "tv_session";

/// How long a session lasts after login, and after each renewal while it is in use.
const SESSION_TTL: Duration = Duration::from_millis(state::SESSION_TTL_MS as u64);

/// Paths whose non-read requests need `write`: notes and assets, batches and upserts of notes,
/// and notifications (D11). Any other change needs `admin`, so a new endpoint is admin-only until it is listed here.
const WRITE_PATHS: [&str; 9] = [
    "/api/notes",
    "/api/ai/apply",
    "/api/notifications",
    "/api/daily",
    "/api/pins",
    "/api/trash",
    "/api/assets",
    "/api/batch",
    "/api/upsert",
];

/// Paths whose POST only reads: running a query and converting between query text and the
/// builder's model send the query in the body but change nothing, and an AI chat turn reads
/// notes, its edits being proposals the user applies through the write routes (ai D15); a change
/// the chat makes itself, its confirmation turned off, needs the caller's own `write` or `admin`
/// scope, which the chat's tools check (ai D22).
const READ_POST_PATHS: [&str; 3] = ["/api/query", "/api/ai/chat", "/api/ai/discard"];

/// `/api/agents/{name}/threads` and `/api/agents/{name}/threads/{id}`: `Some(false)` for the
/// list, `Some(true)` for one thread (agents G7).
fn agent_threads_path(path: &str) -> Option<bool> {
    let rest = path.strip_prefix("/api/agents/")?;
    let mut parts = rest.split('/');
    let name = parts.next()?;
    if name.is_empty() || parts.next()? != "threads" {
        return None;
    }
    match (parts.next(), parts.next()) {
        (None, _) => Some(false),
        (Some(id), None) if !id.is_empty() => Some(true),
        _ => None,
    }
}

/// Whether `path` is an agent's memories, `/api/agents/{name}/memories` or one memory under it,
/// whose changes are note writes (agents G21).
fn agent_memories_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/api/agents/") else {
        return false;
    };
    let mut parts = rest.split('/');
    let name = parts.next().unwrap_or_default();
    if name.is_empty() || parts.next() != Some("memories") {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, _) => true,
        (Some(id), None) => !id.is_empty(),
        _ => false,
    }
}

/// Paths that need `admin` even to read, because they disclose or hand out credentials, or, the
/// activity log, show everything that happens in the vault (activity Y3).
const ADMIN_PATHS: [&str; 4] = [
    "/api/tokens",
    "/api/secrets",
    "/api/integrations",
    "/api/activity",
];

/// The authenticated caller of a request, recorded as the author of what it writes (D13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller(pub Author);

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Caller>()
            .cloned()
            .ok_or_else(unauthorized)
    }
}

/// The scope the request's credentials grant, next to its [`Caller`]: a route that binds a token
/// to what it may touch, such as a device's own push endpoint, reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantedScope(pub Scope);

/// Whether `path` is `/api/devices/{id}/push`, `/api/devices/{id}/bases` or
/// `/api/devices/{id}/health`, which a device sets for itself.
fn is_device_own(path: &str) -> bool {
    path.strip_prefix("/api/devices/")
        .and_then(|rest| {
            rest.strip_suffix("/push")
                .or_else(|| rest.strip_suffix("/bases"))
                .or_else(|| rest.strip_suffix("/health"))
        })
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// Whether `path` is `/api/devices/{id}/commands`, where a device queues an import of its own
/// history.
fn is_device_commands(path: &str) -> bool {
    path.strip_prefix("/api/devices/")
        .and_then(|rest| rest.strip_suffix("/commands"))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// Whether `path` is `/api/devices/{id}/event-failures`, where a device reports its device
/// events that failed for good.
fn is_device_event_failures(path: &str) -> bool {
    path.strip_prefix("/api/devices/")
        .and_then(|rest| rest.strip_suffix("/event-failures"))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// Whether `path` is `/api/devices/{id}/sync-rounds`, where a device reports a sync round.
fn is_device_sync_rounds(path: &str) -> bool {
    path.strip_prefix("/api/devices/")
        .and_then(|rest| rest.strip_suffix("/sync-rounds"))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// Whether `path` is `/api/devices/{id}/commands/{cid}/result`.
fn is_device_command_result(path: &str) -> bool {
    path.strip_prefix("/api/devices/")
        .and_then(|rest| rest.strip_suffix("/result"))
        .and_then(|rest| rest.split_once("/commands/"))
        .is_some_and(|(id, cid)| {
            !id.is_empty() && !id.contains('/') && !cid.is_empty() && !cid.contains('/')
        })
}

/// Whether `method` on `path` is for a password-login session only: every API token, `admin`
/// among them, is refused with 403 `human_only`. Purging notes from the trash is, so no agent,
/// script or AI can empty the trash (ai D21), and so is the browser box's desktop under
/// `/browser`, which only the user watches and takes over (browser B4).
pub fn is_human_only(method: &Method, path: &str) -> bool {
    (*method == Method::POST && path == "/api/trash/empty")
        || (*method == Method::DELETE
            && path
                .strip_prefix("/api/trash/")
                .is_some_and(|id| !id.is_empty() && !id.contains('/')))
        || is_desktop(path)
}

/// Whether `path` is the browser box's desktop, `/browser` or below it.
fn is_desktop(path: &str) -> bool {
    path == "/browser" || path.starts_with("/browser/")
}

/// The answer to a caller other than the signed-in user on an [`is_human_only`] route.
pub fn human_only() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "human_only",
        "Only the signed-in user can empty the trash.",
    )
}

/// [`human_only`] for `path`.
fn human_only_for(path: &str) -> ApiError {
    if is_desktop(path) {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "human_only",
            "Only the signed-in user can open the browser box's desktop.",
        )
    } else {
        human_only()
    }
}

/// Paths a GET or HEAD reads without credentials, with everything under them: the app builds,
/// which a phone downloads before the app is installed or paired. They hold no vault data.
const PUBLIC_READ_PATHS: [&str; 1] = ["/api/clients"];

/// The scope a request needs, or `None` for the public endpoints: health, login, device pairing,
/// which checks the password or a pairing code itself, an agent install code's redemption, which
/// checks the code itself, the agent install script, the sync handshake, which an app reads
/// before it pairs (D27), the server's version, and the app downloads under
/// [`PUBLIC_READ_PATHS`].
pub fn required_scope(method: &Method, path: &str) -> Option<Scope> {
    let under = |prefix: &str| {
        path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    let reads = matches!(*method, Method::GET | Method::HEAD);
    if path == "/api/health"
        || (method == Method::GET && (path == "/api/sync/info" || path == "/api/version"))
        || (reads && PUBLIC_READ_PATHS.iter().any(|p| under(p)))
        || (reads && path == "/agent/install.sh")
        || (method == Method::POST
            && matches!(
                path,
                "/api/login" | "/api/devices/pair" | "/api/agent-codes/redeem"
            ))
    {
        return None;
    }
    if ADMIN_PATHS.iter().any(|p| under(p)) {
        return Some(Scope::Admin);
    }
    // Stopping one of the caller's own chat turns changes nothing (ai D23).
    if matches!(*method, Method::GET | Method::HEAD)
        || path == "/api/logout"
        || (method == Method::POST && READ_POST_PATHS.iter().any(|p| under(p)))
        || (method == Method::DELETE && under("/api/ai/chat/turns"))
        // Starting a thread with an agent is part of chatting with it (agents G7).
        || (method == Method::POST && agent_threads_path(path) == Some(false))
    {
        return Some(Scope::Read);
    }
    // A device sets its own push endpoint and bases, reports on its own commands, its failed
    // device events and its sync rounds, and queues an import of its own history, with its `write` token; the
    // handler checks it is its own.
    // Renaming and deleting a thread are note edits (agents G7).
    if WRITE_PATHS.iter().any(|p| under(p))
        || agent_threads_path(path) == Some(true)
        // An agent's memories are notes (agents G21).
        || agent_memories_path(path)
        || (method == Method::PUT && is_device_own(path))
        || (method == Method::POST && is_device_command_result(path))
        || (method == Method::POST && is_device_commands(path))
        || (method == Method::POST && is_device_event_failures(path))
        || (method == Method::POST && is_device_sync_rounds(path))
    {
        Some(Scope::Write)
    } else {
        Some(Scope::Admin)
    }
}

/// Browser sessions, kept in `state.db` (see [`state::State::create_session`]), so restarting
/// the server keeps the browser signed in.
pub struct Sessions {
    writer: Arc<Writer>,
}

impl Sessions {
    pub fn new(writer: Arc<Writer>) -> Sessions {
        Sessions { writer }
    }

    /// Starts a session and returns its id.
    async fn start(&self, user_agent: Option<String>) -> Result<String, ApiError> {
        blocking(self.writer.clone(), move |w| {
            w.state()
                .create_session(user_agent.as_deref())
                .map(|(id, _)| id)
        })
        .await
    }

    /// Whether session `id` is live, and whether checking it renewed it.
    async fn check(&self, id: String) -> Result<Option<state::SessionCheck>, ApiError> {
        blocking(self.writer.clone(), move |w| w.state().check_session(&id)).await
    }

    async fn end(&self, id: String) -> Result<(), ApiError> {
        blocking(self.writer.clone(), move |w| {
            w.state().end_session(&id).map(drop)
        })
        .await
    }
}

fn unauthorized() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "log in or send an API token",
    )
}

/// What a request's credentials grant.
struct Granted {
    author: Author,
    scope: Scope,
    /// The id of a session this request renewed, whose cookie the response sends again.
    renewed: Option<String>,
}

/// The credentials a request carries. A bearer token wins over a session cookie; a bad one is
/// not retried as the cookie.
async fn identify(sessions: &Arc<Sessions>, headers: &HeaderMap) -> Result<Granted, ApiError> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        let secret = value
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .ok_or_else(unauthorized)?;
        let token = blocking(sessions.writer.clone(), move |w| {
            w.state().authenticate_token(&secret)
        })
        .await?
        .ok_or_else(unauthorized)?;
        return Ok(Granted {
            author: Author::Token(token.name),
            scope: token.scope,
            renewed: None,
        });
    }
    let id = session_id(headers).ok_or_else(unauthorized)?;
    let check = sessions.check(id.clone()).await?.ok_or_else(unauthorized)?;
    Ok(Granted {
        author: Author::Session(OWNER_SESSION.to_owned()),
        scope: Scope::Admin,
        renewed: check.renewed.then_some(id),
    })
}

/// The session id in the request's cookies.
fn session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_owned())
}

/// Middleware: authenticate the request and check its scope against [`required_scope`].
pub async fn authenticate(
    State(sessions): State<Arc<Sessions>>,
    mut req: Request,
    next: Next,
) -> Response {
    let Some(needed) = required_scope(req.method(), req.uri().path()) else {
        return next.run(req).await;
    };
    let granted = match identify(&sessions, req.headers()).await {
        Ok(g) => g,
        Err(e) => return e.into_response(),
    };
    if is_human_only(req.method(), req.uri().path())
        && !matches!(granted.author, Author::Session(_))
    {
        return human_only_for(req.uri().path()).into_response();
    }
    if !granted.scope.allows(needed) {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!(
                "this needs the {} scope; the token has {}",
                needed.as_str(),
                granted.scope.as_str()
            ),
        )
        .into_response();
    }
    req.extensions_mut().insert(Caller(granted.author));
    req.extensions_mut().insert(GrantedScope(granted.scope));
    let mut res = next.run(req).await;
    // A renewed session's cookie goes out again with a fresh Max-Age, so the browser keeps it as
    // long as the server does. A response that sets the cookie itself, such as logout, wins.
    if let Some(id) = granted.renewed
        && !res.headers().contains_key(header::SET_COOKIE)
    {
        res.headers_mut()
            .insert(header::SET_COOKIE, session_cookie(&id, SESSION_TTL));
    }
    res
}

/// Middleware for [`crate::owner_router`]: every request is the owner's session.
pub async fn as_owner(mut req: Request, next: Next) -> Response {
    req.extensions_mut()
        .insert(Caller(Author::Session(OWNER_SESSION.to_owned())));
    req.extensions_mut().insert(GrantedScope(Scope::Admin));
    next.run(req).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginBody {
    password: String,
}

fn session_cookie(value: &str, max_age: Duration) -> HeaderValue {
    let cookie = format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        max_age.as_secs()
    );
    HeaderValue::from_str(&cookie).expect("a session cookie is a valid header value")
}

/// `POST /api/login`: check the password and start a session (OQ-S3).
pub async fn login(
    State(sessions): State<Arc<Sessions>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<Response, ApiError> {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let agent = user_agent.clone();
    let ok = blocking(sessions.writer.clone(), move |w| {
        let ok = w.state().verify_password(&body.password)?;
        // Never the password itself (activity Y2).
        let detail = json!({ "user_agent": agent.as_deref().map(|a| state::one_line(a, 200)) });
        w.state().record_activity(if ok {
            state::NewActivity::new(
                "auth.login",
                "auth",
                &Author::Session(OWNER_SESSION.to_owned()),
                "Signed in with the password",
            )
            .detail(detail)
        } else {
            state::NewActivity::server("auth.login_failed", "auth", "A password login failed")
                .detail(detail)
        });
        Ok::<_, state::Error>(ok)
    })
    .await?;
    if !ok {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "wrong password",
        ));
    }
    let id = sessions.start(user_agent).await?;
    let mut res = Json(json!({ "expires_in": SESSION_TTL.as_secs() })).into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&id, SESSION_TTL));
    Ok(res)
}

/// `POST /api/logout`: end the request's session and clear its cookie.
pub async fn logout(
    State(sessions): State<Arc<Sessions>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(id) = session_id(&headers) {
        sessions.end(id).await?;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie("", Duration::ZERO));
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_by_method_and_path() {
        let cases = [
            (Method::GET, "/api/health", None),
            (Method::POST, "/api/login", None),
            (Method::GET, "/api/login", Some(Scope::Read)),
            (Method::POST, "/api/logout", Some(Scope::Read)),
            (Method::GET, "/api/notes/x", Some(Scope::Read)),
            (Method::POST, "/api/notes", Some(Scope::Write)),
            (Method::PUT, "/api/notes/x", Some(Scope::Write)),
            (Method::DELETE, "/api/notes/x", Some(Scope::Write)),
            (Method::POST, "/api/trash/x/restore", Some(Scope::Write)),
            (Method::POST, "/api/trash/empty", Some(Scope::Write)),
            (Method::DELETE, "/api/trash/x", Some(Scope::Write)),
            (Method::GET, "/api/daily/2026-10-13", Some(Scope::Read)),
            (Method::POST, "/api/daily/2026-10-13", Some(Scope::Write)),
            (Method::GET, "/api/pins", Some(Scope::Read)),
            (Method::PUT, "/api/pins/x", Some(Scope::Write)),
            (Method::DELETE, "/api/pins/x", Some(Scope::Write)),
            (Method::POST, "/api/pins/folders", Some(Scope::Write)),
            (Method::PUT, "/api/pins/folders/x", Some(Scope::Write)),
            (Method::DELETE, "/api/pins/folders/x", Some(Scope::Write)),
            (Method::POST, "/api/pins/move", Some(Scope::Write)),
            (Method::POST, "/api/pinsx", Some(Scope::Admin)),
            (Method::GET, "/api/notifications", Some(Scope::Read)),
            (Method::POST, "/api/notifications", Some(Scope::Write)),
            (
                Method::POST,
                "/api/notifications/x/read",
                Some(Scope::Write),
            ),
            (
                Method::POST,
                "/api/notifications/read-all",
                Some(Scope::Write),
            ),
            (Method::DELETE, "/api/notifications/x", Some(Scope::Write)),
            (Method::DELETE, "/api/notifications", Some(Scope::Write)),
            (Method::POST, "/api/notesx", Some(Scope::Admin)),
            (Method::GET, "/api/types/task", Some(Scope::Read)),
            (Method::PUT, "/api/types/task", Some(Scope::Admin)),
            (Method::POST, "/api/query", Some(Scope::Read)),
            (Method::POST, "/api/query/parse", Some(Scope::Read)),
            (Method::POST, "/api/query/text", Some(Scope::Read)),
            (Method::PUT, "/api/query", Some(Scope::Admin)),
            (Method::POST, "/api/queryx", Some(Scope::Admin)),
            (Method::GET, "/api/search", Some(Scope::Read)),
            (Method::GET, "/api/tokens", Some(Scope::Admin)),
            (Method::GET, "/api/activity", Some(Scope::Admin)),
            (Method::GET, "/api/secrets/x", Some(Scope::Admin)),
            (Method::POST, "/api/something-new", Some(Scope::Admin)),
            (Method::POST, "/api/batch", Some(Scope::Write)),
            (Method::POST, "/api/upsert", Some(Scope::Write)),
            (Method::POST, "/api/assets", Some(Scope::Write)),
            (Method::DELETE, "/api/assets/x", Some(Scope::Write)),
            (Method::PUT, "/api/notes/x/canvas/board", Some(Scope::Write)),
            (Method::GET, "/api/schema", Some(Scope::Read)),
            (Method::GET, "/api/openapi.json", Some(Scope::Read)),
            (
                Method::PUT,
                "/api/types/task/templates/card",
                Some(Scope::Admin),
            ),
            (Method::PUT, "/api/taxonomies/status", Some(Scope::Admin)),
            (Method::GET, "/api/scripts/x", Some(Scope::Read)),
            (Method::PUT, "/api/scripts/x", Some(Scope::Admin)),
            (Method::POST, "/api/devices/pair", None),
            (Method::GET, "/api/devices/pair", Some(Scope::Read)),
            (Method::POST, "/api/devices/pair-codes", Some(Scope::Admin)),
            (Method::GET, "/api/devices", Some(Scope::Read)),
            (Method::PATCH, "/api/devices/x", Some(Scope::Admin)),
            (Method::DELETE, "/api/devices/x", Some(Scope::Admin)),
            (Method::PUT, "/api/devices/x/push", Some(Scope::Write)),
            (Method::PUT, "/api/devices/x/y/push", Some(Scope::Admin)),
            (Method::PUT, "/api/devices//push", Some(Scope::Admin)),
            (Method::PUT, "/api/devices/x/bases", Some(Scope::Write)),
            (Method::PUT, "/api/devices/x/health", Some(Scope::Write)),
            (Method::PUT, "/api/devices/x/y/health", Some(Scope::Admin)),
            (Method::PUT, "/api/devices/x/y/bases", Some(Scope::Admin)),
            (Method::GET, "/api/sync/info", None),
            (Method::GET, "/api/version", None),
            (Method::POST, "/api/sync/info", Some(Scope::Admin)),
            (Method::GET, "/api/sync/changes", Some(Scope::Read)),
            (Method::GET, "/api/devices/x/commands", Some(Scope::Read)),
            (Method::POST, "/api/devices/x/commands", Some(Scope::Write)),
            (Method::POST, "/api/devices//commands", Some(Scope::Admin)),
            (
                Method::POST,
                "/api/devices/x/y/commands",
                Some(Scope::Admin),
            ),
            (
                Method::POST,
                "/api/devices/x/commands/c/result",
                Some(Scope::Write),
            ),
            (
                Method::POST,
                "/api/devices/x/commands//result",
                Some(Scope::Admin),
            ),
            (
                Method::POST,
                "/api/devices/x/event-failures",
                Some(Scope::Write),
            ),
            (
                Method::POST,
                "/api/devices/x/y/event-failures",
                Some(Scope::Admin),
            ),
            (
                Method::POST,
                "/api/devices/x/sync-rounds",
                Some(Scope::Write),
            ),
            (
                Method::POST,
                "/api/devices/x/y/sync-rounds",
                Some(Scope::Admin),
            ),
            (Method::GET, "/api/device-files/android", Some(Scope::Read)),
            (Method::PUT, "/api/device-files/android", Some(Scope::Admin)),
            (
                Method::DELETE,
                "/api/device-files/android",
                Some(Scope::Admin),
            ),
            (Method::GET, "/api/clients", None),
            (
                Method::HEAD,
                "/api/clients/TheVault_0.1.0_aarch64.apk",
                None,
            ),
            (Method::GET, "/api/clients/latest/apk", None),
            (Method::PUT, "/api/clients/x.apk", Some(Scope::Admin)),
            (Method::DELETE, "/api/clients/x.apk", Some(Scope::Admin)),
            (Method::GET, "/api/clientsx", Some(Scope::Read)),
            (Method::GET, "/anything", Some(Scope::Read)),
            (Method::POST, "/api/agent-codes", Some(Scope::Admin)),
            (Method::POST, "/api/agent-codes/redeem", None),
            (Method::GET, "/api/agent-codes/redeem", Some(Scope::Read)),
            (Method::GET, "/agent/install.sh", None),
            (Method::HEAD, "/agent/install.sh", None),
            (Method::POST, "/agent/install.sh", Some(Scope::Admin)),
            // Settings > AI holds credentials; the model list is a read (ai D15).
            (Method::GET, "/api/integrations/ai", Some(Scope::Admin)),
            (
                Method::PUT,
                "/api/integrations/ai/openai/key",
                Some(Scope::Admin),
            ),
            (Method::GET, "/api/ai/models", Some(Scope::Read)),
            (Method::POST, "/api/ai/chat", Some(Scope::Read)),
            // The turns form reads as the chat does; applying a change writes (ai D23).
            (Method::GET, "/api/ai/providers", Some(Scope::Read)),
            (Method::POST, "/api/ai/chat/turns", Some(Scope::Read)),
            (
                Method::GET,
                "/api/ai/chat/turns/t/events",
                Some(Scope::Read),
            ),
            (Method::DELETE, "/api/ai/chat/turns/t", Some(Scope::Read)),
            (Method::DELETE, "/api/ai/chat", Some(Scope::Admin)),
            (Method::POST, "/api/ai/apply", Some(Scope::Write)),
        ];
        for (method, path, want) in cases {
            assert_eq!(required_scope(&method, path), want, "{method} {path}");
        }
    }

    #[test]
    fn only_the_trash_purges_are_human_only() {
        assert!(is_human_only(&Method::POST, "/api/trash/empty"));
        assert!(is_human_only(&Method::DELETE, "/api/trash/x"));
        assert!(!is_human_only(&Method::GET, "/api/trash"));
        assert!(!is_human_only(&Method::GET, "/api/trash/empty"));
        assert!(!is_human_only(&Method::POST, "/api/trash/x/restore"));
        assert!(!is_human_only(&Method::DELETE, "/api/trash/x/restore"));
        assert!(!is_human_only(&Method::DELETE, "/api/notes/x"));
        assert!(is_human_only(&Method::GET, "/browser"));
        assert!(is_human_only(&Method::GET, "/browser/vnc.html"));
        assert!(is_human_only(&Method::GET, "/browser/websockify"));
        assert!(!is_human_only(&Method::GET, "/browsers"));
        assert!(!is_human_only(&Method::GET, "/api/integrations/browser"));
    }

    #[test]
    fn finds_the_session_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; tv_session=abc123; other=1"),
        );
        assert_eq!(session_id(&headers).as_deref(), Some("abc123"));
        headers.insert(header::COOKIE, HeaderValue::from_static("theme=dark"));
        assert_eq!(session_id(&headers), None);
    }
}
