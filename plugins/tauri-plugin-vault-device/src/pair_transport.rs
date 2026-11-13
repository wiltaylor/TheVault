//! Shared Android and desktop pairing admission and pinned HTTP transport.
use std::{net::{IpAddr, SocketAddr, ToSocketAddrs}, time::Duration};
const TIMEOUT: Duration = Duration::from_secs(30);
fn private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || (ip.segments()[0] & 0xfe00 == 0xfc00)
                || (ip.segments()[0] & 0xffc0 == 0xfe80)
        }
    }
}

fn http_addresses(
    url: &reqwest::Url,
    confirmed: bool,
    addresses: Vec<SocketAddr>,
) -> anyhow::Result<Vec<SocketAddr>> {
    if url.scheme() != "http" {
        return Ok(Vec::new());
    }
    if addresses.is_empty() || addresses.iter().any(|a| !private_address(a.ip())) {
        anyhow::bail!(
            "plain HTTP pairing requires a loopback or private LAN address; use HTTPS for a public server"
        );
    }
    if !confirmed {
        anyhow::bail!("confirm the warning: the home feed is not authenticated over plain HTTP");
    }
    Ok(addresses)
}

/// Build the only transport used for new pairings on either platform.
pub fn client(server_url: &str, confirmed: bool) -> anyhow::Result<reqwest::blocking::Client> {
    let parsed = reqwest::Url::parse(server_url)?;
    let mut builder = reqwest::blocking::Client::builder().timeout(TIMEOUT);
    if parsed.scheme() == "http" {
        // Resolve once, validate every candidate, and pin these exact addresses. Disable
        // proxies and redirects so neither can route credentials to an unchecked host.
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("missing server host"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let addresses = (host, parsed.port_or_known_default().unwrap_or(80))
            .to_socket_addrs()?
            .collect();
        let addresses = http_addresses(&parsed, confirmed, addresses)?;
        builder = builder
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses);
    }
    Ok(builder.build()?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn http_requires_private_addresses_and_confirmation() {
        let url = reqwest::Url::parse("http://localhost.example.com").unwrap();
        for ip in [
            "127.0.0.2",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "::1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "febf::1",
        ] {
            let addresses = vec![SocketAddr::new(ip.parse().unwrap(), 80)];
            assert!(
                http_addresses(&url, false, addresses.clone()).is_err(),
                "{ip}"
            );
            assert!(http_addresses(&url, true, addresses).is_ok(), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "172.32.0.1",
            "169.254.0.1",
            "0.0.0.0",
            "::",
            "2001:db8::1",
            "fec0::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                http_addresses(&url, true, vec![SocketAddr::new(ip.parse().unwrap(), 80)]).is_err(),
                "{ip}"
            );
        }
        assert!(http_addresses(&url, true, vec![]).is_err());
        let mixed = vec![
            "127.0.0.1:80".parse().unwrap(),
            "8.8.8.8:80".parse().unwrap(),
        ];
        assert!(http_addresses(&url, true, mixed).is_err());
        let https = reqwest::Url::parse("https://public.example.com").unwrap();
        assert!(http_addresses(&https, false, vec![]).is_ok());
        let resolved = ("localhost", 80).to_socket_addrs().unwrap().collect();
        assert!(http_addresses(&url, true, resolved).is_ok());
    }
}

