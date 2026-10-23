//! The guarded fetcher the chat's `fetch_url` and `download_asset` tools share (ai D27).
//!
//! - Only `http` and `https` URLs, with a host and no user name or password.
//! - The address rule: a host is resolved once, by the client's own DNS resolver
//!   ([`GuardedResolver`]), and refused when any address it resolves to is on the local
//!   network ([`is_local`]): loopback, private (RFC 1918), link-local, unique-local
//!   (`fc00::/7`), carrier-grade NAT (`100.64.0.0/10`), multicast, unspecified, broadcast,
//!   `0.0.0.0/8`, and the IPv4-mapped IPv6 forms of these. The resolver hands the addresses it
//!   checked to the connector, which connects to them: there is no second lookup between the
//!   check and the connection. A URL naming an IP address is never resolved, so it is checked
//!   before the request is sent. The client uses no proxy, which would connect on its behalf.
//! - Redirects are not followed by the client: [`Fetcher`] follows at most [`MAX_REDIRECTS`]
//!   itself, checking each target as it checked the first URL.
//! - The whole fetch, redirects and body included, has [`TIMEOUT`].
//!
//! `ai_settings.fetch_private` (no UI) lifts the address rule: [`Fetcher::new`] with `private`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LOCATION, RANGE};
use reqwest::{Method, StatusCode, Url};

/// The most redirects one fetch follows.
pub const MAX_REDIRECTS: usize = 5;

/// How long a whole fetch may take, redirects and body included.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// What a refusal by the address rule says, to the model and in the panel.
pub const LOCAL_NETWORK: &str = "TheVault does not fetch addresses on the local network.";

/// The fetcher's `User-Agent`.
pub fn web_user_agent() -> String {
    format!(
        "TheVault/{} (+https://github.com/wiltaylor/TheVault)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Whether `ip` is on the local network, by the address rule.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_v4(v4);
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique-local fc00::/7, link-local fe80::/10 and the old site-local fec0::/10.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                // The old IPv4-compatible form ::a.b.c.d.
                || (v6.segments()[..6].iter().all(|s| *s == 0) && is_local_v4(v6_tail(v6)))
        }
    }
}

fn v6_tail(v6: Ipv6Addr) -> Ipv4Addr {
    let o = v6.octets();
    Ipv4Addr::new(o[12], o[13], o[14], o[15])
}

fn is_local_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || a == 0
        // Carrier-grade NAT, 100.64.0.0/10.
        || (a == 100 && (b & 0xc0) == 64)
}

/// The resolver error the address rule raises, found again in the error chain of the request.
#[derive(Debug)]
struct LocalAddress;

impl std::fmt::Display for LocalAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(LOCAL_NETWORK)
    }
}

impl std::error::Error for LocalAddress {}

/// The DNS resolver of a guarded client: the system's lookup, refused when any address is
/// local. The addresses it returns are the ones the connector connects to.
struct GuardedResolver {
    exempt: Arc<Vec<IpAddr>>,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let exempt = self.exempt.clone();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs
                .iter()
                .any(|a| is_local(a.ip()) && !exempt.contains(&a.ip()))
            {
                return Err(Box::new(LocalAddress) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Why a fetch got nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The URL, or a redirect's target, is not one the fetcher takes.
    BadUrl(String),
    /// The address rule refused it: [`LOCAL_NETWORK`].
    LocalNetwork,
    /// More than [`MAX_REDIRECTS`] redirects.
    TooManyRedirects,
    /// The body is larger than the fetch allows: `size` when the server said it.
    TooLarge { limit: u64, size: Option<u64> },
    /// The fetch took longer than its timeout.
    Timeout,
    /// The server answered an error status.
    Status(u16, String),
    /// The connection failed.
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::BadUrl(m) | FetchError::Failed(m) => f.write_str(m),
            FetchError::LocalNetwork => f.write_str(LOCAL_NETWORK),
            FetchError::TooManyRedirects => {
                write!(f, "the address redirected more than {MAX_REDIRECTS} times")
            }
            FetchError::TooLarge { limit, size } => match size {
                Some(s) => write!(f, "the file is {s} bytes, more than the {limit} allowed"),
                None => write!(f, "the file is larger than the {limit} bytes allowed"),
            },
            FetchError::Timeout => write!(f, "the address took longer than {TIMEOUT:?} to answer"),
            FetchError::Status(code, reason) => write!(f, "the server answered {code} {reason}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// What to do with a body longer than the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    /// Keep the first `limit` bytes and say it was cut.
    Cut,
    /// Refuse it with [`FetchError::TooLarge`], on `Content-Length` or while reading.
    Refuse,
}

/// A fetched answer.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// The URL after redirects.
    pub url: Url,
    /// The media type without parameters, lower case, when the server named one.
    pub media_type: Option<String>,
    /// The size the server gave: `Content-Length`, or a ranged answer's total.
    pub size: Option<u64>,
    /// The body read, empty for a HEAD.
    pub body: Vec<u8>,
    /// Whether the body was cut at the limit.
    pub cut: bool,
}

/// The guarded HTTP client. Build one and keep it: it holds the connection pool.
#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    private: bool,
    exempt: Arc<Vec<IpAddr>>,
    timeout: Duration,
}

impl Fetcher {
    /// A fetcher, with the address rule unless `private` (`ai_settings.fetch_private`).
    pub fn new(private: bool) -> Fetcher {
        Fetcher::build(private, Vec::new(), TIMEOUT)
    }

    /// A guarded fetcher that lets the addresses in `exempt` through, for tests that serve a
    /// public stand-in from a loopback address and redirect from it to another.
    #[doc(hidden)]
    pub fn exempting(exempt: Vec<IpAddr>, timeout: Duration) -> Fetcher {
        Fetcher::build(false, exempt, timeout)
    }

    fn build(private: bool, exempt: Vec<IpAddr>, timeout: Duration) -> Fetcher {
        let exempt = Arc::new(exempt);
        let mut b = reqwest::Client::builder()
            .user_agent(web_user_agent())
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(timeout);
        if !private {
            b = b.dns_resolver(Arc::new(GuardedResolver {
                exempt: exempt.clone(),
            }));
        }
        Fetcher {
            client: b.build().unwrap_or_default(),
            private,
            exempt,
            timeout,
        }
    }

    /// Whether the address rule is lifted.
    pub fn private(&self) -> bool {
        self.private
    }

    /// `url` checked as the fetcher takes it: `http` or `https`, a host, no user name or
    /// password, and, unless the rule is lifted, no local IP address as its host.
    pub fn check(&self, url: &str) -> Result<Url, FetchError> {
        let parsed = Url::parse(url.trim())
            .map_err(|_| FetchError::BadUrl(format!("{url:?} is not a URL")))?;
        self.check_url(&parsed)?;
        Ok(parsed)
    }

    fn check_url(&self, url: &Url) -> Result<(), FetchError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchError::BadUrl(format!(
                "only http and https addresses are fetched, not {}:",
                url.scheme()
            )));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::BadUrl(
                "the address must not hold a user name or password".to_owned(),
            ));
        }
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| FetchError::BadUrl("the address names no host".to_owned()))?;
        if self.private {
            return Ok(());
        }
        // An address the URL names is never resolved, so the resolver never sees it.
        let literal = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = literal.parse::<IpAddr>()
            && is_local(ip)
            && !self.exempt.contains(&ip)
        {
            return Err(FetchError::LocalNetwork);
        }
        Ok(())
    }

    /// GET `url`, following redirects, reading at most `limit` bytes of the body.
    pub async fn get(&self, url: &str, limit: u64, over: Over) -> Result<Fetched, FetchError> {
        let url = self.check(url)?;
        self.timed(self.send(url, Method::GET, None, limit, over))
            .await
    }

    /// The final URL, media type and size of `url` without its body: a HEAD, or when the server
    /// refuses that, a GET of its first byte.
    pub async fn probe(&self, url: &str) -> Result<Fetched, FetchError> {
        let url = self.check(url)?;
        let head = self
            .timed(self.send(url.clone(), Method::HEAD, None, 0, Over::Cut))
            .await;
        match head {
            Ok(f) if f.size.is_some() => Ok(f),
            Err(FetchError::Status(..) | FetchError::Failed(_)) | Ok(_) => {
                let mut f = self
                    .timed(self.send(url, Method::GET, Some("bytes=0-0"), 1, Over::Cut))
                    .await?;
                f.body.clear();
                f.cut = false;
                Ok(f)
            }
            Err(e) => Err(e),
        }
    }

    async fn timed(
        &self,
        fut: impl Future<Output = Result<Fetched, FetchError>>,
    ) -> Result<Fetched, FetchError> {
        tokio::time::timeout(self.timeout, fut)
            .await
            .unwrap_or(Err(FetchError::Timeout))
    }

    async fn send(
        &self,
        mut url: Url,
        method: Method,
        range: Option<&str>,
        limit: u64,
        over: Over,
    ) -> Result<Fetched, FetchError> {
        let mut redirects = 0;
        let mut method = method;
        loop {
            self.check_url(&url)?;
            let mut req = self.client.request(method.clone(), url.clone()).header(
                ACCEPT,
                "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8",
            );
            if let Some(r) = range {
                req = req.header(RANGE, r);
            }
            let mut res = req.send().await.map_err(send_error)?;
            let status = res.status();
            if status.is_redirection() && status != StatusCode::NOT_MODIFIED {
                let Some(to) = res
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
                else {
                    return Err(FetchError::Status(
                        status.as_u16(),
                        "with a redirect that names no address".to_owned(),
                    ));
                };
                redirects += 1;
                if redirects > MAX_REDIRECTS {
                    return Err(FetchError::TooManyRedirects);
                }
                url = url.join(&to).map_err(|_| {
                    FetchError::BadUrl(format!("the address redirected to {to:?}, not a URL"))
                })?;
                if status == StatusCode::SEE_OTHER && method != Method::HEAD {
                    method = Method::GET;
                }
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status(
                    status.as_u16(),
                    status.canonical_reason().unwrap_or_default().to_owned(),
                ));
            }
            let headers = res.headers();
            let media_type = headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|t| t.split(';').next())
                .map(|t| t.trim().to_ascii_lowercase())
                .filter(|t| !t.is_empty());
            let length = headers
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            // A ranged answer's total size, from `Content-Range: bytes 0-0/<total>`.
            let total = (status == StatusCode::PARTIAL_CONTENT)
                .then(|| headers.get(CONTENT_RANGE))
                .flatten()
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit_once('/'))
                .and_then(|(_, t)| t.trim().parse::<u64>().ok());
            let size = if status == StatusCode::PARTIAL_CONTENT {
                total
            } else {
                length
            };
            let mut fetched = Fetched {
                url,
                media_type,
                size,
                body: Vec::new(),
                cut: false,
            };
            if method == Method::HEAD {
                return Ok(fetched);
            }
            if over == Over::Refuse
                && let Some(s) = size.filter(|s| *s > limit)
            {
                return Err(FetchError::TooLarge {
                    limit,
                    size: Some(s),
                });
            }
            while let Some(chunk) = res.chunk().await.map_err(send_error)? {
                let room = limit.saturating_sub(fetched.body.len() as u64) as usize;
                if chunk.len() > room {
                    if over == Over::Refuse {
                        return Err(FetchError::TooLarge { limit, size: None });
                    }
                    fetched.body.extend_from_slice(&chunk[..room]);
                    fetched.cut = true;
                    break;
                }
                fetched.body.extend_from_slice(&chunk);
            }
            return Ok(fetched);
        }
    }
}

/// A request error, as the address rule's refusal when the resolver raised it.
fn send_error(e: reqwest::Error) -> FetchError {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&e);
    while let Some(s) = source {
        if s.downcast_ref::<LocalAddress>().is_some() {
            return FetchError::LocalNetwork;
        }
        source = s.source();
    }
    if e.is_timeout() {
        return FetchError::Timeout;
    }
    // The URL is the model's own; the message names it without a query.
    let e = e.without_url();
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        message.push_str(": ");
        message.push_str(&s.to_string());
        source = s.source();
    }
    FetchError::Failed(format!("the address could not be fetched: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_rule_knows_every_local_range() {
        for local in [
            "127.0.0.1",
            "127.255.0.9",
            "10.0.0.1",
            "172.16.4.4",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "224.0.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
        ] {
            assert!(is_local(ip(local)), "{local}");
        }
        for public in [
            "1.1.1.1",
            "8.8.8.8",
            "100.128.0.1",
            "172.32.0.1",
            "2606:4700::1111",
            "::ffff:1.1.1.1",
        ] {
            assert!(!is_local(ip(public)), "{public}");
        }
    }

    #[test]
    fn check_takes_http_and_https_only_and_refuses_user_info() {
        let f = Fetcher::new(false);
        assert!(f.check("https://example.com/a").is_ok());
        assert!(matches!(
            f.check("ftp://example.com/"),
            Err(FetchError::BadUrl(_))
        ));
        assert!(matches!(
            f.check("file:///etc/passwd"),
            Err(FetchError::BadUrl(_))
        ));
        assert!(matches!(
            f.check("https://me:pw@example.com/"),
            Err(FetchError::BadUrl(_))
        ));
        assert!(matches!(f.check("not a url"), Err(FetchError::BadUrl(_))));
    }

    #[test]
    fn literal_local_addresses_are_refused_unless_private() {
        let f = Fetcher::new(false);
        for url in [
            "http://127.0.0.1/",
            "http://10.0.0.1:8080/x",
            "http://169.254.1.1/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:7f00:1]/",
            "http://2130706433/",
            "http://0x7f.1/",
        ] {
            assert_eq!(f.check(url).unwrap_err(), FetchError::LocalNetwork, "{url}");
        }
        let p = Fetcher::new(true);
        assert!(p.check("http://127.0.0.1/").is_ok());
    }
}
