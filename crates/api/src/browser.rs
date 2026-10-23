//! The browser box's desktop through the server, and its Settings > Integrations card (browser
//! B4).
//!
//! - `GET /api/integrations/browser` (admin) answers `{ configured, reachable, version,
//!   playwright_cli, display, error }` from the exec service's `GET /health`, which needs no
//!   token. `configured` is whether `VAULT_BROWSER_URL` is set; the rest are null when it is not
//!   or the box does not answer, `error` then saying why.
//! - `/browser/...` is the box's noVNC: its static files and its `websockify` websocket, proxied
//!   to the desktop address. `/browser` and `/browser/` redirect to [`DESKTOP_PAGE`]. Only a
//!   password-login session may use it: every token, `admin` among them, is a 403 `human_only`
//!   ([`crate::auth::is_human_only`]), so no agent, script or app reaches the desktop. Without
//!   `VAULT_BROWSER_URL` the routes are not there at all.
//!
//! `VAULT_BROWSER_URL` is the exec service, such as `http://browser:7700`; an empty value is
//! unset, as compose passes it when the box is off. The desktop is `VAULT_BROWSER_DESKTOP_URL`
//! when set, else the exec service's host on port 6080, such as `http://browser:6080`. The
//! websocket is relayed byte for byte over plain TCP, so the desktop address is `http://`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use futures_util::TryStreamExt;
use hyper_util::rt::TokioIo;
use reqwest::Url;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::ApiError;

/// The exec service's address.
pub const URL_VAR: &str = "VAULT_BROWSER_URL";

/// The desktop's (noVNC's) address, when it is not the exec service's host on [`DESKTOP_PORT`].
pub const DESKTOP_URL_VAR: &str = "VAULT_BROWSER_DESKTOP_URL";

/// noVNC's port in the box.
pub const DESKTOP_PORT: u16 = 6080;

/// The page "Open desktop" opens.
pub const DESKTOP_PAGE: &str =
    "/browser/vnc.html?autoconnect=1&resize=remote&path=browser/websockify";

/// How long the status check waits for `/health`.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the desktop proxy waits for noVNC to start answering.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest websocket handshake answer read from noVNC.
const MAX_HANDSHAKE: usize = 16 * 1024;

/// The request headers a static file request passes on to noVNC.
const PASSED_REQUEST: [HeaderName; 5] = [
    header::ACCEPT,
    header::ACCEPT_LANGUAGE,
    header::IF_NONE_MATCH,
    header::IF_MODIFIED_SINCE,
    header::RANGE,
];

/// The response headers a static file answer passes back.
const PASSED_RESPONSE: [HeaderName; 7] = [
    header::CONTENT_TYPE,
    header::CONTENT_LENGTH,
    header::CONTENT_RANGE,
    header::ACCEPT_RANGES,
    header::ETAG,
    header::LAST_MODIFIED,
    header::CACHE_CONTROL,
];

/// The handshake headers passed on to noVNC, and back from it.
const WS_REQUEST: [&str; 4] = [
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-protocol",
    "sec-websocket-extensions",
];
const WS_RESPONSE: [&str; 3] = [
    "sec-websocket-accept",
    "sec-websocket-protocol",
    "sec-websocket-extensions",
];

/// Where the browser box is, from the environment.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// The exec service, `None` when the box is not set up.
    pub exec: Option<Url>,
    /// noVNC, `None` when the box is not set up.
    pub desktop: Option<Url>,
}

impl Config {
    /// Reads [`URL_VAR`] and [`DESKTOP_URL_VAR`].
    pub fn from_env() -> Config {
        let var = |name: &str| std::env::var(name).ok();
        Config::from_values(var(URL_VAR).as_deref(), var(DESKTOP_URL_VAR).as_deref())
    }

    /// The config for these values; an empty or unparsable value is unset. A desktop address
    /// without an exec service is ignored: the box is off.
    pub fn from_values(exec: Option<&str>, desktop: Option<&str>) -> Config {
        let parse = |v: Option<&str>| {
            v.map(str::trim)
                .filter(|v| !v.is_empty())
                .and_then(|v| match Url::parse(v) {
                    Ok(u) if u.host_str().is_some() => Some(u),
                    _ => {
                        eprintln!(
                            "vault-server: ignoring the browser box address {v:?}: not a URL"
                        );
                        None
                    }
                })
        };
        let Some(exec) = parse(exec) else {
            return Config::default();
        };
        let desktop = parse(desktop).or_else(|| {
            let mut u = exec.clone();
            u.set_port(Some(DESKTOP_PORT)).ok()?;
            u.set_path("");
            Some(u)
        });
        Config {
            exec: Some(exec),
            desktop,
        }
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_default()
    })
}

/// `<base><path>`, with `base`'s own path kept as a prefix.
fn join(base: &Url, path_and_query: &str) -> String {
    format!("{}{}", base.as_str().trim_end_matches('/'), path_and_query)
}

/// `GET /api/integrations/browser`.
pub async fn status(State(config): State<Arc<Config>>) -> Json<Value> {
    let Some(exec) = &config.exec else {
        return Json(json!({
            "configured": false, "reachable": false, "version": null,
            "playwright_cli": null, "display": null, "error": null,
        }));
    };
    let answer = async {
        let res = http()
            .get(join(exec, "/health"))
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("the browser box did not answer: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("the browser box answered {}", res.status()));
        }
        res.json::<Value>()
            .await
            .map_err(|e| format!("the browser box's health is not JSON: {e}"))
    }
    .await;
    Json(match answer {
        Ok(h) => json!({
            "configured": true,
            "reachable": h.get("ok").and_then(Value::as_bool).unwrap_or(true),
            "version": h.get("version").cloned().unwrap_or(Value::Null),
            "playwright_cli": h.get("playwright_cli").cloned().unwrap_or(Value::Null),
            "display": h.get("display").cloned().unwrap_or(Value::Null),
            "error": null,
        }),
        Err(e) => json!({
            "configured": true, "reachable": false, "version": null,
            "playwright_cli": null, "display": null, "error": e,
        }),
    })
}

/// The `/browser` routes, or `None` when the box is not set up.
pub fn desktop_router(config: &Config) -> Option<axum::Router> {
    let desktop = Arc::new(config.desktop.clone()?);
    Some(
        axum::Router::new()
            .route("/browser", get(open))
            .route("/browser/", get(open))
            .route("/browser/{*path}", get(proxy))
            .with_state(desktop),
    )
}

async fn open() -> Redirect {
    Redirect::to(DESKTOP_PAGE)
}

fn bad_gateway(message: impl Into<String>) -> Response {
    ApiError::new(StatusCode::BAD_GATEWAY, "browser_unreachable", message).into_response()
}

/// `GET /browser/{path}`: a noVNC file, or the websocket when the request asks to upgrade.
async fn proxy(State(desktop): State<Arc<Url>>, req: Request) -> Response {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let upstream = path_and_query
        .strip_prefix("/browser")
        .unwrap_or(path_and_query)
        .to_owned();
    let upgrade = req
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    if upgrade {
        websocket(&desktop, &upstream, req).await
    } else {
        file(&desktop, &upstream, req.method(), req.headers()).await
    }
}

async fn file(desktop: &Url, upstream: &str, method: &Method, headers: &HeaderMap) -> Response {
    let mut out = http().request(method.clone(), join(desktop, upstream));
    for name in &PASSED_REQUEST {
        if let Some(v) = headers.get(name) {
            out = out.header(name, v);
        }
    }
    let res = match out.send().await {
        Ok(res) => res,
        Err(e) => return bad_gateway(format!("the browser box's desktop did not answer: {e}")),
    };
    let mut builder = Response::builder().status(res.status());
    for name in &PASSED_RESPONSE {
        if let Some(v) = res.headers().get(name) {
            builder = builder.header(name, v);
        }
    }
    let body = Body::from_stream(res.bytes_stream().map_err(std::io::Error::other));
    builder
        .body(body)
        .unwrap_or_else(|_| bad_gateway("the browser box's desktop answered badly"))
}

/// Relays the websocket: the handshake goes to noVNC, and once it answers 101 the two
/// connections are joined byte for byte.
async fn websocket(desktop: &Url, upstream: &str, mut req: Request) -> Response {
    let Some(host) = desktop.host_str() else {
        return bad_gateway("the desktop address has no host");
    };
    let port = desktop.port_or_known_default().unwrap_or(80);
    let prefix = desktop.path().trim_end_matches('/');
    let connect = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port))).await;
    let mut conn = match connect {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return bad_gateway(format!("the browser box's desktop did not answer: {e}")),
        Err(_) => return bad_gateway("the browser box's desktop did not answer in time"),
    };
    let mut handshake = format!(
        "GET {prefix}{upstream} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
    );
    for name in WS_REQUEST {
        if let Some(v) = req.headers().get(name).and_then(|v| v.to_str().ok()) {
            handshake.push_str(&format!("{name}: {v}\r\n"));
        }
    }
    handshake.push_str("\r\n");
    if let Err(e) = conn.write_all(handshake.as_bytes()).await {
        return bad_gateway(format!(
            "the browser box's desktop dropped the connection: {e}"
        ));
    }
    let (head, rest) = match read_head(&mut conn).await {
        Ok(v) => v,
        Err(e) => return bad_gateway(e),
    };
    let Some((status, answer)) = parse_head(&head) else {
        return bad_gateway("the browser box's desktop answered the websocket badly");
    };
    if status != 101 {
        return bad_gateway(format!(
            "the browser box's desktop refused the websocket with {status}"
        ));
    }
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        let Ok(client) = on_upgrade.await else {
            return;
        };
        let mut client = TokioIo::new(client);
        if !rest.is_empty() && client.write_all(&rest).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut conn).await;
    });
    let mut res = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::UPGRADE, "websocket")
        .header(header::CONNECTION, "upgrade");
    for name in WS_RESPONSE {
        if let Some((_, v)) = answer.iter().find(|(n, _)| n == name)
            && let Ok(v) = HeaderValue::from_str(v)
        {
            res = res.header(name, v);
        }
    }
    res.body(Body::empty())
        .unwrap_or_else(|_| bad_gateway("the browser box's desktop answered the websocket badly"))
}

/// Reads an HTTP answer's head, through the blank line, and whatever came after it.
async fn read_head(conn: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            return Ok((buf, rest));
        }
        if buf.len() > MAX_HANDSHAKE {
            return Err("the browser box's desktop answered the websocket badly".to_owned());
        }
        let read = tokio::time::timeout(CONNECT_TIMEOUT, conn.read(&mut chunk)).await;
        match read {
            Ok(Ok(0)) => return Err("the browser box's desktop closed the websocket".to_owned()),
            Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]),
            Ok(Err(e)) => {
                return Err(format!(
                    "the browser box's desktop dropped the websocket: {e}"
                ));
            }
            Err(_) => return Err("the browser box's desktop did not answer in time".to_owned()),
        }
    }
}

/// The status and the lower-cased headers of an HTTP answer's head.
fn parse_head(head: &[u8]) -> Option<(u16, Vec<(String, String)>)> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Some((status, headers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_is_the_exec_host_on_6080_unless_named() {
        let c = Config::from_values(Some("http://browser:7700"), None);
        assert_eq!(c.exec.unwrap().as_str(), "http://browser:7700/");
        assert_eq!(c.desktop.unwrap().as_str(), "http://browser:6080/");
        let c = Config::from_values(Some("http://browser:7700"), Some("http://desk:9000"));
        assert_eq!(c.desktop.unwrap().as_str(), "http://desk:9000/");
        let c = Config::from_values(Some("http://browser:7700"), Some(""));
        assert_eq!(c.desktop.unwrap().as_str(), "http://browser:6080/");
    }

    #[test]
    fn an_empty_url_is_unset() {
        for v in [None, Some(""), Some("  "), Some("not a url")] {
            let c = Config::from_values(v, Some("http://desk:9000"));
            assert!(c.exec.is_none() && c.desktop.is_none(), "{v:?}");
        }
    }

    #[test]
    fn reads_a_handshake_answer() {
        let (status, headers) = parse_head(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: abc\r\n\r\n",
        )
        .unwrap();
        assert_eq!(status, 101);
        assert!(headers.contains(&("sec-websocket-accept".to_owned(), "abc".to_owned())));
    }
}
