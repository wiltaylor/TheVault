//! The push dispatcher against a stand-in push service: what it sends, to whom, and what it does
//! with the answers (D4, D9).

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use push::{Delivery, Dispatcher, webpush};
use serde_json::{Value, json};
use state::{NewDevice, NewNotification, PushTarget, State};
use tempfile::TempDir;
use testkit::{Reply, StandIn};

/// A device's WebPush keys: what it registers, and what it decrypts with.
struct Keys {
    secret: SecretKey,
    auth: [u8; 16],
}

impl Keys {
    fn new(seed: u8) -> Keys {
        Keys {
            secret: SecretKey::from_slice(&[seed; 32]).unwrap(),
            auth: [seed; 16],
        }
    }

    fn target(&self, endpoint: String) -> PushTarget {
        PushTarget {
            endpoint,
            p256dh: URL_SAFE_NO_PAD.encode(self.secret.public_key().to_encoded_point(false)),
            auth: URL_SAFE_NO_PAD.encode(self.auth),
        }
    }

    fn open(&self, body: &[u8]) -> Value {
        serde_json::from_slice(&webpush::decrypt(&self.secret, &self.auth, body).unwrap()).unwrap()
    }
}

fn open_state(dir: &TempDir) -> Arc<State> {
    Arc::new(State::open(&dir.path().join("state.db"), None).unwrap())
}

fn device(state: &State, name: &str, push: Option<&PushTarget>) -> String {
    let (d, _) = state
        .pair_device(&NewDevice {
            name,
            platform: "android",
            capabilities: "[]",
            settings: "{}",
        })
        .unwrap();
    if let Some(p) = push {
        state.set_device_push(&d.id, Some(p)).unwrap();
    }
    d.id
}

fn quick() -> Dispatcher {
    Dispatcher::default().with_retries(3, Duration::from_millis(100))
}

/// Wait until the stand-in has received `n` requests.
async fn wait_for(service: &StandIn, n: usize) -> Vec<testkit::Request> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let seen = service.requests();
        if seen.len() >= n {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "only {} of {n} requests arrived",
            seen.len()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn each_notification_event_reaches_every_device_with_an_endpoint() {
    let dir = TempDir::new().unwrap();
    let state = open_state(&dir);
    let service = StandIn::start();
    service.route("/a", Reply::text(201, ""));
    service.route("/b", Reply::text(201, ""));
    let (ka, kb) = (Keys::new(1), Keys::new(2));
    device(&state, "A", Some(&ka.target(service.url("/a"))));
    device(&state, "B", Some(&kb.target(service.url("/b"))));
    device(&state, "No endpoint", None);
    quick().spawn(state.clone());

    let n = state
        .add_notification(&NewNotification {
            title: "Hello".into(),
            source: "api".into(),
            ..Default::default()
        })
        .unwrap();
    let seen = wait_for(&service, 2).await;
    let mut paths: Vec<_> = seen.iter().map(|r| r.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, ["/a", "/b"]);
    for r in &seen {
        assert_eq!(r.method, "POST");
        assert_eq!(r.header("content-encoding"), Some("aes128gcm"));
        assert_eq!(r.header("urgency"), Some("high"));
        assert!(r.header("ttl").is_some());
        let keys = if r.path == "/a" { &ka } else { &kb };
        assert_eq!(
            keys.open(&r.body),
            json!({ "event": "notification.created", "id": n.id })
        );
        // The title is not in the message, only the id.
        assert!(!String::from_utf8_lossy(&r.body).contains("Hello"));
    }

    state.mark_notification_read(&n.id).unwrap();
    state.clear_read_notifications().unwrap();
    let m = state
        .add_notification(&NewNotification {
            title: "Second".into(),
            source: "api".into(),
            ..Default::default()
        })
        .unwrap();
    state.delete_notification(&m.id).unwrap();
    let seen = wait_for(&service, 10).await;
    let mut to_a: Vec<Value> = seen
        .iter()
        .filter(|r| r.path == "/a")
        .map(|r| ka.open(&r.body))
        .collect();
    to_a.sort_by_key(|v| v.to_string());
    let mut want = vec![
        json!({ "event": "notification.created", "id": n.id }),
        json!({ "event": "notification.read", "id": n.id }),
        json!({ "event": "notification.cleared" }),
        json!({ "event": "notification.created", "id": m.id }),
        json!({ "event": "notification.dismissed", "id": m.id }),
    ];
    want.sort_by_key(|v| v.to_string());
    assert_eq!(to_a, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_command_goes_only_to_its_device() {
    let dir = TempDir::new().unwrap();
    let state = open_state(&dir);
    let service = StandIn::start();
    service.route("/a", Reply::text(201, ""));
    service.route("/b", Reply::text(201, ""));
    let (ka, kb) = (Keys::new(1), Keys::new(2));
    device(&state, "A", Some(&ka.target(service.url("/a"))));
    let b = device(&state, "B", Some(&kb.target(service.url("/b"))));
    quick().spawn(state.clone());

    state.events().publish(state::LiveEvent::DeviceCommand {
        device: b.clone(),
        id: "cmd1".into(),
    });
    let seen = wait_for(&service, 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let seen_after = service.requests();
    assert_eq!(seen_after.len(), 1, "{seen_after:?}");
    assert_eq!(seen[0].path, "/b");
    assert_eq!(
        kb.open(&seen[0].body),
        json!({ "event": "device.command", "id": "cmd1" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn note_changes_are_not_pushed() {
    let dir = TempDir::new().unwrap();
    let state = open_state(&dir);
    let service = StandIn::start();
    service.route("/a", Reply::text(201, ""));
    device(&state, "A", Some(&Keys::new(1).target(service.url("/a"))));
    quick().spawn(state.clone());
    state.events().publish(state::LiveEvent::NoteChanged {
        id: "n".into(),
        type_name: "memo".into(),
        version: "1".into(),
        deleted: false,
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(service.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gone_endpoint_is_removed_from_the_device() {
    for status in [410, 404] {
        let dir = TempDir::new().unwrap();
        let state = open_state(&dir);
        let service = StandIn::start();
        service.route("/a", Reply::text(status, "gone"));
        let target = Keys::new(1).target(service.url("/a"));
        let id = device(&state, "A", Some(&target));
        let got = quick().deliver(&state, &id, &target, b"{}", "normal").await;
        assert_eq!(got, Delivery::Gone);
        assert_eq!(
            service.requests().len(),
            1,
            "a gone endpoint is not retried"
        );
        assert_eq!(state.device(&id).unwrap().push, None);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gone_answer_keeps_an_endpoint_registered_since() {
    let dir = TempDir::new().unwrap();
    let state = open_state(&dir);
    let service = StandIn::start();
    service.route("/old", Reply::text(410, "gone"));
    let keys = Keys::new(1);
    let old = keys.target(service.url("/old"));
    let new = keys.target(service.url("/new"));
    let id = device(&state, "A", Some(&new));
    let got = quick().deliver(&state, &id, &old, b"{}", "normal").await;
    assert_eq!(got, Delivery::Gone);
    assert_eq!(state.device(&id).unwrap().push, Some(new));
}

#[tokio::test(flavor = "multi_thread")]
async fn failures_are_retried_with_growing_waits() {
    let dir = TempDir::new().unwrap();
    let state = open_state(&dir);
    let service = StandIn::start();
    service.route("/a", Reply::text(503, "busy"));
    let target = Keys::new(1).target(service.url("/a"));
    let id = device(&state, "A", Some(&target));

    let started = Instant::now();
    let got = quick().deliver(&state, &id, &target, b"{}", "normal").await;
    assert_eq!(got, Delivery::GaveUp);
    assert_eq!(service.requests().len(), 3);
    // Waits of 100 ms then 200 ms between the three attempts.
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(state.device(&id).unwrap().push.is_some());

    // A service that recovers gets the message on a later attempt.
    let service = StandIn::start();
    service.route("/a", Reply::text(429, "slow down"));
    let target = Keys::new(1).target(service.url("/a"));
    let dispatcher = quick();
    let (s, t, i) = (state.clone(), target.clone(), id.clone());
    let delivery =
        tokio::spawn(async move { dispatcher.deliver(&s, &i, &t, b"{}", "normal").await });
    wait_for(&service, 1).await;
    service.route("/a", Reply::text(201, ""));
    assert_eq!(delivery.await.unwrap(), Delivery::Delivered);
    assert_eq!(service.requests().len(), 2);

    // An unreachable endpoint is retried too.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/a", closed.local_addr().unwrap());
    drop(closed);
    let target = Keys::new(1).target(url);
    let got = quick().deliver(&state, &id, &target, b"{}", "normal").await;
    assert_eq!(got, Delivery::GaveUp);
}

#[tokio::test(flavor = "multi_thread")]
async fn other_refusals_are_not_retried() {
    let dir = TempDir::new().unwrap();
    let state = open_state(&dir);
    let service = StandIn::start();
    service.route("/a", Reply::text(400, "bad"));
    let target = Keys::new(1).target(service.url("/a"));
    let id = device(&state, "A", Some(&target));
    let got = quick().deliver(&state, &id, &target, b"{}", "normal").await;
    assert_eq!(got, Delivery::Refused(400));
    assert_eq!(service.requests().len(), 1);
    assert!(state.device(&id).unwrap().push.is_some());
}
