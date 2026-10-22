//! The AI fakes keep the rules the `ai` crate's tests and the routes built on it rely on.

use reqwest::blocking::Client;
use serde_json::{Value, json};
use testkit::{FakeAnthropic, FakeOpenAi, FakeSiwc};

fn form(client: &Client, url: &str, fields: &[(&str, &str)]) -> (u16, Value) {
    let body = fields
        .iter()
        .map(|(k, v)| format!("{k}={}", v.replace(':', "%3A").replace('/', "%2F")))
        .collect::<Vec<_>>()
        .join("&");
    let resp = client
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .unwrap();
    (resp.status().as_u16(), resp.json().unwrap_or(Value::Null))
}

#[test]
fn siwc_refresh_tokens_rotate_and_a_reused_one_is_dead() {
    let auth = FakeSiwc::start();
    let client = Client::new();
    let url = format!("{}/api/accounts/oauth/token", auth.base_url());
    let first = auth.refresh_token();
    let refresh = |token: &str| {
        form(
            &client,
            &url,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &auth.client_id()),
                ("refresh_token", token),
                ("resource", "https://api.openai.com/v1"),
            ],
        )
    };
    let (status, body) = refresh(&first);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["expires_in"], json!(3600));
    let second = body["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(second, first);
    assert_eq!(auth.refresh_token(), second);
    let (status, body) = refresh(&first);
    assert_eq!(
        (status, body["error"].clone()),
        (400, json!("invalid_grant"))
    );
    auth.set_invalid_grant(true);
    let (status, _) = refresh(&second);
    assert_eq!(status, 400);
    assert_eq!(auth.refresh_count(), 1);
}

#[test]
fn siwc_publishes_its_configuration_and_keys() {
    let auth = FakeSiwc::start();
    let client = Client::new();
    let config: Value = client
        .get(format!(
            "{}/.well-known/openid-configuration",
            auth.base_url()
        ))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(config["issuer"], json!(auth.issuer()));
    let jwks: Value = client
        .get(config["jwks_uri"].as_str().unwrap())
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(jwks, auth.jwks());
    assert_eq!(jwks["keys"][0]["alg"], json!("RS256"));
    let token = auth.sign(&json!({"sub": "x"}));
    assert_eq!(token.split('.').count(), 3);
}

#[test]
fn openai_refuses_plan_requests_that_break_the_rules() {
    let fake = FakeOpenAi::start();
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/responses", fake.base_url()))
        .bearer_auth("a-plan-token")
        .json(&json!({"model": "m", "input": [], "store": false, "stream": true, "max_output_tokens": 5}))
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: Value = resp.json().unwrap();
    assert_eq!(
        body["error"]["code"],
        json!("subscription_sharing_unsupported_capability")
    );
    assert_eq!(body["error"]["param"], json!("max_output_tokens"));
    // The same body is fine with the API key.
    let resp = client
        .post(format!("{}/v1/responses", fake.base_url()))
        .bearer_auth(fake.api_key())
        .json(&json!({"model": "m", "input": [], "store": false, "stream": false, "max_output_tokens": 5}))
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

#[test]
fn anthropic_refuses_sampling_parameters() {
    let fake = FakeAnthropic::start();
    let resp = Client::new()
        .post(format!("{}/v1/messages", fake.base_url()))
        .header("x-api-key", fake.api_key())
        .header("anthropic-version", "2026-10-21")
        .json(&json!({"model": "m", "max_tokens": 5, "messages": [], "temperature": 0.2}))
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    assert_eq!(fake.message_requests().len(), 1);
}
