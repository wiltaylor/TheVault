//! S1: agent files through `/api/agents` (agents G1, G2, G6): listing and reading need `read`,
//! writing and deleting need `admin` or a paired device's token, a write takes WCL or the JSON
//! form and is refused with its issues, and the built-in Assistant cannot be deleted or disabled.

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::{PASSWORD, TestServer};

const EMAIL: &str = r#"label = "Email"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TRIAGE: &str = r#"label = "Triage"
account = "OpenAI"

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
            "description": "The chat panel's assistant. It uses the default AI account and its model in Settings > AI.",
            "icon": "sparkles",
            "account": null,
            "model": null,
            "enabled": true,
            "triggers": 0,
            "mention": { "types": null, "excluded_types": ["email", "message", "call", "conversation", "event", "email_thread"] },
            "running": 0,
            "built_in": true,
        }])
    );
    let tools: Vec<&str> = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(tools.contains(&"search_notes") && tools.contains(&"web_search"));
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
        json!({ "notes": true, "archive": true, "delete": true, "types": true, "downloads": true, "taxonomies": true, "agents": true, "skills": true, "tools": true })
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
fn browser_secret_grants_require_owner_and_reconfirmation_after_recreation() {
    let api = Api::start();
    let login = json!({"kind": "login", "username": "test", "password": "made-up-password", "origins": ["https://example.com"]});
    expect(
        api.admin("PUT", "/api/secrets/bank")
            .json(&login)
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    let source = "permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n";
    expect(
        api.put_wcl("clerk", source, api.server.admin_token()),
        StatusCode::OK,
    );
    expect(
        api.admin("DELETE", "/api/secrets/bank").send().unwrap(),
        StatusCode::OK,
    );
    expect(
        api.admin("PUT", "/api/secrets/bank")
            .json(&login)
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    let read = || {
        expect(
            api.admin("GET", "/api/agents/clerk").send().unwrap(),
            StatusCode::OK,
        )
    };
    assert_eq!(read()["browser_secrets_inactive"], json!(["bank"]));
    let device = pair_device(&api);
    let code = expect(
        api.admin("POST", "/api/agent-codes")
            .json(&json!({"name": "test", "scope": "write"}))
            .send()
            .unwrap(),
        StatusCode::CREATED,
    );
    let external = expect(
        api.client
            .post(api.server.url("/api/agent-codes/redeem"))
            .json(&json!({"code": code["code"]}))
            .send()
            .unwrap(),
        StatusCode::CREATED,
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();
    for token in [&device, &external] {
        expect(api.put_wcl("clerk", source, token), StatusCode::FORBIDDEN);
        assert_eq!(read()["browser_secrets_inactive"], json!(["bank"]));
        let hidden = expect(
            api.req("GET", "/api/agents/clerk", token).send().unwrap(),
            StatusCode::OK,
        );
        assert_eq!(hidden["permissions"]["browser_secrets"], json!([]));
        assert!(!hidden["wcl"].as_str().unwrap().contains("bank"));
    }
    expect(
        api.req("GET", "/api/secrets", &external).send().unwrap(),
        StatusCode::FORBIDDEN,
    );
    expect(
        api.put_wcl("clerk", source, api.server.admin_token()),
        StatusCode::OK,
    );
    assert_eq!(read()["browser_secrets_inactive"], json!([]));
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
    assert_eq!(created["account"], "OpenAI");
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
fn browser_secrets_outside_file_writes_cannot_add_unconfirmed_names() {
    let mut api = Api::start();
    expect(
        api.admin("PUT", "/api/secrets/bank")
            .json(&json!({"kind":"login", "username":"test", "password":"made-up", "origins":[]}))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    expect(
        api.put_wcl(
            "clerk",
            "permissions {\n browser = true\n}\n",
            api.server.admin_token(),
        ),
        StatusCode::OK,
    );
    std::fs::write(
        api.server.vault_path().join("agents/clerk.wcl"),
        "permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n",
    )
    .unwrap();
    for restart in [false, true] {
        if restart {
            api.server.restart();
        }
        let agent = expect(
            api.admin("GET", "/api/agents/clerk").send().unwrap(),
            StatusCode::OK,
        );
        assert_eq!(agent["browser_secrets_inactive"], json!(["bank"]));
    }
}

#[test]
fn browser_secrets_deleting_an_agent_revokes_its_confirmed_names() {
    let api = Api::start();
    expect(
        api.admin("PUT", "/api/secrets/bank")
            .json(&json!({"kind":"login", "username":"test", "password":"made-up", "origins":[]}))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    let source = "permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n";
    expect(
        api.put_wcl("clerk", source, api.server.admin_token()),
        StatusCode::OK,
    );
    expect(
        api.admin("DELETE", "/api/agents/clerk").send().unwrap(),
        StatusCode::OK,
    );
    std::fs::write(api.server.vault_path().join("agents/clerk.wcl"), source).unwrap();
    let agent = expect(
        api.admin("GET", "/api/agents/clerk").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(agent["browser_secrets_inactive"], json!(["bank"]));
}

#[test]
fn browser_secrets_external_deletion_revokes_on_startup_and_live_reads() {
    for startup in [true, false] {
        let mut api = Api::start();
        expect(
            api.admin("PUT", "/api/secrets/bank")
                .json(
                    &json!({"kind":"login", "username":"test", "password":"made-up", "origins":[]}),
                )
                .send()
                .unwrap(),
            StatusCode::OK,
        );
        let source = "permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n";
        expect(
            api.put_wcl("clerk", source, api.server.admin_token()),
            StatusCode::OK,
        );
        std::fs::remove_file(api.server.vault_path().join("agents/clerk.wcl")).unwrap();
        if startup {
            api.server.restart();
        }
        expect(
            api.admin("GET", "/api/agents/clerk").send().unwrap(),
            StatusCode::NOT_FOUND,
        );
        std::fs::write(api.server.vault_path().join("agents/clerk.wcl"), source).unwrap();
        api.server.restart();
        let agent = expect(
            api.admin("GET", "/api/agents/clerk").send().unwrap(),
            StatusCode::OK,
        );
        assert_eq!(agent["browser_secrets_inactive"], json!(["bank"]));
    }
}

#[test]
fn browser_secrets_invalid_names_are_not_disclosed_in_issues() {
    let api = Api::start();
    let read = api.token("read");
    for names in ["[\"bank\", \"bank\"]", "[\"bank-invalid!\"]"] {
        std::fs::write(
            api.server.vault_path().join("agents/clerk.wcl"),
            format!("permissions {{\n browser = true\n browser_secrets = {names}\n}}\n"),
        )
        .unwrap();
        let response = api.req("GET", "/api/agents/clerk", &read).send().unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!response.text().unwrap().contains("bank"));
        let list = expect(
            api.req("GET", "/api/agents", &read).send().unwrap(),
            StatusCode::OK,
        );
        assert!(!list.to_string().contains("bank"));
    }
}

#[test]
fn browser_secrets_sync_hides_names_from_all_non_managers_including_devices() {
    let api = Api::start();
    expect(
        api.put_wcl(
            "clerk",
            "permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n",
            api.server.admin_token(),
        ),
        StatusCode::OK,
    );
    let code = expect(
        api.admin("POST", "/api/agent-codes")
            .json(&json!({"name":"probe", "scope":"write"}))
            .send()
            .unwrap(),
        StatusCode::CREATED,
    );
    let external = expect(
        api.client
            .post(api.server.url("/api/agent-codes/redeem"))
            .json(&json!({"code":code["code"]}))
            .send()
            .unwrap(),
        StatusCode::CREATED,
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = api.token("read");
    let write = api.token("write");
    let device = pair_device(&api);
    expect(api.req("GET", "/api/sync/changes?since=0&limit=5000", &external).send().unwrap(), StatusCode::FORBIDDEN);
    for token in [&read, &write, &device] {
        let page = expect(
            api.req("GET", "/api/sync/changes?since=0&limit=5000", token)
                .send()
                .unwrap(),
            StatusCode::OK,
        );
        let entry = page["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"] == "agents/clerk.wcl")
            .unwrap();
        assert_eq!(entry["browser_secrets_hidden"], true);
        assert!(!entry["text"].as_str().unwrap().contains("bank"));
    }
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
    form["permissions"]["change_types"] = json!(["email"]);
    form["permissions"]["change_fields"] = json!({ "email": ["processed"] });
    form["triggers"][0]["timezone"] = json!("Australia/Melbourne");
    let saved = expect(
        api.admin("PUT", "/api/agents/triage")
            .json(&form)
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(saved["label"], "Mail sorter");
    assert_eq!(saved["permissions"]["change_types"], json!(["email"]));
    assert_eq!(
        saved["permissions"]["change_fields"],
        json!({ "email": ["processed"] })
    );
    assert_eq!(saved["effort"], "low");
    assert_eq!(saved["permissions"]["web"]["download"], false);
    assert_eq!(saved["triggers"][0]["timezone"], "Australia/Melbourne");
    assert_eq!(saved["triggers"][1], created["triggers"][1]);
    let wcl = saved["wcl"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(api.server.vault_path().join("agents/triage.wcl")).unwrap(),
        wcl
    );
    assert!(wcl.starts_with("label   = \"Mail sorter\"\n"), "{wcl}");
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

/// A device paired with `api`'s server: its `write` token.
fn pair_device(api: &Api) -> String {
    let res = api
        .client
        .post(api.server.url("/api/devices/pair"))
        .json(&json!({ "password": PASSWORD, "name": "Phone", "platform": "android" }))
        .send()
        .unwrap();
    expect(res, StatusCode::CREATED)["token"]
        .as_str()
        .unwrap()
        .to_owned()
}

// Explicit admin tokens may manage definitions, including permission grants.
#[test]
fn an_admin_token_writes_agent_files() {
    let api = Api::start();
    let device = api.server.admin_token().to_owned();

    let created = expect(
        api.req("PUT", "/api/agents/clerk", &device)
            .json(&json!({ "label": "Clerk" }))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(created["label"], "Clerk");
    assert!(api.server.vault_path().join("agents/clerk.wcl").is_file());

    let granted = expect(
        api.req("PUT", "/api/agents/clerk", &device)
            .json(&json!({
                "label": "Clerk",
                "permissions": {
                    "scripts": { "run_code": true },
                    "subagents": { "allow": true, "max": 2, "agents": ["triage"] },
                },
            }))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(granted["permissions"]["scripts"]["run_code"], true);
    assert_eq!(
        granted["permissions"]["subagents"],
        json!({ "allow": true, "max": 2, "agents": ["triage"] })
    );
    let wcl = std::fs::read_to_string(api.server.vault_path().join("agents/clerk.wcl")).unwrap();
    assert!(wcl.contains("run_code"), "{wcl}");

    // The same checks as for the web app.
    let err = expect(
        api.req("PUT", "/api/agents/clerk", &device)
            .json(&json!({ "permissions": { "tools": ["summon_cat"] } }))
            .send()
            .unwrap(),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["error"], "invalid");
    let err = expect(
        api.req("DELETE", "/api/agents/assistant", &device)
            .send()
            .unwrap(),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["error"], "built_in");

    let gone = expect(
        api.req("DELETE", "/api/agents/clerk", &device)
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(gone, json!({ "name": "clerk", "deleted": true }));
    assert!(!api.server.vault_path().join("agents/clerk.wcl").exists());

    // A write-scoped device still cannot use other admin routes.
    let device = pair_device(&api);
    let refused = expect(
        api.req("POST", "/api/integrations/ai/accounts", &device)
            .json(&json!({ "name": "Mine", "provider": "anthropic" }))
            .send()
            .unwrap(),
        StatusCode::FORBIDDEN,
    );
    assert_eq!(refused["error"], "forbidden");
    expect(
        api.req("DELETE", "/api/types/email", &device)
            .send()
            .unwrap(),
        StatusCode::FORBIDDEN,
    );
    assert!(api.server.vault_path().join("types/email.wcl").is_file());

    // An API token with `write` is not a device, whatever it is called.
    // Existing user-kind tokens with this prefix still need a real device record.
    let state = state::State::open(&api.server.vault_path().join(".vault/state.db"), None).unwrap();
    let (_, named) = state
        .create_token("device-fake", state::Scope::Write)
        .unwrap();
    expect(
        api.put_wcl("clerk", "label = \"Mine\"\n", &named),
        StatusCode::FORBIDDEN,
    );
}
