//! The fake Google server's OAuth endpoints over HTTP: code exchange with PKCE, refresh, the
//! `invalid_grant` switch and userinfo (google-sync D1, D18).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::blocking::{Client, Response};
use serde_json::Value;
use sha2::{Digest, Sha256};
use testkit::FakeGoogle;
use testkit::fake_google::{DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN};

const VERIFIER: &str = "a-verifier-of-at-least-forty-three-characters-long-0123456789";
const REDIRECT: &str = "http://127.0.0.1:53682/";

fn post_form(url: &str, form: &[(&str, &str)]) -> (u16, Value) {
    let res = Client::new().post(url).form(form).send().unwrap();
    read(res)
}

fn read(res: Response) -> (u16, Value) {
    let status = res.status().as_u16();
    (status, res.json().unwrap_or(Value::Null))
}

fn refresh(google: &FakeGoogle, token: &str) -> (u16, Value) {
    post_form(
        google.endpoints()["token"].as_str().unwrap(),
        &[
            ("grant_type", "refresh_token"),
            ("client_id", DEFAULT_CLIENT_ID),
            ("client_secret", DEFAULT_CLIENT_SECRET),
            ("refresh_token", token),
        ],
    )
}

fn userinfo(google: &FakeGoogle, access: &str) -> (u16, Value) {
    let res = Client::new()
        .get(google.endpoints()["userinfo"].as_str().unwrap())
        .bearer_auth(access)
        .send()
        .unwrap();
    read(res)
}

fn auth_url() -> String {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={DEFAULT_CLIENT_ID}&redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2F&response_type=code&scope=openid%20email&state=st%2B1&code_challenge={challenge}&code_challenge_method=S256&access_type=offline&prompt=consent"
    )
}

/// The `code` and `state` of the URL consent sent the browser to.
fn code_and_state(back: &str) -> (String, String) {
    let url = reqwest::Url::parse(back).unwrap();
    let get = |name: &str| {
        url.query_pairs()
            .find_map(|(k, v)| (k == name).then(|| v.into_owned()))
            .unwrap()
    };
    (get("code"), get("state"))
}

#[test]
fn the_endpoints_point_at_the_fake_on_its_own_port() {
    let google = FakeGoogle::start();
    let other = FakeGoogle::start();
    assert_ne!(google.addr().port(), 0);
    assert_ne!(google.addr().port(), other.addr().port());
    assert_eq!(google.host(), "127.0.0.1");
    let endpoints = google.endpoints();
    let base = google.base_url();
    assert_eq!(endpoints["token"], format!("{base}/token"));
    assert_eq!(endpoints["userinfo"], format!("{base}/oauth2/v3/userinfo"));
    assert_eq!(endpoints["gmail"], format!("{base}/gmail/v1"));
    assert_eq!(endpoints["calendar"], format!("{base}/calendar/v3"));
}

#[test]
fn a_code_from_consent_exchanges_once_for_tokens_that_read_userinfo() {
    let google = FakeGoogle::builder().account("wil@example.com").start();
    let back = google.consent(&auth_url());
    assert!(back.starts_with(REDIRECT), "{back}");
    let (code, state) = code_and_state(&back);
    assert_eq!(state, "st+1");

    let exchange = |code: &str, verifier: &str| {
        post_form(
            &google.url("/token"),
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", REDIRECT),
                ("client_id", DEFAULT_CLIENT_ID),
                ("client_secret", DEFAULT_CLIENT_SECRET),
            ],
        )
    };

    // A wrong verifier fails, and spends the code.
    let (other, _) = code_and_state(&google.consent(&auth_url()));
    let (status, body) = exchange(&other, "not-the-verifier");
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_grant"))
    );
    assert_eq!(exchange(&other, VERIFIER).0, 400);

    let (status, body) = exchange(&code, VERIFIER);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["refresh_token"], DEFAULT_REFRESH_TOKEN);
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["scope"], "openid email");
    let access = body["access_token"].as_str().unwrap().to_owned();

    // Used once.
    let (status, body) = exchange(&code, VERIFIER);
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_grant"))
    );

    let (status, body) = userinfo(&google, &access);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email"], "wil@example.com");
    assert_eq!(body["email_verified"], true);
    assert_eq!(userinfo(&google, "made-up").0, 401);

    // The record holds every request, form bodies and headers included.
    let tokens = google.requests_to("/token");
    assert_eq!(tokens.len(), 4);
    assert!(tokens.iter().all(|r| r.method == "POST"));
    assert_eq!(tokens[2].form("code").as_deref(), Some(code.as_str()));
    assert_eq!(tokens[2].form("code_verifier").as_deref(), Some(VERIFIER));
    let info = google.requests_to("/oauth2/v3/userinfo");
    assert_eq!(
        info[0].header("authorization"),
        Some(format!("Bearer {access}").as_str())
    );
}

#[test]
fn a_refresh_issues_a_new_access_token_and_no_refresh_token() {
    let google = FakeGoogle::start();
    let (status, body) = refresh(&google, DEFAULT_REFRESH_TOKEN);
    assert_eq!(status, 200, "{body}");
    assert!(body.get("refresh_token").is_none(), "{body}");
    assert_eq!(body["expires_in"], 3599);
    let first = body["access_token"].as_str().unwrap().to_owned();
    let second = refresh(&google, DEFAULT_REFRESH_TOKEN).1["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(first, second);
    assert_eq!(google.access_tokens(), [first.clone(), second]);
    assert_eq!(userinfo(&google, &first).1["email"], "me@example.com");

    // A refresh token it never issued, a wrong client, and a body that is not a form.
    let (status, body) = refresh(&google, "someone-elses");
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_grant"))
    );
    let (status, body) = post_form(
        &google.url("/token"),
        &[
            ("grant_type", "refresh_token"),
            ("client_id", DEFAULT_CLIENT_ID),
            ("client_secret", "wrong"),
            ("refresh_token", DEFAULT_REFRESH_TOKEN),
        ],
    );
    assert_eq!(
        (status, body["error"].as_str()),
        (401, Some("invalid_client"))
    );
    let res = Client::new()
        .post(google.url("/token"))
        .json(&serde_json::json!({ "grant_type": "refresh_token" }))
        .send()
        .unwrap();
    let (status, body) = read(res);
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_request"))
    );
}

#[test]
fn the_invalid_grant_switch_refuses_refresh_and_exchange_until_it_is_off() {
    let google = FakeGoogle::builder()
        .client("my-id", "my-secret")
        .refresh_token("my-refresh")
        .invalid_grant(true)
        .start();
    let attempt = || {
        post_form(
            &google.url("/token"),
            &[
                ("grant_type", "refresh_token"),
                ("client_id", "my-id"),
                ("client_secret", "my-secret"),
                ("refresh_token", "my-refresh"),
            ],
        )
    };
    let (status, body) = attempt();
    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_grant");
    assert_eq!(
        body["error_description"],
        "Token has been expired or revoked."
    );

    let (code, _) = code_and_state(&google.consent(&auth_url()));
    let (status, body) = post_form(
        &google.url("/token"),
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", VERIFIER),
            ("redirect_uri", REDIRECT),
            ("client_id", "my-id"),
            ("client_secret", "my-secret"),
        ],
    );
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_grant"))
    );

    google.set_invalid_grant(false);
    let (status, body) = attempt();
    assert_eq!(status, 200, "{body}");
    google.set_invalid_grant(true);
    assert_eq!(attempt().1["error"], "invalid_grant");
    assert!(google.access_tokens().len() == 1);
}

#[test]
fn gmail_and_calendar_answer_404_until_their_tickets_add_them() {
    let google = FakeGoogle::start();
    let res = Client::new()
        .get(google.url("/gmail/v1/users/me/labels"))
        .send()
        .unwrap();
    let (status, body) = read(res);
    assert_eq!(status, 404);
    assert_eq!(body["error"]["code"], 404);
    assert_eq!(google.requests().len(), 1);
    google.clear_requests();
    assert!(google.requests().is_empty());
}

#[test]
fn the_consent_page_over_http_redirects_with_a_code_as_consent_does() {
    let google = FakeGoogle::start();
    let query = auth_url().split_once('?').unwrap().1.to_owned();
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let res = client
        .get(google.url(&format!("{}?{query}", testkit::fake_google::AUTH_PATH)))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 302);
    let back = res.headers()["location"].to_str().unwrap().to_owned();
    assert!(back.starts_with(REDIRECT), "{back}");
    let (code, state) = code_and_state(&back);
    assert_eq!(state, "st+1");
    let (status, body) = post_form(
        &google.url("/token"),
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", VERIFIER),
            ("redirect_uri", REDIRECT),
            ("client_id", DEFAULT_CLIENT_ID),
            ("client_secret", DEFAULT_CLIENT_SECRET),
        ],
    );
    assert_eq!(status, 200, "{body}");

    // Without a redirect there is nowhere to go.
    let res = client
        .get(google.url(testkit::fake_google::AUTH_PATH))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 400);
}
