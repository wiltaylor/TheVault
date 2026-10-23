//! The browser box's exec service (browser B2). It runs inside the box as the desktop user,
//! `agent`, and lets the server run shell commands there and read the files they leave, so
//! agents can drive `playwright-cli` on the box's desktop.
//!
//! Every request but `GET /health` needs `Authorization: Bearer <BROWSER_TOKEN>`; a missing or
//! wrong token is 401. Errors answer `{"error": "<message>"}`.
//!
//! - `POST /run {command, timeout_s?, cwd?}` runs `bash -lc <command>` with `DISPLAY` set to the
//!   desktop, `HOME` the agent's home and the working directory `/work`, or `cwd` (absolute, or
//!   relative to `/work`) when it is a directory under `/work` or `/home/agent`. `timeout_s` is 1
//!   to 300, default 60. It answers `{exit, stdout, stderr, truncated, ms, timed_out}`: stdout
//!   and stderr each keep their last 64 KiB, `truncated` saying either was cut; `exit` is the exit
//!   code, or 128 plus the signal that ended the command. A timeout kills the command's whole
//!   process group (exit 137, `timed_out` true). At most 4 runs at a time; a fifth is 429.
//! - `GET /file?path=` answers the bytes of a file under `/work` or `/home/agent` (absolute, or
//!   relative to `/work`), with its media type by extension. The path is canonicalised, so a
//!   symlink that leads out of both is refused (403). Over 25 MiB is 413, no such file 404.
//! - `GET /files?dir=` lists a directory there, `/work` without `dir`: `{dir, entries: [{name,
//!   kind, size, modified_ms}]}`, `kind` one of `file`, `dir`, `symlink` and `other`, sorted by
//!   name.
//! - `GET /health` answers `{ok, version, display, playwright_cli}` without a token.

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// The most of stdout, and of stderr, a run answers: the tail is kept.
pub const OUTPUT_CAP: usize = 64 * 1024;
/// The largest file `GET /file` answers.
pub const FILE_CAP: u64 = 25 * 1024 * 1024;
/// How many runs may go at once.
pub const MAX_RUNS: usize = 4;
pub const DEFAULT_TIMEOUT_S: u64 = 60;
pub const MAX_TIMEOUT_S: u64 = 300;
/// How long to wait for output after the command exits, for a background process that kept the
/// pipes open.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// What the service needs; `main` reads it from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    /// The bearer token every request but `/health` must carry.
    pub token: String,
    /// `/work`: the default working directory, and a root files are read from.
    pub work_dir: PathBuf,
    /// `/home/agent`: `HOME` for commands, and the other root files are read from.
    pub home_dir: PathBuf,
    /// The X display commands get as `DISPLAY`, `:1`.
    pub display: String,
    /// The installed `@playwright/cli` version, reported by `/health`.
    pub playwright_cli: Option<String>,
}

struct AppState {
    config: Config,
    /// The roots as configured and as canonicalised: a path must be lexically under one of the
    /// first and, resolved, under one of the second.
    lexical_roots: Vec<PathBuf>,
    roots: Vec<PathBuf>,
    work: PathBuf,
    home: PathBuf,
    runs: Semaphore,
}

/// The service's routes over `config`. Fails when the work or home directory does not exist.
pub fn router(config: Config) -> io::Result<Router> {
    let work = config.work_dir.canonicalize()?;
    let home = config.home_dir.canonicalize()?;
    let lexical_roots = vec![
        lexical(&absolute(&config.work_dir)?),
        lexical(&absolute(&config.home_dir)?),
    ];
    let state = AppState {
        roots: vec![work.clone(), home.clone()],
        lexical_roots,
        work,
        home,
        runs: Semaphore::new(MAX_RUNS),
        config,
    };
    Ok(Router::new()
        .route("/health", get(health))
        .route("/run", post(run))
        .route("/file", get(file))
        .route("/files", get(files))
        .with_state(Arc::new(state)))
}

/// Serves the routes on `listener` until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    config: Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    axum::serve(listener, router(config)?)
        .with_graceful_shutdown(shutdown)
        .await
}

struct ApiError(StatusCode, String);

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError(status, message.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn authorize(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if constant_time_eq(given.as_bytes(), state.config.token.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "a valid Authorization: Bearer <BROWSER_TOKEN> header is needed",
        ))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "display": state.config.display,
        "playwright_cli": state.config.playwright_cli,
    }))
}

#[derive(Deserialize)]
struct RunRequest {
    command: String,
    timeout_s: Option<u64>,
    cwd: Option<String>,
}

async fn run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<RunRequest>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &headers)?;
    let Json(req) = body.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.body_text()))?;
    if req.command.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "command must not be empty",
        ));
    }
    let timeout_s = req.timeout_s.unwrap_or(DEFAULT_TIMEOUT_S);
    if !(1..=MAX_TIMEOUT_S).contains(&timeout_s) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("timeout_s must be 1 to {MAX_TIMEOUT_S}, not {timeout_s}"),
        ));
    }
    let cwd = match req.cwd.as_deref() {
        None | Some("") => state.work.clone(),
        Some(raw) => {
            let dir = state.resolve(raw)?;
            if !dir.is_dir() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("cwd {raw} is not a directory"),
                ));
            }
            dir
        }
    };
    let _permit = state.runs.try_acquire().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            format!("{MAX_RUNS} commands are already running; try again when one finishes"),
        )
    })?;
    let outcome = run_command(&state, &req.command, &cwd, Duration::from_secs(timeout_s))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not run bash: {e}"),
            )
        })?;
    let first: String = req.command.chars().take(120).collect();
    eprintln!(
        "browser-agent: run exit={} ms={} timed_out={} cmd={}",
        outcome.exit,
        outcome.ms,
        outcome.timed_out,
        first.replace('\n', " ")
    );
    Ok(Json(json!({
        "exit": outcome.exit,
        "stdout": outcome.stdout,
        "stderr": outcome.stderr,
        "truncated": outcome.truncated,
        "ms": outcome.ms,
        "timed_out": outcome.timed_out,
    })))
}

struct Outcome {
    exit: i32,
    stdout: String,
    stderr: String,
    truncated: bool,
    ms: u128,
    timed_out: bool,
}

/// The last `OUTPUT_CAP` bytes a stream wrote, and whether it wrote more.
#[derive(Default)]
struct Tail {
    bytes: Vec<u8>,
    cut: bool,
}

impl Tail {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > OUTPUT_CAP {
            let excess = self.bytes.len() - OUTPUT_CAP;
            self.bytes.drain(..excess);
            self.cut = true;
        }
    }

    fn text(&self) -> String {
        let mut bytes = &self.bytes[..];
        if self.cut {
            // The cut may land inside a character: start at the next one.
            while let Some((&b, rest)) = bytes.split_first() {
                if b & 0b1100_0000 != 0b1000_0000 {
                    break;
                }
                bytes = rest;
            }
        }
        String::from_utf8_lossy(bytes).into_owned()
    }
}

async fn read_tail(mut stream: impl AsyncRead + Unpin, tail: Arc<Mutex<Tail>>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => tail
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(&buf[..n]),
        }
    }
}

/// Kills the command's process group when dropped armed: on a timeout, and when the request is
/// dropped because the client went away.
struct GroupKill(Option<u32>);

impl GroupKill {
    fn kill(&mut self) {
        if let Some(pid) = self.0.take()
            && let Some(pid) = rustix::process::Pid::from_raw(pid as i32)
        {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn run_command(
    state: &AppState,
    command: &str,
    cwd: &Path,
    timeout: Duration,
) -> io::Result<Outcome> {
    let started = Instant::now();
    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .env("DISPLAY", &state.config.display)
        .env("HOME", &state.home)
        .env_remove("BROWSER_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, which a timeout kills whole.
        .process_group(0)
        .spawn()?;
    let mut group = GroupKill(child.id());
    let out = Arc::new(Mutex::new(Tail::default()));
    let err = Arc::new(Mutex::new(Tail::default()));
    let readers = [
        child
            .stdout
            .take()
            .map(|s| tokio::spawn(read_tail(s, out.clone()))),
        child
            .stderr
            .take()
            .map(|s| tokio::spawn(read_tail(s, err.clone()))),
    ];
    let (status, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => (status?, false),
        Err(_) => {
            group.kill();
            (child.wait().await?, true)
        }
    };
    // The command is done; what it left running is its own, unless it timed out.
    group.0 = None;
    for reader in readers.into_iter().flatten() {
        let abort = reader.abort_handle();
        if tokio::time::timeout(DRAIN_GRACE, reader).await.is_err() {
            abort.abort();
        }
    }
    let exit = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    let out = out.lock().unwrap_or_else(|e| e.into_inner());
    let err = err.lock().unwrap_or_else(|e| e.into_inner());
    Ok(Outcome {
        exit,
        stdout: out.text(),
        stderr: err.text(),
        truncated: out.cut || err.cut,
        ms: started.elapsed().as_millis(),
        timed_out,
    })
}

impl AppState {
    /// `raw`, absolute or relative to the work directory, resolved and confined to the roots.
    fn resolve(&self, raw: &str) -> ApiResult<PathBuf> {
        if raw.is_empty() {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "path is empty"));
        }
        let given = Path::new(raw);
        let joined = if given.is_absolute() {
            given.to_path_buf()
        } else {
            self.work.join(given)
        };
        let outside = || {
            ApiError::new(
                StatusCode::FORBIDDEN,
                format!(
                    "{raw} is outside {} and {}",
                    self.config.work_dir.display(),
                    self.config.home_dir.display()
                ),
            )
        };
        let lexical_path = lexical(&joined);
        if !self
            .lexical_roots
            .iter()
            .chain(&self.roots)
            .any(|r| lexical_path.starts_with(r))
        {
            return Err(outside());
        }
        let real = joined.canonicalize().map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => {
                ApiError::new(StatusCode::NOT_FOUND, format!("{raw} does not exist"))
            }
            _ => ApiError::new(StatusCode::BAD_REQUEST, format!("{raw}: {e}")),
        })?;
        // A symlink may lead out of the roots: the resolved path must stay inside.
        if !self.roots.iter().any(|r| real.starts_with(r)) {
            return Err(outside());
        }
        Ok(real)
    }
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// `path` with `.` and `..` worked out, without looking at the file system.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

async fn file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    query: Result<Query<FileQuery>, QueryRejection>,
) -> ApiResult<Response> {
    authorize(&state, &headers)?;
    let Query(query) = query.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.body_text()))?;
    let path = state.resolve(&query.path)?;
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("{}: {e}", query.path)))?;
    if !meta.is_file() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{} is not a file", query.path),
        ));
    }
    if meta.len() > FILE_CAP {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "{} is {} bytes, more than the {} MiB limit",
                query.path,
                meta.len(),
                FILE_CAP / (1024 * 1024)
            ),
        ));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("{}: {e}", query.path)))?;
    // The file may have grown since the check.
    if bytes.len() as u64 > FILE_CAP {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "{} is more than the {} MiB limit",
                query.path,
                FILE_CAP / (1024 * 1024)
            ),
        ));
    }
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(media_type(&path)),
    );
    Ok(response)
}

/// The media type of a file by its extension.
pub fn media_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "yml" | "yaml" => "application/yaml",
        "txt" | "log" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "xml" => "application/xml",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "webm" => "video/webm",
        "mp4" => "video/mp4",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

#[derive(Deserialize)]
struct FilesQuery {
    dir: Option<String>,
}

async fn files(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    query: Result<Query<FilesQuery>, QueryRejection>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &headers)?;
    let Query(query) = query.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.body_text()))?;
    let dir = match query.dir.as_deref() {
        None | Some("") => state.work.clone(),
        Some(raw) => state.resolve(raw)?,
    };
    if !dir.is_dir() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{} is not a directory", dir.display()),
        ));
    }
    let mut reader = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("{}: {e}", dir.display())))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("{}: {e}", dir.display())))?
    {
        let Ok(meta) = tokio::fs::symlink_metadata(entry.path()).await else {
            continue;
        };
        let kind = if meta.file_type().is_symlink() {
            "symlink"
        } else if meta.is_dir() {
            "dir"
        } else if meta.is_file() {
            "file"
        } else {
            "other"
        };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);
        entries.push(json!({
            "name": entry.file_name().to_string_lossy(),
            "kind": kind,
            "size": meta.len(),
            "modified_ms": modified_ms,
        }));
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(Json(
        json!({ "dir": dir.to_string_lossy(), "entries": entries }),
    ))
}
