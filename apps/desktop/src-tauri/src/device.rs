//! Who this device is: pairing with the home server, the device file, and the device token in
//! the Secret Service keyring (D2).
//!
//! Pairing posts the vault password or a pairing code to `POST /api/devices/pair` and gets a
//! `write` token bound to a new device row. The token goes into the keyring under the device id;
//! the server URL, the device id and its name go into `device.json` beside the local vault, so the
//! app knows it is paired without unlocking the keyring. So does the id of the vault the server
//! holds, which the pairing answers: a pairing with a server that holds another vault than the
//! local vault is a copy of starts a fresh local vault ([`crate::sync_loop`], ticket 145).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The keyring service the device token is stored under.
const KEYRING_SERVICE: &str = "thevault";

pub const HTTP_WARNING: &str = "The home feed is not authenticated over plain HTTP. Someone on the network can read or change it. Use HTTPS when possible.";

/// A startup notice, not an admission check. Existing public HTTP links still sync.
pub struct HttpWarning(std::sync::atomic::AtomicBool);

impl HttpWarning {
    pub fn new(device: Option<&Device>) -> Self {
        Self(std::sync::atomic::AtomicBool::new(device.is_some_and(
            |d| {
                d.server_url
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("http://")
            },
        )))
    }

    pub fn take(&self) -> Option<&'static str> {
        self.0
            .swap(false, std::sync::atomic::Ordering::Relaxed)
            .then_some(HTTP_WARNING)
    }
}

// Compile the same transport and its tests for both app workspaces.
#[path = "../../../../plugins/tauri-plugin-vault-device/src/pair_transport.rs"]
mod transport;

/// A paired device, as `device.json` records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub server_url: String,
    pub device_id: String,
    pub device_name: String,
    /// The id of the vault the home server holds, as pairing answered it; `None` for a pairing
    /// made before ticket 145, or with a server too old to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<String>,
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
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("token deletion is unavailable".into())
    }
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
    fn delete(&self, device_id: &str) -> Result<(), String> {
        match Keyring::entry(device_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(&e)),
        }
    }
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
    #[serde(default)]
    pub confirm_http: bool,
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
static PAIRING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Native owner action only. Leave the link intact on credential deletion failure.
pub fn unpair(dir: &Path, tokens: &dyn TokenStore) -> Result<(), String> {
    let _guard = PAIRING.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(device) = load(dir)? {
        tokens.delete(&device.device_id)?;
        std::fs::remove_file(device_file(dir)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn pair(dir: &Path, tokens: &dyn TokenStore, req: &PairRequest) -> Result<Device, String> {
    let _guard = PAIRING.lock().unwrap_or_else(|p| p.into_inner());
    if load(dir)?.is_some() {
        return Err("already paired; use Unpair this device in the native tray menu first".into());
    }
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
    let client = transport::client(&server_url, req.confirm_http).map_err(|e| e.to_string())?;
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
    let vault_id = match answer["vault_id"].as_str().filter(|v| !v.is_empty()) {
        Some(v) => Some(v.to_owned()),
        // A server from before pairing named the vault names it in its sync info, if at all.
        None => client
            .get(format!("{server_url}/api/sync/info"))
            .send()
            .ok()
            .and_then(|r| r.json::<Value>().ok())
            .and_then(|info| info["vault_id"].as_str().map(str::to_owned))
            .filter(|v| !v.is_empty()),
    };
    let device = Device {
        server_url,
        device_id: field("device_id")?,
        device_name: name,
        vault_id,
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

    struct NoTokens;

    #[test]
    fn confirmed_private_pairing_stores_the_link_and_token() {
        use std::io::{Read, Write};
        struct Tokens(std::sync::Mutex<String>);
        impl TokenStore for Tokens {
            fn delete(&self, _: &str) -> Result<(), String> {
                self.0.lock().unwrap().clear();
                Ok(())
            }
            fn get(&self, _: &str) -> Result<String, String> {
                Ok(self.0.lock().unwrap().clone())
            }
            fn set(&self, _: &str, token: &str) -> Result<(), String> {
                *self.0.lock().unwrap() = token.into();
                Ok(())
            }
        }
        for code in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://localhost:{}", listener.local_addr().unwrap().port());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let len: usize = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + len {
                            let body: Value = serde_json::from_slice(&bytes[end + 4..]).unwrap();
                            assert_eq!(body["platform"], "linux");
                            assert_eq!(body[if code { "code" } else { "password" }], "fake");
                            break;
                        }
                    }
                }
                let body = r#"{"device_id":"d","token":"t","vault_id":"v"}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            });
            let dir = tempfile::tempdir().unwrap();
            let tokens = Tokens(std::sync::Mutex::new(String::new()));
            let req = PairRequest {
                server_url: url,
                confirm_http: true,
                password: (!code).then(|| "fake".into()),
                code: code.then(|| "fake".into()),
                name: Some("test".into()),
            };
            let old = Device {
                server_url: "https://old.invalid".into(),
                device_id: "old".into(),
                device_name: "old".into(),
                vault_id: None,
            };
            save(dir.path(), &old).unwrap();
            tokens.set("old", "old-token").unwrap();
            assert!(
                pair(dir.path(), &NoTokens, &req)
                    .unwrap_err()
                    .contains("already paired")
            );
            assert_eq!(load(dir.path()).unwrap(), Some(old));
            assert_eq!(tokens.get("old").unwrap(), "old-token");
            unpair(dir.path(), &tokens).unwrap();
            assert!(load(dir.path()).unwrap().is_none());
            assert_eq!(tokens.get("old").unwrap(), "");
            let d = pair(dir.path(), &tokens, &req).unwrap();
            assert_eq!(load(dir.path()).unwrap(), Some(d));
            assert_eq!(tokens.get("d").unwrap(), "t");
            server.join().unwrap();
        }
    }
    impl TokenStore for NoTokens {
        fn get(&self, _: &str) -> Result<String, String> {
            panic!("no token access")
        }
        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            panic!("no token storage")
        }
    }

    #[test]
    fn pairing_refuses_public_http_and_unconfirmed_private_http() {
        let dir = tempfile::tempdir().unwrap();
        for code in [false, true] {
            for (url, confirmed, message) in [
                ("http://8.8.8.8", true, "private LAN"),
                ("http://8.8.8.8", false, "private LAN"),
                ("http://127.0.0.1:9", false, "confirm"),
            ] {
                let req = PairRequest {
                    server_url: url.into(),
                    confirm_http: confirmed,
                    password: (!code).then(|| "fake".into()),
                    code: code.then(|| "fake".into()),
                    name: Some("test".into()),
                };
                assert!(
                    pair(dir.path(), &NoTokens, &req)
                        .unwrap_err()
                        .contains(message)
                );
                assert!(load(dir.path()).unwrap().is_none());
            }
        }
        let req: PairRequest =
            serde_json::from_value(json!({"server_url":"http://127.0.0.1","password":"fake"}))
                .unwrap();
        assert!(!req.confirm_http);
    }

    #[test]
    fn existing_http_warning_once_per_start_without_changing_link() {
        for url in ["http://8.8.8.8", "http://127.0.0.1"] {
            let d = Device {
                server_url: url.into(),
                device_id: "d".into(),
                device_name: "test".into(),
                vault_id: None,
            };
            let warning = HttpWarning::new(Some(&d));
            assert_eq!(warning.take(), Some(HTTP_WARNING));
            assert_eq!(warning.take(), None);
            assert_eq!(d.remote("token".into()).url, url);
            assert_eq!(HttpWarning::new(Some(&d)).take(), Some(HTTP_WARNING));
        }
        assert_eq!(HttpWarning::new(None).take(), None);
        let d = Device {
            server_url: "https://example.com".into(),
            device_id: "d".into(),
            device_name: "test".into(),
            vault_id: None,
        };
        assert_eq!(HttpWarning::new(Some(&d)).take(), None);
    }

    #[test]
    fn the_device_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        let device = Device {
            server_url: "https://vault.example.net".into(),
            device_id: "01J00000000000000000000000".into(),
            device_name: "laptop".into(),
            vault_id: Some("0123456789abcdef0123456789abcdef".into()),
        };
        save(dir.path(), &device).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(device));
    }

    #[test]
    fn a_device_file_from_before_vault_ids_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("device.json"),
            r#"{"server_url":"https://vault.example.net","device_id":"d1","device_name":"laptop"}"#,
        )
        .unwrap();
        let device = load(dir.path()).unwrap().unwrap();
        assert_eq!(device.vault_id, None);
        assert_eq!(device.device_id, "d1");
    }
}
