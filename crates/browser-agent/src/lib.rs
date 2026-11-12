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

use std::collections::{BTreeSet, HashMap};
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
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{OwnedRwLockReadGuard, RwLock, Semaphore};

mod secret_fill;

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
    runs: Arc<Semaphore>,
    profiles: Arc<Mutex<()>>,
    profile_locks: Arc<Mutex<HashMap<String, Arc<RwLock<()>>>>>,
    retired: Arc<Mutex<BTreeSet<String>>>,
}

/// The service's routes over `config`. Fails when the work or home directory does not exist.
pub fn router(config: Config) -> io::Result<Router> {
    let work = config.work_dir.canonicalize()?;
    let home = config.home_dir.canonicalize()?;
    let retired = load_retired(&home)?;
    let lexical_roots = vec![
        lexical(&absolute(&config.work_dir)?),
        lexical(&absolute(&config.home_dir)?),
    ];
    let state = AppState {
        roots: vec![work.clone(), home.clone()],
        lexical_roots,
        work,
        home,
        runs: Arc::new(Semaphore::new(MAX_RUNS)),
        profiles: Arc::new(Mutex::new(())),
        profile_locks: Arc::new(Mutex::new(HashMap::new())),
        retired: Arc::new(Mutex::new(retired)),
        config,
    };
    Ok(Router::new()
        .route("/health", get(health))
        .route("/run", post(run))
        .route("/secret/origin", post(secret_fill::origin))
        .route("/secret/fill", post(secret_fill::fill))
        .route("/file", get(file))
        .route("/files", get(files))
        .route("/profile", axum::routing::delete(delete_profile))
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

/// An authenticated server-chosen profile identity. The shell never supplies this header.
pub fn identity_signature(token: &str, identity: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key");
    mac.update(b"thevault-browser-profile-v1:");
    mac.update(identity.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn profile_id<'a>(state: &AppState, headers: &'a HeaderMap) -> ApiResult<&'a str> {
    authorize(state, headers)?;
    let id = headers
        .get("x-vault-agent")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("x-vault-agent-signature")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !constant_time_eq(
            signature.as_bytes(),
            identity_signature(&state.config.token, id).as_bytes(),
        )
    {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "a valid signed agent identity is needed",
        ));
    }
    Ok(id)
}

const PROFILE_LIMIT: usize = 64;
const PROFILE_BYTES: u64 = 512 * 1024 * 1024;

const RETIRED_INDEX: &str = ".retired-profile-ids.json";

fn store_retired(home: &Path, ids: &BTreeSet<String>) -> io::Result<()> {
    let tmp = home.join(format!(
        ".retired-profile-ids-{}.tmp",
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(ids)?)?;
    file.sync_all()?;
    std::fs::rename(&tmp, home.join(RETIRED_INDEX))?;
    std::fs::File::open(home)?.sync_all()?;
    Ok(())
}

fn load_retired(home: &Path) -> io::Result<BTreeSet<String>> {
    let index = home.join(RETIRED_INDEX);
    let mut ids = match std::fs::symlink_metadata(&index) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            serde_json::from_slice(&std::fs::read(&index)?)?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
        _ => {
            return Err(io::Error::other(
                "retired profile index is not a regular file",
            ));
        }
    };
    let markers = home.join(".retired-profiles");
    match std::fs::symlink_metadata(&markers) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            let entries: Vec<_> = std::fs::read_dir(&markers)?.collect::<io::Result<_>>()?;
            for entry in &entries {
                let id = entry.file_name().to_string_lossy().into_owned();
                let meta = std::fs::symlink_metadata(entry.path())?;
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || id.is_empty()
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(io::Error::other("invalid retired profile marker"));
                }
                ids.insert(id);
            }
            store_retired(home, &ids)?;
            // Only regular marker files were read. The index is durable before pruning.
            for entry in entries {
                std::fs::remove_file(entry.path())?;
            }
            std::fs::remove_dir(&markers)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        _ => {
            return Err(io::Error::other(
                "retired profile markers are not a real directory",
            ));
        }
    }
    Ok(ids)
}

fn profile_lock(state: &AppState, id: &str) -> Arc<RwLock<()>> {
    state
        .profile_locks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(id.to_owned())
        .or_default()
        .clone()
}

async fn profile_access(
    state: &AppState,
    headers: &HeaderMap,
) -> ApiResult<OwnedRwLockReadGuard<()>> {
    let id = profile_id(state, headers)?;
    let guard = profile_lock(state, id).read_owned().await;
    if state
        .retired
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
    {
        return Err(ApiError::new(
            StatusCode::GONE,
            "this browser profile was retired",
        ));
    }
    Ok(guard)
}

fn safe_dir(path: &Path) -> ApiResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::create_dir(path)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        _ => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "profile path is not a real directory",
        )),
    }
}

fn disk_usage(path: &Path) -> io::Result<u64> {
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let m = std::fs::symlink_metadata(entry.path())?;
        bytes = bytes.saturating_add(if m.is_dir() && !m.file_type().is_symlink() {
            disk_usage(&entry.path())?
        } else {
            m.len()
        });
        if bytes > PROFILE_BYTES {
            break;
        }
    }
    Ok(bytes)
}

fn scoped(state: &AppState, headers: &HeaderMap, admission: bool) -> ApiResult<AppState> {
    let id = profile_id(state, headers)?;
    let _guard = state.profiles.lock().unwrap_or_else(|e| e.into_inner());
    let root = state.home.join("profiles");
    safe_dir(&root)?;
    let profile = root.join(id);
    if !profile.exists()
        && std::fs::read_dir(&root)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .count()
            >= PROFILE_LIMIT
    {
        return Err(ApiError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "browser profile limit (64) reached",
        ));
    }
    safe_dir(&profile)?;
    let bytes = if admission {
        disk_usage(&profile)
    } else {
        Ok(0)
    }
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if admission && bytes > PROFILE_BYTES {
        return Err(ApiError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "browser profile exceeds 512 MiB; use Reset browser profile on the agent's Permissions page before running commands",
        ));
    }
    let home = profile.join("home");
    let work = profile.join("work");
    safe_dir(&home)?;
    safe_dir(&work)?;
    for dir in [".config", ".cache", ".runtime"] {
        safe_dir(&home.join(dir))?;
    }
    Ok(AppState {
        config: state.config.clone(),
        lexical_roots: vec![home.clone(), work.clone()],
        roots: vec![home.clone(), work.clone()],
        home,
        work,
        runs: state.runs.clone(),
        profiles: state.profiles.clone(),
        profile_locks: state.profile_locks.clone(),
        retired: state.retired.clone(),
    })
}

async fn delete_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let id = profile_id(&state, &headers)?;
    let lock = profile_lock(&state, id);
    let _exclusive = lock.write().await;
    {
        let mut retired = state.retired.lock().unwrap_or_else(|e| e.into_inner());
        retired.insert(id.to_owned());
        store_retired(&state.home, &retired)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    let root = state.home.join("profiles");
    let path = root.join(id);
    if root.exists() {
        safe_dir(&root)?;
    }
    if path.exists() {
        safe_dir(&path)?;
        let owned_path = path.clone();
        tokio::task::spawn_blocking(move || make_profile_directories_writable(&owned_path))
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
        let profile = scoped(&state, &headers, false)?;
        let outcome = run_command(
            &profile,
            "if command -v playwright-cli >/dev/null 2>&1; then playwright-cli close-all; fi",
            &profile.work,
            Duration::from_secs(30),
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if outcome.exit != 0 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "could not close this profile's browser sessions",
            ));
        }
    }
    let owned_path = path.clone();
    tokio::task::spawn_blocking(move || {
        kill_profile_processes(&owned_path)?;
        if owned_path.exists() {
            make_profile_directories_writable(&owned_path)?;
            for _ in 0..2 {
                if !owned_path.exists() {
                    break;
                }
                walk_profile_tree(&owned_path, true)?;
            }
        }
        Ok::<(), ApiError>(())
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    let _guard = state.profiles.lock().unwrap_or_else(|e| e.into_inner());
    if root.exists() {
        safe_dir(&root)?;
    }
    if std::fs::symlink_metadata(&path).is_ok() {
        safe_dir(&path)?;
    }
    state
        .profile_locks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id);
    Ok(Json(json!({"ok": true})))
}

/// Cache tools can leave 0555 directories. Only directories in the managed tree
/// are changed. Symlinks (including links to another profile) are never followed.
fn make_profile_directories_writable(path: &Path) -> ApiResult<()> {
    walk_profile_tree(path, false)
}

fn walk_error<'a>(
    root: &Path,
    components: impl Iterator<Item = &'a std::ffi::CStr>,
    tail: Option<&'a std::ffi::CStr>,
    error: impl std::fmt::Display,
) -> io::Error {
    // Construct labels only on failure. Retaining every full ancestor path
    // makes a deep tree quadratic in memory even with an iterative walk.
    let names: Vec<_> = components.chain(tail).collect();
    let mut label = root.display().to_string();
    for (i, name) in names.iter().enumerate() {
        if names.len() > 8 && i >= 3 && i < names.len() - 3 {
            if i == 3 {
                label.push_str("/...");
            }
            continue;
        }
        label.push('/');
        label.push_str(&name.to_string_lossy());
    }
    io::Error::other(format!("{label} (depth {}): {error}", names.len()))
}

fn walk_profile_tree(path: &Path, remove: bool) -> ApiResult<()> {
    use rustix::fs::{
        AtFlags, CWD, Dir, FileType, Mode, OFlags, chmodat, fstat, openat, statat, unlinkat,
    };
    use std::os::fd::AsRawFd;
    fn prepare(
        parent: impl rustix::fd::AsFd,
        name: impl rustix::path::Arg,
    ) -> io::Result<rustix::fd::OwnedFd> {
        let pinned = openat(
            parent,
            name,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mode = Mode::from_raw_mode(fstat(&pinned)?.st_mode) | Mode::RWXU;
        chmodat(
            CWD,
            format!("/proc/self/fd/{}", pinned.as_raw_fd()),
            mode,
            AtFlags::empty(),
        )
        .map_err(|e| {
            io::Error::other(format!(
                "pinned-directory chmod requires mounted /proc/self/fd; no path-based fallback: {e}"
            ))
        })?;
        let fd = openat(
            &pinned,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        // The pinned descriptor is dropped here, before descent.
        Ok(fd)
    }
    fn children(fd: &rustix::fd::OwnedFd) -> io::Result<Vec<std::ffi::CString>> {
        let mut entries = Dir::read_from(fd)?;
        let mut names = Vec::new();
        while let Some(entry) = entries.read() {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            names.push(name.to_owned());
        }
        // Dir and its duplicate fd are dropped before descent.
        Ok(names)
    }
    let result = (|| -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other("profile path is not a real directory"));
        }
        let mut fd = prepare(CWD, path)?;
        let names = children(&fd)?;
        let stat = fstat(&fd)?;
        let mut stack = vec![(names, stat.st_dev, stat.st_ino, None::<std::ffi::CString>)];
        while !stack.is_empty() {
            let Some(name) = stack.last_mut().expect("current frame").0.pop() else {
                let completed = stack.pop().expect("current frame");
                if let Some((_, dev, ino, _)) = stack.last() {
                    let parent = openat(
                        &fd,
                        "..",
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )?;
                    let stat = fstat(&parent)?;
                    if (stat.st_dev, stat.st_ino) != (*dev, *ino) {
                        return Err(io::Error::other("profile directory moved during cleanup"));
                    }
                    if remove {
                        unlinkat(
                            &parent,
                            completed.3.as_ref().expect("child name").as_c_str(),
                            AtFlags::REMOVEDIR,
                        )
                        .map_err(|e| {
                            walk_error(
                                path,
                                stack.iter().filter_map(|f| f.3.as_deref()),
                                completed.3.as_deref(),
                                e,
                            )
                        })?;
                    }
                    fd = parent;
                }
                continue;
            };
            let stat = statat(&fd, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(|e| {
                walk_error(
                    path,
                    stack.iter().filter_map(|f| f.3.as_deref()),
                    Some(&name),
                    e,
                )
            })?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
                if remove {
                    unlinkat(&fd, name.as_c_str(), AtFlags::empty()).map_err(|e| {
                        walk_error(
                            path,
                            stack.iter().filter_map(|f| f.3.as_deref()),
                            Some(&name),
                            e,
                        )
                    })?;
                }
                continue;
            }
            let child = prepare(&fd, name.as_c_str()).map_err(|e| {
                walk_error(
                    path,
                    stack.iter().filter_map(|f| f.3.as_deref()),
                    Some(&name),
                    e,
                )
            })?;
            let names = children(&child).map_err(|e| {
                walk_error(
                    path,
                    stack.iter().filter_map(|f| f.3.as_deref()),
                    Some(&name),
                    e,
                )
            })?;
            let stat = fstat(&child)?;
            fd = child;
            stack.push((names, stat.st_dev, stat.st_ino, Some(name)));
        }
        if remove {
            unlinkat(CWD, path, AtFlags::REMOVEDIR)?;
        }
        Ok(())
    })();
    result.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("preparing {} for removal: {e}", path.display()),
        )
    })
}

/// A successful shell can leave detached children behind. Close those too before
/// acknowledging deletion, otherwise they can recreate the retired directory.
fn kill_profile_processes(path: &Path) -> ApiResult<()> {
    let home = path.join("home");
    let expected = format!("HOME={}", home.display());
    for _ in 0..100 {
        let mut found = false;
        for entry in std::fs::read_dir("/proc")
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .flatten()
        {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            if pid <= 1 || pid as u32 == std::process::id() {
                continue;
            }
            let owns_home = std::fs::read(entry.path().join("environ"))
                .is_ok_and(|env| env.split(|b| *b == 0).any(|v| v == expected.as_bytes()));
            let owns_cwd =
                std::fs::read_link(entry.path().join("cwd")).is_ok_and(|cwd| cwd.starts_with(path));
            if owns_home || owns_cwd {
                found = true;
                // Numeric pid only. Permission errors leave deletion pending.
                let pid = rustix::process::Pid::from_raw(pid).expect("positive process id");
                if let Err(e) = rustix::process::kill_process(pid, rustix::process::Signal::KILL)
                    && e != rustix::io::Errno::SRCH
                {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "could not stop this profile's processes",
                    ));
                }
            }
        }
        if !found {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(ApiError::new(
        StatusCode::CONFLICT,
        "profile processes have not stopped; cleanup will retry",
    ))
}

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
        "profile_protocol": 1,
    }))
}

#[derive(Deserialize)]
struct RunRequest {
    command: String,
    timeout_s: Option<u64>,
    cwd: Option<String>,
    #[serde(default)]
    secret_browser: bool,
}

async fn run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<RunRequest>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    let _profile = profile_access(&state, &headers).await?;
    let state = scoped(&state, &headers, true)?;
    let Json(req) = body.map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.body_text()))?;
    secret_fill::configure(&state, req.secret_browser)?;
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
    eprintln!(
        "browser-agent: run exit={} ms={} timed_out={}",
        outcome.exit, outcome.ms, outcome.timed_out,
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
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("DISPLAY", &state.config.display)
        .env("HOME", &state.home)
        .env("XDG_CONFIG_HOME", state.home.join(".config"))
        .env("XDG_CACHE_HOME", state.home.join(".cache"))
        .env("XDG_RUNTIME_DIR", state.home.join(".runtime"))
        .env("TMPDIR", state.home.join(".runtime"))
        .env("PWTEST_CLI_GLOBAL_CONFIG", state.home.join(".secret-browser"))
        .env_remove("BROWSER_TOKEN")
        .env_remove("VAULT_BROWSER_TOKEN")
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
                    self.work.display(),
                    self.home.display()
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
    let _profile = profile_access(&state, &headers).await?;
    let state = scoped(&state, &headers, false)?;
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
    let _profile = profile_access(&state, &headers).await?;
    let state = scoped(&state, &headers, false)?;
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
