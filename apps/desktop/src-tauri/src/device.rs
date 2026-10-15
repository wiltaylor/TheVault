//! Who this device is: pairing with the home server, the device file, and the device token in
//! the Secret Service keyring (D2).
//!
//! Pairing posts the vault password or a pairing code to `POST /api/devices/pair` and gets a
//! `write` token bound to a new device row. The token goes into the keyring under the device id;
//! the server URL, the device id and its name go into `device.json` beside the local vault, so the
//! app knows it is paired without unlocking the keyring.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The keyring service the device token is stored under.
const KEYRING_SERVICE: &str = "thevault";

/// How long pairing waits for the server.
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);

/// A paired device, as `device.json` records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub server_url: String,
    pub device_id: String,
    pub device_name: String,
}

impl Device {
    /// The sync engine's view of this device, with its token.
    pub fn remote(&self, token: String) -> sync::Remote {
        sync::Remote {
            url: self.server_url.clone(),
            token,
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
        }
    }
}

fn device_file(dir: &Path) -> PathBuf {
    dir.join("device.json")
}

/// The device `dir` was paired as, or `None` before pairing.
pub fn load(dir: &Path) -> Result<Option<Device>, String> {
    let path = device_file(dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{} is not a device file: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

/// Records `device` in `dir`, replacing the file in one rename.
pub fn save(dir: &Path, device: &Device) -> Result<(), String> {
    let path = device_file(dir);
    let tmp = dir.join("device.json.tmp");
    let text = serde_json::to_vec_pretty(device).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Where device tokens are kept.
pub trait TokenStore: Send + Sync {
    fn get(&self, device_id: &str) -> Result<String, String>;
    fn set(&self, device_id: &str, token: &str) -> Result<(), String>;
}

/// The Secret Service keyring of the desktop session (D2). Its calls block: make them from a
/// thread of their own, not inside an async runtime.
pub struct Keyring;

impl Keyring {
    fn entry(device_id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, device_id).map_err(|e| keyring_error(&e))
    }
}

fn keyring_error(e: &keyring::Error) -> String {
    format!("the Secret Service keyring: {e}")
}

impl TokenStore for Keyring {
    fn get(&self, device_id: &str) -> Result<String, String> {
        Keyring::entry(device_id)?
            .get_password()
            .map_err(|e| keyring_error(&e))
    }

    fn set(&self, device_id: &str, token: &str) -> Result<(), String> {
        Keyring::entry(device_id)?
            .set_password(token)
            .map_err(|e| keyring_error(&e))
    }
}

/// What the setup page sends to pair: the server and either the password or a pairing code.
#[derive(Debug, Clone, Deserialize)]
pub struct PairRequest {
    pub server_url: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// A server URL as the user typed it, without a trailing slash, or why it is not one.
pub fn normalise_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or("the server URL must start with http:// or https://")?;
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return Err("the server URL has no host".to_owned());
    }
    Ok(url.to_owned())
}

/// The name a device takes when the user gives none: the machine's host name.
pub fn default_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Linux desktop".to_owned())
}

/// Pairs with the server in `req`, stores the token in `tokens` and records the device in `dir`.
/// It blocks while it waits for the server.
pub fn pair(dir: &Path, tokens: &dyn TokenStore, req: &PairRequest) -> Result<Device, String> {
    let server_url = normalise_url(&req.server_url)?;
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map_or_else(default_name, str::to_owned);
    let mut body = json!({ "name": name, "platform": "linux", "capabilities": [] });
    match (
        req.password.as_deref().filter(|p| !p.is_empty()),
        req.code.as_deref().map(str::trim).filter(|c| !c.is_empty()),
    ) {
        (Some(password), None) => body["password"] = json!(password),
        (None, Some(code)) => body["code"] = json!(code),
        _ => return Err("enter either the password or a pairing code".to_owned()),
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(PAIR_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .post(format!("{server_url}/api/devices/pair"))
        .json(&body)
        .send()
        .map_err(|e| format!("cannot reach {server_url}: {e}"))?;
    let status = res.status();
    let answer: Value = res.json().unwrap_or(Value::Null);
    if !status.is_success() {
        let message = answer
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| format!("the server answered {status}"), str::to_owned);
        return Err(message);
    }
    let field = |k: &str| {
        answer
            .get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("the server's pairing answer has no `{k}`"))
    };
    let device = Device {
        server_url,
        device_id: field("device_id")?,
        device_name: name,
    };
    tokens.set(&device.device_id, &field("token")?)?;
    save(dir, &device)?;
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_lose_their_trailing_slash_and_need_a_scheme() {
        assert_eq!(
            normalise_url(" https://vault.example.net/ ").unwrap(),
            "https://vault.example.net"
        );
        assert_eq!(
            normalise_url("http://127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert!(normalise_url("vault.example.net").is_err());
        assert!(normalise_url("https://").is_err());
    }

    #[test]
    fn the_device_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        let device = Device {
            server_url: "https://vault.example.net".into(),
            device_id: "01J00000000000000000000000".into(),
            device_name: "laptop".into(),
        };
        save(dir.path(), &device).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(device));
    }
}
