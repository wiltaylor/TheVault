//! Public credential checks must be bounded before expensive verification.
use reqwest::blocking::Client;
use serde_json::json;
use testkit::TestServer;

#[test]
fn login_attempts_are_limited() {
    let server = TestServer::start();
    let client = Client::new();
    for _ in 0..5 {
        let response = client.post(server.url("/api/login"))
            .json(&json!({"password": "wrong"})).send().unwrap();
        assert_eq!(response.status(), 401);
    }
    let response = client.post(server.url("/api/login"))
        .header("X-Forwarded-For", "203.0.113.99")
        .json(&json!({"password": testkit::PASSWORD})).send().unwrap();
    assert_eq!(response.status(), 429, "even a valid password must be limited before hashing");
    assert!(response.headers().contains_key("retry-after"));
}
