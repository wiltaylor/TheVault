//! A fake of the browser box's exec service (`crates/browser-agent`, browser B2), for the chat's
//! browser tools (browser B5). Point the server's `VAULT_BROWSER_URL` at [`FakeBrowser::base_url`]
//! and `VAULT_BROWSER_TOKEN` at [`FakeBrowser::token`].
//!
//! ```no_run
//! use testkit::{BrowserRun, FakeBrowser};
//!
//! let fake = FakeBrowser::start();
//! fake.push_run(BrowserRun::ok("### Page\n- button \"Go\" [ref=e21]"));
//! fake.file("/work/shot.png", "image/png", vec![0x89, 0x50]);
//! // ... after the calls:
//! let ran = fake.commands(); // each /run's command and timeout_s
//! ```
//!
//! It answers as the service does:
//!
//! | Route | Answer |
//! |---|---|
//! | `GET /health` | `{ok, version, display, playwright_cli}`, no token needed |
//! | `POST /run {command, timeout_s, cwd?}` | the next queued [`BrowserRun`], else exit 0 with stdout `ran: <command>`: `{exit, stdout, stderr, truncated, ms, timed_out}` |
//! | `GET /file?path=` | a file a test added with [`FakeBrowser::file`], its bytes with its media type; 404 when there is none |
//! | `GET /files?dir=` | the files added under `dir` (default `/work`): `{dir, entries: [{name, kind, size, modified_ms}]}` |
//!
//! Every route but health needs `Authorization: Bearer <token>` (401 otherwise). A path not
//! starting with `/` is relative to `/work`. Errors are `{error}`. It binds `127.0.0.1` on a free
//! port and records every request.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::fake_http::{Answer, Listener, listen, lock};
use crate::standin::{Reply, Request};

/// The token the fake accepts.
pub const DEFAULT_BROWSER_TOKEN: &str = "browser-test-token-0123456789";

/// The work directory relative paths resolve under.
pub const BROWSER_WORK: &str = "/work";

/// What one `/run` answers.
#[derive(Debug, Clone)]
pub enum BrowserRun {
    /// The command ran.
    Ran {
        exit: i64,
        stdout: String,
        stderr: String,
        timed_out: bool,
        truncated: bool,
        ms: u64,
    },
    /// An error status with `{error}`, such as 429 when four runs are going.
    Status(u16, String),
}

impl BrowserRun {
    /// Exit 0 with `stdout`.
    pub fn ok(stdout: &str) -> BrowserRun {
        BrowserRun::Ran {
            exit: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
            timed_out: false,
            truncated: false,
            ms: 42,
        }
    }

    /// Exit `exit` with `stdout` and `stderr`.
    pub fn exited(exit: i64, stdout: &str, stderr: &str) -> BrowserRun {
        BrowserRun::Ran {
            exit,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
            timed_out: false,
            truncated: false,
            ms: 42,
        }
    }

    /// Killed at its timeout: exit 137.
    pub fn timed_out(stdout: &str) -> BrowserRun {
        BrowserRun::Ran {
            exit: 137,
            stdout: stdout.to_owned(),
            stderr: String::new(),
            timed_out: true,
            truncated: false,
            ms: 1000,
        }
    }

    /// 429: four commands are already running.
    pub fn busy() -> BrowserRun {
        BrowserRun::Status(
            429,
            "4 commands are already running; try again when one finishes".to_owned(),
        )
    }
}

#[derive(Default)]
struct Shared {
    runs: Mutex<VecDeque<BrowserRun>>,
    files: Mutex<BTreeMap<String, (String, Vec<u8>)>>,
    /// What the next command that writes a file (`--filename=` or `curl -o`) leaves there.
    capture: Mutex<Option<(String, Vec<u8>)>>,
    requests: Mutex<Vec<Request>>,
}

/// The file a command writes, as `browser_capture` quotes it: `--filename='<path>'` or
/// `-o '<path>'`.
fn written(command: &str) -> Option<String> {
    for mark in ["--filename='", "-o '"] {
        if let Some(rest) = command.split(mark).nth(1) {
            return rest.split('\'').next().map(str::to_owned);
        }
    }
    None
}

/// A running fake browser box. Dropping it stops it.
pub struct FakeBrowser {
    listener: Listener,
    shared: Arc<Shared>,
}

impl FakeBrowser {
    pub fn start() -> FakeBrowser {
        let shared = Arc::new(Shared::default());
        let s = shared.clone();
        let listener = listen("the fake browser box", move |r| {
            lock(&s.requests).push(r.clone());
            Answer::Whole(answer(&s, r))
        });
        FakeBrowser { listener, shared }
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener.addr
    }

    /// `http://<addr>`, for `VAULT_BROWSER_URL`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.listener.addr)
    }

    /// The token, for `VAULT_BROWSER_TOKEN`.
    pub fn token(&self) -> &'static str {
        DEFAULT_BROWSER_TOKEN
    }

    /// Answer the next `/run` with `run`.
    pub fn push_run(&self, run: BrowserRun) {
        lock(&self.shared.runs).push_back(run);
    }

    /// Hold a file at `path` (relative paths are under `/work`) for `/file` and `/files`.
    pub fn file(&self, path: &str, media_type: &str, bytes: Vec<u8>) {
        lock(&self.shared.files).insert(absolute(path), (media_type.to_owned(), bytes));
    }

    /// Leave `bytes` at the path the next command that writes a file names, as a screenshot or a
    /// download in the box would.
    pub fn capture(&self, media_type: &str, bytes: Vec<u8>) {
        *lock(&self.shared.capture) = Some((media_type.to_owned(), bytes));
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }

    /// The body of every `/run` so far: `{command, timeout_s, cwd?}`.
    pub fn commands(&self) -> Vec<Value> {
        self.requests()
            .iter()
            .filter(|r| r.route() == "/run")
            .map(Request::json)
            .collect()
    }
}

fn absolute(path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{BROWSER_WORK}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

fn error(status: u16, message: &str) -> Reply {
    Reply::json(status, &json!({ "error": message }))
}

fn answer(s: &Shared, r: &Request) -> Reply {
    if r.route() == "/health" {
        return Reply::json(
            200,
            &json!({ "ok": true, "version": "0.0.0-fake", "display": ":1", "playwright_cli": "0.1.22" }),
        );
    }
    let expected = format!("Bearer {DEFAULT_BROWSER_TOKEN}");
    if r.header("authorization") != Some(expected.as_str()) {
        return error(401, "missing or wrong bearer token");
    }
    match (r.method.as_str(), r.route()) {
        ("POST", "/run") => {
            let body = r.json();
            let command = body["command"].as_str().unwrap_or_default().to_owned();
            if command.trim().is_empty() {
                return error(400, "command must not be empty");
            }
            let timeout = body["timeout_s"].as_u64().unwrap_or(60);
            if !(1..=300).contains(&timeout) {
                return error(400, &format!("timeout_s must be 1 to 300, not {timeout}"));
            }
            if let Some(path) = written(&command)
                && let Some(file) = lock(&s.capture).take()
            {
                lock(&s.files).insert(absolute(&path), file);
            }
            let next = lock(&s.runs).pop_front();
            match next.unwrap_or_else(|| BrowserRun::ok(&format!("ran: {command}"))) {
                BrowserRun::Status(status, message) => error(status, &message),
                BrowserRun::Ran {
                    exit,
                    stdout,
                    stderr,
                    timed_out,
                    truncated,
                    ms,
                } => Reply::json(
                    200,
                    &json!({
                        "exit": exit, "stdout": stdout, "stderr": stderr,
                        "truncated": truncated, "ms": ms, "timed_out": timed_out,
                    }),
                ),
            }
        }
        ("GET", "/file") => {
            let Some(path) = r.query("path").filter(|p| !p.is_empty()) else {
                return error(400, "path is empty");
            };
            match lock(&s.files).get(&absolute(&path)) {
                Some((media, bytes)) => Reply {
                    status: 200,
                    headers: vec![("content-type".into(), media.clone())],
                    body: bytes.clone(),
                    delay: std::time::Duration::ZERO,
                },
                None => error(404, &format!("{path} does not exist")),
            }
        }
        ("GET", "/files") => {
            let dir = absolute(
                &r.query("dir")
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| BROWSER_WORK.to_owned()),
            );
            let prefix = if dir == "/" {
                "/".to_owned()
            } else {
                format!("{dir}/")
            };
            let entries: Vec<Value> = lock(&s.files)
                .iter()
                .filter_map(|(p, (_, bytes))| {
                    let rest = p.strip_prefix(&prefix)?;
                    (!rest.contains('/')).then(|| {
                        json!({ "name": rest, "kind": "file", "size": bytes.len(), "modified_ms": 0 })
                    })
                })
                .collect();
            Reply::json(200, &json!({ "dir": dir, "entries": entries }))
        }
        _ => error(404, "no such route"),
    }
}
