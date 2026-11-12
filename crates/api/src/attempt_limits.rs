//! Shared, pre-handler limits for public credential checks. No credentials are retained.
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
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
use serde::Serialize;
use serde_json::json;

const WINDOW: Duration = Duration::from_secs(60);
const NOTICE_COOLDOWN: Duration = Duration::from_secs(900);
const HISTORY: Duration = Duration::from_secs(24 * 60 * 60);
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_LIMIT: usize = 2 * 1024 * 1024;
const MAX_ADDRESSES: usize = 65536;
type Key = Option<IpAddr>;
type Bucket = Arc<Mutex<Address>>;

struct Started {
    bucket: Bucket,
    at: Instant,
    id: u64,
}

#[derive(Default)]
struct Address {
    failures: VecDeque<Instant>,
    last: Option<Instant>,
    until: Option<Instant>,
    strikes: u32,
    in_flight: usize,
}

fn prune(failures: &mut VecDeque<Instant>, now: Instant) {
    while failures
        .front()
        .is_some_and(|t| now.duration_since(*t) >= WINDOW)
    {
        failures.pop_front();
    }
}

impl Address {
    fn reset(&mut self) {
        let in_flight = self.in_flight;
        *self = Self {
            in_flight,
            ..Self::default()
        };
    }

    fn retry(&mut self, now: Instant, global_full: bool) -> Option<u64> {
        if self.last.is_some_and(|t| now.duration_since(t) >= HISTORY) {
            self.reset();
        }
        if let Some(until) = self.until.take().filter(|t| *t > now) {
            self.until = Some(until);
            let left = until.duration_since(now);
            return Some(left.as_secs() + u64::from(left.subsec_nanos() != 0));
        }
        prune(&mut self.failures, now);
        // In-flight checks are provisional failures from admission, not free slots that a
        // disconnect can refund. Only a successful result clears the address's charges.
        let charged = self.failures.len() + self.in_flight;
        if charged >= 5 || (global_full && charged != 0) {
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
    // History is bounded. Active requests own temporary buckets separately, including a new
    // clean client when history is full. Slow bodies never reach this map.
    addresses: HashMap<Key, Bucket>,
    pending: HashMap<Key, Bucket>,
    // Only non-in-flight history is indexed, so the oldest eligible victim is O(log n).
    eviction: BTreeSet<(Instant, Key)>,
    provisional: BTreeSet<(Instant, u64)>,
    next_id: u64,
    failures: VecDeque<Instant>,
    notified: Option<Instant>,
    pruned: Option<Instant>,
}

impl Counters {
    fn bucket(&self, key: Key) -> Option<Bucket> {
        self.pending
            .get(&key)
            .or_else(|| self.addresses.get(&key))
            .cloned()
    }

    fn unindex(&mut self, key: Key) {
        if let Some(bucket) = self.addresses.get(&key)
            && let Some(last) = bucket.lock().expect("attempt bucket").last
        {
            self.eviction.remove(&(last, key));
        }
    }

    fn index(&mut self, key: Key) {
        if let Some(bucket) = self.addresses.get(&key) {
            let b = bucket.lock().expect("attempt bucket");
            if b.in_flight == 0
                && let Some(last) = b.last
            {
                self.eviction.insert((last, key));
            }
        }
    }

    fn precheck(&mut self, key: Key, now: Instant) -> Option<u64> {
        prune(&mut self.failures, now);
        while self
            .provisional
            .first()
            .is_some_and(|(at, _)| now.duration_since(*at) >= WINDOW)
        {
            self.provisional.pop_first();
        }
        // retry may expire history, so keep its eviction entry consistent.
        self.unindex(key);
        let global_full = self.failures.len() + self.provisional.len() >= 50;
        let retry = self
            .bucket(key)
            .and_then(|b| b.lock().expect("attempt bucket").retry(now, global_full));
        let expired = self.addresses.get(&key).is_some_and(|b| {
            let b = b.lock().expect("attempt bucket");
            b.last.is_none() && b.in_flight == 0
        });
        if expired {
            self.addresses.remove(&key);
        }
        self.index(key);
        retry
    }

    fn begin(&mut self, key: Key, now: Instant) -> Result<Started, u64> {
        if let Some(delay) = self.precheck(key, now) {
            return Err(delay);
        }
        if !self.addresses.contains_key(&key)
            && self
                .pruned
                .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            self.pruned = Some(now);
            // Bounded work, not a full-table scan, even after every bucket has expired.
            for _ in 0..64 {
                if !self
                    .eviction
                    .first()
                    .is_some_and(|(at, _)| now.duration_since(*at) >= HISTORY)
                {
                    break;
                }
                let (_, old) = self.eviction.pop_first().expect("oldest history");
                self.addresses.remove(&old);
            }
        }
        self.unindex(key);
        let bucket = self.bucket(key).unwrap_or_default();
        {
            let mut b = bucket.lock().expect("attempt bucket");
            b.in_flight += 1;
        }
        self.pending.insert(key, bucket.clone());
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("attempt sequence exhausted");
        self.provisional.insert((now, id));
        Ok(Started {
            bucket,
            at: now,
            id,
        })
    }

    fn record(&mut self, key: Key, started: &Started, status: StatusCode, now: Instant) -> bool {
        // Every lock follows counters -> bucket, and is released before any await.
        self.unindex(key);
        self.provisional.remove(&(started.at, started.id));
        let failed = !status.is_success();
        {
            let mut b = started.bucket.lock().expect("attempt bucket");
            b.in_flight -= 1;
            if b.in_flight == 0 {
                self.pending.remove(&key);
            }
            if !failed {
                // Other in-flight charges survive this successful check.
                b.reset();
                self.addresses.remove(&key);
            } else {
                b.last = Some(now);
                prune(&mut b.failures, now);
                b.failures.push_back(now);
            }
        }
        if !failed {
            return false;
        }
        if !self.addresses.contains_key(&key) {
            if self.addresses.len() >= MAX_ADDRESSES
                && let Some((_, oldest)) = self.eviction.pop_first()
            {
                self.addresses.remove(&oldest);
            }
            if self.addresses.len() < MAX_ADDRESSES {
                self.addresses.insert(key, started.bucket.clone());
            }
            // With every retained bucket in flight, still count this failure globally.
        }
        self.index(key);
        prune(&mut self.failures, now);
        if self.failures.len() == 50 {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        let notify = self.failures.len() >= 5
            && self
                .notified
                .is_none_or(|t| now.duration_since(t) >= NOTICE_COOLDOWN);
        if notify {
            self.notified = Some(now);
        }
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

#[derive(Clone, Serialize)]
pub(crate) struct Diagnostic {
    client_address: Option<String>,
    client_address_source: &'static str,
    trusted_proxies: Vec<serde_json::Value>,
}

pub(crate) async fn diagnostic(
    axum::Extension(info): axum::Extension<Diagnostic>,
) -> axum::Json<Diagnostic> {
    axum::Json(info)
}

pub(crate) struct Limits {
    writer: Arc<Writer>,
    counters: Mutex<Counters>,
    proxies: Vec<Proxy>,
}

impl Limits {
    pub(crate) fn from_env(writer: Arc<Writer>) -> Self {
        let value = std::env::var("VAULT_TRUSTED_PROXIES").unwrap_or_default();
        let weak = Arc::downgrade(&writer);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let Some(writer) = weak.upgrade() else { break };
                if crate::blocking(writer, |w| w.state().prune_auth_failure_history()).await.is_err() {
                    tracing::warn!("could not prune authentication failure history");
                }
            }
        });
        Self {
            writer,
            counters: Mutex::default(),
            proxies: proxies(&value),
        }
    }
}

#[cfg(test)]
fn client_address(proxies: &[Proxy], peer: Key, headers: &HeaderMap) -> Key {
    resolve(proxies, peer, headers).0
}

fn resolve(proxies: &[Proxy], peer: Key, headers: &HeaderMap) -> (Key, &'static str) {
    let peer = peer.map(|ip| ip.to_canonical());
    let forwarded = proxies
        .iter()
        .find(|p| Some(p.address) == peer)
        .and_then(|p| {
            // Reject duplicate headers rather than guessing which one a proxy wrote.
            let mut values = headers.get_all(p.header).iter();
            let value = values.next()?.to_str().ok()?;
            if values.next().is_some() {
                return None;
            }
            let value = if p.header == "x-forwarded-for" {
                value.rsplit(',').next()
            } else {
                Some(value)
            }?;
            Some((key(value.trim().parse::<IpAddr>().ok()?), p.header))
        });
    forwarded.unwrap_or((peer.map(key), if peer.is_some() { "peer" } else { "unavailable" }))
}

fn proxies(value: &str) -> Vec<Proxy> {
    value
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|entry| {
            let (kind, ip) = entry
                .trim()
                .split_once('=')
                .expect("VAULT_TRUSTED_PROXIES entries must be caddy=IP or cloudflare=IP");
            let header = match kind {
                "caddy" => "x-forwarded-for",
                "cloudflare" => "cf-connecting-ip",
                _ => panic!("unknown VAULT_TRUSTED_PROXIES proxy type"),
            };
            Proxy {
                address: ip
                    .parse::<IpAddr>()
                    .expect("VAULT_TRUSTED_PROXIES requires literal IP addresses")
                    .to_canonical(),
                header,
            }
        })
        .collect()
}

fn limited(seconds: u64) -> Response {
    let mut response = crate::ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "attempt_limit",
        "Too many credential checks. Try again later.",
    )
    .into_response();
    response.headers_mut().insert(
        RETRY_AFTER,
        seconds.to_string().parse().expect("integer header"),
    );
    response
}

/// Counts an unfinished admitted check as a failure even if the HTTP task is cancelled.
struct Attempt {
    limits: Arc<Limits>,
    key: Key,
    started: Started,
    finished: bool,
    info: Diagnostic,
}

impl Attempt {
    fn finish(mut self, status: StatusCode) -> Option<tokio::task::JoinHandle<()>> {
        let notify = self
            .limits
            .counters
            .lock()
            .expect("attempt counters")
            .record(self.key, &self.started, status, Instant::now());
        self.finished = true;
        notify.then(|| notify_owner(self.limits.clone(), self.info.clone()))
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            let notify = self
                .limits
                .counters
                .lock()
                .expect("attempt counters")
                .record(
                    self.key,
                    &self.started,
                    StatusCode::REQUEST_TIMEOUT,
                    Instant::now(),
                );
            if notify {
                drop(notify_owner(self.limits.clone(), self.info.clone()));
            }
        }
    }
}

fn notify_owner(limits: Arc<Limits>, info: Diagnostic) -> tokio::task::JoinHandle<()> {
    // Detached from the HTTP future, so disconnecting cannot suppress the notification.
    tokio::spawn(async move {
        let result = crate::blocking(limits.writer.clone(), move |w| w.notify(&state::NewNotification {
            title: "Repeated sign-in failures".into(),
            body: Some(format!("Five or more password or pairing/code checks failed in one minute. Attempt limits are active. Latest key: {} ({}).", info.client_address.as_deref().unwrap_or("unavailable"), info.client_address_source)),
            source: "server:auth".into(), note_id: None, url: None,
        })).await;
        if result.is_err() {
            tracing::warn!("could not store authentication failure notification");
        }
    })
}

pub(crate) async fn check(
    State(limits): State<Arc<Limits>>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let (address, source) = resolve(&limits.proxies, peer, request.headers());
    let info = Diagnostic {
        client_address: address.map(|ip| match ip {
            IpAddr::V4(_) => ip.to_string(),
            IpAddr::V6(_) => format!("{ip}/64"),
        }),
        client_address_source: source,
        trusted_proxies: limits.proxies.iter().map(|p| json!({"address": p.address.to_string(), "header": p.header})).collect(),
    };
    request.extensions_mut().insert(info.clone());
    if request.method() != Method::POST
        || !matches!(
            request.uri().path(),
            "/api/login" | "/api/devices/pair" | "/api/agent-codes/redeem"
        )
    {
        return next.run(request).await;
    }
    let kind = match request.uri().path() {
        "/api/login" => "auth.login_failed",
        "/api/devices/pair" => "auth.pair_failed",
        _ => "auth.agent_code_failed",
    };
    let user_agent = request.headers().get("user-agent").and_then(|v| v.to_str().ok()).map(|v| state::one_line(v, 200));
    if let Some(seconds) = limits
        .counters
        .lock()
        .expect("attempt counters")
        .precheck(address, Instant::now())
    {
        return limited(seconds);
    }
    // Bound uploads before reserving a credential check. A partial body cannot hold an
    // address lock or consume that address's in-flight budget.
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(BODY_TIMEOUT, to_bytes(body, BODY_LIMIT)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => {
            return crate::ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_limit",
                "Credential request body is too large or invalid.",
            )
            .into_response();
        }
        Err(_) => {
            return crate::ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "body_timeout",
                "Credential request body timed out.",
            )
            .into_response();
        }
    };
    let started = match limits
        .counters
        .lock()
        .expect("attempt counters")
        .begin(address, Instant::now())
    {
        Ok(started) => started,
        Err(seconds) => return limited(seconds),
    };
    let attempt = Attempt {
        limits: limits.clone(),
        key: address,
        started,
        finished: false,
        info: info.clone(),
    };
    let response = next.run(Request::from_parts(parts, Body::from(body))).await;
    if !response.status().is_success() {
        let _ = crate::blocking(limits.writer.clone(), move |w| {
            w.state().prune_auth_failure_history()?;
            w.state().record_activity(state::NewActivity::server(kind, "auth", "A credential check failed").detail(json!({
                "client_address": info.client_address,
                "client_address_source": info.client_address_source,
                "user_agent": user_agent,
            })));
            Ok::<_, state::Error>(())
        }).await;
    }
    if let Some(notice) = attempt.finish(response.status()) {
        // Dropping this JoinHandle on disconnect detaches rather than cancels the write.
        let _ = notice.await;
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Key {
        Some(s.parse().unwrap())
    }

    #[test]
    fn failure_window_expires() {
        let now = Instant::now();
        let mut b = Address {
            failures: VecDeque::from([now; 5]),
            last: Some(now),
            ..Address::default()
        };
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
            assert_eq!(
                b.retry(now + Duration::from_secs(1), false),
                Some(delay - 1)
            );
            now += Duration::from_secs(delay);
        }
        now += HISTORY;
        assert_eq!(b.retry(now, true), None);
        for _ in 0..5 {
            b.last = Some(now);
            b.failures.push_back(now);
        }
        assert_eq!(b.retry(now, false), Some(60));
    }

    #[test]
    fn canonical_keys_and_trusted_header_fallbacks() {
        let trusted = proxies("caddy=127.0.0.1");
        let peer = ip("::ffff:127.0.0.1");
        let mut headers = HeaderMap::new();
        assert_eq!(client_address(&trusted, peer, &headers), ip("127.0.0.1"));
        headers.insert(
            "x-forwarded-for",
            "spoofed, ::ffff:192.0.2.1".parse().unwrap(),
        );
        assert_eq!(client_address(&trusted, peer, &headers), ip("192.0.2.1"));
        headers.append("x-forwarded-for", "192.0.2.2".parse().unwrap());
        assert_eq!(client_address(&trusted, peer, &headers), ip("127.0.0.1"));
        headers.clear();
        headers.insert("x-forwarded-for", "2001:db8:1:2::ab".parse().unwrap());
        assert_eq!(
            client_address(&trusted, peer, &headers),
            ip("2001:db8:1:2::")
        );
        assert_eq!(
            client_address(&trusted, ip("192.0.2.3"), &headers),
            ip("192.0.2.3")
        );
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
        assert!(matches!(c.begin(k, now), Err(60)));
        assert_eq!(c.provisional.len(), 5);
        assert!(c.failures.is_empty());
        for b in checks {
            c.record(k, &b, StatusCode::UNAUTHORIZED, now);
        }
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
        let mut c = Counters {
            pruned: Some(now),
            ..Counters::default()
        };
        for i in 0..MAX_ADDRESSES as u32 {
            let k = Some(IpAddr::V4((0x0a000000u32 + i).into()));
            c.addresses.insert(
                k,
                Arc::new(Mutex::new(Address {
                    last: Some(now),
                    failures: VecDeque::from([now]),
                    ..Address::default()
                })),
            );
            c.index(k);
        }
        let oldest = ip("10.0.0.0");
        c.unindex(oldest);
        c.addresses.get(&oldest).unwrap().lock().unwrap().last = Some(now - Duration::from_secs(1));
        c.index(oldest);
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
        c.unindex(owner);
        c.addresses.get(&owner).unwrap().lock().unwrap().last = Some(now - Duration::from_secs(2));
        c.index(owner);
        let held = c.begin(owner, now).unwrap();
        let other = ip("192.0.2.2");
        let b = c.begin(other, now).unwrap();
        c.record(other, &b, StatusCode::UNAUTHORIZED, now);
        assert!(c.addresses.contains_key(&owner));
        c.record(owner, &held, StatusCode::UNAUTHORIZED, now);
        assert_eq!(c.eviction.len(), MAX_ADDRESSES);
    }

    #[test]
    fn capacity_eviction_uses_the_index_without_scanning_buckets() {
        let now = Instant::now();
        let mut c = Counters {
            pruned: Some(now),
            ..Counters::default()
        };
        for i in 0..MAX_ADDRESSES as u32 {
            let k = Some(IpAddr::V4((0x0a000000u32 + i).into()));
            c.addresses.insert(
                k,
                Arc::new(Mutex::new(Address {
                    last: Some(now),
                    ..Address::default()
                })),
            );
            c.index(k);
        }
        // An unrelated poisoned bucket makes a regression to locking every bucket fail
        // deterministically, rather than relying only on a wall-clock speed assertion.
        let unrelated = c.addresses.get(&ip("10.0.255.255")).unwrap().clone();
        assert!(
            std::panic::catch_unwind(|| {
                let _held = unrelated.lock().unwrap();
                panic!("unrelated eviction sentinel");
            })
            .is_err()
        );
        let start = Instant::now();
        for i in 0..2000u32 {
            let k = Some(IpAddr::V4((0xc0000200u32 + i).into()));
            let check = c.begin(k, now).unwrap();
            c.record(k, &check, StatusCode::UNAUTHORIZED, now);
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "2000 indexed replacements must stay cheap"
        );
        assert_eq!(c.addresses.len(), MAX_ADDRESSES);
        assert_eq!(c.eviction.len(), MAX_ADDRESSES);
        assert!(c.pending.is_empty());
        assert!(c.provisional.is_empty());
    }

    #[test]
    fn provisional_global_failures_are_removed_only_on_success() {
        let now = Instant::now();
        let mut c = Counters::default();
        let mut checks = Vec::new();
        for i in 1..=50u32 {
            let k = Some(IpAddr::V4((0xc0000200u32 + i).into()));
            checks.push((k, c.begin(k, now).unwrap()));
        }
        assert_eq!(c.provisional.len(), 50);
        assert!(matches!(c.begin(checks[0].0, now), Err(60)));
        let clean = ip("203.0.113.1");
        let owner = c.begin(clean, now).unwrap();
        c.record(clean, &owner, StatusCode::OK, now);
        for (k, check) in checks {
            c.record(k, &check, StatusCode::OK, now);
        }
        assert!(c.provisional.is_empty());
        assert!(c.failures.is_empty());
        assert!(c.addresses.is_empty());
        assert!(c.eviction.is_empty());
    }
}
