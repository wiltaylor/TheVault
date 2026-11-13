//! TV-266: fake Gmail and a fixed vault clock, never a live account.
use std::ffi::OsStr;

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::fake_google::{
    DEFAULT_ACCOUNT, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN,
};
use testkit::{FakeGoogle, TestServer};

fn expect(response: Response, status: u16) -> Value {
    let got = response.status().as_u16();
    let text = response.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

struct Fixture {
    server: TestServer,
    google: FakeGoogle,
    client: Client,
    cookie: String,
    id: String,
    selection: Value,
    bytes: Vec<u8>,
}

impl Fixture {
    fn agent_token(&self) -> String {
        let issued = expect(
            self.client
                .post(self.server.url("/api/agent-codes"))
                .header("cookie", &self.cookie)
                .json(&json!({"name":"metadata-test","scope":"write"}))
                .send()
                .unwrap(),
            201,
        );
        let redeemed = expect(
            self.client
                .post(self.server.url("/api/agent-codes/redeem"))
                .json(&json!({"code":issued["code"]}))
                .send()
                .unwrap(),
            201,
        );
        redeemed["token"].as_str().unwrap().to_owned()
    }
    fn new() -> Self {
        let google = FakeGoogle::start();
        let bytes = "item,amount\nInvoice,123\n".repeat(40000).into_bytes();
        google.add_message(json!({"id":"invoice", "at":1768000000000_i64,
            "labels":["INBOX"],"text":"Invoice attached", "from":"billing@example.test",
            "attachments":[{"filename":"invoice.csv","mime":"text/csv", "data":String::from_utf8(bytes.clone()).unwrap()}]}));
        let server = TestServer::start_with_args(
            |_| {},
            &[
                OsStr::new("--test-clock"),
                OsStr::new("2026-01-10T12:00:00Z"),
            ],
        );
        let client = Client::new();
        let login = client
            .post(server.url("/api/login"))
            .json(&json!({"password":testkit::PASSWORD}))
            .send()
            .unwrap();
        assert_eq!(login.status(), 200);
        let cookie = login.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let settings = json!({"gmail_mode":"all","endpoints":google.endpoints()}).to_string();
        for (name, value) in [
            ("google_account", DEFAULT_ACCOUNT),
            ("google_client_id", DEFAULT_CLIENT_ID),
            ("google_client_secret", DEFAULT_CLIENT_SECRET),
            ("google_refresh_token", DEFAULT_REFRESH_TOKEN),
            ("google_settings", settings.as_str()),
        ] {
            server.seed_secret(name, value);
        }
        let task = format!(
            r#"label = "Gmail"
schedule = "*/10 * * * *"
script = "google_gmail"
timeout = "PT5M"
allow_domains = ["{}"]
secrets = ["google_account", "google_client_id", "google_client_secret", "google_refresh_token", "google_settings", "google_gmail_state", "google_gmail_trashed"]
paused = true
"#,
            google.host()
        );
        expect(
            client
                .put(server.url("/api/tasks/google_gmail"))
                .bearer_auth(server.admin_token())
                .body(task)
                .send()
                .unwrap(),
            200,
        );
        let run = expect(
            client
                .post(server.url("/api/tasks/google_gmail/run?wait=true"))
                .bearer_auth(server.admin_token())
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run}");
        let list = expect(
            client
                .get(server.url("/api/notes?type=email"))
                .bearer_auth(server.admin_token())
                .send()
                .unwrap(),
            200,
        );
        let id = list["notes"][0]["id"].as_str().unwrap().to_owned();
        let selection =
            json!({"account":DEFAULT_ACCOUNT,"message":"invoice","part":"att-invoice-0"});
        Self {
            server,
            google,
            client,
            cookie,
            id,
            selection,
            bytes,
        }
    }
    fn path(&self) -> String {
        self.server
            .url(&format!("/api/integrations/google/attachments/{}", self.id))
    }
    fn fetch(&self, selection: &Value) -> Response {
        self.client
            .post(self.path())
            .header("cookie", &self.cookie)
            .json(selection)
            .send()
            .unwrap()
    }
    fn note(&self) -> Value {
        expect(
            self.client
                .get(self.server.url(&format!("/api/notes/{}", self.id)))
                .bearer_auth(self.server.admin_token())
                .send()
                .unwrap(),
            200,
        )
    }
}

#[test]
fn owner_fetch_stores_large_invoice_once_and_invalidates_old_evidence() {
    let f = Fixture::new();
    let before = f.note();
    assert_eq!(before["note"]["fields"]["gmail_account"], DEFAULT_ACCOUNT);
    assert_eq!(
        before["note"]["fields"]["gmail_deferred_parts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let release = f
        .server
        .url(&format!("/api/notes/{}/agent-safety/release", f.id));
    expect(
        f.client
            .post(&release)
            .header("cookie", &f.cookie)
            .json(&json!({"fingerprint":before["agent_safety"]["fingerprint"]}))
            .send()
            .unwrap(),
        200,
    );
    let old = f.note()["agent_safety"]["fingerprint"].clone();
    f.google.clear_requests();
    let out = expect(f.fetch(&f.selection), 200);
    let asset = out["asset"].as_str().unwrap();
    let after = f.note();
    assert_eq!(after["note"]["fields"]["attachments"], json!([asset]));
    assert_eq!(after["note"]["fields"]["gmail_deferred_parts"], json!([]));
    assert_eq!(after["agent_safety"]["status"], "held");
    assert_ne!(after["agent_safety"]["fingerprint"], old);
    let evidence = expect(
        f.client
            .get(
                f.server
                    .url(&format!("/api/agent-safety/evidence/{}", f.id)),
            )
            .header("cookie", &f.cookie)
            .send()
            .unwrap(),
        200,
    );
    assert!(evidence.to_string().contains(asset));
    let response = f
        .client
        .get(f.server.url(&format!("/api/assets/{asset}")))
        .bearer_auth(f.server.admin_token())
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().unwrap().as_ref(), f.bytes);
    expect(f.fetch(&f.selection), 400);
    assert_eq!(
        f.google
            .requests_to("/gmail/v1/users/me/messages/invoice/attachments/")
            .len(),
        1
    );
    assert_eq!(f.note()["note"]["fields"]["attachments"], json!([asset]));
}

#[test]
fn preview_read_and_invalid_selections_download_nothing() {
    let f = Fixture::new();
    f.google.clear_requests();
    let preview = expect(
        f.client
            .get(f.path())
            .header("cookie", &f.cookie)
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(preview["parts"][0]["size"], f.bytes.len());
    assert_eq!(preview["parts"][0]["fetchable"], true);
    f.note();
    for field in ["account", "message", "part"] {
        let mut bad = f.selection.clone();
        bad[field] = json!("unknown");
        expect(f.fetch(&bad), 400);
    }
    for method in ["GET", "POST"] {
        let req = f
            .client
            .request(method.parse().unwrap(), f.path())
            .bearer_auth(f.server.admin_token());
        let req = if method == "POST" {
            req.json(&f.selection)
        } else {
            req
        };
        expect(req.send().unwrap(), 403);
    }
    assert!(f.google.requests().is_empty(), "{:?}", f.google.requests());
}

#[test]
fn provider_failure_is_fixed_text_and_does_not_store_bytes() {
    let f = Fixture::new();
    f.google.set_invalid_grant(true);
    let error = expect(f.fetch(&f.selection), 400);
    assert!(
        error
            .to_string()
            .contains("Gmail attachment fetch failed or exceeded its limits")
    );
    assert!(!error.to_string().contains(DEFAULT_REFRESH_TOKEN));
    assert!(f.note()["note"]["fields"]["attachments"].is_null());
    assert_eq!(
        f.note()["note"]["fields"]["gmail_deferred_parts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn device_restores_imported_email_but_agent_http_restore_remains_held() {
    let f = Fixture::new();
    let device = expect(
        f.client
            .post(f.server.url("/api/devices/pair"))
            .json(&json!({"password":testkit::PASSWORD,"name":"restore","platform":"android"}))
            .send()
            .unwrap(),
        201,
    );
    {
        let token = device["token"].as_str().unwrap();
        let before = f.note();
        expect(
            f.client
                .post(
                    f.server
                        .url(&format!("/api/notes/{}/agent-safety/release", f.id)),
                )
                .header("cookie", &f.cookie)
                .json(&json!({"fingerprint":before["agent_safety"]["fingerprint"]}))
                .send()
                .unwrap(),
            200,
        );
        expect(
            f.client
                .delete(f.server.url(&format!(
                    "/api/notes/{}?base_version={}",
                    f.id,
                    before["version"].as_str().unwrap()
                )))
                .header("cookie", &f.cookie)
                .send()
                .unwrap(),
            200,
        );
        // Existing read-safety middleware cannot read a trashed note. It still
        // refuses an agent token before the writer, even after owner release.
        let denied = expect(
            f.client
                .post(f.server.url(&format!("/api/trash/{}/restore", f.id)))
                .bearer_auth(f.agent_token())
                .send()
                .unwrap(),
            403,
        );
        assert_eq!(denied["error"], "held");
        expect(
            f.client
                .post(f.server.url(&format!("/api/trash/{}/restore", f.id)))
                .bearer_auth(token)
                .send()
                .unwrap(),
            200,
        );
        let after = f.note();
        for field in ["gmail_ref", "gmail_account", "gmail_deferred_parts"] {
            assert_eq!(
                before["note"]["fields"][field],
                after["note"]["fields"][field]
            );
        }
    }
}

#[test]
fn agent_token_cannot_preview_or_fetch() {
    let f = Fixture::new();
    let token = f.agent_token();
    f.google.clear_requests();
    for method in ["GET", "POST"] {
        let request = f
            .client
            .request(method.parse().unwrap(), f.path())
            .bearer_auth(&token);
        let request = if method == "POST" {
            request.json(&f.selection)
        } else {
            request
        };
        expect(request.send().unwrap(), 403);
    }
    assert!(f.google.requests().is_empty());
}

#[test]
fn metadata_create_and_update_refuse_agent_tokens_and_scripts_but_allow_owner() {
    let f = Fixture::new();
    let token = f.agent_token();
    for field in ["gmail_ref", "gmail_account", "gmail_deferred_parts"] {
        let value = if field == "gmail_deferred_parts" {
            json!(["forged"])
        } else {
            json!("forged")
        };
        let note = json!({"fields":{"title":"Mail",field:value}});
        let created = expect(
            f.client
                .post(f.server.url("/api/notes"))
                .header("cookie", &f.cookie)
                .json(&json!({"type":"email","note":{"fields":{"title":"Mail"}}}))
                .send()
                .unwrap(),
            201,
        );
        let id = created["id"].as_str().unwrap();
        let version = created["version"].as_str().unwrap();
        let current = expect(
            f.client
                .get(f.server.url(&format!("/api/notes/{id}")))
                .header("cookie", &f.cookie)
                .send()
                .unwrap(),
            200,
        );
        expect(
            f.client
                .post(
                    f.server
                        .url(&format!("/api/notes/{id}/agent-safety/release")),
                )
                .header("cookie", &f.cookie)
                .json(&json!({"fingerprint":current["agent_safety"]["fingerprint"]}))
                .send()
                .unwrap(),
            200,
        );
        for create in [true, false] {
            let (method, path) = if create {
                ("POST", f.server.url("/api/notes"))
            } else {
                ("PUT", f.server.url(&format!("/api/notes/{id}")))
            };
            let response = f
                .client
                .request(method.parse().unwrap(), &path)
                .bearer_auth(&token)
                .json(&json!({"type":"email","base_version":version,"note":note}))
                .send()
                .unwrap();
            assert_eq!(
                expect(response, 422)["error"],
                "rejected",
                "{field} create={create}"
            );
            let expression = if field == "gmail_deferred_parts" {
                "Value::List([Value::String(\"forged\")])"
            } else {
                "Value::String(\"forged\")"
            };
            let body = format!(
                "Value::Map(#{{\"fields\": Value::Map(#{{\"title\": Value::String(\"Mail\"), \"{field}\": {expression}}})}})"
            );
            let action = if create {
                format!("vault::create(\"email\", {body})?")
            } else {
                format!("vault::update(\"{id}\", {body}, \"{version}\")?")
            };
            let source = format!(
                "use vault\nfn run() -> Result[string, string] {{\n    {action}\n    Ok(\"unexpected\")\n}}\n"
            );
            expect(
                f.client
                    .put(f.server.url("/api/scripts/metadata_probe"))
                    .header("cookie", &f.cookie)
                    .json(&json!({"source":source}))
                    .send()
                    .unwrap(),
                200,
            );
            expect(
                f.client
                    .put(f.server.url("/api/tasks/metadata_probe"))
                    .header("cookie", &f.cookie)
                    .body("schedule = \"0 0 1 1 *\"\nscript = \"metadata_probe\"\npaused = true\n")
                    .send()
                    .unwrap(),
                200,
            );
            let run = expect(
                f.client
                    .post(f.server.url("/api/tasks/metadata_probe/run?wait=true"))
                    .header("cookie", &f.cookie)
                    .send()
                    .unwrap(),
                200,
            );
            assert_eq!(run["outcome"], "failed", "{field} create={create}: {run}");
            assert!(
                run.to_string()
                    .contains("only the owner session or Gmail import"),
                "{run}"
            );
            expect(
                f.client
                    .request(method.parse().unwrap(), path)
                    .header("cookie", &f.cookie)
                    .json(&json!({"type":"email","base_version":version,"note":note}))
                    .send()
                    .unwrap(),
                if create { 201 } else { 200 },
            );
        }
    }
}
