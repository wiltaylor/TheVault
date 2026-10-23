//! S1: agent files through `/api/agents` (agents G1, G2, G6): listing and reading need `read`,
//! writing and deleting need `admin`, a write takes WCL or the JSON form and is refused with its
//! issues, and the built-in Assistant cannot be deleted or disabled.

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::TestServer;

const EMAIL: &str = r#"label = "Email"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TRIAGE: &str = r#"label = "Triage"
provider = "openai"

permissions {
  tools = ["search_notes", "get_note", "web_search"]
  types = ["email"]

  confirm {
    notes = false
  }
}

trigger nightly {
  schedule = "0 2 * * *"
  prompt   = "Summarise the day."
}

trigger new_email {
  on     = ["create"]
  type   = "email"
  where  = "title != \"\""
  prompt = "File this email."
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/email.wcl"), EMAIL).unwrap();
        });
        Api {
            server,
            client: Client::new(),
        }
    }

    fn req(&self, method: &str, path: &str, token: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(AUTHORIZATION, format!("Bearer {token}"))
    }

    fn admin(&self, method: &str, path: &str) -> RequestBuilder {
        self.req(method, path, self.server.admin_token())
    }

    fn put_wcl(&self, name: &str, wcl: &str, token: &str) -> Response {
        self.req("PUT", &format!("/api/agents/{name}"), token)
            .header(CONTENT_TYPE, "text/plain")
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn token(&self, scope: &str) -> String {
        let res = self
            .admin("POST", "/api/tokens")
            .json(&json!({ "name": format!("t-{scope}"), "scope": scope }))
            .send()
            .unwrap();
        expect(res, StatusCode::CREATED)["secret"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

#[test]
fn the_vault_starts_with_the_assistant() {
    let api = Api::start();
    let list = expect(
        api.admin("GET", "/api/agents").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(
        list["agents"],
        json!([{
            "name": "assistant",
            "label": "Assistant",
            "description": "The chat panel's assistant. It uses the default provider and model in Settings > AI.",
            "icon": "sparkles",
            "provider": null,
            "model": null,
            "enabled": true,
            "triggers": 0,
            "built_in": true,
        }])
    );
    let a = expect(
        api.admin("GET", "/api/agents/assistant").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(a["built_in"], true);
    assert_eq!(a["system_prompt"], Value::Null);
    assert_eq!(a["permissions"]["tools"], Value::Null);
    assert_eq!(a["permissions"]["types"], Value::Null);
    assert_eq!(
        a["permissions"]["confirm"],
        json!({ "notes": true, "archive": true, "delete": true, "types": true, "downloads": true, "taxonomies": true })
    );
    assert_eq!(
        a["permissions"]["web"],
        json!({ "search": true, "read": true, "download": true })
    );
    assert!(
        a["wcl"]
            .as_str()
            .unwrap()
            .contains("label       = \"Assistant\"")
    );
}

#[test]
fn read_lists_and_reads_write_cannot_change_admin_writes() {
    let api = Api::start();
    let read = api.token("read");
    let write = api.token("write");

    // Admin creates.
    let created = expect(
        api.put_wcl("triage", TRIAGE, api.server.admin_token()),
        StatusCode::OK,
    );
    assert_eq!(created["name"], "triage");
    assert_eq!(created["provider"], "openai");
    assert_eq!(created["permissions"]["confirm"]["notes"], false);
    assert_eq!(
        created["permissions"]["tools"],
        json!(["search_notes", "get_note", "web_search"])
    );
    assert_eq!(
        created["triggers"],
        json!([
            { "name": "nightly", "kind": "schedule", "schedule": "0 2 * * *", "timezone": "UTC", "prompt": "Summarise the day." },
            { "name": "new_email", "kind": "event", "on": ["create"], "type": ["email"], "where": "title != \"\"", "prompt": "File this email." },
        ])
    );
    assert!(api.server.vault_path().join("agents/triage.wcl").is_file());

    // Read lists and reads.
    let list = expect(
        api.req("GET", "/api/agents", &read).send().unwrap(),
        StatusCode::OK,
    );
    let names: Vec<&str> = list["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["assistant", "triage"]);
    assert_eq!(list["agents"][1]["triggers"], 2);
    assert_eq!(list["agents"][1]["built_in"], false);
    let one = expect(
        api.req("GET", "/api/agents/triage", &read).send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(one["label"], "Triage");
    let text = api
        .req("GET", "/api/agents/triage", &read)
        .header(ACCEPT, "text/plain")
        .send()
        .unwrap();
    assert_eq!(text.status(), StatusCode::OK);
    assert_eq!(text.text().unwrap(), one["wcl"].as_str().unwrap());
    expect(
        api.req("GET", "/api/agents/nobody", &read).send().unwrap(),
        StatusCode::NOT_FOUND,
    );

    // Neither read nor write may change an agent.
    for token in [&read, &write] {
        expect(
            api.put_wcl("triage", "label = \"Mine\"\n", token),
            StatusCode::FORBIDDEN,
        );
        expect(
            api.req("DELETE", "/api/agents/triage", token)
                .send()
                .unwrap(),
            StatusCode::FORBIDDEN,
        );
    }
    let still = expect(
        api.admin("GET", "/api/agents/triage").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(still["label"], "Triage");

    // Admin deletes.
    let gone = expect(
        api.admin("DELETE", "/api/agents/triage").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(gone, json!({ "name": "triage", "deleted": true }));
    assert!(!api.server.vault_path().join("agents/triage.wcl").exists());
    expect(
        api.admin("DELETE", "/api/agents/triage").send().unwrap(),
        StatusCode::NOT_FOUND,
    );
}

#[test]
fn the_json_form_is_written_as_canonical_wcl_and_reads_back() {
    let api = Api::start();
    let created = expect(
        api.put_wcl("triage", TRIAGE, api.server.admin_token()),
        StatusCode::OK,
    );
    // The agent screen sends back what it read, changed.
    let mut form = created.clone();
    form.as_object_mut().unwrap().remove("wcl");
    form["label"] = json!("Mail sorter");
    form["effort"] = json!("low");
    form["permissions"]["web"]["download"] = json!(false);
    form["triggers"][0]["timezone"] = json!("Australia/Melbourne");
    let saved = expect(
        api.admin("PUT", "/api/agents/triage")
            .json(&form)
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(saved["label"], "Mail sorter");
    assert_eq!(saved["effort"], "low");
    assert_eq!(saved["permissions"]["web"]["download"], false);
    assert_eq!(saved["triggers"][0]["timezone"], "Australia/Melbourne");
    assert_eq!(saved["triggers"][1], created["triggers"][1]);
    let wcl = saved["wcl"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(api.server.vault_path().join("agents/triage.wcl")).unwrap(),
        wcl
    );
    assert!(wcl.starts_with("label    = \"Mail sorter\"\n"), "{wcl}");
    assert!(wcl.contains("trigger nightly {"), "{wcl}");

    // `{ "wcl": ... }` is the WCL text.
    let saved = expect(
        api.admin("PUT", "/api/agents/triage")
            .json(&json!({ "wcl": "label = \"Plain\"\n" }))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(saved["label"], "Plain");
    assert_eq!(saved["triggers"], json!([]));
}

#[test]
fn a_bad_agent_is_refused_with_its_issues() {
    let api = Api::start();
    let admin = api.server.admin_token();
    let bad = r#"provider = "gemini"

permissions {
  tools = ["teleport"]
  types = ["invoice"]
}

trigger t {
  on     = ["publish"]
  prompt = "Go."
}
"#;
    let err = expect(api.put_wcl("triage", bad, admin), StatusCode::BAD_REQUEST);
    assert_eq!(err["error"], "invalid");
    assert_eq!(err["file"], "agents/triage.wcl");
    let at: Vec<&str> = err["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["at"].as_str().unwrap())
        .collect();
    assert_eq!(at, ["provider", "trigger t.on"]);

    // The file alone checks; then the types and the tools.
    let unknown = "permissions {\n  tools = [\"teleport\"]\n  types = [\"invoice\"]\n}\n";
    let err = expect(
        api.put_wcl("triage", unknown, admin),
        StatusCode::BAD_REQUEST,
    );
    let at: Vec<&str> = err["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["at"].as_str().unwrap())
        .collect();
    assert_eq!(at, ["permissions.types", "permissions.tools"]);

    let err = expect(
        api.put_wcl("Bad-Name", "label = \"x\"\n", admin),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["error"], "bad_request");

    let form = json!({ "triggers": [{ "name": "t", "prompt": "Go." }] });
    let err = expect(
        api.admin("PUT", "/api/agents/triage")
            .json(&form)
            .send()
            .unwrap(),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["issues"][0]["at"], "trigger t");
    let err = expect(
        api.admin("PUT", "/api/agents/triage")
            .json(&json!({ "colour": "red" }))
            .send()
            .unwrap(),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["error"], "bad_request");
    assert!(!api.server.vault_path().join("agents/triage.wcl").exists());
}

#[test]
fn the_assistant_cannot_be_deleted_or_disabled() {
    let api = Api::start();
    let admin = api.server.admin_token();
    let err = expect(
        api.admin("DELETE", "/api/agents/assistant").send().unwrap(),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(
        err,
        json!({ "error": "built_in", "message": "Assistant is built in and cannot be deleted." })
    );
    let err = expect(
        api.put_wcl(
            "assistant",
            "label = \"Assistant\"\nenabled = false\n",
            admin,
        ),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(
        err,
        json!({ "error": "built_in", "message": "Assistant is built in and cannot be disabled." })
    );
    let err = expect(
        api.admin("PUT", "/api/agents/assistant")
            .json(&json!({ "label": "Assistant", "enabled": false }))
            .send()
            .unwrap(),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["error"], "built_in");
    // Every other field may change.
    let saved = expect(
        api.admin("PUT", "/api/agents/assistant")
            .json(&json!({ "label": "Helper", "model": "gpt-6", "system_prompt": "Be brief." }))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(saved["label"], "Helper");
    assert_eq!(saved["enabled"], true);
    assert_eq!(saved["built_in"], true);
    assert!(
        api.server
            .vault_path()
            .join("agents/assistant.wcl")
            .is_file()
    );
}
