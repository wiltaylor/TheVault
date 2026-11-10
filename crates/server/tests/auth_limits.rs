//! Public credential checks must be bounded before expensive verification.
use reqwest::blocking::Client;
use serde_json::json;
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
