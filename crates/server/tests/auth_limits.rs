//! Public credential checks must be bounded before expensive verification.
use reqwest::blocking::Client;
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    time::Duration,
};
use testkit::TestServer;

#[test]
fn login_attempts_are_limited() {
    let server = TestServer::start();
    let client = Client::new();
    for _ in 0..5 {
        let response = client
            .post(server.url("/api/login"))
            .json(&json!({"password": "wrong"}))
            .send()
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    let response = client
        .post(server.url("/api/login"))
        .header("X-Forwarded-For", "203.0.113.99")
        .header("CF-Connecting-IP", "203.0.113.99")
        .json(&json!({"password": testkit::PASSWORD}))
        .send()
        .unwrap();
    assert_eq!(
        response.status(),
        429,
        "even a valid password must be limited before hashing"
    );
    assert!(response.headers().contains_key("retry-after"));
}

fn check(
    server: &TestServer,
    client: &Client,
    path: &str,
    body: serde_json::Value,
    ip: &str,
) -> reqwest::blocking::Response {
    client
        .post(server.url(path))
        .header("X-Forwarded-For", ip)
        .json(&body)
        .send()
        .unwrap()
}

#[test]
fn all_three_endpoints_share_failures_and_notify_once() {
    let server = TestServer::start();
    let client = Client::new();
    for _ in 0..2 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/login",
                json!({"password":"wrong"}),
                "198.51.100.1"
            )
            .status(),
            401
        );
        assert_eq!(
            check(
                &server,
                &client,
                "/api/devices/pair",
                json!({"password":"wrong", "name":"Phone", "platform":"android"}),
                "198.51.100.2"
            )
            .status(),
            401
        );
    }
    assert_eq!(
        check(
            &server,
            &client,
            "/api/agent-codes/redeem",
            json!({"code":"0000000000"}),
            "198.51.100.3"
        )
        .status(),
        410
    );
    for path in ["/api/login", "/api/devices/pair", "/api/agent-codes/redeem"] {
        assert_eq!(
            check(&server, &client, path, json!({}), "198.51.100.4").status(),
            429
        );
    }
    let notifications: serde_json::Value = client
        .get(server.url("/api/notifications"))
        .bearer_auth(server.admin_token())
        .send()
        .unwrap()
        .json()
        .unwrap();
    let items = notifications["items"].as_array().unwrap();
    assert_eq!(
        items
            .iter()
            .filter(|n| n["source"] == "server:auth")
            .count(),
        1,
        "{notifications}"
    );
}

#[test]
fn trusted_caddy_uses_last_hop_and_success_resets_failures() {
    let server =
        TestServer::start_with_env(|_| {}, &[("VAULT_TRUSTED_PROXIES", "caddy=127.0.0.1")]);
    let client = Client::new();
    for _ in 0..4 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/login",
                json!({"password":"wrong"}),
                "spoofed, 198.51.100.1"
            )
            .status(),
            401
        );
    }
    assert_eq!(
        check(
            &server,
            &client,
            "/api/login",
            json!({"password":testkit::PASSWORD}),
            "198.51.100.1"
        )
        .status(),
        200
    );
    // Successes do not consume either budget.
    for _ in 0..6 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/login",
                json!({"password":testkit::PASSWORD}),
                "198.51.100.1"
            )
            .status(),
            200
        );
    }
    for _ in 0..5 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/login",
                json!({"password":"wrong"}),
                "spoofed, 198.51.100.1"
            )
            .status(),
            401
        );
    }
    assert_eq!(
        check(
            &server,
            &client,
            "/api/login",
            json!({"password":testkit::PASSWORD}),
            "different-spoof, 198.51.100.1"
        )
        .status(),
        429
    );
    assert_eq!(
        check(
            &server,
            &client,
            "/api/login",
            json!({"password":testkit::PASSWORD}),
            "198.51.100.2"
        )
        .status(),
        200
    );
}

#[test]
fn cloudflare_and_global_budget_keep_a_clean_address_available() {
    let server =
        TestServer::start_with_env(|_| {}, &[("VAULT_TRUSTED_PROXIES", "cloudflare=127.0.0.1")]);
    let client = Client::new();
    for i in 1..=50 {
        let response = client
            .post(server.url("/api/agent-codes/redeem"))
            .header("CF-Connecting-IP", format!("198.51.100.{i}"))
            .header("X-Forwarded-For", "203.0.113.1")
            .json(&json!({"code":"0000000000"}))
            .send()
            .unwrap();
        assert_eq!(response.status(), 410);
    }
    let attempt = |ip: &str| {
        client
            .post(server.url("/api/login"))
            .header("CF-Connecting-IP", ip)
            .json(&json!({"password":testkit::PASSWORD}))
            .send()
            .unwrap()
    };
    assert_eq!(attempt("198.51.100.1").status(), 429);
    assert_eq!(attempt("198.51.100.200").status(), 200);
    let notifications: serde_json::Value = client
        .get(server.url("/api/notifications"))
        .bearer_auth(server.admin_token())
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(
        notifications["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["source"] == "server:auth")
            .count(),
        1
    );
}

#[test]
fn successful_pairing_resets_password_failures() {
    let server = TestServer::start();
    let client = Client::new();
    for _ in 0..4 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/login",
                json!({"password":"wrong"}),
                ""
            )
            .status(),
            401
        );
    }
    assert_eq!(
        check(
            &server,
            &client,
            "/api/devices/pair",
            json!({"password":testkit::PASSWORD, "name":"Phone", "platform":"android"}),
            ""
        )
        .status(),
        201
    );
    for _ in 0..4 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/login",
                json!({"password":"wrong"}),
                ""
            )
            .status(),
            401
        );
    }
    assert_eq!(
        check(
            &server,
            &client,
            "/api/login",
            json!({"password":testkit::PASSWORD}),
            ""
        )
        .status(),
        200
    );
}

#[test]
fn pairing_code_failures_are_also_limited() {
    let server = TestServer::start();
    let client = Client::new();
    for _ in 0..5 {
        assert_eq!(
            check(
                &server,
                &client,
                "/api/devices/pair",
                json!({"code":"bad", "name":"Phone", "platform":"android"}),
                "198.51.100.1"
            )
            .status(),
            401
        );
    }
    assert_eq!(
        check(
            &server,
            &client,
            "/api/devices/pair",
            json!({"password":testkit::PASSWORD, "name":"Phone", "platform":"android"}),
            "198.51.100.2"
        )
        .status(),
        429
    );
}

#[test]
fn ipv6_privacy_addresses_share_a_64_but_not_the_owners_prefix() {
    let server =
        TestServer::start_with_env(|_| {}, &[("VAULT_TRUSTED_PROXIES", "cloudflare=127.0.0.1")]);
    let client = Client::new();
    for i in 1..=55 {
        let response = client
            .post(server.url("/api/agent-codes/redeem"))
            .header("CF-Connecting-IP", format!("2001:db8:1:2::{i:x}"))
            .json(&json!({"code":"0000000000"}))
            .send()
            .unwrap();
        assert_eq!(response.status(), if i <= 5 { 410 } else { 429 });
    }
    assert_eq!(
        client
            .post(server.url("/api/login"))
            .header("CF-Connecting-IP", "2001:db8:1:3::1")
            .json(&json!({"password":testkit::PASSWORD}))
            .send()
            .unwrap()
            .status(),
        200
    );
}

#[test]
fn incomplete_body_neither_holds_the_shared_key_nor_waits_forever() {
    let server = TestServer::start();
    let url = reqwest::Url::parse(&server.url("/api/login")).unwrap();
    let mut slow = TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(slow, "POST /api/login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n").unwrap();
    let mut reader = BufReader::new(slow.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("100 Continue"), "{line}");
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "\r\n");
    slow.write_all(b"{").unwrap();
    // The slow request is now being read. Its address must not be reserved or locked.
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    assert_eq!(
        client
            .post(server.url("/api/login"))
            .json(&json!({"password":testkit::PASSWORD}))
            .send()
            .unwrap()
            .status(),
        200
    );
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("408 Request Timeout"), "{line}");
}

#[test]
fn concurrent_wrong_passwords_cannot_bypass_the_failure_budget() {
    let server = TestServer::start();
    let client = Client::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(12));
    let statuses = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let barrier = barrier.clone();
                let url = server.url("/api/login");
                let client = client.clone();
                scope.spawn(move || {
                    barrier.wait();
                    client
                        .post(url)
                        .json(&json!({"password":"wrong"}))
                        .send()
                        .unwrap()
                        .status()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        statuses
            .iter()
            .all(|s| s.as_u16() == 401 || s.as_u16() == 429)
    );
    assert_eq!(statuses.iter().filter(|s| s.as_u16() == 401).count(), 5);
    assert_eq!(
        client
            .post(server.url("/api/login"))
            .json(&json!({"password":testkit::PASSWORD}))
            .send()
            .unwrap()
            .status(),
        429
    );
}
