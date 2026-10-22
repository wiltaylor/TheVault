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

use std::convert::Infallible;
use std::sync::{PoisonError, RwLock};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, header,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde_json::{Value, json};
use state::LiveEvent;
use sync::Remote;

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
    /// Credentials that need `admin`, which a device token never has.
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
    if under(path, "/api/tokens")
        || under(path, "/api/secrets")
        || under(path, "/api/integrations/ai")
        || under(path, "/api/ai")
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
}

impl Home {
    pub fn new() -> anyhow::Result<Home> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        Ok(Home {
            client,
            remote: RwLock::new(None),
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
        // The device holds no AI credentials (ai D16).
        assert_eq!(route(&get, "/api/integrations/ai"), Route::NeedsWebApp);
        assert_eq!(
            route(&post, "/api/integrations/ai/chatgpt/start"),
            Route::NeedsWebApp
        );
        assert_eq!(route(&get, "/api/ai/models"), Route::NeedsWebApp);
        assert_eq!(route(&get, "/api/aim"), Route::Local);
        assert_eq!(route(&get, "/api/tasks"), Route::ForwardOrLocal);
        assert_eq!(route(&get, "/api/tasks/noop"), Route::Local);
        assert_eq!(route(&get, "/api/tasks/noop/runs"), Route::Forward);
        assert_eq!(route(&get, "/api/tasks/noop/runs/7"), Route::Forward);
        assert_eq!(route(&post, "/api/tasks/noop/run"), Route::Forward);
        assert_eq!(route(&Method::PUT, "/api/tasks/noop"), Route::Local);
        assert_eq!(route(&get, "/api/cron"), Route::Local);
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
