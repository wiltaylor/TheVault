//! The requests an app's embedded server sends on to its home server (D18, D20).
//!
//! The device holds a copy of the vault files, so notes, definitions, assets, search and the
//! rest are answered from the local copy and keep working offline. Some `/api` routes are backed
//! by state only the home server holds in its `state.db`: paired devices, notifications, task
//! runs, the app builds it hands out. The local `state.db` of a device has none of it, so those
//! routes go to the home server with the device token, their bodies streaming both ways.
//! `docs/sync.md` "What the apps answer locally" lists every route and where it is answered.
//!
//! When the home server cannot be reached, a forwarded route answers 503 with `error: "offline"`;
//! when the device is not paired, or the server no longer accepts its token, 503 with
//! `error: "not_paired"`. A device token has `write` scope, never `admin` (D2), so what needs
//! `admin` on the server answers 403 with `error: "needs_web_app"`: it is managed in the web app.
//!
//! The AI chat runs on the home server, which holds the credentials (ai D23): the chat, its turns,
//! the model and provider lists and `POST /api/ai/apply` are forwarded, and the device never
//! applies an AI change to its local copy; the change arrives with the next sync. Unpaired or
//! offline, they answer 503 `ai_needs_server`. The webview cannot read a stream through the
//! app's custom scheme, which answers whole bodies, so `GET /api/app/ai/turns/{turn}?after=<n>`
//! reads the turn's event stream here, on the device, and hands the webview the events after
//! the first `n` each time it asks (see [`Home::poll_turn`]).

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, header,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde_json::{Value, json};
use state::LiveEvent;
use sync::Remote;
use tokio::sync::Notify;

/// How long a forwarded request waits to connect before the server counts as offline.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The most bytes of a forwarded answer that are read to rewrite it, such as a history listing.
const MAX_REWRITE: usize = 16 * 1024 * 1024;

/// The message of a forwarded route while the home server cannot be reached.
pub const OFFLINE_MESSAGE: &str = "Needs a connection to your server";

/// The message of a forwarded route while the device is not paired.
pub const NOT_PAIRED_MESSAGE: &str = "Not paired";

/// The message of a route the device token may not use (D2).
pub const NEEDS_WEB_APP_MESSAGE: &str = "Manage this in the web app";

/// The message of an AI route while the home server cannot be reached or the device is unpaired.
pub const AI_NEEDS_SERVER_MESSAGE: &str =
    "The AI chat needs the home server, which is not reachable.";

/// How long a poll of a turn's events waits for one before answering with none.
const POLL_WAIT: Duration = Duration::from_secs(2);

/// How long a relayed turn waits for the webview to ask again before it stops reading it here. The
/// turn runs on on the home server (agents G17); a later poll reads it from there again.
const RELAY_IDLE: Duration = Duration::from_secs(30);

/// Where one `/api` request is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    /// From the local copy of the vault.
    Local,
    /// By the home server.
    Forward,
    /// By the home server, or from the local copy when the server cannot answer.
    ForwardOrLocal,
    /// `GET /api/notes/{id}/history`: the server's versions, or the device's own offline.
    History,
    /// `GET /api/notes/{id}/history/{version}`: the device's own version, or the server's.
    Version { id: String, version: String },
    /// `POST /api/notes/{id}/restore`: a local write of the version's content.
    Restore { id: String },
    /// `GET /api/events`: the server's events with the local note changes.
    Events,
    /// `GET /api/app`: which device and app build this is.
    App,
    /// `GET /api/app/sync`: where the app's sync loop stands.
    AppSync,
    /// `POST /api/app/sync`: ask the app's sync loop for a round now.
    AppSyncNow,
    /// `POST /api/app/sync/adopt`: queue the local notes the home server lacks, "Send this
    /// device's notes to the server" (ticket 150).
    AppSyncAdopt,
    /// `GET /api/app/device-events`: the device events that failed or were skipped, for
    /// "Capture problems".
    AppDeviceEvents,
    /// `POST /api/app/device-events/{id}/retry` (`retry` true) or `.../dismiss`.
    AppDeviceEvent { id: i64, retry: bool },
    /// `GET /api/version`: the home server's version, read from its `/api/sync/info`, which
    /// every home server answers.
    ServerVersion,
    /// The AI chat, its turns, models and providers, and applying its changes: the home server's,
    /// which holds the credentials (ai D23).
    Ai,
    /// `GET /api/app/ai/turns/{turn}`: the turn's events so far, for a webview that cannot read
    /// a stream.
    AppAiTurn { turn: String },
    /// Credentials, and changes to agent files, that need `admin`, which a device token never
    /// has (agents G2).
    NeedsWebApp,
}

/// Whether `path` is `prefix` or below it.
fn under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// One path segment: not empty, no slash.
fn segment(s: &str) -> Option<&str> {
    (!s.is_empty() && !s.contains('/')).then_some(s)
}

/// `/api/agents/{name}/threads` or below: an agent's threads, which the home server writes as
/// it runs their turns, so the device reads and starts them there (agents G7).
fn is_agent_threads(path: &str) -> bool {
    path.strip_prefix("/api/agents/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(name, rest)| {
            segment(name).is_some() && (rest == "threads" || rest.starts_with("threads/"))
        })
}

/// Where `method` on `path` is answered.
pub(crate) fn route(method: &Method, path: &str) -> Route {
    let get = *method == Method::GET || *method == Method::HEAD;
    if path == "/api/app" && get {
        return Route::App;
    }
    if path == "/api/app/sync" {
        if get {
            return Route::AppSync;
        }
        if *method == Method::POST {
            return Route::AppSyncNow;
        }
    }
    if path == "/api/app/sync/adopt" && *method == Method::POST {
        return Route::AppSyncAdopt;
    }
    if path == "/api/app/device-events" && get {
        return Route::AppDeviceEvents;
    }
    if *method == Method::POST
        && let Some(rest) = path.strip_prefix("/api/app/device-events/")
        && let Some((id, action)) = rest.split_once('/')
        && let Ok(id) = id.parse::<i64>()
        && (action == "retry" || action == "dismiss")
    {
        return Route::AppDeviceEvent {
            id,
            retry: action == "retry",
        };
    }
    if path == "/api/version" && get {
        return Route::ServerVersion;
    }
    if get && let Some(turn) = path.strip_prefix("/api/app/ai/turns/").and_then(segment) {
        return Route::AppAiTurn {
            turn: turn.to_owned(),
        };
    }
    if under(path, "/api/ai/chat")
        || path == "/api/ai/models"
        || path == "/api/ai/providers"
        || path == "/api/ai/apply"
        || path == "/api/ai/discard"
        || is_agent_threads(path)
    {
        return Route::Ai;
    }
    // Emptying the trash is for the signed-in user only (ai D21). The embedded server answers
    // every request as the owner, so it cannot tell the user from the device token, and the
    // home server refuses the device token: it is done in the web app.
    if api::is_human_only(method, path)
        || under(path, "/api/tokens")
        || under(path, "/api/secrets")
        || under(path, "/api/integrations/ai")
        || under(path, "/api/ai")
    {
        return Route::NeedsWebApp;
    }
    // An agent file is a definition the device only reads: a change made here would never reach
    // the home server (agents G2).
    if matches!(*method, Method::PUT | Method::DELETE)
        && path
            .strip_prefix("/api/agents/")
            .and_then(segment)
            .is_some()
    {
        return Route::NeedsWebApp;
    }
    if path == "/api/events" && get {
        return Route::Events;
    }
    if under(path, "/api/notifications")
        || under(path, "/api/devices")
        || under(path, "/api/clients")
    {
        return Route::Forward;
    }
    if path == "/api/tasks" && get {
        return Route::ForwardOrLocal;
    }
    // Agent triggers run only on the home server (agents G10), so their runs are read there, and
    // so are the calls of agents it answered (agents G19).
    if get
        && path
            .strip_prefix("/api/agents/calls/")
            .and_then(segment)
            .is_some()
    {
        return Route::Forward;
    }
    if get
        && path
            .strip_prefix("/api/agents/")
            .and_then(|rest| rest.strip_suffix("/runs"))
            .and_then(segment)
            .is_some()
    {
        return Route::Forward;
    }
    if let Some(rest) = path.strip_prefix("/api/tasks/") {
        let mut parts = rest.split('/');
        let is_runs = matches!(
            (parts.next(), parts.next(), parts.next(), parts.next()),
            (Some(n), Some("runs"), None | Some(_), None) if !n.is_empty()
        );
        let is_run =
            *method == Method::POST && rest.strip_suffix("/run").and_then(segment).is_some();
        if (is_runs && get) || is_run {
            return Route::Forward;
        }
    }
    if let Some(rest) = path.strip_prefix("/api/notes/") {
        let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if segment(id).is_some() {
            if get && tail == "history" {
                return Route::History;
            }
            if get && let Some(version) = tail.strip_prefix("history/").and_then(segment) {
                return Route::Version {
                    id: id.to_owned(),
                    version: version.to_owned(),
                };
            }
            if *method == Method::POST && tail == "restore" {
                return Route::Restore { id: id.to_owned() };
            }
        }
    }
    Route::Local
}

/// Why a request could not reach the home server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unavailable {
    Offline,
    NotPaired,
}

impl Unavailable {
    pub fn response(self) -> Response<Body> {
        match self {
            Unavailable::Offline => {
                error(StatusCode::SERVICE_UNAVAILABLE, "offline", OFFLINE_MESSAGE)
            }
            Unavailable::NotPaired => error(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_paired",
                NOT_PAIRED_MESSAGE,
            ),
        }
    }
}

/// A JSON error answer in the API's form.
pub(crate) fn error(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    json_response(status, &json!({ "error": code, "message": message }))
}

pub(crate) fn json_response(status: StatusCode, value: &Value) -> Response<Body> {
    let mut res = Response::new(Body::from(value.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}

/// Request headers that stay on the device: hop-by-hop ones, the webview's own credentials, and
/// content coding, which the client here does not undo.
const DROPPED_REQUEST: [&str; 12] = [
    "host",
    "connection",
    "keep-alive",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "cookie",
    "authorization",
    "accept-encoding",
    "origin",
];

/// Response headers that stay on the server's side.
const DROPPED_RESPONSE: [&str; 5] = [
    "connection",
    "keep-alive",
    "transfer-encoding",
    "set-cookie",
    "upgrade",
];

/// The home server of a paired device, as the embedded server reaches it.
pub(crate) struct Home {
    client: reqwest::Client,
    remote: RwLock<Option<Remote>>,
    /// The chat turns whose events are read here for the webview, by turn id.
    relays: Arc<Mutex<HashMap<String, Arc<Relay>>>>,
}

/// A chat turn's events as read from the home server, kept for the webview's polls.
struct Relay {
    state: Mutex<Relayed>,
    /// Woken when an event arrives or the stream ends.
    arrived: Notify,
}

#[derive(Default)]
struct Relayed {
    events: Vec<Value>,
    done: bool,
    /// When the webview last asked.
    asked: Option<Instant>,
}

impl Relay {
    fn lock(&self) -> std::sync::MutexGuard<'_, Relayed> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the webview has not asked for [`RELAY_IDLE`].
    fn idle(&self) -> bool {
        self.lock().asked.is_none_or(|t| t.elapsed() >= RELAY_IDLE)
    }
}

impl Home {
    pub fn new() -> anyhow::Result<Home> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        Ok(Home {
            client,
            remote: RwLock::new(None),
            relays: Arc::default(),
        })
    }

    pub fn set_remote(&self, remote: Option<Remote>) {
        *self.remote.write().unwrap_or_else(PoisonError::into_inner) = remote;
    }

    pub fn remote(&self) -> Option<Remote> {
        self.remote
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Send `req` to the home server with the device token, its body streaming.
    pub async fn send(&self, req: Request<Body>) -> Result<reqwest::Response, Unavailable> {
        let Some(remote) = self.remote() else {
            return Err(Unavailable::NotPaired);
        };
        let (parts, body) = req.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or(parts.uri.path(), |pq| pq.as_str());
        let url = format!("{}{path}", remote.url.trim_end_matches('/'));
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &parts.headers {
            if DROPPED_REQUEST.contains(&name.as_str()) {
                continue;
            }
            if name == header::CONTENT_LENGTH
                && axum::body::HttpBody::size_hint(&body).exact().is_none()
            {
                continue;
            }
            headers.append(name.clone(), value.clone());
        }
        let mut rb = self
            .client
            .request(parts.method.clone(), url)
            .headers(headers)
            .bearer_auth(&remote.token);
        if !matches!(parts.method, Method::GET | Method::HEAD) {
            rb = rb.body(reqwest::Body::wrap_stream(body.into_data_stream()));
        }
        let res = rb.send().await.map_err(|_| Unavailable::Offline)?;
        let status = res.status();
        let is_json = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("json"));
        // A proxy in front of the server answers these while the server is down.
        if matches!(status.as_u16(), 502..=504) && !is_json {
            return Err(Unavailable::Offline);
        }
        // The server no longer accepts the token: the device was revoked.
        if status == StatusCode::UNAUTHORIZED {
            return Err(Unavailable::NotPaired);
        }
        Ok(res)
    }

    /// `req` answered by the home server, or the error saying why it could not be.
    pub async fn forward(&self, req: Request<Body>) -> Response<Body> {
        match self.send(req).await {
            Ok(res) => translate(res).await,
            Err(why) => why.response(),
        }
    }

    /// An AI route answered by the home server, or 503 `ai_needs_server` when it cannot be
    /// reached. Its answers pass unchanged, a 403 `needs_admin` among them.
    pub async fn forward_ai(&self, req: Request<Body>) -> Response<Body> {
        match self.send(req).await {
            Ok(res) => pass(res),
            Err(_) => ai_needs_server(),
        }
    }

    /// `GET /api/app/ai/turns/{turn}?after=<n>`: `{ events: [{event, data}], next, done }`, the
    /// turn's events after the first `n`, waiting up to [`POLL_WAIT`] for one. The first poll
    /// opens the turn's event stream on the home server, from its first event, and a task here
    /// reads it into a buffer that any number of the webview's readers poll, each with its own
    /// `after`. A turn the webview stops asking about for [`RELAY_IDLE`] is let go here, which
    /// closes the stream but leaves the turn running on the home server (agents G17); the next
    /// poll opens it again from its first event, so the counts still match. `done` says the
    /// stream ended: the last event is the
    /// turn's `done`, `error` or `refusal`, or, when the stream broke, an `error` saying so.
    pub async fn poll_turn(&self, turn: &str, after: usize) -> Response<Body> {
        let existing = self
            .relays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(turn)
            .cloned();
        let relay = match existing {
            Some(r) => r,
            None => match self.open_relay(turn).await {
                Ok(r) => r,
                Err(res) => return res,
            },
        };
        relay.lock().asked = Some(Instant::now());
        let deadline = tokio::time::Instant::now() + POLL_WAIT;
        loop {
            let arrived = relay.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            {
                let st = relay.lock();
                if st.events.len() > after || st.done {
                    break;
                }
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                break;
            }
        }
        let (events, next, done) = {
            let mut st = relay.lock();
            st.asked = Some(Instant::now());
            let from = after.min(st.events.len());
            (st.events[from..].to_vec(), st.events.len(), st.done)
        };
        if done {
            self.relays
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(turn);
        }
        json_response(
            StatusCode::OK,
            &json!({ "events": events, "next": next, "done": done }),
        )
    }

    /// Open `turn`'s event stream on the home server and start reading it into a relay.
    async fn open_relay(&self, turn: &str) -> Result<Arc<Relay>, Response<Body>> {
        let ask = Request::get(format!("/api/ai/chat/turns/{turn}/events"))
            .header(header::ACCEPT, "text/event-stream")
            .body(Body::empty())
            .unwrap_or_default();
        let res = match self.send(ask).await {
            Ok(res) if res.status().is_success() => res,
            Ok(res) => return Err(pass(res)),
            Err(_) => return Err(ai_needs_server()),
        };
        let relay = Arc::new(Relay {
            state: Mutex::new(Relayed {
                asked: Some(Instant::now()),
                ..Relayed::default()
            }),
            arrived: Notify::new(),
        });
        self.relays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(turn.to_owned(), relay.clone());
        let reading = relay.clone();
        let relays = self.relays.clone();
        let turn = turn.to_owned();
        tokio::spawn(async move {
            relay_stream(res, &reading).await;
            reading.lock().done = true;
            reading.arrived.notify_waiters();
            // A turn the webview left is let go; one it reads is, by its last poll.
            if reading.idle() {
                relays
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&turn);
            }
        });
        Ok(relay)
    }

    /// `GET /api/version`: `{ server_version }` from the home server's `GET /api/sync/info`, so a
    /// home server older than `/api/version` still says its version.
    pub async fn server_version(&self) -> Response<Body> {
        let ask = Request::get("/api/sync/info")
            .body(Body::empty())
            .unwrap_or_default();
        let res = match self.send(ask).await {
            Ok(res) if res.status().is_success() => res,
            Ok(res) => return translate(res).await,
            Err(why) => return why.response(),
        };
        match read_json(res).await {
            Some(info) => json_response(
                StatusCode::OK,
                &json!({ "server_version": info["server_version"] }),
            ),
            None => Unavailable::Offline.response(),
        }
    }

    /// `GET /api/events`: the server's events, with the device's own `note.changed` in place of
    /// the server's. A note changes on the device when the device writes it or pulls it, with a
    /// version of its own, so the server's `note.changed` would name versions the device does not
    /// hold. The local events are subscribed before the server is asked, so none is missed, and
    /// the stream ends when the server's does: the client reconnects.
    pub async fn events(&self, writer: &writer::Writer, req: Request<Body>) -> Response<Body> {
        let local = writer.state().events().subscribe();
        let res = match self.send(req).await {
            Ok(res) if res.status().is_success() => res,
            Ok(res) => return translate(res).await,
            Err(why) => return why.response(),
        };
        let mut builder = Response::builder().status(res.status());
        for (name, value) in res.headers() {
            if !DROPPED_RESPONSE.contains(&name.as_str()) && name != header::CONTENT_LENGTH {
                builder = builder.header(name, value);
            }
        }
        let remote = server_events(res.bytes_stream())
            .map(Some)
            .chain(stream::once(async { None }));
        let local = stream::unfold(local, |mut sub| async move {
            loop {
                match sub.recv().await {
                    Ok((_, event)) => {
                        if let Some(frame) = local_frame(&event) {
                            return Some((Some(Ok(frame)), sub));
                        }
                    }
                    Err(_) => return None,
                }
            }
        });
        let merged = stream::select(remote, local)
            .take_while(|item| std::future::ready(item.is_some()))
            .filter_map(std::future::ready);
        builder
            .body(Body::from_stream(merged))
            .unwrap_or_else(|_| Unavailable::Offline.response())
    }
}

/// Read a turn's event stream into `relay` until it ends, or until the webview has not asked for
/// [`RELAY_IDLE`], when the stream is dropped and the turn runs on on the home server.
async fn relay_stream(res: reqwest::Response, relay: &Relay) {
    let mut body = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut ended = false;
    loop {
        if relay.idle() {
            return;
        }
        let chunk = match tokio::time::timeout(Duration::from_secs(5), body.next()).await {
            Err(_) => continue,
            Ok(None | Some(Err(_))) => break,
            Ok(Some(Ok(chunk))) => chunk,
        };
        buf.extend_from_slice(&chunk);
        let frames = sse_frames(&mut buf);
        if frames.is_empty() {
            continue;
        }
        {
            let mut st = relay.lock();
            for f in frames {
                ended |= matches!(f["event"].as_str(), Some("done" | "error" | "refusal"));
                st.events.push(f);
            }
        }
        relay.arrived.notify_waiters();
    }
    if !ended {
        relay.lock().events.push(json!({
            "event": "error",
            "data": { "kind": "unavailable", "message": AI_NEEDS_SERVER_MESSAGE },
        }));
    }
}

/// Take every complete event off the front of `buf`, as `{event, data}` with `data` parsed as
/// JSON. Comments, such as heartbeats, are left out.
fn sse_frames(buf: &mut Vec<u8>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Some(end) = buf.windows(2).position(|w| w == b"\n\n") {
        let frame: Vec<u8> = buf.drain(..end + 2).collect();
        let text = String::from_utf8_lossy(&frame[..end]);
        let mut event = "message".to_owned();
        let mut data: Vec<&str> = Vec::new();
        for line in text.lines() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => event = value.to_owned(),
                "data" => data.push(value),
                _ => {}
            }
        }
        if data.is_empty() {
            continue;
        }
        let data = data.join("\n");
        let data = serde_json::from_str(&data).unwrap_or(Value::String(data));
        out.push(json!({ "event": event, "data": data }));
    }
    out
}

/// 503 `ai_needs_server`.
pub(crate) fn ai_needs_server() -> Response<Body> {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "ai_needs_server",
        AI_NEEDS_SERVER_MESSAGE,
    )
}

/// The server's answer as it is: status, headers and streaming body.
fn pass(res: reqwest::Response) -> Response<Body> {
    let mut builder = Response::builder().status(res.status());
    for (name, value) in res.headers() {
        if !DROPPED_RESPONSE.contains(&name.as_str()) {
            builder = builder.header(name, value);
        }
    }
    let body = Body::from_stream(res.bytes_stream().map_err(std::io::Error::other));
    builder
        .body(body)
        .unwrap_or_else(|_| Unavailable::Offline.response())
}

/// The server's answer as the webview gets it: its status, headers and streaming body, with a 403
/// rewritten to say the web app manages it.
pub(crate) async fn translate(res: reqwest::Response) -> Response<Body> {
    if res.status() == StatusCode::FORBIDDEN {
        let detail = read_json(res).await.unwrap_or(Value::Null);
        let mut out = json!({ "error": "needs_web_app", "message": NEEDS_WEB_APP_MESSAGE });
        if let Some(message) = detail.get("message") {
            out["detail"] = message.clone();
        }
        return json_response(StatusCode::FORBIDDEN, &out);
    }
    pass(res)
}

/// The JSON body of a server answer, read whole.
pub(crate) async fn read_json(res: reqwest::Response) -> Option<Value> {
    let bytes = read_limited(res).await?;
    serde_json::from_slice(&bytes).ok()
}

/// The body of a server answer, read whole up to [`MAX_REWRITE`].
pub(crate) async fn read_limited(res: reqwest::Response) -> Option<Bytes> {
    if res.content_length().is_some_and(|n| n > MAX_REWRITE as u64) {
        return None;
    }
    let mut out = Vec::new();
    let mut body = res.bytes_stream();
    while let Some(chunk) = body.next().await {
        out.extend_from_slice(&chunk.ok()?);
        if out.len() > MAX_REWRITE {
            return None;
        }
    }
    Some(Bytes::from(out))
}

/// A JSON object answer with `key` set to `value`. Anything else passes unchanged.
pub(crate) async fn with_key(res: Response<Body>, key: &str, value: &str) -> Response<Body> {
    let (mut parts, body) = res.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_REWRITE).await else {
        return Unavailable::Offline.response();
    };
    let Ok(Value::Object(mut obj)) = serde_json::from_slice::<Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    obj.insert(key.to_owned(), json!(value));
    let bytes = Value::Object(obj).to_string();
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(bytes))
}

/// The server's event stream with its `note.changed` events and event ids left out. Ids from two
/// sources would not count up together; the client does not resume from them.
fn server_events<S>(upstream: S) -> impl futures_util::Stream<Item = Result<Bytes, Infallible>>
where
    S: futures_util::Stream<Item = reqwest::Result<Bytes>>,
{
    upstream
        .take_while(|chunk| std::future::ready(chunk.is_ok()))
        .scan(Vec::new(), |buf: &mut Vec<u8>, chunk| {
            if let Ok(bytes) = chunk {
                buf.extend_from_slice(&bytes);
            }
            std::future::ready(Some(complete_frames(buf)))
        })
        .filter(|kept| std::future::ready(!kept.is_empty()))
        .map(Ok)
}

/// Take every complete frame off the front of `buf`, and return those to pass on.
fn complete_frames(buf: &mut Vec<u8>) -> Bytes {
    let mut out = Vec::new();
    while let Some(end) = buf.windows(2).position(|w| w == b"\n\n") {
        let frame: Vec<u8> = buf.drain(..end + 2).collect();
        let text = String::from_utf8_lossy(&frame[..end]);
        let is_note_changed = text.lines().any(|l| {
            l.strip_prefix("event:")
                .is_some_and(|name| name.trim() == "note.changed")
        });
        if is_note_changed {
            continue;
        }
        for line in text.lines() {
            if line == "id" || line.starts_with("id:") {
                continue;
            }
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out.push(b'\n');
    }
    Bytes::from(out)
}

/// A local event as an SSE frame, for the device's own `note.changed` only: the notifications
/// and devices are the home server's.
fn local_frame(event: &LiveEvent) -> Option<Bytes> {
    let LiveEvent::NoteChanged {
        id,
        type_name,
        version,
        deleted,
        ..
    } = event
    else {
        return None;
    };
    let data = json!({ "id": id, "type": type_name, "version": version, "deleted": deleted });
    Some(Bytes::from(format!(
        "event: note.changed\ndata: {data}\n\n"
    )))
}

/// A request for the same method and path as `parts`, with `headers` and `body`.
pub(crate) fn request(
    method: Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
    body: Body,
) -> Request<Body> {
    let mut req = Request::new(body);
    *req.method_mut() = method;
    *req.uri_mut() = uri.clone();
    *req.headers_mut() = headers.clone();
    req
}

/// The `Accept` header asking for a note as WCL text.
pub(crate) fn accept_text() -> (HeaderName, HeaderValue) {
    (header::ACCEPT, HeaderValue::from_static("text/plain"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_home_server_state_to_the_home_server() {
        let get = Method::GET;
        let post = Method::POST;
        let id = "01J9Z3K8Q2W6E4R5T7Y8U9I0OP";
        assert_eq!(route(&get, "/api/devices"), Route::Forward);
        assert_eq!(route(&Method::PATCH, "/api/devices/x"), Route::Forward);
        assert_eq!(route(&post, "/api/devices/x/commands"), Route::Forward);
        assert_eq!(route(&post, "/api/devices/pair-codes"), Route::Forward);
        assert_eq!(route(&get, "/api/device-files"), Route::Local);
        assert_eq!(route(&get, "/api/notifications"), Route::Forward);
        assert_eq!(route(&post, "/api/notifications/x/read"), Route::Forward);
        assert_eq!(route(&get, "/api/clients/latest/apk"), Route::Forward);
        assert_eq!(route(&get, "/api/events"), Route::Events);
        assert_eq!(route(&get, "/api/tokens"), Route::NeedsWebApp);
        assert_eq!(route(&Method::PUT, "/api/secrets/k"), Route::NeedsWebApp);
        // The device holds no AI credentials (ai D16): Settings > AI is the web app's, and the
        // chat is the home server's (ai D23).
        assert_eq!(route(&get, "/api/integrations/ai"), Route::NeedsWebApp);
        assert_eq!(
            route(&post, "/api/integrations/ai/chatgpt/start"),
            Route::NeedsWebApp
        );
        assert_eq!(route(&get, "/api/ai/models"), Route::Ai);
        assert_eq!(route(&get, "/api/ai/providers"), Route::Ai);
        assert_eq!(route(&post, "/api/ai/chat"), Route::Ai);
        assert_eq!(route(&post, "/api/ai/chat/turns"), Route::Ai);
        assert_eq!(route(&get, "/api/ai/chat/turns/t/events"), Route::Ai);
        assert_eq!(route(&Method::DELETE, "/api/ai/chat/turns/t"), Route::Ai);
        assert_eq!(route(&post, "/api/ai/apply"), Route::Ai);
        assert_eq!(route(&post, "/api/ai/discard"), Route::Ai);
        assert_eq!(route(&get, "/api/agents/assistant/threads"), Route::Ai);
        assert_eq!(route(&post, "/api/agents/assistant/threads"), Route::Ai);
        assert_eq!(route(&get, "/api/agents/triage/threads/01J"), Route::Ai);
        assert_eq!(
            route(&Method::PATCH, "/api/agents/triage/threads/01J"),
            Route::Ai
        );
        assert_eq!(
            route(&Method::DELETE, "/api/agents/triage/threads/01J"),
            Route::Ai
        );
        assert_eq!(route(&get, "/api/ai/other"), Route::NeedsWebApp);
        assert_eq!(
            route(&get, "/api/app/ai/turns/t"),
            Route::AppAiTurn {
                turn: "t".to_owned()
            }
        );
        assert_eq!(route(&get, "/api/aim"), Route::Local);
        assert_eq!(route(&get, "/api/tasks"), Route::ForwardOrLocal);
        assert_eq!(route(&get, "/api/tasks/noop"), Route::Local);
        assert_eq!(route(&get, "/api/tasks/noop/runs"), Route::Forward);
        assert_eq!(route(&get, "/api/agents/triage/runs"), Route::Forward);
        assert_eq!(route(&get, "/api/agents/calls/01J"), Route::Forward);
        assert_eq!(route(&get, "/api/tasks/noop/runs/7"), Route::Forward);
        assert_eq!(route(&post, "/api/tasks/noop/run"), Route::Forward);
        assert_eq!(route(&Method::PUT, "/api/tasks/noop"), Route::Local);
        assert_eq!(route(&get, "/api/cron"), Route::Local);
        assert_eq!(route(&get, "/api/agents"), Route::Local);
        assert_eq!(route(&get, "/api/agents/assistant"), Route::Local);
        assert_eq!(
            route(&Method::PUT, "/api/agents/assistant"),
            Route::NeedsWebApp
        );
        assert_eq!(
            route(&Method::DELETE, "/api/agents/triage"),
            Route::NeedsWebApp
        );
        assert_eq!(
            route(&get, &format!("/api/notes/{id}/history")),
            Route::History
        );
        assert_eq!(
            route(&get, &format!("/api/notes/{id}/history/v1")),
            Route::Version {
                id: id.to_owned(),
                version: "v1".to_owned()
            }
        );
        assert_eq!(
            route(&post, &format!("/api/notes/{id}/restore")),
            Route::Restore { id: id.to_owned() }
        );
        assert_eq!(route(&get, &format!("/api/notes/{id}")), Route::Local);
        assert_eq!(route(&get, "/api/pins"), Route::Local);
        assert_eq!(route(&get, "/api/trash"), Route::Local);
        assert_eq!(
            route(&post, &format!("/api/trash/{id}/restore")),
            Route::Local
        );
        // Only the signed-in user empties the trash (ai D21).
        assert_eq!(route(&post, "/api/trash/empty"), Route::NeedsWebApp);
        assert_eq!(
            route(&Method::DELETE, &format!("/api/trash/{id}")),
            Route::NeedsWebApp
        );
        assert_eq!(route(&get, "/api/app"), Route::App);
        assert_eq!(route(&get, "/api/app/sync"), Route::AppSync);
        assert_eq!(route(&post, "/api/app/sync"), Route::AppSyncNow);
        assert_eq!(route(&post, "/api/app/sync/adopt"), Route::AppSyncAdopt);
        assert_eq!(
            route(&get, "/api/app/device-events"),
            Route::AppDeviceEvents
        );
        assert_eq!(
            route(&post, "/api/app/device-events/7/retry"),
            Route::AppDeviceEvent { id: 7, retry: true }
        );
        assert_eq!(
            route(&post, "/api/app/device-events/7/dismiss"),
            Route::AppDeviceEvent {
                id: 7,
                retry: false
            }
        );
        assert_eq!(route(&post, "/api/app/device-events/x/retry"), Route::Local);
        assert_eq!(route(&get, "/api/version"), Route::ServerVersion);
        assert_eq!(route(&get, "/api/sync/info"), Route::Local);
    }

    #[test]
    fn sse_frames_are_parsed_and_heartbeats_skipped() {
        let mut buf = b": heartbeat\n\nevent: text\ndata: {\"delta\":\"a\"}\n\nevent: do".to_vec();
        let frames = sse_frames(&mut buf);
        assert_eq!(
            frames,
            vec![json!({ "event": "text", "data": { "delta": "a" } })]
        );
        assert_eq!(buf, b"event: do");
    }

    #[test]
    fn server_events_lose_note_changes_and_ids() {
        let mut buf = b": open\n\nevent: note.changed\nid: 3\ndata: {}\n\nevent: notification.created\nid: 4\ndata: {\"id\":\"n\"}\n\nevent: dev".to_vec();
        let kept = complete_frames(&mut buf);
        assert_eq!(
            std::str::from_utf8(&kept).unwrap(),
            ": open\n\nevent: notification.created\ndata: {\"id\":\"n\"}\n\n"
        );
        assert_eq!(buf, b"event: dev");
    }
}
