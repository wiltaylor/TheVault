//! S1: device pairing by password and by code, device token scope and revocation, the devices
//! API and its settings (D2, D3, D25).

use std::thread;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::{PASSWORD, TestServer};

const NOTE_TYPE: &str = r#"label = "Memo"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// A P-256 public key and auth secret, base64url, the shape a WebPush subscription carries.
const P256DH: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";

fn seed(vault: &std::path::Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), NOTE_TYPE).unwrap();
}

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn send(rb: RequestBuilder, status: u16) -> Value {
    expect(rb.send().unwrap(), status)
}

fn pair(server: &TestServer, body: Value) -> Response {
    Client::new()
        .post(server.url("/api/devices/pair"))
        .json(&body)
        .send()
        .unwrap()
}

fn pair_with_password(server: &TestServer, name: &str) -> (String, String) {
    let out = expect(
        pair(
            server,
            json!({ "password": PASSWORD, "name": name, "platform": "android", "capabilities": ["sms", "calls"] }),
        ),
        201,
    );
    (
        out["device_id"].as_str().unwrap().to_owned(),
        out["token"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn pairing_by_password_gives_a_write_token_bound_to_the_device() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());

    let wrong = pair(
        &server,
        json!({ "password": "nope", "name": "Pixel", "platform": "android" }),
    );
    assert_eq!(expect(wrong, 401)["error"], "unauthorized");
    let both = pair(
        &server,
        json!({ "password": PASSWORD, "code": "AAAA-BBBB", "name": "Pixel", "platform": "android" }),
    );
    expect(both, 400);
    let platform = pair(
        &server,
        json!({ "password": PASSWORD, "name": "Pixel", "platform": "windows" }),
    );
    expect(platform, 400);

    let (id, token) = pair_with_password(&server, "Pixel");
    assert!(token.starts_with("tv_"), "{token}");

    // The answer names the vault, as `GET /api/sync/info` does, for the device's pairing record
    // (ticket 143).
    let again = expect(
        pair(
            &server,
            json!({ "password": PASSWORD, "name": "Tablet", "platform": "android" }),
        ),
        201,
    );
    let info = send(Client::new().get(server.url("/api/sync/info")), 200);
    assert!(
        again["vault_id"].as_str().is_some_and(|v| !v.is_empty()),
        "{again}"
    );
    assert_eq!(again["vault_id"], info["vault_id"]);
    send(
        admin.delete(server.url(&format!(
            "/api/devices/{}",
            again["device_id"].as_str().unwrap()
        ))),
        204,
    );

    let list = send(admin.get(server.url("/api/devices")), 200);
    let devices = list["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    let d = &devices[0];
    assert_eq!(d["id"], id.as_str());
    assert_eq!(d["name"], "Pixel");
    assert_eq!(d["platform"], "android");
    assert_eq!(d["capabilities"], json!(["sms", "calls"]));
    assert_eq!(d["push"], false);
    assert_eq!(d["settings"]["sms_send"], false);
    assert_eq!(d["settings"]["contacts_write"], false);
    assert_eq!(d["settings"]["commands"], json!([]));
    assert!(
        d["settings"]["skip_pattern"]
            .as_str()
            .unwrap()
            .contains("verification")
    );
    assert!(d["last_seen"].is_null());

    // The token reaches write endpoints, and is the author of what it writes.
    let device = client(&token);
    let note = send(
        device
            .post(server.url("/api/notes?type=memo"))
            .json(&json!({ "note": { "fields": { "title": "From the phone" }, "blocks": [] } })),
        201,
    );
    let note_id = note["id"].as_str().unwrap();
    let history = send(
        admin.get(server.url(&format!("/api/notes/{note_id}/history"))),
        200,
    );
    let author = history.to_string();
    assert!(
        author.contains(&format!("device-{}", id.to_lowercase())),
        "{author}"
    );
    send(device.get(server.url("/api/devices")), 200);
    send(device.get(server.url("/api/notes")), 200);
    let seen = send(admin.get(server.url("/api/devices")), 200);
    assert!(seen["devices"][0]["last_seen"].is_i64(), "{seen}");

    // It does not reach admin endpoints.
    for rb in [
        device.get(server.url("/api/tokens")),
        device
            .put(server.url("/api/types/other"))
            .json(&json!({ "wcl": NOTE_TYPE })),
        device
            .patch(server.url(&format!("/api/devices/{id}")))
            .json(&json!({ "name": "Mine now" })),
        device.delete(server.url(&format!("/api/devices/{id}"))),
        device.post(server.url("/api/devices/pair-codes")),
    ] {
        assert_eq!(send(rb, 403)["error"], "forbidden");
    }
}

#[test]
fn a_pairing_code_works_once_and_expires() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());

    // The device side needs no credentials, and the admin side does.
    let anon = Client::new()
        .post(server.url("/api/devices/pair-codes"))
        .send()
        .unwrap();
    expect(anon, 401);

    let issued = send(admin.post(server.url("/api/devices/pair-codes")), 201);
    let code = issued["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 9, "{code}");
    let base = server.base_url();
    assert_eq!(issued["server_url"], base.as_str());
    assert_eq!(
        issued["payload"],
        json!({ "server_url": base, "code": code })
    );
    assert!(issued["qr_svg"].as_str().unwrap().contains("<svg"));
    assert!(issued["expires"].is_i64());

    let first = pair(
        &server,
        json!({ "code": code, "name": "Laptop", "platform": "linux" }),
    );
    let out = expect(first, 201);
    assert_eq!(out["device"]["platform"], "linux");
    let again = pair(
        &server,
        json!({ "code": code, "name": "Laptop 2", "platform": "linux" }),
    );
    assert_eq!(expect(again, 401)["error"], "unauthorized");

    // A server URL can be named, such as the address a phone reaches the server on.
    let named = send(
        admin
            .post(server.url("/api/devices/pair-codes"))
            .json(&json!({ "server_url": "https://vault.example/", "ttl_secs": 1 })),
        201,
    );
    assert_eq!(named["payload"]["server_url"], "https://vault.example");
    thread::sleep(Duration::from_millis(1_200));
    let late = pair(
        &server,
        json!({ "code": named["code"], "name": "Late", "platform": "android" }),
    );
    expect(late, 401);

    for bad in [
        json!({ "ttl_secs": 0 }),
        json!({ "ttl_secs": 100_000 }),
        json!({ "server_url": "ftp://x" }),
    ] {
        send(
            admin.post(server.url("/api/devices/pair-codes")).json(&bad),
            400,
        );
    }

    let list = send(admin.get(server.url("/api/devices")), 200);
    assert_eq!(list["devices"].as_array().unwrap().len(), 1);
}

#[test]
fn revoking_a_device_makes_its_token_fail() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());
    let (id, token) = pair_with_password(&server, "Pixel");
    let device = client(&token);
    send(
        device
            .put(server.url(&format!("/api/devices/{id}/push")))
            .json(
                &json!({ "endpoint": "https://push.example/up/1", "p256dh": P256DH, "auth": AUTH }),
            ),
        200,
    );

    send(admin.delete(server.url(&format!("/api/devices/{id}"))), 204);
    assert_eq!(
        send(device.get(server.url("/api/notes")), 401)["error"],
        "unauthorized"
    );
    let list = send(admin.get(server.url("/api/devices")), 200);
    assert_eq!(list["devices"], json!([]));
    let tokens = send(admin.get(server.url("/api/tokens")), 200).to_string();
    assert!(!tokens.contains("device-"), "{tokens}");
    send(admin.delete(server.url(&format!("/api/devices/{id}"))), 404);
}

#[test]
fn settings_round_trip() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());
    let (id, _) = pair_with_password(&server, "Pixel");
    let url = server.url(&format!("/api/devices/{id}"));

    let settings = json!({
        "commands": ["send_sms", "show_notification"],
        "contacts_write": true,
        "sms_send": true,
        "exclusions": [
            { "kind": "number", "value": "+61400000000" },
            { "kind": "contact", "value": "42" },
            { "kind": "thread", "value": "7" },
        ],
        "skip_pattern": "(?i)code",
    });
    let out = send(
        admin
            .patch(&url)
            .json(&json!({ "name": "Work phone", "settings": settings })),
        200,
    );
    assert_eq!(out["name"], "Work phone");
    assert_eq!(out["settings"], settings);
    let listed = send(admin.get(server.url("/api/devices")), 200);
    assert_eq!(listed["devices"][0]["settings"], settings);
    assert_eq!(listed["devices"][0]["name"], "Work phone");

    // Keys left out keep their value.
    let out = send(
        admin
            .patch(&url)
            .json(&json!({ "settings": { "sms_send": false } })),
        200,
    );
    assert_eq!(out["settings"]["sms_send"], false);
    assert_eq!(out["settings"]["contacts_write"], true);
    assert_eq!(out["settings"]["skip_pattern"], "(?i)code");
    assert_eq!(out["name"], "Work phone");

    for bad in [
        json!({ "settings": { "skip_pattern": "(" } }),
        json!({ "settings": { "colour": "red" } }),
        json!({ "settings": { "exclusions": [{ "kind": "email", "value": "a@b" }] } }),
        json!({ "name": "  " }),
    ] {
        send(admin.patch(&url).json(&bad), 400);
    }
    send(
        admin
            .patch(server.url("/api/devices/01J0000000000000000000NOPE"))
            .json(&json!({ "name": "x" })),
        404,
    );
}

#[test]
fn only_the_device_itself_sets_its_push_endpoint() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());
    let (a, token_a) = pair_with_password(&server, "Phone A");
    let (b, _) = pair_with_password(&server, "Phone B");
    let device_a = client(&token_a);
    let push = json!({ "endpoint": "https://push.example/up/a", "p256dh": P256DH, "auth": AUTH });

    let out = send(
        device_a
            .put(server.url(&format!("/api/devices/{a}/push")))
            .json(&push),
        200,
    );
    assert_eq!(out["push"], true);
    send(
        device_a
            .put(server.url(&format!("/api/devices/{b}/push")))
            .json(&push),
        403,
    );
    for bad in [
        json!({ "endpoint": "https://push.example/up/a" }),
        json!({ "endpoint": "not a url", "p256dh": P256DH, "auth": AUTH }),
        json!({ "endpoint": "https://push.example/up/a", "p256dh": "c2hvcnQ", "auth": AUTH }),
    ] {
        send(
            device_a
                .put(server.url(&format!("/api/devices/{a}/push")))
                .json(&bad),
            400,
        );
    }
    let cleared = send(
        device_a
            .put(server.url(&format!("/api/devices/{a}/push")))
            .json(&json!({ "endpoint": null })),
        200,
    );
    assert_eq!(cleared["push"], false);
    // The owner may set any device's endpoint.
    send(
        admin
            .put(server.url(&format!("/api/devices/{b}/push")))
            .json(&push),
        200,
    );
}

#[test]
fn a_phone_reports_its_capture_health_and_the_device_list_carries_it() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());
    let (a, token_a) = pair_with_password(&server, "Phone A");
    let (b, _) = pair_with_password(&server, "Phone B");
    let device_a = client(&token_a);
    let listed = |id: &str| -> Value {
        let list = send(admin.get(server.url("/api/devices")), 200);
        list["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .cloned()
            .unwrap()
    };
    // Before its first report a device has no health.
    assert_eq!(listed(&a)["health"], Value::Null);

    let health = json!({
        "problems": ["SMS: permission missing", "Battery optimisation is on"],
        "kinds": [{ "kind": "sms", "state": "permission_missing" }],
        "battery_optimised": true,
        "reported": 1,
    });
    let path = format!("/api/devices/{a}/health");
    let out = send(device_a.put(server.url(&path)).json(&health), 200);
    assert_eq!(out["health"]["problems"], health["problems"]);
    let shown = listed(&a);
    assert_eq!(shown["health"]["problems"], health["problems"]);
    assert_eq!(shown["health"]["kinds"], health["kinds"]);
    assert_eq!(shown["health"]["battery_optimised"], true);
    // The server says when it heard the report, whatever the phone put there.
    let reported = shown["health"]["reported"].as_i64().unwrap();
    assert!(reported > 1_700_000_000_000, "{shown}");
    assert_eq!(listed(&b)["health"], Value::Null);

    // A report with nothing wrong clears the problems.
    send(
        device_a
            .put(server.url(&path))
            .json(&json!({ "problems": [] })),
        200,
    );
    assert_eq!(listed(&a)["health"]["problems"], json!([]));

    // Only the device itself, or the owner, reports its health, and the shape is checked.
    send(
        device_a
            .put(server.url(&format!("/api/devices/{b}/health")))
            .json(&health),
        403,
    );
    for bad in [
        json!([]),
        json!({}),
        json!({ "problems": "SMS" }),
        json!({ "problems": [1] }),
        json!({ "problems": [""] }),
        json!({ "problems": ["x".repeat(301)] }),
        json!({ "problems": vec!["SMS"; 33] }),
        json!({ "problems": [], "padding": "x".repeat(17 * 1024) }),
    ] {
        send(device_a.put(server.url(&path)).json(&bad), 400);
    }
    send(
        admin
            .put(server.url(&format!("/api/devices/{b}/health")))
            .json(&json!({ "problems": [] })),
        200,
    );
    send(
        admin
            .put(server.url("/api/devices/01J0000000000000000000NOPE/health"))
            .json(&json!({ "problems": [] })),
        404,
    );
}
