//! What the AI fakes share: a listener on a free port that hands each request to a handler,
//! and answers that are either whole or a server-sent event stream written frame by frame.

use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::standin::{Reply, Request, read_request, write_reply};

/// An answer: whole, or streamed.
#[derive(Debug, Clone)]
pub(crate) enum Answer {
    Whole(Reply),
    Stream(Streamed),
}

/// A `text/event-stream` answer. The body has no length and ends when the connection closes,
/// so a cut stream looks to the client as a real one does.
#[derive(Debug, Clone)]
pub(crate) struct Streamed {
    pub frames: Vec<String>,
    /// Close the connection after this many frames, without the rest.
    pub cut_after: Option<usize>,
    /// The pause after each frame.
    pub delay: Duration,
    /// Response headers beyond the content type.
    pub headers: Vec<(String, String)>,
}

/// One SSE frame: `event: <event>\ndata: <data>\n\n`.
pub(crate) fn frame(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn write_stream(conn: TcpStream, s: &Streamed) -> std::io::Result<()> {
    let mut out = conn;
    let mut head = String::from(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n",
    );
    for (n, v) in &s.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.flush()?;
    let n = s.cut_after.unwrap_or(s.frames.len()).min(s.frames.len());
    for f in &s.frames[..n] {
        // Each frame goes in two writes, split mid-way, so clients meet frames cut across reads.
        let bytes = f.as_bytes();
        let mid = bytes.len() / 2;
        out.write_all(&bytes[..mid])?;
        out.flush()?;
        out.write_all(&bytes[mid..])?;
        out.flush()?;
        if !s.delay.is_zero() {
            thread::sleep(s.delay);
        }
    }
    if s.cut_after.is_some() {
        // A broken-off answer: half of the next frame, then the connection drops.
        if let Some(next) = s.frames.get(n) {
            out.write_all(&next.as_bytes()[..next.len() / 2])?;
            out.flush()?;
        }
    }
    let _ = out.shutdown(Shutdown::Write);
    Ok(())
}

pub(crate) fn write_answer(conn: TcpStream, answer: &Answer) -> std::io::Result<()> {
    match answer {
        Answer::Whole(r) => write_reply(conn, r),
        Answer::Stream(s) => write_stream(conn, s),
    }
}

/// A running listener. Dropping it stops accepting.
pub(crate) struct Listener {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

/// Bind `127.0.0.1` on a free port and answer each request with `handle`, which also records
/// it. Each connection carries one request.
pub(crate) fn listen<F>(what: &str, handle: F) -> Listener
where
    F: Fn(&Request) -> Answer + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind {what}: {e}"));
    let addr = listener.local_addr().expect("listener address");
    let stop = Arc::new(AtomicBool::new(false));
    let handle = Arc::new(handle);
    let s = stop.clone();
    thread::spawn(move || {
        for conn in listener.incoming() {
            if s.load(Ordering::SeqCst) {
                return;
            }
            let Ok(conn) = conn else { continue };
            let handle = handle.clone();
            thread::spawn(move || {
                let Ok(Some(request)) = read_request(&conn) else {
                    return;
                };
                let answer = handle(&request);
                let _ = write_answer(conn, &answer);
            });
        }
    });
    Listener { addr, stop }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Split `text` into pieces of at most `n` characters, at least one piece.
pub(crate) fn pieces(text: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars.chunks(n.max(1)).map(|c| c.iter().collect()).collect()
}
