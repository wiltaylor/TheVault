//! Pairing with the home server (D2): `POST /api/devices/pair` with the vault password or the
//! single-use code from a pairing QR code, which returns the device's id and its token, and the
//! id of the vault the server holds, which the pairing record keeps so that joining another vault
//! starts a fresh local vault (ticket 143). The Kotlin side keeps the token in the Android
//! Keystore; the Rust core holds it only in memory.

use std::{net::{IpAddr, SocketAddr, ToSocketAddrs}, time::Duration};

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
    /// The id of the vault the server holds; `None` from a server too old to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<String>,
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
    pair_inner(url, secret, name, false)
}

/// Pair after the owner explicitly accepts the plain-HTTP warning.
pub fn pair_confirmed(url: &str, secret: &Secret, name: &str) -> anyhow::Result<Paired> {
    pair_inner(url, secret, name, true)
}

fn private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00 == 0xfc00)
            || (ip.segments()[0] & 0xffc0 == 0xfe80),
    }
}

fn http_addresses(url: &reqwest::Url, confirmed: bool, addresses: Vec<SocketAddr>) -> anyhow::Result<Vec<SocketAddr>> {
    if url.scheme() != "http" { return Ok(Vec::new()); }
    if addresses.is_empty() || addresses.iter().any(|a| !private_address(a.ip())) {
        anyhow::bail!("plain HTTP pairing requires a loopback or private LAN address; use HTTPS for a public server");
    }
    if !confirmed {
        anyhow::bail!("confirm the warning: the home feed is not authenticated over plain HTTP");
    }
    Ok(addresses)
}

fn pair_inner(url: &str, secret: &Secret, name: &str, confirmed: bool) -> anyhow::Result<Paired> {
    let server_url = server_url(url)?;
    let parsed = reqwest::Url::parse(&server_url)?;
    let mut builder = reqwest::blocking::Client::builder().timeout(TIMEOUT);
    if parsed.scheme() == "http" {
        // Resolve once, validate every candidate, and pin these exact addresses. Disable
        // proxies and redirects so neither can route credentials to an unchecked host.
        let host = parsed.host_str().ok_or_else(|| anyhow::anyhow!("missing server host"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let addresses = (host, parsed.port_or_known_default().unwrap_or(80)).to_socket_addrs()?.collect();
        let addresses = http_addresses(&parsed, confirmed, addresses)?;
        builder = builder.no_proxy().redirect(reqwest::redirect::Policy::none()).resolve_to_addrs(host, &addresses);
    }
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("give this device a name");
    }
    let mut body = json!({ "name": name, "platform": PLATFORM, "capabilities": [] });
    match secret {
        Secret::Password(p) => body["password"] = json!(p),
        Secret::Code(c) => body["code"] = json!(c.trim()),
    }
    let client = builder.build()?;
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
    Ok(Paired {
        server_url,
        device_id: field("device_id")?,
        device_name: answer["device"]["name"].as_str().unwrap_or(name).to_owned(),
        token: field("token")?,
        vault_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_requires_private_addresses_and_confirmation() {
        let url = reqwest::Url::parse("http://localhost.example.com").unwrap();
        for ip in ["127.0.0.2", "10.1.2.3", "172.16.0.1", "192.168.1.1", "::1", "fc00::1", "fdff::1", "fe80::1", "febf::1"] {
            let addresses = vec![SocketAddr::new(ip.parse().unwrap(), 80)];
            assert!(http_addresses(&url, false, addresses.clone()).is_err(), "{ip}");
            assert!(http_addresses(&url, true, addresses).is_ok(), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "169.254.0.1", "0.0.0.0", "::", "2001:db8::1", "fec0::1", "::ffff:127.0.0.1"] {
            assert!(http_addresses(&url, true, vec![SocketAddr::new(ip.parse().unwrap(), 80)]).is_err(), "{ip}");
        }
        assert!(http_addresses(&url, true, vec![]).is_err());
        let mixed = vec!["127.0.0.1:80".parse().unwrap(), "8.8.8.8:80".parse().unwrap()];
        assert!(http_addresses(&url, true, mixed).is_err());
        let https = reqwest::Url::parse("https://public.example.com").unwrap();
        assert!(http_addresses(&https, false, vec![]).is_ok());
        let resolved = ("localhost", 80).to_socket_addrs().unwrap().collect();
        assert!(http_addresses(&url, true, resolved).is_ok());
        assert!(pair("http://127.0.0.1:9", &Secret::Password("fake".into()), "test").unwrap_err().to_string().contains("confirm"));
    }

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
