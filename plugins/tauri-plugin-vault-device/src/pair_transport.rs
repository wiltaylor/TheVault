//! Shared Android and desktop pairing admission and pinned HTTP transport.
use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    time::Duration,
};
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
    client_resolving(&parsed, confirmed, |host, port| {
        Ok((host, port).to_socket_addrs()?.collect())
    })
}

fn client_resolving(
    parsed: &reqwest::Url,
    confirmed: bool,
    resolve: impl FnOnce(&str, u16) -> anyhow::Result<Vec<SocketAddr>>,
) -> anyhow::Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder().timeout(TIMEOUT);
    if parsed.scheme() == "http" {
        // Resolve once, validate every candidate, and pin these exact addresses. Disable
        // proxies and redirects so neither can route credentials to an unchecked host.
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("missing server host"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let addresses = resolve(host, parsed.port_or_known_default().unwrap_or(80))?;
        let addresses = http_addresses(parsed, confirmed, addresses)?;
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
    fn http_pairing_ignores_dead_proxy() {
        const CHILD: &str = "VAULT_PAIR_PROXY_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            checked_dns_is_pinned_and_redirects_are_not_followed();
            return;
        }
        // Isolate environment changes from parallel tests. The synthetic hostname avoids
        // reqwest's automatic loopback proxy bypass even though its pinned address is local.
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["http_pairing_ignores_dead_proxy", "--nocapture"])
            .env(CHILD, "1")
            .env("HTTP_PROXY", &proxy)
            .env("http_proxy", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("all_proxy", &proxy)
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[test]
    fn checked_dns_is_pinned_and_redirects_are_not_followed() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..n]).contains("host: not-in-dns.invalid"));
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://8.8.8.8/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let url =
            reqwest::Url::parse(&format!("http://not-in-dns.invalid:{}", address.port())).unwrap();
        let client = client_resolving(&url, true, |host, _| {
            assert_eq!(host, "not-in-dns.invalid");
            Ok(vec![address])
        })
        .unwrap();
        assert_eq!(client.get(url).send().unwrap().status(), 302);
        server.join().unwrap();
    }
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
