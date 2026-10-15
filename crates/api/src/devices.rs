//! Paired devices: the desktop and Android apps (D2, D3).
//!
//! A device pairs once at `POST /api/devices/pair` with the vault password or a single-use code
//! from `POST /api/devices/pair-codes`, and gets a `write` token bound to it. Settings > Devices
//! lists, renames and revokes devices, and holds each device's settings: the commands it accepts,
//! its contacts-write and SMS-send switches, its exclusion list and its skip pattern (D22, D23,
//! D25). Revoking deletes the device's token and its push endpoint.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use qrcode::QrCode;
use qrcode::render::svg;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use state::{Author, Device, NewDevice, PushTarget, Scope};
use writer::Writer;

use crate::auth::GrantedScope;
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
    })
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
fn server_url_from(headers: &HeaderMap) -> Option<String> {
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

fn check_server_url(url: &str) -> Result<String, ApiError> {
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
fn qr_svg(payload: &str) -> Result<String, ApiError> {
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
        let device = state.device(&id)?;
        let own = matches!(&author, Author::Token(name) if *name == device.token_id);
        if !own && !scope.allows(Scope::Admin) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "only the device itself or an admin may set its push endpoint",
            ));
        }
        Ok(state.set_device_push(&id, target.as_ref())?)
    })
    .await?;
    Ok(Json(device_json(&device)))
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
