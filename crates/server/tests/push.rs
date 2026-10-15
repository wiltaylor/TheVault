//! S1: the server pushes notification events to each paired device's UnifiedPush endpoint as
//! RFC 8291 WebPush messages that carry ids only, and forgets an endpoint the push service says
//! is gone (D4, D9). Note changes are pushed too, coalesced, and never back to the device that
//! made them. A device's settings changing is pushed to that device alone, coalesced as well.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::{PASSWORD, Reply, StandIn, TestServer};

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

fn send(rb: RequestBuilder, status: u16) -> Value {
    let res = rb.send().unwrap();
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// A paired phone: its id, its token, and the WebPush keys it decrypts with.
struct Phone {
    id: String,
    token: String,
    secret: SecretKey,
    auth: [u8; 16],
}

impl Phone {
    fn pair(server: &TestServer, name: &str, seed: u8) -> Phone {
        let out = send(
            Client::new()
                .post(server.url("/api/devices/pair"))
                .json(&json!({ "password": PASSWORD, "name": name, "platform": "android" })),
            201,
        );
        Phone {
            id: out["device_id"].as_str().unwrap().to_owned(),
            token: out["token"].as_str().unwrap().to_owned(),
            secret: SecretKey::from_slice(&[seed; 32]).unwrap(),
            auth: [seed; 16],
        }
    }

    fn register(&self, server: &TestServer, endpoint: &str) {
        let p256dh = URL_SAFE_NO_PAD.encode(self.secret.public_key().to_encoded_point(false));
        let out = send(
            client(&self.token)
                .put(server.url(&format!("/api/devices/{}/push", self.id)))
                .json(&json!({
                    "endpoint": endpoint,
                    "p256dh": p256dh,
                    "auth": URL_SAFE_NO_PAD.encode(self.auth),
                })),
            200,
        );
        assert_eq!(out["push"], true);
    }

    fn open(&self, body: &[u8]) -> Value {
        let plain = push::webpush::decrypt(&self.secret, &self.auth, body)
            .expect("the message decrypts with the device's keys");
        serde_json::from_slice(&plain).unwrap()
    }
}

fn wait_for(service: &StandIn, n: usize) -> Vec<testkit::Request> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let seen = service.requests();
        if seen.len() >= n {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "only {} of {n} pushes arrived",
            seen.len()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn notify(admin: &Client, server: &TestServer, title: &str) -> String {
    let n = send(
        admin
            .post(server.url("/api/notifications"))
            .json(&json!({ "title": title })),
        201,
    );
    n["id"].as_str().unwrap().to_owned()
}

#[test]
fn notification_events_reach_registered_devices_encrypted_and_by_id() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let service = StandIn::start();
    service.route("/up/a", Reply::text(201, ""));
    let a = Phone::pair(&server, "Pixel", 3);
    a.register(&server, &service.url("/up/a"));
    // A second device that never registered an endpoint is sent nothing.
    let _quiet = Phone::pair(&server, "Tablet", 4);

    let id = notify(&admin, &server, "Build finished");
    let first = wait_for(&service, 1);
    assert_eq!(first[0].method, "POST");
    assert_eq!(first[0].path, "/up/a");
    assert_eq!(first[0].header("content-encoding"), Some("aes128gcm"));
    assert_eq!(
        a.open(&first[0].body),
        json!({ "event": "notification.created", "id": id })
    );

    send(
        admin.post(server.url(&format!("/api/notifications/{id}/read"))),
        200,
    );
    send(admin.delete(server.url("/api/notifications")), 200);
    let other = notify(&admin, &server, "Another");
    send(
        admin.delete(server.url(&format!("/api/notifications/{other}"))),
        200,
    );

    let seen = wait_for(&service, 5);
    let mut events: Vec<Value> = seen.iter().map(|r| a.open(&r.body)).collect();
    events.sort_by_key(|v| v.to_string());
    let mut want = vec![
        json!({ "event": "notification.created", "id": id }),
        json!({ "event": "notification.read", "id": id }),
        json!({ "event": "notification.cleared" }),
        json!({ "event": "notification.created", "id": other }),
        json!({ "event": "notification.dismissed", "id": other }),
    ];
    want.sort_by_key(|v| v.to_string());
    assert_eq!(events, want);
    // Only the registered device's endpoint was ever called.
    assert!(seen.iter().all(|r| r.path == "/up/a"));
    thread::sleep(Duration::from_millis(300));
    assert_eq!(service.requests().len(), 5);
}

#[test]
fn a_gone_endpoint_is_removed_from_the_device() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let service = StandIn::start();
    service.route("/up/gone", Reply::text(410, "unsubscribed"));
    let phone = Phone::pair(&server, "Pixel", 5);
    phone.register(&server, &service.url("/up/gone"));

    notify(&admin, &server, "Hello");
    wait_for(&service, 1);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let devices = send(admin.get(server.url("/api/devices")), 200);
        let d = devices["devices"]
            .as_array()
            .unwrap_or_else(|| panic!("{devices}"))
            .iter()
            .find(|d| d["id"] == phone.id.as_str())
            .unwrap()
            .clone();
        if d["push"] == false {
            break;
        }
        assert!(Instant::now() < deadline, "the endpoint was kept: {d}");
        thread::sleep(Duration::from_millis(20));
    }
    // With no endpoint left, the next notification is not pushed.
    notify(&admin, &server, "Again");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(service.requests().len(), 1);
}

const MEMO: &str = r#"label = "Memo"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn memo_type(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

fn memo(c: &Client, server: &TestServer, title: &str) -> String {
    let out = send(
        c.post(server.url("/api/notes?type=memo"))
            .json(&json!({ "note": { "fields": { "title": title }, "blocks": [] } })),
        201,
    );
    out["id"].as_str().unwrap().to_owned()
}

/// The pushes `phone` got at `path`, oldest first.
fn pushed(service: &StandIn, phone: &Phone, path: &str) -> Vec<Value> {
    service
        .requests()
        .iter()
        .filter(|r| r.path == path)
        .map(|r| phone.open(&r.body))
        .collect()
}

#[test]
fn note_changes_are_pushed_coalesced_and_never_to_the_device_that_made_them() {
    let server = TestServer::start_seeded(memo_type);
    let admin = client(server.admin_token());
    let service = StandIn::start();
    service.route("/up/a", Reply::text(201, ""));
    service.route("/up/b", Reply::text(201, ""));
    let a = Phone::pair(&server, "Pixel", 6);
    let b = Phone::pair(&server, "Tablet", 7);
    a.register(&server, &service.url("/up/a"));
    b.register(&server, &service.url("/up/b"));

    // A change after a quiet spell is pushed within seconds, to every device, by id only.
    let first = memo(&admin, &server, "Secret title");
    let seen = wait_for(&service, 2);
    let started = Instant::now();
    for r in &seen {
        assert_eq!(r.header("urgency"), Some("normal"));
        assert!(!String::from_utf8_lossy(&r.body).contains("Secret"));
    }
    let want = json!({ "event": "note.changed", "ids": [first] });
    assert_eq!(pushed(&service, &a, "/up/a"), std::slice::from_ref(&want));
    assert_eq!(pushed(&service, &b, "/up/b"), [want]);

    // A burst of changes, one of them the phone's own, sends each device one push once the
    // window has passed. The phone is not told about its own write.
    let second = memo(&admin, &server, "Two");
    let third = memo(&admin, &server, "Three");
    let own = memo(&client(&a.token), &server, "From the phone");
    thread::sleep(Duration::from_secs(5));
    assert_eq!(
        service.requests().len(),
        2,
        "nothing more inside the window"
    );
    wait_for(&service, 4);
    assert!(
        started.elapsed() >= Duration::from_secs(8),
        "the second push waited for the window"
    );
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        pushed(&service, &a, "/up/a")[1..],
        [json!({ "event": "note.changed", "ids": [second, third] })]
    );
    assert_eq!(
        pushed(&service, &b, "/up/b")[1..],
        [json!({ "event": "note.changed", "ids": [second, third, own] })]
    );

    // A change only the phone made goes to the other device alone.
    let alone = memo(&client(&a.token), &server, "Phone only");
    wait_for(&service, 5);
    thread::sleep(Duration::from_secs(2));
    assert_eq!(pushed(&service, &a, "/up/a").len(), 2);
    assert_eq!(
        pushed(&service, &b, "/up/b")[2..],
        [json!({ "event": "note.changed", "ids": [alone] })]
    );
}

/// The `device.settings` events on `/api/events`, as the device with `token` reads them, each as
/// its data, on a thread of their own.
fn settings_events(server: &TestServer, token: &str) -> mpsc::Receiver<Value> {
    let res = client(token).get(server.url("/api/events")).send().unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut event = String::new();
        for line in BufReader::new(res).lines() {
            let Ok(line) = line else { return };
            if line.is_empty() {
                event.clear();
            } else if let Some(v) = line.strip_prefix("event:") {
                event = v.trim().to_owned();
            } else if let Some(v) = line.strip_prefix("data:")
                && event == "device.settings"
            {
                let data = serde_json::from_str(v.trim()).unwrap_or(Value::Null);
                if tx.send(data).is_err() {
                    return;
                }
            }
        }
    });
    rx
}

/// Ticket 84: changing a device's settings in Settings > Devices tells that device at once, on
/// the live event stream and by one coalesced push to it alone; a rename alone tells nobody.
#[test]
fn a_settings_change_is_streamed_and_pushed_to_its_device_only() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let service = StandIn::start();
    service.route("/up/a", Reply::text(201, ""));
    service.route("/up/b", Reply::text(201, ""));
    let a = Phone::pair(&server, "Pixel", 8);
    let b = Phone::pair(&server, "Tablet", 9);
    a.register(&server, &service.url("/up/a"));
    b.register(&server, &service.url("/up/b"));
    let events = settings_events(&server, &a.token);

    let patch = |id: &str, body: Value| {
        send(
            admin
                .patch(server.url(&format!("/api/devices/{id}")))
                .json(&body),
            200,
        )
    };
    // A burst, as the Settings > Devices switches send one by one.
    patch(&a.id, json!({ "settings": { "contacts_write": true } }));
    patch(
        &a.id,
        json!({ "settings": { "commands": ["upsert_contact"] } }),
    );
    patch(&a.id, json!({ "settings": { "sms_send": true } }));
    patch(&b.id, json!({ "name": "Old tablet" }));

    for _ in 0..3 {
        let data = events
            .recv_timeout(Duration::from_secs(10))
            .expect("a device.settings event");
        assert_eq!(data, json!({ "device": a.id }));
    }
    assert!(
        events.recv_timeout(Duration::from_millis(500)).is_err(),
        "a rename alone is no settings change"
    );

    let seen = wait_for(&service, 1);
    assert_eq!(seen[0].path, "/up/a");
    assert_eq!(seen[0].header("urgency"), Some("normal"));
    assert_eq!(a.open(&seen[0].body), json!({ "event": "device.settings" }));
    thread::sleep(Duration::from_secs(2));
    assert_eq!(
        service.requests().len(),
        1,
        "one push for the burst, to A only"
    );
}
