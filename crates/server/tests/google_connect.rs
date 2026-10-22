//! S1 with the fake Google server: Settings > Integrations > Google's connect routes, start,
//! finish, disconnect, the settings and the status read (google-sync D1, D18, D19, D21).

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

/// The Google secrets a connection leaves, by name.
const SECRETS: [&str; 5] = [
    "google_account",
    "google_client_id",
    "google_client_secret",
    "google_refresh_token",
    "google_settings",
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

    /// The `google_settings` secret, pointing at the fake.
    fn endpoints(&self) {
        self.put_secret(
            "google_settings",
            &json!({ "endpoints": self.google.endpoints() }).to_string(),
        );
    }

    fn put_secret(&self, name: &str, value: &str) {
        expect(
            self.client
                .put(self.server.url(&format!("/api/secrets/{name}")))
                .json(&json!({ "value": value }))
                .send()
                .unwrap(),
            StatusCode::OK,
        );
    }

    fn put_settings(&self, settings: Value) -> Response {
        self.client
            .put(self.server.url("/api/integrations/google/settings"))
            .json(&settings)
            .send()
            .unwrap()
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
    api.endpoints();
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
    // The account is a secret, and the settings keep the endpoints. No note holds either.
    assert_eq!(
        api.server.secret("google_account").as_deref(),
        Some(DEFAULT_ACCOUNT)
    );
    let settings = api.server.secret_json("google_settings");
    assert_eq!(settings["endpoints"], api.google.endpoints(), "{settings}");
    assert_eq!(settings["gmail_mode"], "labels", "{settings}");
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
fn finish_writes_the_settings_keeping_the_endpoints_the_start_used() {
    let api = Api::start();
    // The flow starts against the fake's endpoints; then the settings go, as if never saved.
    api.endpoints();
    let auth = api.connect_start("loopback");
    expect(
        api.client
            .delete(api.server.url("/api/secrets/google_settings"))
            .send()
            .unwrap(),
        StatusCode::OK,
    );

    expect(api.finish(&api.google.consent(&auth)), StatusCode::OK);

    let settings = api.server.secret_json("google_settings");
    assert_eq!(
        settings,
        json!({
            "gmail_mode": "labels",
            "gmail_labels": [],
            "gmail_window_days": 90,
            "gmail_backfill_to": null,
            "calendars": ["primary"],
            "default_calendar": "primary",
            "endpoints": api.google.endpoints(),
        })
    );
    assert_eq!(api.secrets(), SECRETS);
    // The shipped Google tasks are unpaused.
    let status = api.status();
    assert_eq!(status["gmail"]["paused"], false, "{status}");
    assert_eq!(status["calendar"]["paused"], false, "{status}");
    // The status never shows the test seam.
    assert!(status["settings"].get("endpoints").is_none(), "{status}");
    assert_eq!(status["settings"]["calendars"], json!(["primary"]));
}

#[test]
fn finish_refuses_a_wrong_state() {
    let api = Api::start();
    api.tasks(true);
    api.endpoints();
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
        vec![
            "google_client_id",
            "google_client_secret",
            "google_settings"
        ]
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
    api.endpoints();
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
fn disconnect_deletes_the_sign_in_and_account_keeps_the_settings_and_clears_the_cursors() {
    let api = Api::start();
    api.tasks(true);
    api.endpoints();
    let auth = api.connect_start("loopback");
    expect(api.finish(&api.google.consent(&auth)), StatusCode::OK);
    assert_eq!(api.secrets(), SECRETS);
    let gmail = json!({
        "history_id": "42", "backfill_page": "p2", "last_run": "2026-10-18T09:00:00Z",
        "last_error": null, "notified_invalid_grant": false,
    });
    let calendar = json!({
        "sync_tokens": { "primary": "tok" }, "window_end": "2027-10-18",
        "last_run": "2026-10-18T09:05:00Z", "last_error": null, "notified_invalid_grant": false,
    });
    api.put_secret("google_gmail_state", &gmail.to_string());
    api.put_secret("google_calendar_state", &calendar.to_string());
    let settings = api.server.secret("google_settings");

    let out = expect(
        api.post("/api/integrations/google/disconnect")
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(out["connected"], false);
    assert_eq!(
        api.secrets(),
        [
            "google_calendar_state",
            "google_gmail_state",
            "google_settings"
        ]
    );
    assert_eq!(api.server.secret("google_settings"), settings);
    assert_eq!(
        api.server.secret_json("google_gmail_state"),
        json!({
            "history_id": null, "backfill_page": null, "last_run": "2026-10-18T09:00:00Z",
            "last_error": null, "notified_invalid_grant": false,
        })
    );
    assert_eq!(
        api.server.secret_json("google_calendar_state"),
        json!({
            "sync_tokens": {}, "window_end": null,
            "last_run": "2026-10-18T09:05:00Z", "last_error": null, "notified_invalid_grant": false,
        })
    );
    for task in TASKS {
        assert!(api.paused(task), "{task} is not paused");
    }
    let status = api.status();
    assert_eq!(status["connected"], false);
    assert_eq!(status["account"], Value::Null);
    assert_eq!(status["gmail"]["paused"], true);
    assert_eq!(status["gmail"]["last_run"], "2026-10-18T09:00:00Z");

    // Disconnecting again, with nothing left to remove, is fine.
    expect(
        api.post("/api/integrations/google/disconnect")
            .send()
            .unwrap(),
        StatusCode::OK,
    );
}

#[test]
fn the_status_reports_the_account_settings_and_each_sides_last_run_error_and_pause() {
    let api = Api::start();
    api.tasks(false);
    api.put_secret("google_account", "me@example.com");
    api.put_secret(
        "google_settings",
        &json!({ "gmail_mode": "all", "calendars": ["primary", "team"], "endpoints": api.google.endpoints() })
            .to_string(),
    );
    api.put_secret(
        "google_gmail_state",
        &json!({ "last_run": "2026-10-18T09:00:00Z", "last_error": "Token has been expired or revoked." })
            .to_string(),
    );
    api.put_secret(
        "google_calendar_state",
        &json!({ "last_run": "2026-10-18T09:05:00Z", "last_error": null }).to_string(),
    );

    let status = api.status();
    // The account is stored, but with no refresh token the sync is not connected.
    assert_eq!(status["connected"], false);
    assert_eq!(status["account"], "me@example.com");
    // The stored settings over the defaults, without the endpoints.
    assert_eq!(
        status["settings"],
        json!({
            "gmail_mode": "all",
            "gmail_labels": [],
            "gmail_window_days": 90,
            "gmail_backfill_to": null,
            "calendars": ["primary", "team"],
            "default_calendar": "primary",
        })
    );
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
    for res in [
        Client::new()
            .get(api.server.url("/api/integrations/google"))
            .bearer_auth(read["secret"].as_str().unwrap())
            .send()
            .unwrap(),
        Client::new()
            .put(api.server.url("/api/integrations/google/settings"))
            .bearer_auth(read["secret"].as_str().unwrap())
            .json(&json!({}))
            .send()
            .unwrap(),
    ] {
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
}

#[test]
fn put_settings_validates_and_saves_keeping_the_endpoints() {
    let api = Api::start();
    api.endpoints();
    let good = json!({
        "gmail_mode": "labels",
        "gmail_labels": [" Work ", "INBOX", "", "Work"],
        "gmail_window_days": 30,
        "gmail_backfill_to": "2025-01-31",
        "calendars": ["primary", " team@group.calendar.google.com "],
        "default_calendar": "team@group.calendar.google.com",
    });
    let out = expect(api.put_settings(good.clone()), StatusCode::OK);
    let saved = json!({
        "gmail_mode": "labels",
        "gmail_labels": ["Work", "INBOX"],
        "gmail_window_days": 30,
        "gmail_backfill_to": "2025-01-31",
        "calendars": ["primary", "team@group.calendar.google.com"],
        "default_calendar": "team@group.calendar.google.com",
    });
    assert_eq!(out["settings"], saved);
    let mut stored = saved.clone();
    stored["endpoints"] = api.google.endpoints();
    assert_eq!(api.server.secret_json("google_settings"), stored);

    // An empty back-fill date is none, and an empty default calendar is the first listed.
    let mut cleared = good.clone();
    cleared["gmail_backfill_to"] = json!("");
    cleared["default_calendar"] = json!("");
    let out = expect(api.put_settings(cleared), StatusCode::OK);
    assert_eq!(out["settings"]["gmail_backfill_to"], Value::Null);
    assert_eq!(out["settings"]["default_calendar"], "primary");

    for (key, value, says) in [
        ("gmail_mode", json!("some"), "labels or all"),
        ("gmail_window_days", json!(0), "1 to 3650"),
        ("gmail_window_days", json!(3651), "1 to 3650"),
        ("gmail_backfill_to", json!("2025-02-30"), "YYYY-MM-DD"),
        ("gmail_backfill_to", json!("31/01/2025"), "YYYY-MM-DD"),
        ("calendars", json!([]), "at least one calendar"),
        ("calendars", json!(["  "]), "at least one calendar"),
    ] {
        let mut bad = good.clone();
        bad[key] = value.clone();
        let body = expect(api.put_settings(bad), StatusCode::BAD_REQUEST);
        assert!(
            body["message"].as_str().unwrap().contains(says),
            "{key} = {value}: {body}"
        );
    }
    // A field it does not know is refused too, so endpoints cannot be set this way.
    let mut bad = good.clone();
    bad["endpoints"] = json!({ "token": "http://example.com" });
    let res = api.put_settings(bad);
    assert!(res.status().is_client_error(), "{}", res.status());
    // Nothing refused changed what was saved.
    assert_eq!(
        api.server.secret_json("google_settings")["gmail_window_days"],
        30
    );
    assert_eq!(
        api.server.secret_json("google_settings")["endpoints"],
        api.google.endpoints()
    );
}

impl Api {
    /// The shipped `google_gmail` script as the `google_gmail` task, reaching the fake, and the
    /// do-nothing `google_calendar` task, both paused.
    fn slow_gmail_tasks(&self) {
        self.tasks(true);
        let task = format!(
            r#"label         = "Gmail sync"
schedule      = "*/10 * * * *"
script        = "google_gmail"
timeout       = "PT5M"
retries       = 0
allow_domains = ["{host}"]
secrets       = ["google_client_id", "google_client_secret", "google_refresh_token", "google_settings", "google_gmail_state"]
paused        = true
"#,
            host = self.google.host()
        );
        expect(
            self.client
                .put(self.server.url("/api/tasks/google_gmail"))
                .body(task)
                .send()
                .unwrap(),
            StatusCode::OK,
        );
    }

    /// Wait until `side`'s status reports `running` as `want`.
    fn wait_running(&self, side: &str, want: bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let status = self.status();
            if status[side]["running"] == want {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "{side} running never became {want}: {status:#}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[test]
fn finish_answers_at_once_while_the_first_gmail_run_is_slow() {
    let api = Api::with_google(
        FakeGoogle::builder()
            .gmail_delay(Duration::from_secs(2))
            .start(),
    );
    api.slow_gmail_tasks();
    api.endpoints();
    let auth = api.connect_start("loopback");
    let pasted = api.google.consent(&auth);

    let began = Instant::now();
    let out = expect(api.finish(&pasted), StatusCode::OK);
    let took = began.elapsed();
    assert!(took < Duration::from_secs(1), "finish took {took:?}");
    assert_eq!(out["connected"], true);
    // The first runs started before the answer, so the card shows both sides syncing.
    assert_eq!(out["gmail"]["running"], true, "{out:#}");
    assert_eq!(out["gmail"]["paused"], false);
    assert_eq!(out["calendar"]["paused"], false);

    // While the run waits on Gmail, the status and the other routes answer at once.
    let began = Instant::now();
    let status = api.status();
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "status took {:?}",
        began.elapsed()
    );
    assert_eq!(status["gmail"]["running"], true, "{status:#}");
    let began = Instant::now();
    expect(api.get("/api/tasks").send().unwrap(), StatusCode::OK);
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "tasks took {:?}",
        began.elapsed()
    );

    api.google.set_gmail_delay(None);
    let status = api.wait_running("gmail", false);
    assert_eq!(status["gmail"]["last_error"], Value::Null, "{status:#}");
    let summary = status["gmail"]["summary"].as_str().unwrap_or_default();
    assert!(summary.starts_with("Gmail: 0 new"), "{status:#}");
    // The time is RFC 3339 in UTC, ending Z, whichever script wrote it.
    let last = status["gmail"]["last_run"].as_str().unwrap();
    assert!(last.ends_with('Z') && last.len() == 20, "{last}");
}

#[test]
fn the_status_reports_running_per_side_from_the_scheduler() {
    let api = Api::with_google(
        FakeGoogle::builder()
            .gmail_delay(Duration::from_millis(500))
            .start(),
    );
    api.slow_gmail_tasks();
    api.endpoints();
    let auth = api.connect_start("loopback");
    expect(api.finish(&api.google.consent(&auth)), StatusCode::OK);
    let status = api.wait_running("gmail", false);
    assert_eq!(status["calendar"]["running"], false, "{status:#}");

    // A run started by hand shows as running until it ends.
    let client = api.client.clone();
    let url = api.server.url("/api/tasks/google_gmail/run?wait=true");
    let run = std::thread::spawn(move || client.post(url).send().unwrap().status());
    let status = api.wait_running("gmail", true);
    assert_eq!(status["calendar"]["running"], false, "{status:#}");
    assert_eq!(run.join().unwrap(), StatusCode::OK);
    let status = api.status();
    assert_eq!(status["gmail"]["running"], false, "{status:#}");

    // Without `wait`, Run now answers 202 once the run starts, with the run still running, and
    // the run is read by its id until it ends (ticket 115).
    let res = api
        .client
        .post(api.server.url("/api/tasks/google_gmail/run"))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let started: Value = res.json().unwrap();
    assert_eq!(started["outcome"], "running", "{started:#}");
    assert_eq!(started["ended"], Value::Null, "{started:#}");
    assert_eq!(started["task"], "google_gmail");
    let status = api.status();
    assert_eq!(status["gmail"]["running"], true, "{status:#}");
    // A second Run now while it goes is refused.
    let again = api
        .client
        .post(api.server.url("/api/tasks/google_gmail/run"))
        .send()
        .unwrap();
    assert_eq!(again.status(), StatusCode::CONFLICT);
    let url = api
        .server
        .url(&format!("/api/tasks/google_gmail/runs/{}", started["id"]));
    let deadline = Instant::now() + Duration::from_secs(60);
    let ended = loop {
        let run = expect(api.client.get(url.clone()).send().unwrap(), StatusCode::OK);
        if run["outcome"] != "running" {
            break run;
        }
        assert!(Instant::now() < deadline, "the run never ended: {run:#}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(ended["outcome"], "succeeded", "{ended:#}");
    assert!(ended["ended"].is_i64(), "{ended:#}");
    api.wait_running("gmail", false);

    // With no task, the side is not running.
    expect(
        api.client
            .delete(api.server.url("/api/tasks/google_calendar"))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    let status = api.status();
    assert_eq!(status["calendar"]["installed"], false);
    assert_eq!(status["calendar"]["running"], false);
}

#[test]
fn the_status_writes_every_last_run_as_utc_ending_z() {
    let api = Api::start();
    api.tasks(false);
    api.put_secret(
        "google_gmail_state",
        &json!({ "last_run": "2026-10-18T09:00:00+00:00" }).to_string(),
    );
    api.put_secret(
        "google_calendar_state",
        &json!({ "last_run": "2026-10-18T19:05:00+10:00" }).to_string(),
    );
    let status = api.status();
    assert_eq!(status["gmail"]["last_run"], "2026-10-18T09:00:00Z");
    assert_eq!(status["calendar"]["last_run"], "2026-10-18T09:05:00Z");
}
