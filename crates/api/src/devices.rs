//! Paired devices: the desktop and Android apps (D2, D3).
//!
//! A device pairs once at `POST /api/devices/pair` with the vault password or a single-use code
//! from `POST /api/devices/pair-codes`, and gets a `write` token bound to it. Settings > Devices
//! lists, renames and revokes devices, and holds each device's settings: the commands it accepts,
//! its contacts-write and SMS-send switches, its exclusion list and its skip pattern (D22, D23,
//! D25). Revoking deletes the device's token and its push endpoint.
//!
//! `/api/device-files/{platform}` reads and writes `devices/<platform>.wcl`, the device events'
//! handlers and the commands devices of the platform accept (D5). A device fetches the commands
//! server scripts queued for it from `GET /api/devices/{id}/commands?status=pending` and reports
//! on each at `POST /api/devices/{id}/commands/{cid}/result` (D22). Settings > Devices queues one
//! itself at `POST /api/devices/{id}/commands`, as "Import from phone" does.
//!
//! A phone reports its capture health, what stops it capturing, at `PUT /api/devices/{id}/health`
//! with each settings sync, and the device list carries the last report (ticket 91).
//!
//! A device reports each device event whose handler failed for good at
//! `POST /api/devices/{id}/event-failures`, which adds one notification under the bell from
//! `device:<id>`, so the web app shows it too.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use qrcode::QrCode;
use qrcode::render::svg;
use schema::{DEVICE_COMMANDS, DEVICE_EVENTS, DEVICE_PLATFORMS, DeviceFile, DeviceHook};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use state::{Author, CommandStatus, Device, NewDevice, PushTarget, Scope};
use writer::Writer;

use crate::auth::GrantedScope;
use crate::defs::{WclBody, body, problems_json, text_or_json};
use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// How long a pairing code works when the request names no lifetime.
const DEFAULT_CODE_TTL_SECS: u64 = 10 * 60;

/// The longest lifetime a pairing code may ask for.
const MAX_CODE_TTL_SECS: u64 = 60 * 60;

/// The default skip pattern: messages that look like one-time codes and verification texts
/// (D25). Matched case-insensitively against an SMS body.
pub const DEFAULT_SKIP_PATTERN: &str = r"(?i)\b(verification|security|confirmation|login|sign[- ]?in|one[- ]?time|access)\s+code\b|\b(otp|passcode|2fa)\b|\bcode\s*(is|:)\s*[0-9]{4,8}\b|\b[0-9]{4,8}\s+is\s+your\b";

/// What a device's exclusion list entry names (D25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExclusionKind {
    Contact,
    Number,
    Thread,
}

/// A contact, number or thread whose messages and calls are never captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exclusion {
    kind: ExclusionKind,
    value: String,
}

/// A device's settings, stored as JSON in `state.db` and synced to the device (D22, D23, D25).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// The commands server scripts may send this device, from those its `devices/<platform>.wcl`
    /// declares.
    #[serde(default)]
    commands: Vec<String>,
    /// Whether device scripts may write and delete phone contacts.
    #[serde(default)]
    contacts_write: bool,
    /// Whether device scripts may send SMS.
    #[serde(default)]
    sms_send: bool,
    #[serde(default)]
    exclusions: Vec<Exclusion>,
    /// A regular expression; an SMS whose body matches it is not captured. Empty skips nothing.
    #[serde(default = "default_skip_pattern")]
    skip_pattern: String,
}

fn default_skip_pattern() -> String {
    DEFAULT_SKIP_PATTERN.to_owned()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            commands: Vec::new(),
            contacts_write: false,
            sms_send: false,
            exclusions: Vec::new(),
            skip_pattern: default_skip_pattern(),
        }
    }
}

/// A plain name of a command or capability: lower case letters, digits, `_`, `-` and `.`.
fn is_plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
}

impl Settings {
    fn check(&self) -> Result<(), ApiError> {
        if let Some(bad) = self.commands.iter().find(|c| !is_plain_name(c)) {
            return Err(ApiError::bad_request(format!(
                "command name {bad:?} must be 1 to 64 lower case letters, digits, `_`, `-` or `.`"
            )));
        }
        if self.commands.len() > 64 {
            return Err(ApiError::bad_request("a device takes at most 64 commands"));
        }
        if self.exclusions.len() > 1000 {
            return Err(ApiError::bad_request(
                "the exclusion list takes at most 1000 entries",
            ));
        }
        if let Some(bad) = self
            .exclusions
            .iter()
            .find(|e| e.value.trim().is_empty() || e.value.len() > 256)
        {
            return Err(ApiError::bad_request(format!(
                "exclusion {:?} must have a value of 1 to 256 characters",
                bad.value
            )));
        }
        if self.skip_pattern.len() > 4096 {
            return Err(ApiError::bad_request(
                "the skip pattern is longer than 4096 characters",
            ));
        }
        regex::Regex::new(&self.skip_pattern).map_err(|e| {
            ApiError::bad_request(format!("the skip pattern is not a valid regex: {e}"))
        })?;
        Ok(())
    }
}

/// A device name is what the user reads in Settings > Devices.
fn check_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "a device name must be 1 to 64 characters with no control characters",
        ));
    }
    Ok(name.to_owned())
}

fn parse_json(text: &str, what: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| {
        // `state.db` only ever holds what this module wrote, so this means it was edited by hand.
        json!({ "error": format!("the stored {what} is not JSON") })
    })
}

fn device_json(d: &Device) -> Value {
    json!({
        "id": d.id,
        "name": d.name,
        "platform": d.platform,
        "capabilities": parse_json(&d.capabilities, "capabilities"),
        "token": d.token_id,
        "push": d.push.is_some(),
        "last_seen": d.last_seen_ms,
        "created": d.created_ms,
        "settings": parse_json(&d.settings, "settings"),
        "health": health_json(d),
    })
}

/// The capture health the device last reported, with `reported`, when the server heard it, or
/// null before its first report (ticket 91).
fn health_json(d: &Device) -> Value {
    match &d.health {
        None => Value::Null,
        Some(text) => {
            let mut health = parse_json(text, "capture health");
            if let Value::Object(m) = &mut health {
                m.insert("reported".to_owned(), json!(d.health_ms));
            }
            health
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairBody {
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    code: Option<String>,
    name: String,
    platform: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

/// `POST /api/devices/pair`: check the password or pairing code, record the device and return
/// its id with a `write` token bound to it. The token is shown once.
pub async fn pair(State(w): Shared, Json(body): Json<PairBody>) -> Result<Response, ApiError> {
    let name = check_name(&body.name)?;
    if let Some(bad) = body.capabilities.iter().find(|c| !is_plain_name(c)) {
        return Err(ApiError::bad_request(format!(
            "capability {bad:?} must be 1 to 64 lower case letters, digits, `_`, `-` or `.`"
        )));
    }
    if body.capabilities.len() > 64 {
        return Err(ApiError::bad_request(
            "a device has at most 64 capabilities",
        ));
    }
    let capabilities = serde_json::to_string(&body.capabilities)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let settings = serde_json::to_string(&Settings::default())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let (device, token) = blocking(w, move |w| {
        let state = w.state();
        let ok = match (&body.password, &body.code) {
            (Some(password), None) => state.verify_password(password)?,
            (None, Some(code)) => state.take_pair_code(code)?,
            _ => {
                return Err(ApiError::bad_request(
                    "send either `password` or `code`, not both",
                ));
            }
        };
        if !ok {
            let what = if body.code.is_some() {
                "the pairing code is wrong, used or expired"
            } else {
                "wrong password"
            };
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                what,
            ));
        }
        Ok(state.pair_device(&NewDevice {
            name: &name,
            platform: &body.platform,
            capabilities: &capabilities,
            settings: &settings,
        })?)
    })
    .await?;
    let out = json!({
        "device_id": device.id,
        "token": token,
        "device": device_json(&device),
    });
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PairCodeBody {
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    ttl_secs: Option<u64>,
}

/// The server's URL as the caller reached it: the browser's `Origin`, else the forwarded or plain
/// `Host` with the forwarded scheme.
pub(crate) fn server_url_from(headers: &HeaderMap) -> Option<String> {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').next().unwrap_or("").trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    if let Some(origin) = get(header::ORIGIN.as_str()).filter(|o| o != "null") {
        return Some(origin);
    }
    let host = get("x-forwarded-host").or_else(|| get(header::HOST.as_str()))?;
    let scheme = get("x-forwarded-proto").unwrap_or_else(|| "http".to_owned());
    Some(format!("{scheme}://{host}"))
}

pub(crate) fn check_server_url(url: &str) -> Result<String, ApiError> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(host)
            if !host.is_empty() && url.len() <= 2048 && !url.contains(char::is_whitespace) =>
        {
            Ok(url.to_owned())
        }
        _ => Err(ApiError::bad_request(format!(
            "server_url {url:?} must be an http or https URL"
        ))),
    }
}

/// The pairing QR code as an SVG image.
pub(crate) fn qr_svg(payload: &str) -> Result<String, ApiError> {
    let code = QrCode::new(payload.as_bytes()).map_err(|e| {
        ApiError::bad_request(format!("the pairing payload does not fit a QR code: {e}"))
    })?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(240, 240)
        .build())
}

/// `POST /api/devices/pair-codes`: issue a pairing code that works once, for ten minutes or
/// `ttl_secs`. The QR payload is `{server_url, code}`; `server_url` defaults to the address the
/// request came in on.
pub async fn create_pair_code(
    State(w): Shared,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let body: PairCodeBody = if body.iter().all(u8::is_ascii_whitespace) {
        PairCodeBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?
    };
    let ttl = body.ttl_secs.unwrap_or(DEFAULT_CODE_TTL_SECS);
    if !(1..=MAX_CODE_TTL_SECS).contains(&ttl) {
        return Err(ApiError::bad_request(format!(
            "ttl_secs must be 1 to {MAX_CODE_TTL_SECS}"
        )));
    }
    let server_url = match body.server_url.or_else(|| server_url_from(&headers)) {
        Some(url) => check_server_url(&url)?,
        None => {
            return Err(ApiError::bad_request(
                "the request has no Host header; send server_url",
            ));
        }
    };
    let issued = blocking(w, move |w| w.state().create_pair_code(ttl as i64 * 1000)).await?;
    let payload = json!({ "server_url": server_url, "code": issued.code });
    let qr = qr_svg(&payload.to_string())?;
    let out = json!({
        "code": issued.code,
        "expires": issued.expires_ms,
        "server_url": server_url,
        "payload": payload,
        "qr_svg": qr,
    });
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

/// `GET /api/devices`: every paired device, oldest first.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let devices = blocking(w, |w| w.state().list_devices()).await?;
    let devices: Vec<Value> = devices.iter().map(device_json).collect();
    Ok(Json(json!({ "devices": devices })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchBody {
    #[serde(default)]
    name: Option<String>,
    /// Keys to change; the ones left out keep their value.
    #[serde(default)]
    settings: Option<Map<String, Value>>,
}

/// `PATCH /api/devices/{id}`: rename a device or change some of its settings.
pub async fn update(
    State(w): Shared,
    Path(id): Path<String>,
    Json(body): Json<PatchBody>,
) -> Result<Json<Value>, ApiError> {
    let name = body.name.as_deref().map(check_name).transpose()?;
    let device = blocking(w, move |w| {
        let state = w.state();
        let settings = match body.settings {
            None => None,
            Some(changes) => {
                let current = state.device(&id)?;
                let mut merged = match serde_json::from_str::<Value>(&current.settings) {
                    Ok(Value::Object(m)) => m,
                    _ => Map::new(),
                };
                merged.extend(changes);
                let settings: Settings = serde_json::from_value(Value::Object(merged))
                    .map_err(|e| ApiError::bad_request(format!("settings: {e}")))?;
                settings.check()?;
                Some(
                    serde_json::to_string(&settings)
                        .map_err(|e| ApiError::bad_request(e.to_string()))?,
                )
            }
        };
        Ok::<_, ApiError>(state.update_device(&id, name.as_deref(), settings.as_deref())?)
    })
    .await?;
    Ok(Json(device_json(&device)))
}

/// `DELETE /api/devices/{id}`: revoke a device. Its token stops working at once and its push
/// endpoint is forgotten.
pub async fn revoke(State(w): Shared, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    blocking(w, move |w| w.state().revoke_device(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushBody {
    endpoint: Option<String>,
    #[serde(default)]
    p256dh: Option<String>,
    #[serde(default)]
    auth: Option<String>,
}

fn decoded_len(value: &str, what: &str) -> Result<usize, ApiError> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map(|b| b.len())
        .map_err(|_| ApiError::bad_request(format!("{what} is not base64url")))
}

fn push_target(body: PushBody) -> Result<Option<PushTarget>, ApiError> {
    let Some(endpoint) = body.endpoint else {
        return Ok(None);
    };
    check_server_url(&endpoint)
        .map_err(|_| ApiError::bad_request("endpoint must be an http or https URL"))?;
    let (Some(p256dh), Some(auth)) = (body.p256dh, body.auth) else {
        return Err(ApiError::bad_request(
            "a push endpoint needs its p256dh and auth keys",
        ));
    };
    if decoded_len(&p256dh, "p256dh")? != 65 {
        return Err(ApiError::bad_request(
            "p256dh must be an uncompressed P-256 public key (65 bytes)",
        ));
    }
    if decoded_len(&auth, "auth")? != 16 {
        return Err(ApiError::bad_request("auth must be 16 bytes"));
    }
    Ok(Some(PushTarget {
        endpoint,
        p256dh,
        auth,
    }))
}

/// `PUT /api/devices/{id}/push`: register the device's UnifiedPush endpoint and WebPush keys, or
/// clear them with `{"endpoint": null}`. Only the device's own token, or the owner, may.
pub async fn put_push(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path(id): Path<String>,
    Json(body): Json<PushBody>,
) -> Result<Json<Value>, ApiError> {
    let target = push_target(body)?;
    let device = blocking(w, move |w| {
        let state = w.state();
        own_or_admin(&state.device(&id)?, &author, scope, "set its push endpoint")?;
        Ok::<_, ApiError>(state.set_device_push(&id, target.as_ref())?)
    })
    .await?;
    Ok(Json(device_json(&device)))
}

/// The largest capture health report a device may send, in bytes of JSON.
const MAX_HEALTH_BYTES: usize = 16 * 1024;

/// The most problems a capture health report may list, and the longest one, in characters.
const MAX_HEALTH_PROBLEMS: usize = 32;
const MAX_HEALTH_PROBLEM_CHARS: usize = 300;

/// A capture health report as a device sends it: a JSON object whose `problems` is the list of
/// what stops capture, in words, empty when all is well. The rest, such as each capture kind's
/// state, is the device's own and kept as sent.
fn check_health(body: &Value) -> Result<String, ApiError> {
    let Value::Object(m) = body else {
        return Err(ApiError::bad_request(
            "capture health must be a JSON object",
        ));
    };
    let Some(problems) = m.get("problems").and_then(Value::as_array) else {
        return Err(ApiError::bad_request(
            "capture health needs `problems`, a list of strings",
        ));
    };
    if problems.len() > MAX_HEALTH_PROBLEMS {
        return Err(ApiError::bad_request(format!(
            "capture health lists at most {MAX_HEALTH_PROBLEMS} problems"
        )));
    }
    if problems.iter().any(|p| {
        p.as_str()
            .is_none_or(|p| p.trim().is_empty() || p.chars().count() > MAX_HEALTH_PROBLEM_CHARS)
    }) {
        return Err(ApiError::bad_request(format!(
            "each capture health problem must be a string of 1 to {MAX_HEALTH_PROBLEM_CHARS} characters"
        )));
    }
    let mut kept = m.clone();
    // The server says when it heard the report.
    kept.remove("reported");
    let text = serde_json::to_string(&kept).map_err(|e| ApiError::bad_request(e.to_string()))?;
    if text.len() > MAX_HEALTH_BYTES {
        return Err(ApiError::bad_request(format!(
            "capture health is larger than {MAX_HEALTH_BYTES} bytes"
        )));
    }
    Ok(text)
}

/// `PUT /api/devices/{id}/health`: the device reports its capture health with its settings sync,
/// `{ problems, ... }`, which Settings > Devices shows on its card (ticket 91). Only the device's
/// own token, or the owner, may. Answers the device.
pub async fn put_health(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let health = check_health(&body)?;
    let device = blocking(w, move |w| {
        let state = w.state();
        own_or_admin(
            &state.device(&id)?,
            &author,
            scope,
            "report its capture health",
        )?;
        Ok::<_, ApiError>(state.set_device_health(&id, &health)?)
    })
    .await?;
    Ok(Json(device_json(&device)))
}

// ---- device files ---------------------------------------------------------------------------

fn hooks_json(hooks: &[DeviceHook], key: &str) -> Value {
    hooks
        .iter()
        .map(|h| json!({ key: h.name, "script": h.script, "function": h.function }))
        .collect()
}

/// A device file as JSON: its handlers and commands, or the error that stops it parsing.
fn device_file_json(platform: &str, text: &str) -> Value {
    match DeviceFile::parse(platform, text) {
        Ok(f) => json!({
            "platform": platform,
            "handlers": hooks_json(&f.handlers, "event"),
            "commands": hooks_json(&f.commands, "name"),
        }),
        Err(e) => json!({ "platform": platform, "error": e.to_string() }),
    }
}

/// `GET /api/device-files`: every `devices/<platform>.wcl` with its handlers and commands, and
/// the closed lists of platforms, events and commands it may name.
pub async fn files(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let files = blocking(w, |w| w.device_files()).await?;
    let files: Vec<Value> = files
        .iter()
        .map(|(platform, text)| device_file_json(platform, text))
        .collect();
    Ok(Json(json!({
        "files": files,
        "platforms": DEVICE_PLATFORMS,
        "events": DEVICE_EVENTS,
        "commands": DEVICE_COMMANDS,
    })))
}

/// `GET /api/device-files/{platform}`: the file as JSON with its `wcl`, or as WCL text.
pub async fn file(
    State(w): Shared,
    Path(platform): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let p = platform.clone();
    let wcl = blocking(w, move |w| w.read_device_file(&p)).await?;
    let mut json = device_file_json(&platform, &wcl);
    json["wcl"] = json!(wcl);
    Ok(text_or_json(&headers, wcl, json))
}

/// `PUT /api/device-files/{platform}`: create or replace the file from WCL text, or JSON
/// `{ "wcl": "..." }`. Stored in canonical form. The JSON answer re-checks every script the file
/// names, now as a device script, in `scripts`: `[{ "name", "problems" }]`.
pub async fn put_file(
    State(w): Shared,
    Path(platform): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: WclBody = body(&headers, &raw, |wcl| WclBody { wcl })?;
    let p = platform.clone();
    let (wcl, scripts) = blocking(w, move |w| {
        let wcl = w.put_device_file(&p, &b.wcl)?;
        let mut scripts = Vec::new();
        if let Ok(file) = DeviceFile::parse(&p, &wcl) {
            for name in file.scripts() {
                let source = w.read_script(name)?;
                let problems = w.script_problems(name, &source)?;
                scripts.push(json!({ "name": name, "problems": problems_json(&problems) }));
            }
        }
        Ok::<_, writer::Error>((wcl, scripts))
    })
    .await?;
    let mut json = device_file_json(&platform, &wcl);
    json["wcl"] = json!(wcl);
    json["scripts"] = json!(scripts);
    Ok(text_or_json(&headers, wcl, json))
}

/// `DELETE /api/device-files/{platform}`.
pub async fn delete_file(
    State(w): Shared,
    Path(platform): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let p = platform.clone();
    blocking(w, move |w| w.delete_device_file(&p)).await?;
    Ok(Json(json!({ "platform": platform, "deleted": true })))
}

// ---- device commands ------------------------------------------------------------------------

/// Refuse a caller that is neither the device itself nor an admin.
fn own_or_admin(
    device: &Device,
    author: &Author,
    scope: Scope,
    what: &str,
) -> Result<(), ApiError> {
    let own = matches!(author, Author::Token(name) if *name == device.token_id);
    if own || scope.allows(Scope::Admin) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!("only the device itself or an admin may {what}"),
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandsQuery {
    #[serde(default)]
    status: Option<String>,
}

/// `GET /api/devices/{id}/commands?status=`: the device's commands, oldest first, only those
/// with `status` when it is given. Only the device itself or an admin may list them.
pub async fn commands(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path(id): Path<String>,
    Query(q): Query<CommandsQuery>,
) -> Result<Json<Value>, ApiError> {
    let status = q
        .status
        .as_deref()
        .map(|s| {
            CommandStatus::parse(s).ok_or_else(|| {
                ApiError::bad_request(format!(
                    "status {s:?} must be pending, delivered, done, failed or expired"
                ))
            })
        })
        .transpose()?;
    let list = blocking(w, move |w| {
        let state = w.state();
        own_or_admin(&state.device(&id)?, &author, scope, "list its commands")?;
        Ok::<_, ApiError>(state.device_commands(&id, status)?)
    })
    .await?;
    let list: Vec<Value> = list.iter().map(writer::command_json).collect();
    Ok(Json(json!({ "commands": list })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueBody {
    name: String,
    #[serde(default)]
    payload: Option<Value>,
}

/// The command a device may queue for itself with its own `write` token, from the app's Settings >
/// Devices: asking itself to import its history. Every other command needs an admin.
const SELF_QUEUED_COMMANDS: [&str; 1] = ["import_history"];

/// `POST /api/devices/{id}/commands`: queue command `name` with `payload` for the device, from
/// Settings > Devices, such as asking a phone to import its history. Refused, as a script's
/// `vault::device_command` is, unless the device's platform file declares the command and the
/// device has it enabled. An admin may queue any command; a device, through its app, only
/// `import_history`, and only for itself. Answers 201 with the command.
pub async fn queue_command(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path(id): Path<String>,
    Json(body): Json<QueueBody>,
) -> Result<Response, ApiError> {
    let command = blocking(w, move |w| {
        // The device by its id only: a name in the path is not found.
        let device = w.state().device(&id)?;
        if !scope.allows(Scope::Admin) && !SELF_QUEUED_COMMANDS.contains(&body.name.as_str()) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                format!("only an admin may queue command {:?}", body.name),
            ));
        }
        own_or_admin(&device, &author, scope, "queue its commands")?;
        let payload = body.payload.unwrap_or_else(|| json!({}));
        Ok::<_, ApiError>(w.device_command(&id, &body.name, &payload, "settings")?)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(writer::command_json(&command))).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultBody {
    status: String,
    #[serde(default)]
    result: Option<Value>,
}

/// `POST /api/devices/{id}/commands/{cid}/result`: the device reports `delivered`, or `done` or
/// `failed` with a result. A command that is finished or expired answers 409. An `upsert_contact`
/// reported done links its person to the contact in the result.
pub async fn command_result(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path((id, cid)): Path<(String, String)>,
    Json(body): Json<ResultBody>,
) -> Result<Json<Value>, ApiError> {
    let status = match CommandStatus::parse(&body.status) {
        Some(s @ (CommandStatus::Delivered | CommandStatus::Done | CommandStatus::Failed)) => s,
        _ => {
            return Err(ApiError::bad_request(format!(
                "status {:?} must be delivered, done or failed",
                body.status
            )));
        }
    };
    let result = body.result.map(|r| r.to_string());
    if result
        .as_ref()
        .is_some_and(|r| r.len() > writer::MAX_COMMAND_PAYLOAD)
    {
        return Err(ApiError::bad_request(format!(
            "the result is larger than {} bytes",
            writer::MAX_COMMAND_PAYLOAD
        )));
    }
    let command = blocking(w, move |w| {
        let state = w.state();
        own_or_admin(
            &state.device(&id)?,
            &author,
            scope,
            "report on its commands",
        )?;
        let command = state.report_device_command(&id, &cid, status, result.as_deref())?;
        // An `upsert_contact` done links its person to the contact the phone reported (D24).
        if let Err(e) = w.link_upserted_contact(&command) {
            eprintln!("person of command {cid} not linked to its contact: {e}");
        }
        Ok::<_, ApiError>(command)
    })
    .await?;
    Ok(Json(writer::command_json(&command)))
}

// ---- device event failures ------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FailureBody {
    event: String,
    /// Who the event is from or about, as the device shows it.
    #[serde(default)]
    who: Option<String>,
    /// The handler's error.
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    attempts: Option<i64>,
}

/// What failed to be saved, by device event: the bell's title.
fn failure_title(event: &str) -> &'static str {
    match event {
        "call_ended" => "A call could not be saved",
        "contact_changed" | "contact_deleted" => "A contact change could not be saved",
        "share_received" => "A share could not be saved",
        _ => "A message could not be saved",
    }
}

/// The page the notification opens: the app's Settings > Devices > This device.
const FAILURE_URL: &str = "/settings/devices#this-device";

/// The longest handler message a failure notification carries, in characters.
const MAX_FAILURE_MESSAGE: usize = 2000;

/// `POST /api/devices/{id}/event-failures`: the device reports a device event whose handler
/// failed on every pass, `{ event, who, message, attempts }`; other members, such as the
/// device's `id` and `at` for the event, are ignored. Adds one notification under the
/// bell from `device:<id>` that opens Settings > Devices. Only the device itself or an admin may.
/// Answers 201 with the notification.
pub async fn event_failure(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path(id): Path<String>,
    Json(body): Json<FailureBody>,
) -> Result<Response, ApiError> {
    if !DEVICE_EVENTS.contains(&body.event.as_str()) {
        return Err(ApiError::bad_request(format!(
            "{:?} is not a device event: {}",
            body.event,
            DEVICE_EVENTS.join(", ")
        )));
    }
    let made = blocking(w, move |w| {
        let device = w.state().device(&id)?;
        own_or_admin(&device, &author, scope, "report its event failures")?;
        let mut what = body.event.clone();
        if let Some(who) = body.who.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            what.push_str(&format!(" from {who}"));
        }
        let mut text = format!("{what} on {}", device.name);
        match body.attempts {
            Some(n) => text.push_str(&format!(" failed {n} times")),
            None => text.push_str(" failed"),
        }
        if let Some(m) = body
            .message
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let m: String = m.chars().take(MAX_FAILURE_MESSAGE).collect();
            text.push_str(&format!(": {m}"));
        }
        text.push_str(". Retry or dismiss it in the app's Settings > Devices > This device.");
        let n = w.notify(&state::NewNotification {
            title: failure_title(&body.event).to_owned(),
            body: Some(text),
            note_id: None,
            url: Some(FAILURE_URL.to_owned()),
            source: format!("device:{}", device.id),
        })?;
        Ok::<_, ApiError>(n)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": made.id,
            "created": made.created_ms,
            "title": made.title,
            "body": made.body,
            "url": made.url,
            "source": made.source,
        })),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_skip_pattern_catches_codes_and_keeps_conversation() {
        let re = regex::Regex::new(DEFAULT_SKIP_PATTERN).unwrap();
        for skipped in [
            "Your verification code is 123456",
            "G-482913 is your Google verification code.",
            "123456 is your login code",
            "Use OTP 9921 to sign in",
            "Your code: 4821",
        ] {
            assert!(re.is_match(skipped), "{skipped}");
        }
        for kept in [
            "Running late, see you at 6",
            "The door code is on the fridge",
            "Call me back when you can",
        ] {
            assert!(!re.is_match(kept), "{kept}");
        }
    }

    #[test]
    fn missing_settings_take_their_defaults() {
        let s: Settings = serde_json::from_value(json!({ "sms_send": true })).unwrap();
        assert!(s.sms_send);
        assert!(!s.contacts_write);
        assert_eq!(s.skip_pattern, DEFAULT_SKIP_PATTERN);
        s.check().unwrap();
    }

    #[test]
    fn bad_settings_are_refused() {
        let bad = [
            json!({ "skip_pattern": "(" }),
            json!({ "commands": ["Send SMS"] }),
            json!({ "exclusions": [{ "kind": "number", "value": " " }] }),
        ];
        for b in bad {
            let s: Settings = serde_json::from_value(b.clone()).unwrap();
            assert!(s.check().is_err(), "{b}");
        }
        assert!(
            serde_json::from_value::<Settings>(
                json!({ "exclusions": [{ "kind": "email", "value": "x" }] })
            )
            .is_err()
        );
        assert!(serde_json::from_value::<Settings>(json!({ "colour": "red" })).is_err());
    }

    #[test]
    fn the_server_url_comes_from_origin_then_forwarded_then_host() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(
            server_url_from(&h).as_deref(),
            Some("http://127.0.0.1:8080")
        );
        h.insert("x-forwarded-host", "vault.example".parse().unwrap());
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(
            server_url_from(&h).as_deref(),
            Some("https://vault.example")
        );
        h.insert(header::ORIGIN, "https://notes.example".parse().unwrap());
        assert_eq!(
            server_url_from(&h).as_deref(),
            Some("https://notes.example")
        );
    }

    #[test]
    fn the_qr_code_renders_as_svg() {
        let svg = qr_svg(r#"{"server_url":"https://vault.example","code":"ABCD-EFGH"}"#).unwrap();
        assert!(svg.contains("<svg"), "{svg}");
    }
}
