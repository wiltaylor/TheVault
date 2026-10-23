//! A fake of the browser box's desktop side for the server's `/browser/` proxy (browser B4):
//! noVNC's static files, its `websockify` websocket, which echoes every frame, and the exec
//! service's unauthenticated `GET /health`. Point `VAULT_BROWSER_URL` and
//! `VAULT_BROWSER_DESKTOP_URL` at its `base_url()`.
//!
//! ```no_run
//! use testkit::{FakeNoVnc, WsClient};
//!
//! let fake = FakeNoVnc::start();
//! // VAULT_BROWSER_URL = fake.base_url(), VAULT_BROWSER_DESKTOP_URL = fake.base_url().
//! // GET /vnc.html answers testkit::NOVNC_PAGE.
//! let mut ws = WsClient::connect(fake.addr(), "/websockify", &[]).unwrap();
//! ws.send_binary(b"hello").unwrap();
//! assert_eq!(ws.read_frame().unwrap(), (2, b"hello".to_vec()));
//! let seen = fake.requests(); // each request's method, path and headers
//! ```
//!
//! It binds `127.0.0.1` on a free port. `GET /vnc.html` answers [`NOVNC_PAGE`] as `text/html`,
//! `GET /health` answers [`FakeNoVnc::health`], `GET /websockify` with `Upgrade: websocket`
//! completes the handshake, choosing `binary` when the client offers it, and sends each data
//! frame back unmasked with its opcode; anything else is 404.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::fake_http::lock;
use crate::standin::{Reply, Request, read_request, write_reply};

/// The page `GET /vnc.html` answers.
pub const NOVNC_PAGE: &str = "<!doctype html><title>noVNC</title><p>fake noVNC</p>";

/// The websocket handshake's fixed GUID (RFC 6455).
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// `Sec-WebSocket-Accept` for `key`.
pub fn ws_accept(key: &str) -> String {
    let digest = ring::digest::digest(
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        format!("{key}{WS_GUID}").as_bytes(),
    );
    STANDARD.encode(digest.as_ref())
}

/// A running fake. Dropping it stops accepting.
pub struct FakeNoVnc {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl FakeNoVnc {
    pub fn start() -> FakeNoVnc {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake noVNC");
        let addr = listener.local_addr().expect("listener address");
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (s, seen) = (stop.clone(), requests.clone());
        thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(conn) = conn else { continue };
                let seen = seen.clone();
                thread::spawn(move || {
                    let Ok(Some(request)) = read_request(&conn) else {
                        return;
                    };
                    lock(&seen).push(request.clone());
                    let _ = answer(conn, &request);
                });
            }
        });
        FakeNoVnc {
            addr,
            stop,
            requests,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Every request so far.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.requests).clone()
    }

    /// What `GET /health` answers.
    pub fn health() -> Value {
        json!({ "ok": true, "version": "0.0.0-fake", "display": ":1", "playwright_cli": "0.1.22" })
    }
}

impl Drop for FakeNoVnc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

fn answer(conn: TcpStream, req: &Request) -> std::io::Result<()> {
    let upgrade = req
        .header("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    match (req.method.as_str(), req.route()) {
        ("GET", "/websockify") if upgrade => echo(conn, req),
        ("GET" | "HEAD", "/vnc.html") => {
            let mut page = Reply::text(200, NOVNC_PAGE);
            page.headers = vec![("content-type".into(), "text/html".into())];
            write_reply(conn, &page)
        }
        ("GET", "/health") => write_reply(conn, &Reply::json(200, &FakeNoVnc::health())),
        _ => write_reply(conn, &Reply::text(404, "not found")),
    }
}

/// Completes the handshake, then sends every data frame back until a close.
fn echo(mut conn: TcpStream, req: &Request) -> std::io::Result<()> {
    let key = req.header("sec-websocket-key").unwrap_or_default();
    let mut head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n",
        ws_accept(key)
    );
    if req
        .header("sec-websocket-protocol")
        .is_some_and(|p| p.split(',').any(|p| p.trim() == "binary"))
    {
        head.push_str("Sec-WebSocket-Protocol: binary\r\n");
    }
    head.push_str("\r\n");
    conn.write_all(head.as_bytes())?;
    let mut reader = conn.try_clone()?;
    loop {
        let (opcode, payload) = read_frame(&mut reader)?;
        match opcode {
            8 => {
                write_frame(&mut conn, 8, &payload, false)?;
                return Ok(());
            }
            9 => write_frame(&mut conn, 10, &payload, false)?,
            _ => write_frame(&mut conn, opcode, &payload, false)?,
        }
    }
}

/// Reads one frame, unmasking it: its opcode and payload. Fragments are not joined.
fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h)?;
    let opcode = h[0] & 0x0f;
    let masked = h[1] & 0x80 != 0;
    let len = match h[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    let mut mask = [0u8; 4];
    if masked {
        r.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; usize::try_from(len).map_err(std::io::Error::other)?];
    r.read_exact(&mut payload)?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok((opcode, payload))
}

/// Writes one final frame, masked as a client's must be when `masked`.
fn write_frame(
    w: &mut impl Write,
    opcode: u8,
    payload: &[u8],
    masked: bool,
) -> std::io::Result<()> {
    let mut out = vec![0x80 | opcode];
    let bit = if masked { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(bit | n as u8),
        n if n <= usize::from(u16::MAX) => {
            out.push(bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    if masked {
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    } else {
        out.extend_from_slice(payload);
    }
    w.write_all(&out)?;
    w.flush()
}

/// A minimal websocket client, for checking a proxied websocket end to end.
pub struct WsClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    /// The status line and headers of the handshake answer, names lower-cased.
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

/// The `Sec-WebSocket-Key` the client sends.
pub const WS_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

impl WsClient {
    /// Opens a websocket to `path` on `addr`, offering the `binary` protocol, with `headers`
    /// added, such as a cookie or an `Authorization`. It answers whatever the server said: a
    /// status other than 101 leaves the connection unusable.
    pub fn connect(
        addr: SocketAddr,
        path: &str,
        headers: &[(&str, &str)],
    ) -> std::io::Result<WsClient> {
        let mut writer = TcpStream::connect(addr)?;
        writer.set_read_timeout(Some(Duration::from_secs(20)))?;
        let mut req = format!(
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {WS_KEY}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: binary\r\n"
        );
        for (n, v) in headers {
            req.push_str(&format!("{n}: {v}\r\n"));
        }
        req.push_str("\r\n");
        writer.write_all(req.as_bytes())?;
        let mut reader = BufReader::new(writer.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let status = line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut headers = Vec::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let l = line.trim_end();
            if l.is_empty() {
                break;
            }
            if let Some((n, v)) = l.split_once(':') {
                headers.push((n.trim().to_ascii_lowercase(), v.trim().to_owned()));
            }
        }
        Ok(WsClient {
            reader,
            writer,
            status,
            headers,
        })
    }

    /// The handshake answer's header `name`.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn send_binary(&mut self, payload: &[u8]) -> std::io::Result<()> {
        write_frame(&mut self.writer, 2, payload, true)
    }

    pub fn send_text(&mut self, text: &str) -> std::io::Result<()> {
        write_frame(&mut self.writer, 1, text.as_bytes(), true)
    }

    /// The next frame's opcode and payload.
    pub fn read_frame(&mut self) -> std::io::Result<(u8, Vec<u8>)> {
        read_frame(&mut self.reader)
    }

    /// Sends a close and reads the close sent back.
    pub fn close(mut self) -> std::io::Result<(u8, Vec<u8>)> {
        write_frame(&mut self.writer, 8, &1000u16.to_be_bytes(), true)?;
        read_frame(&mut self.reader)
    }
}
