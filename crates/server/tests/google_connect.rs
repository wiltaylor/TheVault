//! S1 with the fake Google server: Settings > Integrations > Google's connect routes, start,
//! finish, disconnect and the status read (google-sync D1, D18, D19).

use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Url, redirect};
use serde_json::{Value, json};
use testkit::{
    FakeGoogle, TestServer,
    fake_google::{DEFAULT_ACCOUNT, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET},
};

const SECRETS: [&str; 3] = [
    "google_client_id",
    "google_client_secret",
    "google_refresh_token",
];

const TASKS: [&str; 2] = ["google_gmail", "google_calendar"];

struct Api {
    server: TestServer,
    client: Client,
    google: FakeGoogle,
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn dict(name: &str, fields: Value) -> Value {
    json!({ "kind": "dict", "labels": [name], "fields": fields })
}

/// The query parameters of `url`.
fn params(url: &str) -> Vec<(String, String)> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn param(url: &str, name: &str) -> Option<String> {
    params(url)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

impl Api {
    fn start() -> Api {
        Api::with_google(FakeGoogle::start())
    }

    fn with_google(google: FakeGoogle) -> Api {
        let server = TestServer::start();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder()
            .default_headers(headers)
            .redirect(redirect::Policy::none())
            .build()
            .unwrap();
        Api {
            server,
            client,
            google,
        }
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.client.get(self.server.url(path))
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.client.post(self.server.url(path))
    }

    /// The `google_gmail` and `google_calendar` tasks, running a script that does nothing, with
    /// `paused` as given. Tickets 97 and 98 ship the real ones.
    fn tasks(&self, paused: bool) {
        expect(
            self.client
                .put(self.server.url("/api/scripts/google_noop"))
                .json(&json!({ "source": "fn run() { }\n" }))
                .send()
                .unwrap(),
            StatusCode::OK,
        );
        for name in TASKS {
            expect(
                self.client
                    .put(self.server.url(&format!("/api/tasks/{name}")))
                    .body(format!(
                        "schedule = \"*/5 * * * *\"\nscript = \"google_noop\"\npaused = {paused}\n"
                    ))
                    .send()
                    .unwrap(),
                StatusCode::OK,
            );
        }
    }

    fn paused(&self, task: &str) -> bool {
        let t = expect(
            self.get(&format!("/api/tasks/{task}")).send().unwrap(),
            StatusCode::OK,
        );
        t["paused"].as_bool().unwrap()
    }

    /// The settings note, with its endpoints at the fake.
    fn settings(&self) -> String {
        let made = expect(
            self.post("/api/notes?type=google_sync")
                .json(&json!({ "note": {
                    "fields": { "title": "Google" },
                    "blocks": [dict("endpoints", self.google.endpoints())],
                } }))
                .send()
                .unwrap(),
            StatusCode::CREATED,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn settings_notes(&self) -> Vec<Value> {
        let out = expect(
            self.get("/api/notes?type=google_sync").send().unwrap(),
            StatusCode::OK,
        );
        out["notes"].as_array().unwrap().clone()
    }

    fn fields(&self, id: &str) -> Value {
        let note = expect(
            self.get(&format!("/api/notes/{id}")).send().unwrap(),
            StatusCode::OK,
        );
        note["note"]["fields"].clone()
    }

    fn secrets(&self) -> Vec<String> {
        let out = expect(self.get("/api/secrets").send().unwrap(), StatusCode::OK);
        out["secrets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_owned())
            .filter(|n| n.starts_with("google_"))
            .collect()
    }

    fn connect_start(&self, redirect: &str) -> String {
        let out = expect(
            self.post("/api/integrations/google/start")
                .json(&json!({
                    "client_id": DEFAULT_CLIENT_ID,
                    "client_secret": DEFAULT_CLIENT_SECRET,
                    "redirect": redirect,
                }))
                .send()
                .unwrap(),
            StatusCode::OK,
        );
        out["auth_url"].as_str().unwrap().to_owned()
    }

    fn finish(&self, url: &str) -> Response {
        self.post("/api/integrations/google/finish")
            .json(&json!({ "url": url }))
            .send()
            .unwrap()
    }

    fn status(&self) -> Value {
        expect(
            self.get("/api/integrations/google").send().unwrap(),
            StatusCode::OK,
        )
    }

    fn runs(&self, task: &str) -> Vec<Value> {
        let out = expect(
            self.get(&format!("/api/tasks/{task}/runs")).send().unwrap(),
            StatusCode::OK,
        );
        out["runs"].as_array().unwrap().clone()
    }
}

#[test]
fn start_answers_googles_consent_url_with_offline_access_pkce_and_state() {
    let api = Api::start();
    let url = api.connect_start("loopback");

    let parsed = Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("accounts.google.com"));
    assert_eq!(parsed.path(), "/o/oauth2/v2/auth");
    assert_eq!(param(&url, "client_id").as_deref(), Some(DEFAULT_CLIENT_ID));
    assert_eq!(param(&url, "response_type").as_deref(), Some("code"));
    let scope = param(&url, "scope").unwrap();
    let scopes: Vec<&str> = scope.split(' ').collect();
    for s in [
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/calendar",
        "openid",
        "email",
    ] {
        assert!(scopes.contains(&s), "{s} missing from {scope}");
    }
    assert_eq!(param(&url, "access_type").as_deref(), Some("offline"));
    assert_eq!(param(&url, "prompt").as_deref(), Some("consent"));
    assert_eq!(
        param(&url, "redirect_uri").as_deref(),
        Some("http://127.0.0.1:53682/")
    );
    assert_eq!(
        param(&url, "code_challenge_method").as_deref(),
        Some("S256")
    );
    let challenge = param(&url, "code_challenge").unwrap();
    assert_eq!(challenge.len(), 43, "a base64url SHA-256: {challenge}");
    let state = param(&url, "state").unwrap();
    assert!(state.len() >= 32, "{state}");
    // Each start is its own flow.
    let again = api.connect_start("loopback");
    assert_ne!(param(&again, "state").unwrap(), state);
    assert_ne!(param(&again, "code_challenge").unwrap(), challenge);

    // The client id and secret are stored; the refresh token is not there yet.
    assert_eq!(
        api.secrets(),
        vec!["google_client_id", "google_client_secret"]
    );
    assert_eq!(api.status()["connected"], false);
}

#[test]
fn start_in_direct_mode_redirects_to_the_callback_route_on_the_pages_origin() {
    let api = Api::start();
    let out = expect(
        api.post("/api/integrations/google/start")
            .header("origin", "https://vault.example.ts.net")
            .json(&json!({
                "client_id": DEFAULT_CLIENT_ID,
                "client_secret": DEFAULT_CLIENT_SECRET,
                "redirect": "direct",
            }))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    let url = out["auth_url"].as_str().unwrap();
    assert_eq!(
        param(url, "redirect_uri").as_deref(),
        Some("https://vault.example.ts.net/integrations/google/callback")
    );

    // A redirect that is neither is refused.
    let res = api
        .post("/api/integrations/google/start")
        .json(&json!({ "client_id": "a", "client_secret": "b", "redirect": "popup" }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn finish_with_the_pasted_loopback_url_connects_and_unpauses_the_tasks() {
    let api = Api::start();
    api.tasks(true);
    let id = api.settings();
    let auth = api.connect_start("loopback");
    let pasted = api.google.consent(&auth);
    assert!(pasted.starts_with("http://127.0.0.1:53682/?"), "{pasted}");

    let out = expect(api.finish(&pasted), StatusCode::OK);
    assert_eq!(out["connected"], true);
    assert_eq!(out["account"], DEFAULT_ACCOUNT);

    // The code was exchanged with the verifier, at the fake's token endpoint.
    let exchanges: Vec<_> = api
        .google
        .requests_to("/token")
        .into_iter()
        .filter(|r| r.form("grant_type").as_deref() == Some("authorization_code"))
        .collect();
    assert_eq!(exchanges.len(), 1);
    assert!(exchanges[0].form("code_verifier").is_some());
    assert_eq!(api.google.requests_to("/oauth2/v3/userinfo").len(), 1);

    assert_eq!(api.secrets(), SECRETS);
    // The existing settings note holds the account, and is the only one.
    assert_eq!(api.fields(&id)["account"], DEFAULT_ACCOUNT);
    assert_eq!(api.settings_notes().len(), 1);
    for task in TASKS {
        assert!(!api.paused(task), "{task} is still paused");
    }
    // Each task runs once in the background.
    let deadline = Instant::now() + Duration::from_secs(30);
    for task in TASKS {
        while api.runs(task).is_empty() {
            assert!(Instant::now() < deadline, "{task} never ran");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    let status = api.status();
    assert_eq!(status["connected"], true);
    assert_eq!(status["account"], DEFAULT_ACCOUNT);
    assert_eq!(status["gmail"]["paused"], false);
    assert_eq!(status["calendar"]["paused"], false);

    // A code is good once: pasting the same URL again is refused.
    let res = api.finish(&pasted);
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn finish_creates_the_settings_note_when_there_is_none() {
    let api = Api::start();
    // The flow starts against the fake's endpoints; then the note goes, as if never made.
    let id = api.settings();
    let auth = api.connect_start("loopback");
    let version = expect(
        api.get(&format!("/api/notes/{id}")).send().unwrap(),
        StatusCode::OK,
    )["version"]
        .as_str()
        .unwrap()
        .to_owned();
    expect(
        api.client
            .delete(
                api.server
                    .url(&format!("/api/notes/{id}?base_version={version}")),
            )
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert!(api.settings_notes().is_empty());

    expect(api.finish(&api.google.consent(&auth)), StatusCode::OK);

    let notes = api.settings_notes();
    assert_eq!(notes.len(), 1);
    let made = notes[0]["id"].as_str().unwrap();
    assert_eq!(api.fields(made)["account"], DEFAULT_ACCOUNT);
    assert_eq!(api.secrets(), SECRETS);
    // No task exists yet, and that is not an error.
    assert_eq!(api.status()["gmail"]["paused"], Value::Null);
}

#[test]
fn finish_refuses_a_wrong_state() {
    let api = Api::start();
    api.tasks(true);
    api.settings();
    let auth = api.connect_start("loopback");
    let pasted = api.google.consent(&auth);
    let state = param(&pasted, "state").unwrap();
    let forged = pasted.replace(&state, "not-the-state");

    let body = expect(api.finish(&forged), StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("Connect Google again"),
        "{body}"
    );
    // Nothing was exchanged or stored, and the tasks stay paused.
    assert!(api.google.requests_to("/token").is_empty());
    assert_eq!(
        api.secrets(),
        vec!["google_client_id", "google_client_secret"]
    );
    for task in TASKS {
        assert!(api.paused(task));
    }

    // Text that is no URL at all is refused too.
    expect(api.finish("hello"), StatusCode::BAD_REQUEST);
    // A URL carrying Google's error is refused with it.
    let body = expect(
        api.finish(&format!(
            "http://127.0.0.1:53682/?error=access_denied&state={state}"
        )),
        StatusCode::BAD_REQUEST,
    );
    assert!(
        body["message"].as_str().unwrap().contains("access_denied"),
        "{body}"
    );
}

#[test]
fn finish_returns_googles_error_as_a_400() {
    let api = Api::with_google(FakeGoogle::builder().invalid_grant(true).start());
    api.settings();
    let auth = api.connect_start("loopback");
    let body = expect(
        api.finish(&api.google.consent(&auth)),
        StatusCode::BAD_REQUEST,
    );
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("invalid_grant"), "{message}");
    assert!(!api.secrets().contains(&"google_refresh_token".to_owned()));
}

#[test]
fn disconnect_deletes_the_secrets_pauses_the_tasks_and_clears_the_account() {
    let api = Api::start();
    api.tasks(true);
    let id = api.settings();
    let auth = api.connect_start("loopback");
    expect(api.finish(&api.google.consent(&auth)), StatusCode::OK);
    assert_eq!(api.secrets(), SECRETS);

    let out = expect(
        api.post("/api/integrations/google/disconnect")
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(out["connected"], false);
    assert!(api.secrets().is_empty());
    for task in TASKS {
        assert!(api.paused(task), "{task} is not paused");
    }
    assert_eq!(api.fields(&id)["account"], Value::Null);
    let status = api.status();
    assert_eq!(status["connected"], false);
    assert_eq!(status["account"], Value::Null);
    assert_eq!(status["gmail"]["paused"], true);

    // Disconnecting again, with nothing left to remove, is fine.
    expect(
        api.post("/api/integrations/google/disconnect")
            .send()
            .unwrap(),
        StatusCode::OK,
    );
}

#[test]
fn the_status_reports_each_sides_last_run_error_and_pause() {
    let google = FakeGoogle::start();
    let endpoints = google.endpoints();
    let server = TestServer::start_seeded(|vault| {
        // The shipped type, so the note is read at start.
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(
            vault.join("types/google_sync.wcl"),
            include_str!("../../writer/defaults/google_sync.wcl"),
        )
        .unwrap();
        let dir = vault.join("notes/google_sync");
        std::fs::create_dir_all(&dir).unwrap();
        let mut wcl = String::from(
            "title = \"Google\"\ncreated = \"2026-10-01T00:00:00Z\"\nupdated = \"2026-10-01T00:00:00Z\"\n\
             account = \"me@example.com\"\nlast_gmail_run = \"2026-10-18T09:00:00Z\"\n\
             last_gmail_error = \"Token has been expired or revoked.\"\n\
             last_calendar_run = \"2026-10-18T09:05:00Z\"\n\ndict endpoints {\n",
        );
        for (k, v) in endpoints.as_object().unwrap() {
            wcl.push_str(&format!("  {k} = {v}\n"));
        }
        wcl.push_str("}\n");
        std::fs::write(dir.join("01JABCDEFGHJKMNPQRSTVWXYZ0.wcl"), wcl).unwrap();
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
    );
    let client = Client::builder().default_headers(headers).build().unwrap();
    let api = Api {
        server,
        client,
        google,
    };
    api.tasks(false);

    let status = api.status();
    // The account is on the note, but with no refresh token the sync is not connected.
    assert_eq!(status["connected"], false);
    assert_eq!(status["account"], "me@example.com");
    assert_eq!(status["gmail"]["last_run"], "2026-10-18T09:00:00Z");
    assert_eq!(
        status["gmail"]["last_error"],
        "Token has been expired or revoked."
    );
    assert_eq!(status["gmail"]["paused"], false);
    assert_eq!(status["calendar"]["last_run"], "2026-10-18T09:05:00Z");
    assert_eq!(status["calendar"]["last_error"], Value::Null);
    assert_eq!(status["client_stored"], false);

    // The routes are the admin's: a read token may not even look.
    let read = expect(
        api.post("/api/tokens")
            .json(&json!({ "name": "reader", "scope": "read" }))
            .send()
            .unwrap(),
        StatusCode::CREATED,
    );
    let res = Client::new()
        .get(api.server.url("/api/integrations/google"))
        .bearer_auth(read["secret"].as_str().unwrap())
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
