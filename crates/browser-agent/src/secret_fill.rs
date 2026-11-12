//! Credential requests never enter the shell execution/logging path.
use super::*;
use tokio::io::AsyncWriteExt;

fn refused() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "browser_refused")
}

// No Debug: value must never be included in diagnostic formatting.
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    session: String,
    target: String,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

pub(super) fn configure(state: &AppState, enabled: bool) -> ApiResult<()> {
    let root = state.home.join(".secret-browser");
    safe_dir(&root)?;
    let dir = root.join(".playwright");
    safe_dir(&dir)?;
    let path = dir.join("cli.config.json");
    let data = if enabled {
        r#"{"browser":{"launchOptions":{"args":["--remote-debugging-address=127.0.0.1","--remote-debugging-port=0"]}}}"#
    } else {
        "{}"
    };
    // Changing eligibility closes old sessions, including their CDP listener.
    let previous = std::fs::read_to_string(&path).unwrap_or_default();
    if previous == data {
        return Ok(());
    }
    if !previous.is_empty() {
        kill_profile_processes(state.home.parent().ok_or_else(refused)?)?;
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .mode(0o600)
        .open(path)
        .map_err(|_| refused())?;
    file.write_all(data.as_bytes()).map_err(|_| refused())
}

pub(super) async fn origin(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<Request>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    execute(state, headers, body, false).await
}

pub(super) async fn fill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<Request>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    execute(state, headers, body, true).await
}

async fn execute(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<Request>, JsonRejection>,
    filling: bool,
) -> ApiResult<Json<Value>> {
    let _guard = profile_access(&state, &headers).await?;
    let state = scoped(&state, &headers, true)?;
    let Json(req) = body.map_err(|_| refused())?;
    if req.session.is_empty()
        || req.session.len() > 80
        || !req
            .session
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        || req.target.is_empty()
        || req.target.len() > 1024
        || req.value.as_ref().is_some_and(|s| s.len() > 65536)
        || filling != req.value.is_some()
        || filling != req.origin.is_some()
    {
        return Err(refused());
    }
    let _permit = state.runs.try_acquire().map_err(|_| refused())?;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let mut child = Command::new("node")
            .arg("-e")
            .arg(include_str!("secret-bridge.cjs"))
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", &state.home)
            .env("XDG_CACHE_HOME", state.home.join(".cache"))
            .env("TMPDIR", state.home.join(".runtime"))
            .current_dir(&state.work)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| refused())?;
        let mut stdin = child.stdin.take().ok_or_else(refused)?;
        stdin
            .write_all(&serde_json::to_vec(&req).map_err(|_| refused())?)
            .await
            .map_err(|_| refused())?;
        drop(stdin);
        let mut stdout = child.stdout.take().ok_or_else(refused)?.take(4097);
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| refused())?;
        if bytes.len() > 4096 {
            return Err(refused());
        }
        let status = child.wait().await.map_err(|_| refused())?;
        if !status.success() {
            return Err(refused());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| refused())?;
        // Never relay arbitrary helper output, even on a failure.
        if filling {
            match value["status"].as_str() {
                Some("filled") => Ok(json!({"status":"filled"})),
                Some("origin_mismatch") => Ok(json!({"status":"origin_mismatch"})),
                _ => Err(refused()),
            }
        } else {
            let origin = value["origin"]
                .as_str()
                .filter(|s| s.len() <= 2048)
                .ok_or_else(refused)?;
            Ok(json!({"origin":origin}))
        }
    })
    .await
    .map_err(|_| refused())??;
    Ok(Json(result))
}
