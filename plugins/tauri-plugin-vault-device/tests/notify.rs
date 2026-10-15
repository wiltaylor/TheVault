//! The phone's side of notification mirroring against a real home server (D4, D9): it registers
//! its UnifiedPush endpoint, decrypts what the server pushes there, reads the unread list, and
//! marks a notification read the way a swipe does.

use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use tauri_plugin_vault_device::core::Link;
use tauri_plugin_vault_device::notify::{self, PushTarget};
use tauri_plugin_vault_device::pair::{self, Secret};
use testkit::{PASSWORD, Reply, StandIn, TestServer};

fn admin(server: &TestServer, method: &str, path: &str, body: Option<Value>) -> Value {
    let rb = Client::new()
        .request(method.parse().unwrap(), server.url(path))
        .bearer_auth(server.admin_token());
    let rb = match body {
        Some(b) => rb.json(&b),
        None => rb,
    };
    let res = rb.send().unwrap();
    let status = res.status();
    let body: Value = res.json().unwrap_or(Value::Null);
    assert!(status.is_success(), "{method} {path}: {status} {body}");
    body
}

fn paired(server: &TestServer) -> Link {
    let p = pair::pair(
        &server.base_url(),
        &Secret::Password(PASSWORD.to_owned()),
        "Pixel",
    )
    .unwrap();
    Link {
        server_url: p.server_url,
        token: p.token,
        device_id: p.device_id,
        device_name: p.device_name,
        cache_mb: 64,
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
            "only {} of {n} pushes",
            seen.len()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn push_flag(server: &TestServer, device: &str) -> Value {
    let devices = admin(server, "GET", "/api/devices", None);
    devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == device)
        .unwrap()["push"]
        .clone()
}

#[test]
fn a_pushed_notification_decrypts_is_listed_unread_and_a_swipe_marks_it_read() {
    let server = TestServer::start();
    let service = StandIn::start();
    service.route("/up/pixel", Reply::text(201, ""));
    let link = paired(&server);

    // The connector's keys: a P-256 key pair and a 16-byte auth secret.
    let secret = SecretKey::from_slice(&[7; 32]).unwrap();
    let auth = [9u8; 16];
    let target = PushTarget {
        endpoint: service.url("/up/pixel"),
        p256dh: URL_SAFE_NO_PAD.encode(secret.public_key().to_encoded_point(false)),
        auth: URL_SAFE_NO_PAD.encode(auth),
    };
    notify::set_push(&link, Some(&target)).unwrap();
    assert_eq!(push_flag(&server, &link.device_id), true);

    let created = admin(
        &server,
        "POST",
        "/api/notifications",
        Some(json!({ "title": "Build finished", "body": "All green" })),
    );
    let id = created["id"].as_str().unwrap();
    let pushed = wait_for(&service, 1);
    let plain = push::webpush::decrypt(&secret, &auth, &pushed[0].body).unwrap();
    let message: Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(
        message,
        json!({ "event": "notification.created", "id": id })
    );

    // The push carries the id only; the list carries what the phone shows.
    let unread = notify::unread(&link).unwrap();
    assert_eq!(unread.len(), 1, "{unread:?}");
    assert_eq!(unread[0]["id"], id);
    assert_eq!(unread[0]["title"], "Build finished");
    assert_eq!(unread[0]["body"], "All green");

    // A swipe marks it read: the list empties and every device is told.
    assert!(notify::mark_read(&link, id).unwrap());
    assert!(notify::unread(&link).unwrap().is_empty());
    let pushed = wait_for(&service, 2);
    let plain = push::webpush::decrypt(&secret, &auth, &pushed[1].body).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&plain).unwrap(),
        json!({ "event": "notification.read", "id": id })
    );

    // Gone from the server meanwhile: nothing left to mark, and not an error.
    admin(&server, "DELETE", &format!("/api/notifications/{id}"), None);
    assert!(!notify::mark_read(&link, id).unwrap());

    notify::set_push(&link, None).unwrap();
    assert_eq!(push_flag(&server, &link.device_id), false);
}

#[test]
fn a_revoked_device_is_refused_and_an_unreachable_server_says_so() {
    let server = TestServer::start();
    let link = paired(&server);
    admin(
        &server,
        "DELETE",
        &format!("/api/devices/{}", link.device_id),
        None,
    );
    let refused = notify::unread(&link).unwrap_err().to_string();
    assert!(refused.contains("401"), "{refused}");

    let gone = Link {
        server_url: "http://127.0.0.1:9".into(),
        ..link
    };
    let unreachable = notify::mark_read(&gone, "01J9Z3K8Q2W6E4R5T7Y8U9I0OP")
        .unwrap_err()
        .to_string();
    assert!(unreachable.contains("cannot reach"), "{unreachable}");
}
