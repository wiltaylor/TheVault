//! Shared, pre-handler limits for public credential checks. No credentials are retained.
use std::{collections::{HashMap, VecDeque}, net::{IpAddr, SocketAddr}, sync::{Arc, Mutex}, time::{Duration, Instant}};

use axum::{extract::{ConnectInfo, Request, State}, http::{Method, StatusCode, HeaderMap, header::RETRY_AFTER}, middleware::Next, response::{IntoResponse, Response}};
use writer::Writer;

const WINDOW: Duration = Duration::from_secs(60);
const IDLE: Duration = Duration::from_secs(900);
const MAX_ADDRESSES: usize = 65536;

#[derive(Default)]
struct Address {
    failures: VecDeque<Instant>,
    last: Option<Instant>,
    until: Option<Instant>,
    strikes: u32,
}

fn prune(failures: &mut VecDeque<Instant>, now: Instant) {
    while failures.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
        failures.pop_front();
    }
}

impl Address {
    fn retry(&mut self, now: Instant, global_full: bool) -> Option<u64> {
        if self.last.is_some_and(|t| now.duration_since(t) >= IDLE) {
            *self = Self::default();
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
        None
    }
}

#[derive(Default)]
struct Counters {
    addresses: HashMap<Option<IpAddr>, Arc<tokio::sync::Mutex<Address>>>,
    failures: VecDeque<Instant>,
    notified: Option<Instant>,
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

    fn address(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
        let proxy = self.proxies.iter().find(|p| Some(p.address) == peer);
        proxy.and_then(|p| headers.get(p.header))
            .and_then(|v| v.to_str().ok())
            // Caddy appends its immediate client, so ignore the untrusted prefix.
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
            .or(peer)
    }
}

fn proxies(value: &str) -> Vec<Proxy> {
    value.split(',').filter(|s| !s.trim().is_empty()).map(|entry| {
        let (kind, ip) = entry.trim().split_once('=').expect("VAULT_TRUSTED_PROXIES entries must be caddy=IP or cloudflare=IP");
        let header = match kind {
            "caddy" => "x-forwarded-for",
            "cloudflare" => "cf-connecting-ip",
            _ => panic!("unknown VAULT_TRUSTED_PROXIES proxy type"),
        };
        Proxy { address: ip.parse().expect("VAULT_TRUSTED_PROXIES requires literal IP addresses"), header }
    }).collect()
}

fn limited(seconds: u64) -> Response {
    let mut response = crate::ApiError::new(StatusCode::TOO_MANY_REQUESTS, "attempt_limit", "Too many credential checks. Try again later.").into_response();
    response.headers_mut().insert(RETRY_AFTER, seconds.to_string().parse().expect("integer header"));
    response
}

pub(crate) async fn check(State(limits): State<Arc<Limits>>, request: Request, next: Next) -> Response {
    if request.method() != Method::POST || !matches!(request.uri().path(), "/api/login" | "/api/devices/pair" | "/api/agent-codes/redeem") {
        return next.run(request).await;
    }
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let address = limits.address(peer, request.headers());
    let now = Instant::now();
    let (bucket, global_full) = {
        let mut counters = limits.counters.lock().expect("attempt counters");
        prune(&mut counters.failures, now);
        // Retain locked/in-flight buckets. Idle addresses need no long-term storage.
        counters.addresses.retain(|_, bucket| Arc::strong_count(bucket) > 1 || bucket.try_lock().map_or(true, |b| b.last.is_some_and(|t| now.duration_since(t) < IDLE)));
        if counters.addresses.len() >= MAX_ADDRESSES && !counters.addresses.contains_key(&address) {
            return crate::ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "attempt_capacity", "Credential checks are busy. Try again later.").into_response();
        }
        let bucket = counters.addresses.entry(address).or_default().clone();
        (bucket, counters.failures.len() >= 50)
    };
    // Serialize checks from an address without queueing requests or letting concurrent hashes
    // bypass a failure budget. Other addresses can still sign in during a distributed flood.
    let Ok(mut bucket) = bucket.try_lock() else { return limited(1); };
    if let Some(seconds) = bucket.retry(now, global_full) { return limited(seconds); }
    let response = next.run(request).await;
    let now = Instant::now();
    if response.status().is_success() {
        *bucket = Address::default();
    } else if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::GONE) {
        bucket.last = Some(now);
        bucket.failures.push_back(now);
        let notify = {
            let mut counters = limits.counters.lock().expect("attempt counters");
            prune(&mut counters.failures, now);
            // Only the first 50 are needed for the global threshold.
            if counters.failures.len() == 50 { counters.failures.pop_front(); }
            counters.failures.push_back(now);
            let notify = counters.failures.len() >= 5 && counters.notified.is_none_or(|t| now.duration_since(t) >= IDLE);
            if notify { counters.notified = Some(now); }
            notify
        };
        if notify {
            let result = crate::blocking(limits.writer.clone(), |w| w.notify(&state::NewNotification {
                title: "Repeated sign-in failures".into(),
                body: Some("Five or more password or pairing/code checks failed in one minute. Attempt limits are active.".into()),
                source: "server:auth".into(), note_id: None, url: None,
            })).await;
            if result.is_err() { tracing::warn!("could not store authentication failure notification"); }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_windows_backoff_and_clean_address_exception() {
        let now = Instant::now();
        let mut b = Address::default();
        assert_eq!(b.retry(now, true), None);
        b.last = Some(now);
        b.failures.push_back(now);
        assert_eq!(b.retry(now, true), Some(60));
        assert_eq!(b.retry(now + Duration::from_secs(10), false), Some(51));
        let later = now + Duration::from_secs(61);
        assert_eq!(b.retry(later, false), None);
        b.last = Some(later);
        b.failures.push_back(later);
        assert_eq!(b.retry(later, true), Some(120));
        assert_eq!(b.retry(later + IDLE, true), None);
        for _ in 0..5 { b.failures.push_back(later + IDLE); }
        assert_eq!(b.retry(later + IDLE, false), Some(60));
    }

    #[test]
    fn failure_window_expires() {
        let now = Instant::now();
        let mut b = Address { failures: VecDeque::from([now; 5]), last: Some(now), ..Address::default() };
        assert_eq!(b.retry(now + WINDOW, false), None);
    }
}
