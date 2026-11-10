//! Shared, pre-handler limits for public credential checks. No credentials are retained.
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    body::{Body, to_bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, Method, StatusCode, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};
use writer::Writer;

const WINDOW: Duration = Duration::from_secs(60);
const NOTICE_COOLDOWN: Duration = Duration::from_secs(900);
const HISTORY: Duration = Duration::from_secs(24 * 60 * 60);
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_LIMIT: usize = 2 * 1024 * 1024;
const MAX_ADDRESSES: usize = 65536;
type Key = Option<IpAddr>;
type Bucket = Arc<Mutex<Address>>;

#[derive(Default)]
struct Address {
    failures: VecDeque<Instant>,
    last: Option<Instant>,
    until: Option<Instant>,
    strikes: u32,
    in_flight: usize,
}

fn prune(failures: &mut VecDeque<Instant>, now: Instant) {
    while failures.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
        failures.pop_front();
    }
}

impl Address {
    fn reset(&mut self) {
        let in_flight = self.in_flight;
        *self = Self { in_flight, ..Self::default() };
    }

    fn retry(&mut self, now: Instant, global_full: bool) -> Option<u64> {
        if self.last.is_some_and(|t| now.duration_since(t) >= HISTORY) {
            self.reset();
        }
        if let Some(until) = self.until.take().filter(|t| *t > now) {
            self.until = Some(until);
            return Some(until.duration_since(now).as_secs().saturating_add(1));
        }
        prune(&mut self.failures, now);
        if self.failures.len() >= 5 || (global_full && !self.failures.is_empty()) {
            let delay = (60u64 << self.strikes.min(4)).min(900);
            self.strikes = self.strikes.saturating_add(1);
            self.until = Some(now + Duration::from_secs(delay));
            self.failures.clear();
            return Some(delay);
        }
        // Reserve the remaining budget, but do not count pending checks as failures.
        if self.in_flight + self.failures.len() >= 5 {
            return Some(1);
        }
        None
    }
}

#[derive(Default)]
struct Counters {
    // History is bounded. Active requests own temporary buckets separately, including a new
    // clean client when history is full. Slow bodies never reach this map.
    addresses: HashMap<Key, Bucket>,
    pending: HashMap<Key, Bucket>,
    failures: VecDeque<Instant>,
    notified: Option<Instant>,
    pruned: Option<Instant>,
}

impl Counters {
    fn bucket(&self, key: Key) -> Option<Bucket> {
        self.pending.get(&key).or_else(|| self.addresses.get(&key)).cloned()
    }

    fn precheck(&mut self, key: Key, now: Instant) -> Option<u64> {
        prune(&mut self.failures, now);
        self.bucket(key).and_then(|b| b.lock().expect("attempt bucket").retry(now, self.failures.len() >= 50))
    }

    fn begin(&mut self, key: Key, now: Instant) -> Result<Bucket, u64> {
        if let Some(delay) = self.precheck(key, now) {
            return Err(delay);
        }
        if !self.addresses.contains_key(&key)
            && self.pruned.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            self.pruned = Some(now);
            self.addresses.retain(|_, b| {
                let b = b.lock().expect("attempt bucket");
                b.in_flight != 0 || b.last.is_some_and(|t| now.duration_since(t) < HISTORY)
            });
        }
        let bucket = self.bucket(key).unwrap_or_default();
        bucket.lock().expect("attempt bucket").in_flight += 1;
        self.pending.insert(key, bucket.clone());
        Ok(bucket)
    }

    fn release(&mut self, key: Key, bucket: &Bucket) {
        let mut b = bucket.lock().expect("attempt bucket");
        b.in_flight -= 1;
        if b.in_flight == 0 {
            self.pending.remove(&key);
        }
    }

    fn record(&mut self, key: Key, bucket: &Bucket, status: StatusCode, now: Instant) -> bool {
        // Every lock follows counters -> bucket, and is released before any await.
        self.release(key, bucket);
        let failed = matches!(status, StatusCode::UNAUTHORIZED | StatusCode::GONE);
        {
            let mut b = bucket.lock().expect("attempt bucket");
            if status.is_success() {
                b.reset();
                self.addresses.remove(&key);
            } else if failed {
                b.last = Some(now);
                prune(&mut b.failures, now);
                b.failures.push_back(now);
            }
        }
        if !failed {
            return false;
        }
        if !self.addresses.contains_key(&key) {
            if self.addresses.len() >= MAX_ADDRESSES {
                // Clean history first, then oldest failure. Never evict in-flight checks.
                let oldest = self.addresses.iter().filter_map(|(k, b)| {
                    let b = b.lock().expect("attempt bucket");
                    (b.in_flight == 0).then_some((*k, b.last))
                }).min_by_key(|(_, last)| *last).map(|(k, _)| k);
                if let Some(oldest) = oldest {
                    self.addresses.remove(&oldest);
                }
            }
            if self.addresses.len() < MAX_ADDRESSES {
                self.addresses.insert(key, bucket.clone());
            }
            // If all 65,536 history buckets have checks in flight, no safe eviction exists.
            // The current failure still contributes to the global limit and notification.
        }
        prune(&mut self.failures, now);
        if self.failures.len() == 50 { self.failures.pop_front(); }
        self.failures.push_back(now);
        let notify = self.failures.len() >= 5
            && self.notified.is_none_or(|t| now.duration_since(t) >= NOTICE_COOLDOWN);
        if notify { self.notified = Some(now); }
        notify
    }
}

/// Canonicalize mapped IPv4 before grouping native IPv6 clients by /64.
fn key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(ip) => IpAddr::V4(ip),
        IpAddr::V6(ip) => IpAddr::V6(Ipv6Addr::from(u128::from(ip) & (u128::MAX << 64))),
    }
}

/// Each configured peer address has exactly one trusted header convention.
struct Proxy {
    address: IpAddr,
    header: &'static str,
}

pub(crate) struct Limits {
    writer: Arc<Writer>,
    counters: Mutex<Counters>,
    proxies: Vec<Proxy>,
}

impl Limits {
    pub(crate) fn from_env(writer: Arc<Writer>) -> Self {
        let value = std::env::var("VAULT_TRUSTED_PROXIES").unwrap_or_default();
        Self { writer, counters: Mutex::default(), proxies: proxies(&value) }
    }
}

fn client_address(proxies: &[Proxy], peer: Key, headers: &HeaderMap) -> Key {
    let peer = peer.map(|ip| ip.to_canonical());
    proxies.iter().find(|p| Some(p.address) == peer)
        .and_then(|p| {
            // Reject duplicate headers rather than guessing which one a proxy wrote.
            let mut values = headers.get_all(p.header).iter();
            let value = values.next()?.to_str().ok()?;
            if values.next().is_some() { return None; }
            if p.header == "x-forwarded-for" {
                value.rsplit(',').next()
            } else {
                Some(value)
            }
        })
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .or(peer).map(key)
}

fn proxies(value: &str) -> Vec<Proxy> {
    value.split(',').filter(|s| !s.trim().is_empty()).map(|entry| {
        let (kind, ip) = entry.trim().split_once('=')
            .expect("VAULT_TRUSTED_PROXIES entries must be caddy=IP or cloudflare=IP");
        let header = match kind {
            "caddy" => "x-forwarded-for",
            "cloudflare" => "cf-connecting-ip",
            _ => panic!("unknown VAULT_TRUSTED_PROXIES proxy type"),
        };
        Proxy {
            address: ip.parse::<IpAddr>().expect("VAULT_TRUSTED_PROXIES requires literal IP addresses").to_canonical(),
            header,
        }
    }).collect()
}

fn limited(seconds: u64) -> Response {
    let mut response = crate::ApiError::new(StatusCode::TOO_MANY_REQUESTS,
        "attempt_limit", "Too many credential checks. Try again later.").into_response();
    response.headers_mut().insert(RETRY_AFTER, seconds.to_string().parse().expect("integer header"));
    response
}

/// Releases the in-flight reservation even if the HTTP task is cancelled.
struct Attempt {
    limits: Arc<Limits>,
    key: Key,
    bucket: Bucket,
    finished: bool,
}

impl Attempt {
    fn finish(mut self, status: StatusCode) -> bool {
        let notify = self.limits.counters.lock().expect("attempt counters")
            .record(self.key, &self.bucket, status, Instant::now());
        self.finished = true;
        notify
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            self.limits.counters.lock().expect("attempt counters").release(self.key, &self.bucket);
        }
    }
}

pub(crate) async fn check(
    State(limits): State<Arc<Limits>>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::POST || !matches!(request.uri().path(),
        "/api/login" | "/api/devices/pair" | "/api/agent-codes/redeem") {
        return next.run(request).await;
    }
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let address = client_address(&limits.proxies, peer, request.headers());
    if let Some(seconds) = limits.counters.lock().expect("attempt counters").precheck(address, Instant::now()) {
        return limited(seconds);
    }
    // Bound uploads before reserving a credential check. A partial body cannot hold an
    // address lock or consume that address's in-flight budget.
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(BODY_TIMEOUT, to_bytes(body, BODY_LIMIT)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return crate::ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "body_limit", "Credential request body is too large or invalid.").into_response(),
        Err(_) => return crate::ApiError::new(StatusCode::REQUEST_TIMEOUT, "body_timeout", "Credential request body timed out.").into_response(),
    };
    let bucket = match limits.counters.lock().expect("attempt counters").begin(address, Instant::now()) {
        Ok(bucket) => bucket,
        Err(seconds) => return limited(seconds),
    };
    let attempt = Attempt { limits: limits.clone(), key: address, bucket, finished: false };
    let response = next.run(Request::from_parts(parts, Body::from(body))).await;
    if attempt.finish(response.status()) {
        let result = crate::blocking(limits.writer.clone(), |w| w.notify(&state::NewNotification {
            title: "Repeated sign-in failures".into(),
            body: Some("Five or more password or pairing/code checks failed in one minute. Attempt limits are active.".into()),
            source: "server:auth".into(), note_id: None, url: None,
        })).await;
        if result.is_err() { tracing::warn!("could not store authentication failure notification"); }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Key { Some(s.parse().unwrap()) }

    #[test]
    fn failure_window_expires() {
        let now = Instant::now();
        let mut b = Address { failures: VecDeque::from([now; 5]), last: Some(now), ..Address::default() };
        assert_eq!(b.retry(now + WINDOW, false), None);
    }

    #[test]
    fn backoff_real_request_timeline_stays_capped_until_24_hours_idle() {
        let mut now = Instant::now();
        let mut b = Address::default();
        for delay in [60, 120, 240, 480, 900, 900, 900] {
            // A real request checks the limit first, then records its failure.
            for _ in 0..5 {
                assert_eq!(b.retry(now, false), None);
                b.last = Some(now);
                b.failures.push_back(now);
            }
            assert_eq!(b.retry(now, false), Some(delay));
            assert_eq!(b.retry(now + Duration::from_secs(1), false), Some(delay));
            now += Duration::from_secs(delay);
        }
        now += HISTORY;
        assert_eq!(b.retry(now, true), None);
        for _ in 0..5 { b.last = Some(now); b.failures.push_back(now); }
        assert_eq!(b.retry(now, false), Some(60));
    }

    #[test]
    fn canonical_keys_and_trusted_header_fallbacks() {
        let trusted = proxies("caddy=127.0.0.1");
        let peer = ip("::ffff:127.0.0.1");
        let mut headers = HeaderMap::new();
        assert_eq!(client_address(&trusted, peer, &headers), ip("127.0.0.1"));
        headers.insert("x-forwarded-for", "spoofed, ::ffff:192.0.2.1".parse().unwrap());
        assert_eq!(client_address(&trusted, peer, &headers), ip("192.0.2.1"));
        headers.append("x-forwarded-for", "192.0.2.2".parse().unwrap());
        assert_eq!(client_address(&trusted, peer, &headers), ip("127.0.0.1"));
        headers.clear();
        headers.insert("x-forwarded-for", "2001:db8:1:2::ab".parse().unwrap());
        assert_eq!(client_address(&trusted, peer, &headers), ip("2001:db8:1:2::"));
        assert_eq!(client_address(&trusted, ip("192.0.2.3"), &headers), ip("192.0.2.3"));
        let trusted = proxies("cloudflare=::ffff:127.0.0.1");
        headers.insert("cf-connecting-ip", "192.0.2.1, 192.0.2.2".parse().unwrap());
        assert_eq!(client_address(&trusted, peer, &headers), ip("127.0.0.1"));
        assert_eq!(client_address(&trusted, None, &headers), None);
    }

    #[test]
    fn concurrent_checks_reserve_budget_without_counting_successes() {
        let mut c = Counters::default();
        let now = Instant::now();
        let k = ip("192.0.2.1");
        let checks: Vec<_> = (0..5).map(|_| c.begin(k, now).unwrap()).collect();
        assert!(matches!(c.begin(k, now), Err(1)));
        assert!(c.failures.is_empty());
        for b in checks { c.record(k, &b, StatusCode::UNAUTHORIZED, now); }
        assert!(matches!(c.begin(k, now), Err(60)));
        let clean = ip("192.0.2.2");
        let b = c.begin(clean, now).unwrap();
        c.record(clean, &b, StatusCode::OK, now);
        assert!(!c.addresses.contains_key(&clean));
        assert!(c.pending.is_empty());
    }

    #[test]
    fn full_table_admits_clean_owner_and_evicts_only_after_failure() {
        let now = Instant::now();
        let mut c = Counters { pruned: Some(now), ..Counters::default() };
        for i in 0..MAX_ADDRESSES as u32 {
            let k = Some(IpAddr::V4((0x0a000000u32 + i).into()));
            c.addresses.insert(k, Arc::new(Mutex::new(Address {
                last: Some(now), failures: VecDeque::from([now]), ..Address::default()
            })));
        }
        let oldest = ip("10.0.0.0");
        c.addresses.get(&oldest).unwrap().lock().unwrap().last = Some(now - Duration::from_secs(1));
        let owner = ip("192.0.2.1");
        let b = c.begin(owner, now).unwrap();
        assert_eq!(c.addresses.len(), MAX_ADDRESSES);
        c.record(owner, &b, StatusCode::OK, now);
        assert!(c.addresses.contains_key(&oldest));
        let b = c.begin(owner, now).unwrap();
        c.record(owner, &b, StatusCode::UNAUTHORIZED, now);
        assert_eq!(c.addresses.len(), MAX_ADDRESSES);
        assert!(!c.addresses.contains_key(&oldest));
        assert!(c.addresses.contains_key(&owner));
        // A busy old bucket is protected from eviction even if its failure is older.
        c.addresses.get(&owner).unwrap().lock().unwrap().last = Some(now - Duration::from_secs(2));
        let held = c.begin(owner, now).unwrap();
        let other = ip("192.0.2.2");
        let b = c.begin(other, now).unwrap();
        c.record(other, &b, StatusCode::UNAUTHORIZED, now);
        assert!(c.addresses.contains_key(&owner));
        c.release(owner, &held);
    }
}
