//! Pairing with the home server (D2): `POST /api/devices/pair` with the vault password or the
//! single-use code from a pairing QR code, which returns the device's id and its token. The
//! Kotlin side keeps the token in the Android Keystore; the Rust core holds it only in memory.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How long pairing waits for the server.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The platform the app pairs as.
pub const PLATFORM: &str = "android";

/// The proof pairing sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Secret {
    Password(String),
    /// A code from `POST /api/devices/pair-codes`, as the QR code carries it.
    Code(String),
}

/// What a successful pairing returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paired {
    /// The server's base URL, as the device will reach it.
    pub server_url: String,
    pub device_id: String,
    pub device_name: String,
    pub token: String,
}

/// `url` as a server base URL: trimmed, `http` or `https`, with no trailing `/`.
pub fn server_url(url: &str) -> anyhow::Result<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| anyhow::anyhow!("the server address must start with https:// or http://"))?;
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        anyhow::bail!("{url:?} is not a server address");
    }
    Ok(url.to_owned())
}

/// Pair with the server at `url` as a device called `name`.
pub fn pair(url: &str, secret: &Secret, name: &str) -> anyhow::Result<Paired> {
    let server_url = server_url(url)?;
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("give this device a name");
    }
    let mut body = json!({ "name": name, "platform": PLATFORM, "capabilities": [] });
    match secret {
        Secret::Password(p) => body["password"] = json!(p),
        Secret::Code(c) => body["code"] = json!(c.trim()),
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()?;
    let res = client
        .post(format!("{server_url}/api/devices/pair"))
        .json(&body)
        .send()
        .map_err(|e| anyhow::anyhow!("cannot reach {server_url}: {e}"))?;
    let status = res.status();
    let answer: Value = res.json().unwrap_or(Value::Null);
    if !status.is_success() {
        let message = answer["message"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("the server answered {status}"));
        anyhow::bail!("{message}");
    }
    let field = |k: &str| {
        answer[k]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("the server's answer has no {k}"))
    };
    Ok(Paired {
        server_url,
        device_id: field("device_id")?,
        device_name: answer["device"]["name"].as_str().unwrap_or(name).to_owned(),
        token: field("token")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_are_http_and_lose_their_trailing_slash() {
        assert_eq!(
            server_url(" https://v.example.net/ ").unwrap(),
            "https://v.example.net"
        );
        assert_eq!(
            server_url("http://10.0.2.2:8080").unwrap(),
            "http://10.0.2.2:8080"
        );
        assert!(server_url("v.example.net").is_err());
        assert!(server_url("https://").is_err());
        assert!(server_url("ftp://x").is_err());
    }
}
