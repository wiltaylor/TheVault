//! The home server's HTTP API, as the device's sync engine calls it with its device token.
//!
//! Small JSON requests go through a blocking client with a connect timeout and a total timeout
//! ([`Timeouts::request`]). Assets go through an async client on a runtime of the engine's own,
//! with no total timeout: an upload streams from the file a block at a time and a download streams
//! to a file, and either fails only when no bytes have moved for [`Timeouts::stall`], so a slow
//! transfer that keeps moving finishes and a dead one fails (ticket 141).

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_LENGTH, IF_MATCH, IF_NONE_MATCH};
use serde_json::Value;

use crate::{Error, Remote, Result};

/// How long the engine waits on the network (ticket 141).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Opening a connection to the server.
    pub connect: Duration,
    /// The whole of one JSON request, such as a note push or a page of the change feed.
    pub request: Duration,
    /// An asset upload or download with no bytes moving. A transfer has no total limit.
    pub stall: Duration,
    /// How long after an asset failed to upload the next round tries it again. Each failure in a
    /// row doubles it, up to an hour.
    pub asset_retry: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(15),
            request: Duration::from_secs(60),
            stall: Duration::from_secs(60),
            asset_retry: Duration::from_secs(30),
        }
    }
}

/// How much one read of an asset file sends.
pub(crate) const CHUNK: usize = 64 * 1024;

/// After the last block of an upload is handed to the connection, the operating system may still
/// hold this much of it in its buffers, and it moves at no less than [`MIN_RATE`]: the answer may
/// take that much longer than [`Timeouts::stall`] to come.
const TAIL_BYTES: u64 = 8 * 1024 * 1024;
const MIN_RATE: u64 = 16 * 1024;

/// An answer from the server: its status and its body as JSON (`Null` when it is not JSON).
pub(crate) struct Answer {
    pub status: StatusCode,
    pub body: Value,
    /// The size of the body as it came, for the progress estimate's feed rate.
    pub bytes: u64,
}

impl Answer {
    /// The body of a success, or the error the status and body describe.
    pub fn ok(self, what: &str) -> Result<Value> {
        if self.status.is_success() {
            Ok(self.body)
        } else {
            Err(Error::Server {
                what: what.to_owned(),
                status: self.status.as_u16(),
                body: self.body.to_string(),
            })
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.body.get(key).and_then(Value::as_str)
    }
}

/// When a transfer last moved bytes, and whether its body has all been handed over.
struct Progress {
    start: Instant,
    /// Milliseconds after `start`.
    last_ms: AtomicU64,
    done: AtomicBool,
}

impl Progress {
    fn new() -> Arc<Progress> {
        Arc::new(Progress {
            start: Instant::now(),
            last_ms: AtomicU64::new(0),
            done: AtomicBool::new(false),
        })
    }

    fn touch(&self) {
        self.last_ms
            .store(self.start.elapsed().as_millis() as u64, Ordering::SeqCst);
    }

    fn idle(&self) -> Duration {
        self.start
            .elapsed()
            .saturating_sub(Duration::from_millis(self.last_ms.load(Ordering::SeqCst)))
    }
}

/// Resolves once `progress` has not moved for `stall`, or, once the body is all handed over,
/// for `stall` and the time the last `tail` bytes may take to leave the operating system's
/// buffers.
async fn stalled(progress: &Progress, stall: Duration, tail: u64) {
    let tick = (stall / 4).clamp(Duration::from_millis(10), Duration::from_secs(1));
    let grace = Duration::from_secs(tail.min(TAIL_BYTES) / MIN_RATE);
    loop {
        tokio::time::sleep(tick).await;
        let limit = if progress.done.load(Ordering::SeqCst) {
            stall + grace
        } else {
            stall
        };
        if progress.idle() >= limit {
            return;
        }
    }
}

fn secs(d: Duration) -> String {
    if d.as_secs() > 0 {
        format!("{} s", d.as_secs())
    } else {
        format!("{} ms", d.as_millis())
    }
}

/// A transfer error: the round fails when the server cannot be reached at all, and only the one
/// asset when the connection broke or stalled part way.
fn transfer_error(e: &reqwest::Error, what: &str) -> Error {
    if e.is_connect() {
        Error::Http(e.to_string())
    } else {
        Error::Transfer(format!("{what} failed part way: {e}"))
    }
}

/// The body of an asset upload: `file` read [`CHUNK`] bytes at a time as the connection takes
/// them, never the whole file at once. Each block read marks `progress`.
fn file_body(file: std::fs::File, progress: Arc<Progress>) -> reqwest::Body {
    let stream = futures_util::stream::unfold(Some(file), move |file| {
        let progress = progress.clone();
        async move {
            let mut file = file?;
            let mut buf = vec![0u8; CHUNK];
            match file.read(&mut buf) {
                Ok(0) => {
                    progress.done.store(true, Ordering::SeqCst);
                    progress.touch();
                    None
                }
                Ok(n) => {
                    buf.truncate(n);
                    progress.touch();
                    Some((Ok::<_, std::io::Error>(buf), Some(file)))
                }
                Err(e) => Some((Err(e), None)),
            }
        }
    });
    reqwest::Body::wrap_stream(stream)
}

/// The server of one paired device.
pub(crate) struct Home {
    client: Client,
    remote: Remote,
    timeouts: Timeouts,
    /// The asset transfers' runtime and client, made at the first transfer.
    transfers: OnceLock<(tokio::runtime::Runtime, reqwest::Client)>,
}

impl Drop for Home {
    fn drop(&mut self) {
        // Never blocks, so the engine may be dropped anywhere.
        if let Some((rt, _)) = self.transfers.take() {
            rt.shutdown_background();
        }
    }
}

impl Home {
    pub fn new(remote: Remote, timeouts: Timeouts) -> Result<Home> {
        let client = Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.request)
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(Home {
            client,
            remote,
            timeouts,
            transfers: OnceLock::new(),
        })
    }

    pub fn remote(&self) -> &Remote {
        &self.remote
    }

    pub fn timeouts(&self) -> &Timeouts {
        &self.timeouts
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.remote.url.trim_end_matches('/'))
    }

    fn authed(&self, rb: RequestBuilder) -> RequestBuilder {
        rb.header(AUTHORIZATION, format!("Bearer {}", self.remote.token))
            .header(ACCEPT, "application/json")
    }

    fn send(&self, rb: RequestBuilder) -> Result<Answer> {
        let res = rb.send().map_err(|e| Error::Http(e.to_string()))?;
        let status = res.status();
        let text = res.text().map_err(|e| Error::Http(e.to_string()))?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Answer {
            status,
            body,
            bytes: text.len() as u64,
        })
    }

    pub fn get(&self, path: &str) -> Result<Answer> {
        self.send(self.authed(self.client.get(self.url(path))))
    }

    /// `GET /api/sync/info`, which needs no token.
    pub fn info(&self) -> Result<Answer> {
        self.send(self.client.get(self.url("/api/sync/info")))
    }

    pub fn post_json(&self, path: &str, body: &Value) -> Result<Answer> {
        self.send(self.authed(self.client.post(self.url(path))).json(body))
    }

    pub fn put_json(&self, path: &str, body: &Value) -> Result<Answer> {
        self.send(self.authed(self.client.put(self.url(path))).json(body))
    }

    /// `POST` with no body and the note's base in `If-Match`.
    pub fn post_at(&self, path: &str, base: &str) -> Result<Answer> {
        self.send(
            self.authed(self.client.post(self.url(path)))
                .header(IF_MATCH, format!("\"{base}\"")),
        )
    }

    /// `PUT /api/notes/{id}` with `body` (`{"wcl"}` or `{"note"}`), at `base`, or creating the
    /// note of `type_name` under its id when `base` is `None`.
    pub fn put_note(
        &self,
        id: &str,
        type_name: &str,
        body: &Value,
        base: Option<&str>,
    ) -> Result<Answer> {
        let rb = self
            .authed(
                self.client
                    .put(self.url(&format!("/api/notes/{id}?type={type_name}"))),
            )
            .json(body);
        let rb = match base {
            Some(base) => rb.header(IF_MATCH, format!("\"{base}\"")),
            None => rb.header(IF_NONE_MATCH, "*"),
        };
        self.send(rb)
    }

    pub fn delete_note(&self, id: &str, base: &str) -> Result<Answer> {
        self.send(
            self.authed(self.client.delete(self.url(&format!("/api/notes/{id}"))))
                .header(IF_MATCH, format!("\"{base}\"")),
        )
    }

    /// The asset transfers' runtime and client.
    fn transfers(&self) -> Result<&(tokio::runtime::Runtime, reqwest::Client)> {
        if let Some(t) = self.transfers.get() {
            return Ok(t);
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Http(format!("starting the transfer runtime: {e}")))?;
        let client = reqwest::Client::builder()
            .connect_timeout(self.timeouts.connect)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        let _ = self.transfers.set((rt, client));
        Ok(self.transfers.get().expect("just set"))
    }

    /// `PUT /api/assets/{id}` with the file at `path`, streamed from disk. Fails with
    /// [`Error::Transfer`] when no bytes move for [`Timeouts::stall`], and with [`Error::Http`]
    /// only when the server cannot be reached at all.
    pub fn put_asset_file(&self, id: &str, filename: &str, path: &Path) -> Result<Answer> {
        let file = std::fs::File::open(path)
            .map_err(|e| Error::Transfer(format!("reading {}: {e}", path.display())))?;
        let size = file
            .metadata()
            .map_err(|e| Error::Transfer(format!("reading {}: {e}", path.display())))?
            .len();
        let (rt, client) = self.transfers()?;
        let progress = Progress::new();
        let request = client
            .put(self.url(&format!("/api/assets/{id}")))
            .query(&[("filename", filename)])
            .header(AUTHORIZATION, format!("Bearer {}", self.remote.token))
            .header(ACCEPT, "application/json")
            .header(CONTENT_LENGTH, size)
            .body(file_body(file, progress.clone()));
        let what = format!("sending asset {id}");
        let stall = self.timeouts.stall;
        rt.block_on(async {
            let send = async {
                let res = request
                    .send()
                    .await
                    .map_err(|e| transfer_error(&e, &what))?;
                progress.touch();
                let status = res.status();
                let text = res.text().await.map_err(|e| transfer_error(&e, &what))?;
                Ok(Answer {
                    status,
                    body: serde_json::from_str(&text).unwrap_or(Value::Null),
                    bytes: text.len() as u64,
                })
            };
            tokio::select! {
                answer = send => answer,
                () = stalled(&progress, stall, size) => Err(Error::Transfer(format!(
                    "{what} stalled: no bytes moved for {}", secs(stall)
                ))),
            }
        })
    }

    /// Download asset `id` into a new file at `dest`, streamed to disk, telling `on_bytes` the
    /// size of each block as it arrives (ticket 144). On failure the partial file is removed.
    /// Fails with [`Error::Transfer`] when no bytes move for [`Timeouts::stall`].
    pub fn download_asset(
        &self,
        id: &str,
        dest: &Path,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<()> {
        let (rt, client) = self.transfers()?;
        let what = format!("fetching asset {id}");
        let stall = self.timeouts.stall;
        let progress = Progress::new();
        let request = client
            .get(self.url(&format!("/api/assets/{id}")))
            .header(AUTHORIZATION, format!("Bearer {}", self.remote.token));
        let result = rt.block_on(async {
            let fetch = async {
                let mut res = request
                    .send()
                    .await
                    .map_err(|e| transfer_error(&e, &what))?;
                progress.touch();
                let status = res.status();
                if !status.is_success() {
                    return Err(Error::Server {
                        what: what.clone(),
                        status: status.as_u16(),
                        body: res.text().await.unwrap_or_default(),
                    });
                }
                let mut out = std::fs::File::create(dest)
                    .map_err(|e| Error::Transfer(format!("writing {}: {e}", dest.display())))?;
                while let Some(chunk) = res.chunk().await.map_err(|e| transfer_error(&e, &what))? {
                    progress.touch();
                    out.write_all(&chunk)
                        .map_err(|e| Error::Transfer(format!("writing {}: {e}", dest.display())))?;
                    on_bytes(chunk.len() as u64);
                }
                out.sync_all()
                    .map_err(|e| Error::Transfer(format!("writing {}: {e}", dest.display())))?;
                Ok(())
            };
            tokio::select! {
                done = fetch => done,
                () = stalled(&progress, stall, 0) => Err(Error::Transfer(format!(
                    "{what} stalled: no bytes moved for {}", secs(stall)
                ))),
            }
        });
        if result.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_upload_body_is_a_stream_of_small_blocks_not_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        // Sparse: 200 MB on disk costs nothing.
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(200 * 1024 * 1024).unwrap();
        let body = file_body(std::fs::File::open(&path).unwrap(), Progress::new());
        assert!(body.as_bytes().is_none(), "the body is held in memory");
    }

    /// The process's peak resident memory, in bytes, from `/proc/self/status`.
    fn peak_rss() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
        let kb: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        kb * 1024
    }

    #[test]
    fn a_200_mb_asset_uploads_without_the_process_holding_it() {
        use futures_util::StreamExt;

        // A server that counts what it receives and keeps none of it.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let app = axum::Router::new().route(
                    "/api/assets/{id}",
                    axum::routing::put(|req: axum::extract::Request| async move {
                        let mut stream = req.into_body().into_data_stream();
                        let mut received = 0u64;
                        while let Some(chunk) = stream.next().await {
                            received += chunk.unwrap().len() as u64;
                        }
                        axum::Json(serde_json::json!({ "received": received }))
                    }),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        let addr = rx.recv().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let size: u64 = 200 * 1024 * 1024;
        std::fs::File::create(&path).unwrap().set_len(size).unwrap();
        let home = Home::new(
            Remote {
                url: format!("http://{addr}"),
                token: "t".into(),
                device_id: "d".into(),
                device_name: "Phone".into(),
            },
            Timeouts::default(),
        )
        .unwrap();
        let before = peak_rss();
        let answer = home
            .put_asset_file("01J00000000000000000000000", "big.bin", &path)
            .unwrap();
        let grew = peak_rss().saturating_sub(before);
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.body["received"], size);
        assert!(
            grew < 64 * 1024 * 1024,
            "the upload grew the process by {grew} bytes"
        );
    }

    #[test]
    fn the_defaults_bound_connects_and_json_but_not_transfers() {
        let t = Timeouts::default();
        assert_eq!(t.connect, Duration::from_secs(15));
        assert_eq!(t.request, Duration::from_secs(60));
        assert_eq!(t.stall, Duration::from_secs(60));
    }
}
