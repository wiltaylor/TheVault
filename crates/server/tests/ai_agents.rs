//! S1 with the fake Anthropic provider: the chat's agent tools (agents G20). `list_agents` and
//! `get_agent` read; `propose_agent_change` and `propose_agent_delete` check without writing and
//! emit `agent_proposal` with its `proposer`; `POST /api/ai/apply` applies one, refuses a stale
//! one, checks it again against the proposing agent, and answers a write-scope caller 403
//! `needs_admin`; a change that would give an agent more than the proposing agent has, or touch
//! its own file, is refused on propose and on apply; `confirm.agents` off applies at once for an
//! admin or write caller, a read caller still getting the proposal; an unattended run with the switch on is not offered the propose tools.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

const MEMO: &str = r#"label = "Memo"

field status {
  kind = "string"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
  }
}
"#;

const TASK: &str = r#"label = "Task"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// An agent with a tools list, one note type, Internet search off and every confirmation on.
const CLERK: &str = r#"label = "Clerk"

permissions {
  tools = ["get_note", "propose_edit", "list_agents", "get_agent", "propose_agent_change", "propose_agent_delete"]
  types = ["memo"]

  web {
    search = false
  }
}
"#;

/// An agent within Clerk's permissions.
const HELPER: &str = r#"label = "Helper"

permissions {
  tools = ["get_note"]
  types = ["memo"]

  web {
    search = false
  }
}
"#;

const MODEL: &str = "claude-opus-5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
            std::fs::write(vault.join("types/task.wcl"), TASK).unwrap();
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Api { server, client }
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn secret(&self, name: &str, value: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/secrets/{name}")))
            .bearer_auth(self.token())
            .json(&json!({ "value": value }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    /// The fake as the default provider, with Assistant's confirmation switches `confirm`.
    fn anthropic(&self, fake: &FakeAnthropic, confirm: Value) {
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            self.secret(&name, &value);
        }
        let res = self
            .client
            .put(self.server.url("/api/integrations/ai/settings"))
            .bearer_auth(self.token())
            .json(&json!({ "confirm": confirm }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn put_agent(&self, name: &str, wcl: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/agents/{name}")))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(wcl.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    /// `agents/<name>.wcl` as stored, or `None` when there is none.
    fn agent_wcl(&self, name: &str) -> Option<String> {
        let res = self
            .client
            .get(self.server.url(&format!("/api/agents/{name}")))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        if !res.status().is_success() {
            return None;
        }
        let v: Value = res.json().unwrap();
        Some(v["wcl"].as_str().unwrap().to_owned())
    }

    fn get(&self, path: &str) -> Value {
        let res = self
            .client
            .get(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{path}");
        res.json().unwrap()
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        if let Some(name) = name.strip_prefix("agent-") {
            // Legacy admin install codes must remain restricted after upgrade.
            let state = state::State::open(&self.server.vault_path().join(".vault/state.db"), None)
                .unwrap();
            let code = state
                .create_agent_code(name, scope.parse().unwrap(), 60_000)
                .unwrap();
            return state.redeem_agent_code(&code.code).unwrap().unwrap().1;
        }
        let res = self
            .client
            .post(self.server.url("/api/tokens"))
            .bearer_auth(self.token())
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        v["secret"].as_str().unwrap().to_owned()
    }

    /// One turn as `token`, run by `agent` (Assistant when `None`): its events.
    fn chat_as(&self, token: &str, agent: Option<&str>) -> Vec<(String, Value)> {
        let mut body = json!({ "messages": [user("Do it.")] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    /// One tool call by `agent` as the admin, then an answer: the turn's events.
    fn one(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(&self.token(), agent)
    }

    /// `POST /api/ai/apply` as `token`: the status and body.
    fn apply_as(&self, token: &str, change: &Value) -> (StatusCode, Value) {
        let res = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .bearer_auth(token)
            .json(change)
            .send()
            .unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn apply(&self, change: &Value) -> (StatusCode, Value) {
        self.apply_as(&self.token(), change)
    }

    fn apply_session(&self, change: &Value) -> (StatusCode, Value) {
        let login = self
            .client
            .post(self.server.url("/api/login"))
            .json(&json!({"password":testkit::PASSWORD}))
            .send()
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login.headers()[reqwest::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let response = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .header(reqwest::header::COOKIE, cookie)
            .json(change)
            .send()
            .unwrap();
        let status = response.status();
        (status, response.json().unwrap_or(Value::Null))
    }
}

fn read_events(res: Response) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    for line in BufReader::new(res).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                out.push((
                    std::mem::take(&mut name),
                    serde_json::from_str(&data).unwrap(),
                ));
                data.clear();
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("event:") {
            name = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push_str(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    out
}

fn find<'a>(events: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    events.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn last<'a>(events: &'a [(String, Value)], name: &str) -> &'a Value {
    &events
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
        .1
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

/// The tool result the model got back in the last request, as JSON, after checking its flag.
fn last_result(fake: &FakeAnthropic, is_error: bool) -> Value {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    let result = r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone();
    assert_eq!(result["is_error"], is_error, "{result}");
    serde_json::from_str(result["content"].as_str().unwrap()).unwrap()
}

/// The names of the tools request `request` offered.
fn tools_in(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn agent_tools_list_read_and_propose_create_change_and_delete_that_apply() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    api.put_agent("clerk", CLERK);

    // Assistant has every tool, so the agent tools are offered.
    fake.push(AnthropicMessage::new().tool_use("toolu_l", "list_agents", json!({})));
    fake.push(AnthropicMessage::new().tool_use("toolu_g", "get_agent", json!({ "name": "clerk" })));
    fake.push(AnthropicMessage::text("Read."));
    let events = api.chat_as(&api.token(), None);
    assert_eq!(last(&events, "done")["stop"], "end_turn", "{events:?}");
    let tools = tools_in(&fake, 0);
    for t in [
        "list_agents",
        "get_agent",
        "propose_agent_change",
        "propose_agent_delete",
    ] {
        assert!(tools.contains(&t.to_owned()), "{t} in {tools:?}");
    }
    let r = fake.message_requests()[1].json();
    let listed: Value = serde_json::from_str(
        r["messages"].as_array().unwrap().last().unwrap()["content"][0]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let agents = listed["agents"].as_array().unwrap();
    assert_eq!(agents[0]["name"], "assistant", "Assistant first: {listed}");
    assert_eq!(agents[0]["builtin"], true);
    let clerk = agents.iter().find(|a| a["name"] == "clerk").unwrap();
    assert_eq!(clerk["label"], "Clerk");
    assert_eq!(clerk["enabled"], true);
    assert_eq!(clerk["builtin"], false);
    assert_eq!(clerk["triggers"], 0);
    let got = last_result(&fake, false);
    assert_eq!(got["wcl"].as_str(), api.agent_wcl("clerk").as_deref());
    assert_eq!(got["agent"]["permissions"]["types"], json!(["memo"]));
    assert!(
        got["format"]
            .as_str()
            .unwrap()
            .starts_with("## Agent files")
    );

    // Create: proposed, not written; applied, written.
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({ "name": "helper", "wcl": HELPER }),
    );
    let p = last(&events, "agent_proposal").clone();
    assert_eq!(p["kind"], "agent");
    assert_eq!(p["op"], "create");
    assert_eq!(p["name"], "helper");
    assert_eq!(p["exists"], false);
    assert_eq!(p["before"], Value::Null);
    assert_eq!(p["proposer"], "assistant");
    assert!(p["after"].as_str().unwrap().contains("label = \"Helper\""));
    assert!(api.agent_wcl("helper").is_none(), "nothing written yet");
    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        done,
        json!({ "op": "agent_create", "agent_name": "helper" })
    );
    assert_eq!(
        api.agent_wcl("helper").as_deref(),
        p["after"].as_str(),
        "written as proposed"
    );

    // Change: before and after; a file changed since is stale.
    let changed = HELPER.replace("Helper", "Helper two");
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({ "name": "helper", "wcl": changed }),
    );
    let p = last(&events, "agent_proposal").clone();
    assert_eq!(p["op"], "change");
    assert_eq!(p["exists"], true);
    assert_eq!(p["before"].as_str(), api.agent_wcl("helper").as_deref());
    api.put_agent("helper", &HELPER.replace("Helper", "Helper three"));
    let (status, body) = api.apply(&p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    api.put_agent("helper", HELPER);
    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "agent_change");
    assert!(api.agent_wcl("helper").unwrap().contains("Helper two"));

    // The same file again proposes nothing.
    let current = api.agent_wcl("helper").unwrap();
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({ "name": "helper", "wcl": current }),
    );
    assert!(find(&events, "agent_proposal").is_none());
    assert_eq!(last_result(&fake, false)["proposed"], false);

    // Delete.
    let events = api.one(
        &fake,
        None,
        "propose_agent_delete",
        json!({ "name": "helper" }),
    );
    let p = last(&events, "agent_proposal").clone();
    assert_eq!(p["op"], "delete");
    assert_eq!(p["before"].as_str(), api.agent_wcl("helper").as_deref());
    assert!(api.agent_wcl("helper").is_some(), "nothing deleted yet");
    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        done,
        json!({ "op": "agent_delete", "agent_name": "helper" })
    );
    assert!(api.agent_wcl("helper").is_none());

    // Assistant cannot be deleted; invalid WCL goes back with its issues.
    let events = api.one(
        &fake,
        None,
        "propose_agent_delete",
        json!({ "name": "assistant" }),
    );
    assert!(find(&events, "agent_proposal").is_none());
    let r = last_result(&fake, true);
    assert_eq!(r["code"], "built_in", "{r}");
    assert_eq!(r["error"], "Assistant is built in and cannot be deleted.");
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({ "name": "helper", "wcl": "label = \"Helper\"\nprovider = \"nobody\"\n" }),
    );
    assert!(find(&events, "agent_proposal").is_none());
    let r = last_result(&fake, true);
    assert!(r["issues"].to_string().contains("provider"), "{r}");
    assert!(api.agent_wcl("helper").is_none());
}

#[test]
fn browser_secret_switch_changes_always_wait_for_owner_apply() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let free = "permissions { confirm { agents = false\n secrets = false\n}\n}\n";
    api.put_agent("clerk", free);
    api.put_agent("helper", "");
    let events = api.one(
        &fake,
        Some("clerk"),
        "propose_agent_change",
        json!({"name":"helper", "wcl":free}),
    );
    let proposal = last(&events, "agent_proposal");
    assert_eq!(proposal["grants_run_code"], true);
    assert!(!api.agent_wcl("helper").unwrap().contains("secrets"));
    let token = api.token_of("agent-clerk", "admin");
    assert_eq!(api.apply_as(&token, proposal).0, StatusCode::FORBIDDEN);
    assert_eq!(api.apply(proposal).0, StatusCode::OK);
    assert!(api.agent_wcl("helper").unwrap().contains("secrets"));
    let events = api.one(
        &fake,
        Some("helper"),
        "propose_agent_change",
        json!({"name":"helper", "wcl":free}),
    );
    assert!(!events.iter().any(|(event, _)| event == "agent_proposal"));
}

#[test]
fn an_agent_cannot_give_another_agent_more_than_it_has_or_change_itself() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    api.put_agent("clerk", CLERK);

    let refused = [
        (
            "more tools",
            HELPER.replace("[\"get_note\"]", "[\"get_note\", \"propose_delete\"]"),
            "propose_delete",
        ),
        (
            "a tools list removed",
            HELPER.replace("  tools = [\"get_note\"]\n", ""),
            "every tool",
        ),
        (
            "a type outside its own",
            HELPER.replace("[\"memo\"]", "[\"memo\", \"task\"]"),
            "task notes",
        ),
        (
            "a confirm switch turned off",
            HELPER.replace("  web {", "  confirm {\n    notes = false\n  }\n\n  web {"),
            "confirmation of notes",
        ),
        (
            "an Internet switch turned on",
            HELPER.replace("    search = false\n", "    read = true\n"),
            "Internet switch search",
        ),
    ];
    for (what, wcl, says) in &refused {
        let events = api.one(
            &fake,
            Some("clerk"),
            "propose_agent_change",
            json!({ "name": "helper", "wcl": wcl }),
        );
        assert!(
            find(&events, "agent_proposal").is_none(),
            "{what}: {events:?}"
        );
        let r = last_result(&fake, true);
        assert!(r["error"].as_str().unwrap().contains(says), "{what}: {r}");
        assert!(api.agent_wcl("helper").is_none(), "{what}");
    }
    // Its own file, to change or to delete.
    for (tool, input) in [
        (
            "propose_agent_change",
            json!({ "name": "clerk", "wcl": CLERK }),
        ),
        ("propose_agent_delete", json!({ "name": "clerk" })),
    ] {
        let events = api.one(&fake, Some("clerk"), tool, input);
        assert!(find(&events, "agent_proposal").is_none(), "{events:?}");
        assert_eq!(
            last_result(&fake, true)["error"],
            "An agent cannot change its own file."
        );
    }

    // Within its own permissions it proposes, recording itself as the proposer.
    let events = api.one(
        &fake,
        Some("clerk"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": HELPER }),
    );
    let p = last(&events, "agent_proposal").clone();
    assert_eq!(p["proposer"], "clerk");
    assert_eq!(p["agent"], "clerk");

    // Apply checks again: a proposal altered to escalate is refused.
    for (what, wcl, _) in &refused {
        let mut forged = p.clone();
        forged["after"] = json!(wcl);
        let (status, body) = api.apply(&forged);
        assert_eq!(status, StatusCode::FORBIDDEN, "{what}: {body}");
        assert_eq!(body["error"], "not_permitted", "{what}");
        assert!(api.agent_wcl("helper").is_none(), "{what}");
    }
    let mut own = p.clone();
    own["name"] = json!("clerk");
    own["before"] = json!(api.agent_wcl("clerk"));
    let (status, body) = api.apply(&own);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["message"], "An agent cannot change its own file.");

    // Against the proposer as it is now: Clerk lost get_note since, so Helper would have more.
    api.put_agent("clerk", &CLERK.replace("\"get_note\", ", ""));
    let (status, body) = api.apply(&p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("get_note"),
        "{body}"
    );
    api.put_agent("clerk", CLERK);
    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "agent_create");

    // Deleting another agent needs only the tool and the switch.
    let events = api.one(
        &fake,
        Some("clerk"),
        "propose_agent_delete",
        json!({ "name": "helper" }),
    );
    let p = last(&events, "agent_proposal").clone();
    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "agent_delete");
}

#[test]
fn agent_proposal_apply_rechecks_change_permission_escalation() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    api.put_agent("clerk", CLERK);
    let events = api.one(
        &fake,
        Some("clerk"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": HELPER }),
    );
    let proposal = last(&events, "agent_proposal").clone();
    api.put_agent(
        "clerk",
        &CLERK.replace(
            "types = [\"memo\"]",
            "types = [\"memo\"]\nchange_types = []",
        ),
    );
    let (status, result) = api.apply(&proposal);
    assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
    assert!(api.agent_wcl("helper").is_none());
}

#[test]
fn browser_secret_proposal_apply_rechecks_active_names() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let response = api
        .client
        .put(api.server.url("/api/secrets/bank"))
        .bearer_auth(api.token())
        .json(&json!({"kind":"login", "username":"test", "password":"made-up", "origins":[]}))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    api.put_agent(
        "clerk",
        &CLERK.replace(
            "tools = [",
            "browser = true\n browser_secrets = [\"bank\"]\n tools = [",
        ),
    );
    let helper = HELPER.replace(
        "tools = [",
        "browser = true\n browser_secrets = [\"bank\"]\n tools = [",
    );
    let events = api.one(
        &fake,
        Some("clerk"),
        "propose_agent_change",
        json!({"name":"helper", "wcl":helper}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    let response = api
        .client
        .delete(api.server.url("/api/secrets/bank"))
        .bearer_auth(api.token())
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (status, result) = api.apply(&proposal);
    assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
    assert!(api.agent_wcl("helper").is_none());
}

#[test]
fn browser_secrets_apply_narrowing_revokes_removed_confirmations() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    for name in ["bank", "mail"] {
        let response = api
            .client
            .put(api.server.url(&format!("/api/secrets/{name}")))
            .bearer_auth(api.token())
            .json(&json!({"kind":"login", "username":"test", "password":"made-up", "origins":[]}))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    api.put_agent(
        "clerk",
        &CLERK.replace(
            "tools = [",
            "browser = true\n browser_secrets = [\"bank\"]\n tools = [",
        ),
    );
    let broad = HELPER.replace(
        "tools = [",
        "browser = true\n browser_secrets = [\"bank\", \"mail\"]\n tools = [",
    );
    let narrow = broad.replace("[\"bank\", \"mail\"]", "[\"bank\"]");
    api.put_agent("helper", &broad);
    let events = api.one(
        &fake,
        Some("clerk"),
        "propose_agent_change",
        json!({"name":"helper", "wcl":narrow}),
    );
    let (status, result) = api.apply(last(&events, "agent_proposal"));
    assert_eq!(status, StatusCode::OK, "{result}");
    std::fs::write(api.server.vault_path().join("agents/helper.wcl"), broad).unwrap();
    let response = api
        .client
        .get(api.server.url("/api/agents/helper"))
        .bearer_auth(api.token())
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().unwrap()["browser_secrets_inactive"],
        json!(["mail"])
    );
}

#[test]
fn browser_secrets_proposal_events_and_thread_reads_hide_files_for_non_managers() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    api.put_agent("clerk", CLERK);
    api.put_agent(
        "helper",
        &HELPER.replace(
            "tools = [",
            "browser = true\n browser_secrets = [\"bank\"]\n tools = [",
        ),
    );
    let write = api.token_of("writer", "write");
    let read = api.token_of("reader", "read");
    for tool in ["propose_agent_delete", "propose_agent_change"] {
        let response = api
            .client
            .post(api.server.url("/api/agents/clerk/threads"))
            .bearer_auth(&write)
            .json(&json!({}))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let thread = response.json::<Value>().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let input = if tool == "propose_agent_delete" {
            json!({"name":"helper"})
        } else {
            json!({"name":"helper", "wcl":HELPER})
        };
        fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let response = api
            .client
            .post(api.server.url("/api/ai/chat"))
            .bearer_auth(&write)
            .json(&json!({"agent":"clerk", "thread":thread, "messages":[user("Do it.")]}))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let events = read_events(response);
        let proposal = last(&events, "agent_proposal");
        assert!(!proposal["before"].as_str().unwrap().contains("bank"));
        assert_eq!(proposal["browser_secrets_hidden"], true);
        let path = format!("/api/agents/clerk/threads/{thread}");
        let response = api
            .client
            .get(api.server.url(&path))
            .bearer_auth(&read)
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let data = response.json::<Value>().unwrap();
        assert_eq!(data["id"], thread);
        assert_eq!(data["proposals"].as_array().unwrap().len(), 1);
        assert!(
            !data["proposals"][0]["data"]["before"]
                .as_str()
                .unwrap()
                .contains("bank")
        );
        assert_eq!(data["proposals"][0]["data"]["browser_secrets_hidden"], true);
        let response = api
            .client
            .get(api.server.url(&path))
            .bearer_auth(api.token())
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.json::<Value>().unwrap()["proposals"][0]["data"]["before"]
                .as_str()
                .unwrap()
                .contains("bank")
        );
    }
}

#[test]
fn browser_secrets_thread_note_routes_and_events_hide_raw_transcripts() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    api.put_agent("clerk", CLERK);
    api.put_agent(
        "helper",
        &HELPER.replace(
            "tools = [",
            "browser = true\n browser_secrets = [\"bank\"]\n tools = [",
        ),
    );
    let write = api.token_of("writer", "write");
    let read = api.token_of("reader", "read");
    let external = api.token_of("agent-probe", "write");
    let response = api
        .client
        .post(api.server.url("/api/agents/clerk/threads"))
        .bearer_auth(&write)
        .json(&json!({}))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let thread = response.json::<Value>().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_x",
        "propose_agent_change",
        json!({"name":"helper", "wcl":HELPER}),
    ));
    fake.push(AnthropicMessage::text("Done."));
    let response = api
        .client
        .post(api.server.url("/api/ai/chat"))
        .bearer_auth(&write)
        .json(&json!({"agent":"clerk", "thread":thread, "messages":[user("Do it.")]}))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.text().unwrap().contains("bank"));
    let path = format!("/api/notes/{thread}");
    let response = api
        .client
        .get(api.server.url(&path))
        .bearer_auth(api.token())
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.text().unwrap().contains("bank"));
    let history: Value = api
        .client
        .get(api.server.url(&format!("{path}/history")))
        .bearer_auth(api.token())
        .send()
        .unwrap()
        .json()
        .unwrap();
    let mut leaks = Vec::new();
    for (label, token) in [("read", &read), ("write", &write), ("agent", &external)] {
        let mut paths = vec![
            path.clone(),
            "/api/sync/changes?since=0&limit=5000".to_owned(),
            "/api/query?q=from%20agent_thread".to_owned(),
        ];
        for version in history["versions"].as_array().unwrap() {
            paths.push(format!(
                "{path}/history/{}",
                version["version"].as_str().unwrap()
            ));
        }
        for route in paths {
            let response = api
                .client
                .get(api.server.url(&route))
                .bearer_auth(token)
                .send()
                .unwrap();
            if label == "agent" && (route.contains("/history/") || route.starts_with("/api/sync/"))
            {
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
                continue;
            }
            assert_eq!(response.status(), StatusCode::OK, "{label} {route}");
            if response.text().unwrap().contains("bank") {
                leaks.push(format!("{label} {route}"));
            }
        }
    }
    // The model's get_note tool must not recover the private proposal from the same note.
    api.put_agent(
        "clerk",
        &CLERK.replace("types = [\"memo\"]", "types = [\"memo\", \"agent_thread\"]"),
    );
    fake.push(AnthropicMessage::new().tool_use("toolu_read", "get_note", json!({"id":thread})));
    fake.push(AnthropicMessage::text("Read."));
    api.chat_as(&write, Some("clerk"));
    let requests = fake.message_requests();
    let last = requests.last().unwrap().json();
    assert!(last.to_string().contains(&thread));
    if last.to_string().contains("bank") {
        leaks.push("agent get_note".to_owned());
    }
    assert!(leaks.is_empty(), "Login names leaked through {leaks:?}");
}

#[test]
fn browser_secrets_query_filter_oracle_cannot_recover_hidden_names() {
    let mut api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let clerk = CLERK
        .replace("\"get_note\",", "\"query_notes\", \"get_note\",")
        .replace("types = [\"memo\"]", "types = [\"memo\", \"agent_thread\"]");
    api.put_agent("clerk", &clerk);
    api.put_agent(
        "helper",
        &HELPER.replace(
            "tools = [",
            "browser = true\n browser_secrets = [\"bank\"]\n tools = [",
        ),
    );
    let writer = api.token_of("writer", "write");
    let reader = api.token_of("reader", "read");
    let external = api.token_of("agent-oracle", "write");
    let made: Value = api
        .client
        .post(api.server.url("/api/agents/clerk/threads"))
        .bearer_auth(&writer)
        .json(&json!({}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let thread = made["id"].as_str().unwrap();
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_p",
        "propose_agent_change",
        json!({"name":"helper","wcl":HELPER}),
    ));
    fake.push(AnthropicMessage::text("Done."));
    let response = api
        .client
        .post(api.server.url("/api/ai/chat"))
        .bearer_auth(&writer)
        .json(&json!({"agent":"clerk","thread":thread,"messages":[user("Do it.")]}))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    read_events(response);
    let mut leaks = Vec::new();
    for token in [&reader, &writer, &external] {
        for suffix in ["", " sort transcript", " group by transcript"] {
            let mut counts = Vec::new();
            for needle in ["bank", "zzqxv"] {
                let query = format!(
                    "from agent_thread where contains(transcript, {}){suffix}",
                    json!(needle)
                );
                let response = api
                    .client
                    .get(api.server.url("/api/query"))
                    .bearer_auth(token)
                    .query(&[("q", query)])
                    .send()
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                counts.push(response.json::<Value>().unwrap()["total"].clone());
            }
            if counts[0] != counts[1] {
                leaks.push(format!("query {suffix}"));
            }
        }
        let mut counts = Vec::new();
        for needle in ["bank", "zzqxv"] {
            let response = api
                .client
                .get(api.server.url("/api/search"))
                .bearer_auth(token)
                .query(&[("q", needle)])
                .send()
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            counts.push(response.json::<Value>().unwrap()["total"].clone());
        }
        if counts[0] != counts[1] {
            leaks.push("search".to_owned());
        }
    }
    for needle in ["bank", "zzqxv"] {
        let query = format!(
            "from agent_thread where contains(transcript, {})",
            json!(needle)
        );
        let events = api.one(&fake, Some("clerk"), "query_notes", json!({"query":query}));
        if !last(&events, "tool_result")["summary"]["notes"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            leaks.push(format!("query_notes {needle}"));
        }
    }
    let prefix = "browser_secrets = [\\\"b";
    let response: Value = api
        .client
        .get(api.server.url("/api/query"))
        .bearer_auth(&reader)
        .query(&[(
            "q",
            format!(
                "from agent_thread where contains(transcript, {})",
                json!(prefix)
            ),
        )])
        .send()
        .unwrap()
        .json()
        .unwrap();
    if response["total"] != 0 {
        leaks.push("prefix extraction".to_owned());
    }
    assert!(leaks.is_empty(), "hidden-name oracle: {leaks:?}");
    let raw = api
        .client
        .get(api.server.url(&format!("/api/notes/{thread}")))
        .bearer_auth(api.token())
        .send()
        .unwrap();
    let raw: Value = raw.json().unwrap();
    assert!(raw.to_string().contains("bank"));
    // Simulate a pre-upgrade index retaining the raw field at the current generation.
    let db = api.server.vault_path().join(".vault/index.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.busy_timeout(Duration::from_secs(10)).unwrap();
        conn.execute(
            "UPDATE notes SET data=?1 WHERE id=?2",
            rusqlite::params![raw["note"]["fields"].to_string(), thread],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();
    }
    api.server.restart();
    let conn = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 6);
    let data: String = conn
        .query_row("SELECT data FROM notes WHERE id=?1", [thread], |r| r.get(0))
        .unwrap();
    assert!(!data.contains("bank"));
}

#[test]
fn the_switch_off_applies_at_once_for_a_write_caller_and_apply_needs_admin() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "agents": false }));
    let ai = api.get("/api/integrations/ai");
    assert_eq!(ai["confirm"]["agents"], false, "{ai}");

    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({ "name": "helper", "wcl": HELPER }),
    );
    assert!(find(&events, "agent_proposal").is_none(), "{events:?}");
    let applied = last(&events, "applied");
    assert_eq!(applied["op"], "agent_create");
    assert_eq!(applied["agent_name"], "helper");
    assert!(api.agent_wcl("helper").is_some());
    assert_eq!(last_result(&fake, false)["applied"], true);

    // A write caller cannot inherit the Assistant's admin authority.
    for (name, scope) in std::iter::once(("phone", "write")) {
        let write = api.token_of(name, scope);
        fake.push(AnthropicMessage::new().tool_use(
            "toolu_d",
            "propose_agent_delete",
            json!({ "name": "helper" }),
        ));
        fake.push(AnthropicMessage::text("Proposed."));
        let events = api.chat_as(&write, None);
        let p = last(&events, "agent_proposal").clone();
        assert!(find(&events, "applied").is_none(), "{events:?}");
        assert!(api.agent_wcl("helper").is_some());
        let (status, body) = api.apply_as(&write, &p);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "needs_admin");
    }
}

#[test]
fn changed_triggers_wait_for_session_apply_despite_confirmation_and_token_metadata() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({"agents":false}));
    let scheduled = "label = \"Helper\"\n\ntrigger nightly {\n schedule = \"0 2 * * *\"\n prompt = \"Summarise.\"\n}\n";
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({"name":"helper", "wcl":scheduled}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert!(api.agent_wcl("helper").is_none());

    // Neither an ordinary admin token nor an agent token can Apply this sensitive change.
    // A forged client flag and omitted grant metadata do not participate in the decision.
    let mut forged = proposal.clone();
    forged["changes_triggers"] = json!(false);
    forged.as_object_mut().unwrap().remove("grants_run_code");
    for token in [api.token(), api.token_of("agent-trigger-editor", "admin")] {
        let (status, body) = api.apply_as(&token, &forged);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        if token == api.token() {
            assert_eq!(body["error"], "not_permitted");
            assert_eq!(
                body["message"],
                "Agent trigger changes need a signed-in session Apply."
            );
        } else {
            assert_eq!(body["error"], "forbidden");
        }
        assert!(api.agent_wcl("helper").is_none());
    }
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        api.agent_wcl("helper").as_deref(),
        proposal["after"].as_str()
    );
    let scheduled = api.agent_wcl("helper").unwrap();

    // A changed schedule and metadata edits both wait again.
    let rescheduled = scheduled.replace("0 2 * * *", "0 3 * * *");
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({"name":"helper", "wcl":rescheduled}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert_eq!(api.agent_wcl("helper").as_deref(), Some(scheduled.as_str()));
    let (status, body) = api.apply(&proposal);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "not_permitted");
    assert_eq!(
        body["message"],
        "Agent trigger changes need a signed-in session Apply."
    );
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::OK, "{body}");

    let renamed = rescheduled.replace("label = \"Helper\"", "label = \"Renamed\"");
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({"name":"helper", "wcl":renamed}),
    );
    assert!(find(&events, "applied").is_none(), "{events:?}");
    let proposal = last(&events, "agent_proposal").clone();
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::OK, "{body}");

    // Removing a definition is sensitive too, including when the target is disabled.
    let disabled = renamed.replace(
        "label = \"Renamed\"",
        "label = \"Renamed\"\nenabled = false",
    );
    api.put_agent("helper", &disabled);
    let without = "label = \"Renamed\"\nenabled = false\n";
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({"name":"helper", "wcl":without}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    assert!(find(&events, "applied").is_none(), "{events:?}");
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!api.agent_wcl("helper").unwrap().contains("trigger nightly"));
}

#[test]
fn semantic_edits_of_disabled_triggered_agents_wait_and_session_apply_is_thread_bound() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({"agents":false}));
    let original = "label = \"Helper\"\nenabled = false\ntrigger nightly {\n schedule = \"0 2 * * *\"\n prompt = \"Read.\"\n}\n";
    for field in [
        "description = \"Changed\"\n",
        "icon = \"star\"\n",
        "system_prompt = \"Changed\"\n",
        "permissions { tools = []\n}\n",
    ] {
        api.put_agent("helper", original);
        let before = api.agent_wcl("helper").unwrap();
        let after = format!("{field}{original}");
        let events = api.one(
            &fake,
            None,
            "propose_agent_change",
            json!({"name":"helper", "wcl":after}),
        );
        let p = last(&events, "agent_proposal");
        assert!(find(&events, "applied").is_none(), "{events:?}");
        let (status, body) = api.apply(p);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "not_permitted");
        assert_eq!(
            body["message"],
            "Agent trigger changes need a signed-in session Apply."
        );
        assert_eq!(api.agent_wcl("helper").as_deref(), Some(before.as_str()));

        // Use a fresh durable proposal, not a proposal consumed by a refused Apply.
        let response = api
            .client
            .post(api.server.url("/api/agents/assistant/threads"))
            .bearer_auth(api.token())
            .json(&json!({}))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let thread = response.json::<Value>().unwrap()["id"].clone();
        fake.push(AnthropicMessage::new().tool_use(
            "semantic_edit",
            "propose_agent_change",
            json!({"name":"helper", "wcl":after}),
        ));
        fake.push(AnthropicMessage::text("Waiting."));
        let response = api
            .client
            .post(api.server.url("/api/ai/chat"))
            .bearer_auth(api.token())
            .json(&json!({"thread":thread, "messages":[user("Edit helper.")]}))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let events = read_events(response);
        let p = last(&events, "agent_proposal");
        let mut bound = p.clone();
        bound["thread"] = thread;
        let (status, body) = api.apply_session(&bound);
        assert_eq!(status, StatusCode::OK, "{body}");
        let committed = api.agent_wcl("helper").unwrap();
        let (status, body) = api.apply_session(&bound);
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(api.agent_wcl("helper").as_deref(), Some(committed.as_str()));
    }
}

#[test]
fn enabling_and_deleting_triggered_agents_need_session_apply() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({"agents":false}));
    let disabled = "label = \"Helper\"\nenabled = false\ntrigger nightly {\n schedule = \"0 2 * * *\"\n prompt = \"Summarise.\"\n}\n";
    api.put_agent("helper", disabled);
    let before = api.agent_wcl("helper").unwrap();
    let enabled = before.replace("enabled = false", "enabled = true");
    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({"name":"helper", "wcl":enabled}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert_eq!(api.agent_wcl("helper").as_deref(), Some(before.as_str()));
    let (status, body) = api.apply(&proposal);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "not_permitted");
    assert_eq!(
        body["message"],
        "Agent trigger changes need a signed-in session Apply."
    );
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        api.agent_wcl("helper").as_deref(),
        proposal["after"].as_str()
    );

    let before = api.agent_wcl("helper").unwrap();
    let events = api.one(
        &fake,
        None,
        "propose_agent_delete",
        json!({"name":"helper"}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert_eq!(api.agent_wcl("helper").as_deref(), Some(before.as_str()));
    // The delete writer ignores after, so sensitivity must ignore it too.
    let mut forged = proposal.clone();
    forged["after"] = json!(before);
    for change in [&proposal, &forged] {
        let (status, body) = api.apply(change);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "not_permitted");
        assert_eq!(
            body["message"],
            "Agent trigger changes need a signed-in session Apply."
        );
        assert_eq!(api.agent_wcl("helper").as_deref(), Some(before.as_str()));
    }
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(api.agent_wcl("helper").is_none());
}

#[test]
fn changed_note_trigger_filter_and_action_wait_for_session_apply_and_stay_stale_checked() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({"agents":false}));
    let original = "label = \"Helper\"\n\ntrigger arrived {\n on = [\"create\"]\n type = \"memo\"\n where = \"status == \\\"open\\\"\"\n prompt = \"Read.\"\n}\n";
    api.put_agent("helper", original);
    let original = api.agent_wcl("helper").unwrap();
    for changed in [
        original.replace("status == \\\"open\\\"", "status == \\\"closed\\\""),
        original.replace("prompt = \"Read.\"", "prompt = \"Summarise.\""),
    ] {
        let events = api.one(
            &fake,
            None,
            "propose_agent_change",
            json!({"name":"helper", "wcl":changed}),
        );
        let proposal = last(&events, "agent_proposal").clone();
        assert!(find(&events, "applied").is_none(), "{events:?}");
        let (status, body) = api.apply(&proposal);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "not_permitted");
        assert_eq!(
            body["message"],
            "Agent trigger changes need a signed-in session Apply."
        );
        let (status, body) = api.apply_session(&proposal);
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let events = api.one(
        &fake,
        None,
        "propose_agent_change",
        json!({"name":"helper", "wcl":original.replace("prompt = \"Read.\"", "prompt = \"New action.\"")}),
    );
    let proposal = last(&events, "agent_proposal").clone();
    api.put_agent("helper", &original);
    let (status, body) = api.apply_session(&proposal);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert_eq!(api.agent_wcl("helper").as_deref(), Some(original.as_str()));
}

#[test]
fn an_unattended_run_is_offered_the_propose_tools_only_with_the_switch_off() {
    let api = Api::start();
    api.put_agent("clerk", CLERK);
    let runs = api.get("/api/agents/clerk/runs");
    let tools: Vec<&str> = runs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(tools.contains(&"list_agents"), "{tools:?}");
    assert!(tools.contains(&"get_agent"), "{tools:?}");
    assert!(!tools.contains(&"propose_agent_change"), "{tools:?}");
    assert!(!tools.contains(&"propose_agent_delete"), "{tools:?}");

    api.put_agent(
        "clerk",
        &CLERK.replace("  web {", "  confirm {\n    agents = false\n  }\n\n  web {"),
    );
    let runs = api.get("/api/agents/clerk/runs");
    let tools: Vec<&str> = runs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(tools.contains(&"propose_agent_change"), "{tools:?}");
    assert!(tools.contains(&"propose_agent_delete"), "{tools:?}");
}
