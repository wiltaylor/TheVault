//! A fake of the Google APIs the Google sync calls: OAuth's token endpoint and userinfo, Gmail
//! and Calendar (google-sync D1, testing decisions).
//!
//! ```no_run
//! use testkit::FakeGoogle;
//!
//! let google = FakeGoogle::builder()
//!     .client("client-id", "client-secret")
//!     .account("me@example.com")
//!     .start();
//! let endpoints = google.endpoints(); // the `google_sync` note's `endpoints` dict
//! let host = google.host(); // for a task's `allow_domains`
//! // ... after a script ran:
//! let refreshes = google.requests_to("/token");
//! google.set_invalid_grant(true); // every token request now answers `invalid_grant`
//! ```
//!
//! It binds `127.0.0.1` on a free port, so tests running at once never share one. Every request
//! is recorded, whatever it was answered. Each connection carries one request. A path no API
//! answers gets 404 with Google's error body.
//!
//! Each API lives in its own module and keeps its own state, so the APIs grow independently:
//!
//! | Module | Paths | Endpoint key |
//! |--------|-------|--------------|
//! | `oauth` | `/token`, `/oauth2/v3/userinfo`, the consent page `/o/oauth2/v2/auth` | `token`, `userinfo` |
//! | `gmail` | under `/gmail/v1` | `gmail` |
//! | `calendar` | under `/calendar/v3` | `calendar` |
//!
//! A module adds its builder switches in an `impl FakeGoogleBuilder` block, its test controls in
//! an `impl FakeGoogle` block, its configuration and state in its own `Config` and `State`, and
//! answers its requests from its `handle` function. The endpoint values mirror Google's own:
//! `token` is `https://oauth2.googleapis.com/token`, `userinfo` is
//! `https://www.googleapis.com/oauth2/v3/userinfo`, `gmail` is
//! `https://gmail.googleapis.com/gmail/v1` and `calendar` is
//! `https://www.googleapis.com/calendar/v3`, so a script appends the same paths to either.

mod calendar;
mod gmail;
mod oauth;

pub use oauth::{
    AUTH_PATH, DEFAULT_ACCOUNT, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN,
};

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde_json::{Value, json};

use crate::standin::{Reply, Request, read_request, write_reply};

/// The path of the token endpoint.
pub const TOKEN_PATH: &str = "/token";

/// The path of the userinfo endpoint.
pub const USERINFO_PATH: &str = "/oauth2/v3/userinfo";

/// The path every Gmail API path starts with.
pub const GMAIL_PATH: &str = "/gmail/v1";

/// The path every Calendar API path starts with.
pub const CALENDAR_PATH: &str = "/calendar/v3";

/// How a [`FakeGoogle`] starts. Each API's module adds its own switches.
#[derive(Debug, Clone, Default)]
pub struct FakeGoogleBuilder {
    oauth: oauth::Config,
    gmail: gmail::Config,
    calendar: calendar::Config,
}

impl FakeGoogleBuilder {
    /// Bind a free port and start answering.
    pub fn start(self) -> FakeGoogle {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake Google server");
        let addr = listener.local_addr().expect("fake Google address");
        let shared = Arc::new(Shared {
            requests: Mutex::default(),
            stop: AtomicBool::new(false),
            oauth: Mutex::new(oauth::State::new(self.oauth)),
            gmail: Mutex::new(gmail::State::new(self.gmail)),
            calendar: Mutex::new(calendar::State::new(self.calendar)),
        });
        let s = shared.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                if s.stop.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(conn) = conn else { continue };
                let s = s.clone();
                thread::spawn(move || {
                    let _ = serve(conn, &s);
                });
            }
        });
        FakeGoogle { addr, shared }
    }
}

/// What the server threads share: the record of requests and each API's state.
struct Shared {
    requests: Mutex<Vec<Request>>,
    stop: AtomicBool,
    oauth: Mutex<oauth::State>,
    gmail: Mutex<gmail::State>,
    calendar: Mutex<calendar::State>,
}

/// A running fake Google server. Dropping it stops the server.
pub struct FakeGoogle {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

impl FakeGoogle {
    /// A builder with the defaults: see each module for its own.
    pub fn builder() -> FakeGoogleBuilder {
        FakeGoogleBuilder::default()
    }

    /// Start with the defaults.
    pub fn start() -> FakeGoogle {
        Self::builder().start()
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The host name to put on a task's `allow_domains`: `127.0.0.1`.
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// `http://127.0.0.1:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url())
    }

    /// The `google_sync` note's `endpoints` dict pointing at this server: keys `token`,
    /// `userinfo`, `gmail` and `calendar`.
    pub fn endpoints(&self) -> Value {
        json!({
            "token": self.url(TOKEN_PATH),
            "userinfo": self.url(USERINFO_PATH),
            "gmail": self.url(GMAIL_PATH),
            "calendar": self.url(CALENDAR_PATH),
        })
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }

    /// The requests whose path, without its query, starts with `prefix`.
    pub fn requests_to(&self, prefix: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.route().starts_with(prefix))
            .collect()
    }

    /// Forget the requests received so far.
    pub fn clear_requests(&self) {
        lock(&self.shared.requests).clear();
    }

    fn oauth(&self) -> MutexGuard<'_, oauth::State> {
        lock(&self.shared.oauth)
    }

    #[allow(dead_code)]
    fn gmail(&self) -> MutexGuard<'_, gmail::State> {
        lock(&self.shared.gmail)
    }

    #[allow(dead_code)]
    fn calendar(&self) -> MutexGuard<'_, calendar::State> {
        lock(&self.shared.calendar)
    }
}

impl Drop for FakeGoogle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn serve(conn: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some(request) = read_request(&conn)? else {
        return Ok(());
    };
    lock(&shared.requests).push(request.clone());
    let reply = oauth::handle(shared, &request)
        .or_else(|| gmail::handle(shared, &request))
        .or_else(|| calendar::handle(shared, &request))
        .unwrap_or_else(|| error(404, "NOT_FOUND", "Requested entity was not found."));
    write_reply(conn, &reply)
}

/// Google's error body for the REST APIs: `{ "error": { "code", "message", "status" } }`.
fn error(code: u16, status: &str, message: &str) -> Reply {
    Reply::json(
        code,
        &json!({ "error": { "code": code, "message": message, "status": status } }),
    )
}

/// Whether `request` carries a bearer access token the token endpoint issued. The Gmail,
/// Calendar and userinfo endpoints answer [`unauthorized`] otherwise.
fn authorized(shared: &Shared, request: &Request) -> bool {
    let Some(token) = request
        .header("authorization")
        .and_then(|h| h.strip_prefix("Bearer "))
    else {
        return false;
    };
    lock(&shared.oauth).has_access_token(token.trim())
}

/// The 401 Google answers a request with no valid access token.
fn unauthorized() -> Reply {
    error(
        401,
        "UNAUTHENTICATED",
        "Request had invalid authentication credentials. Expected OAuth 2 access token.",
    )
}
