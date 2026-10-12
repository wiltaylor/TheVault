//! Seam S7: a local stand-in for the HTTP APIs scripts call.
//!
//! ```no_run
//! use std::time::Duration;
//! use testkit::{Reply, StandIn};
//!
//! let api = StandIn::start();
//! api.route("/hook", Reply::text(200, "ok"));
//! api.route("/slow", Reply::text(200, "late").delay(Duration::from_secs(5)));
//! let url = api.url("/hook"); // http://127.0.0.1:<port>/hook
//! // ... after the script ran:
//! let seen = api.requests();
//! ```
//!
//! It binds `127.0.0.1` on a free port. A path with no route answers 404. Every request is
//! recorded, whether or not it had a route. Each connection carries one request.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A canned response.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// How long to wait before answering.
    pub delay: Duration,
}

impl Reply {
    pub fn text(status: u16, body: impl Into<String>) -> Reply {
        Reply {
            status,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: body.into().into_bytes(),
            delay: Duration::ZERO,
        }
    }

    pub fn json(status: u16, body: &serde_json::Value) -> Reply {
        Reply {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.to_string().into_bytes(),
            delay: Duration::ZERO,
        }
    }

    pub fn delay(mut self, delay: Duration) -> Reply {
        self.delay = delay;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// One request the stand-in received. Header names are lower case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Default)]
struct Shared {
    routes: Mutex<HashMap<String, Reply>>,
    requests: Mutex<Vec<Request>>,
    stop: AtomicBool,
}

/// A running stand-in server. Dropping it stops the server.
pub struct StandIn {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

impl StandIn {
    pub fn start() -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stand-in server");
        let addr = listener.local_addr().expect("stand-in address");
        let shared = Arc::new(Shared::default());
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
        StandIn { addr, shared }
    }

    /// Answer requests for `path` with `reply`.
    pub fn route(&self, path: &str, reply: Reply) {
        lock(&self.shared.routes).insert(path.to_owned(), reply);
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The host name to put on an allowlist: `127.0.0.1`.
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// `http://127.0.0.1:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn serve(conn: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    if method.is_empty() {
        return Ok(());
    }
    let mut headers = HashMap::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((name, value)) = l.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let len = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    let route = path.split('?').next().unwrap_or_default().to_owned();
    lock(&shared.requests).push(Request {
        method,
        path,
        headers,
        body,
    });
    let reply = lock(&shared.routes)
        .get(&route)
        .cloned()
        .unwrap_or_else(|| Reply::text(404, "no route"));
    thread::sleep(reply.delay);
    let mut out = conn;
    let mut head = format!(
        "HTTP/1.1 {} Stand-in\r\ncontent-length: {}\r\nconnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (n, v) in &reply.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(&reply.body)?;
    out.flush()
}
