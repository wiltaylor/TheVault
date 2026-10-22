//! Who is calling, and what they may do (D11).
//!
//! The browser logs in with the vault's password and gets a session cookie; a session is the
//! owner and may do everything. Agents send `Authorization: Bearer <secret>` for an API token,
//! which carries one [`Scope`]. [`authenticate`] runs before every route: it looks up what the
//! route needs with [`required_scope`], answers 401 when the request carries no valid credentials
//! and 403 when they do not reach that scope, and otherwise hands the handler a [`Caller`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// How long a session lasts after login.
const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Paths whose non-read requests need `write`: notes and assets, batches and upserts of notes,
/// and notifications (D11). Any other change needs `admin`, so a new endpoint is admin-only until it is listed here.
const WRITE_PATHS: [&str; 8] = [
    "/api/notes",
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
const READ_POST_PATHS: [&str; 2] = ["/api/query", "/api/ai/chat"];

/// Paths that need `admin` even to read, because they disclose or hand out credentials.
const ADMIN_PATHS: [&str; 3] = ["/api/tokens", "/api/secrets", "/api/integrations"];

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

/// Whether `path` is `/api/devices/{id}/commands/{cid}/result`.
fn is_device_command_result(path: &str) -> bool {
    path.strip_prefix("/api/devices/")
        .and_then(|rest| rest.strip_suffix("/result"))
        .and_then(|rest| rest.split_once("/commands/"))
        .is_some_and(|(id, cid)| {
            !id.is_empty() && !id.contains('/') && !cid.is_empty() && !cid.contains('/')
        })
}

/// Whether `method` on `path` purges notes from the trash, which only a password-login session
/// may do: every API token, `admin` among them, is refused with 403 `human_only`, so no agent,
/// script or AI can empty the trash (ai D21).
pub fn is_human_only(method: &Method, path: &str) -> bool {
    (*method == Method::POST && path == "/api/trash/empty")
        || (*method == Method::DELETE
            && path
                .strip_prefix("/api/trash/")
                .is_some_and(|id| !id.is_empty() && !id.contains('/')))
}

/// The answer to a caller other than the signed-in user on an [`is_human_only`] route.
pub fn human_only() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "human_only",
        "Only the signed-in user can empty the trash.",
    )
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
    if matches!(*method, Method::GET | Method::HEAD)
        || path == "/api/logout"
        || (method == Method::POST && READ_POST_PATHS.iter().any(|p| under(p)))
    {
        return Some(Scope::Read);
    }
    // A device sets its own push endpoint and bases, reports on its own commands and its failed
    // device events, and queues an import of its own history, with its `write` token; the
    // handler checks it is its own.
    if WRITE_PATHS.iter().any(|p| under(p))
        || (method == Method::PUT && is_device_own(path))
        || (method == Method::POST && is_device_command_result(path))
        || (method == Method::POST && is_device_commands(path))
        || (method == Method::POST && is_device_event_failures(path))
    {
        Some(Scope::Write)
    } else {
        Some(Scope::Admin)
    }
}

/// Live browser sessions, by id. They are kept in memory, so restarting the server logs the
/// browser out.
pub struct Sessions {
    writer: Arc<Writer>,
    live: Mutex<HashMap<String, u128>>,
}

impl Sessions {
    pub fn new(writer: Arc<Writer>) -> Sessions {
        Sessions {
            writer,
            live: Mutex::new(HashMap::new()),
        }
    }

    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<String, u128>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts a session and returns its id.
    fn start(&self) -> String {
        let id = state::random_hex(32);
        let now = now_ms();
        let mut live = self.live();
        live.retain(|_, expires| *expires > now);
        live.insert(id.clone(), now + SESSION_TTL.as_millis());
        id
    }

    fn is_live(&self, id: &str) -> bool {
        self.live()
            .get(id)
            .is_some_and(|expires| *expires > now_ms())
    }

    fn end(&self, id: &str) {
        self.live().remove(id);
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
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
        });
    }
    match session_id(headers) {
        Some(id) if sessions.is_live(&id) => Ok(Granted {
            author: Author::Session(OWNER_SESSION.to_owned()),
            scope: Scope::Admin,
        }),
        _ => Err(unauthorized()),
    }
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
        return human_only().into_response();
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
    next.run(req).await
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
    Json(body): Json<LoginBody>,
) -> Result<Response, ApiError> {
    let ok = blocking(sessions.writer.clone(), move |w| {
        w.state().verify_password(&body.password)
    })
    .await?;
    if !ok {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "wrong password",
        ));
    }
    let id = sessions.start();
    let mut res = Json(json!({ "expires_in": SESSION_TTL.as_secs() })).into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie(&id, SESSION_TTL));
    Ok(res)
}

/// `POST /api/logout`: end the request's session and clear its cookie.
pub async fn logout(State(sessions): State<Arc<Sessions>>, headers: HeaderMap) -> Response {
    if let Some(id) = session_id(&headers) {
        sessions.end(&id);
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, session_cookie("", Duration::ZERO));
    res
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
