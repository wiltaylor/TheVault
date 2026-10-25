//! S1 with the fake Anthropic provider: the chat's agent tools (agents G20). `list_agents` and
//! `get_agent` read; `propose_agent_change` and `propose_agent_delete` check without writing and
//! emit `agent_proposal` with its `proposer`; `POST /api/ai/apply` applies one, refuses a stale
//! one, checks it again against the proposing agent, and answers a write-scope caller 403
//! `needs_admin`; a change that would give an agent more than the proposing agent has, or touch
//! its own file, is refused on propose and on apply; `confirm.agents` off applies at once for an
//! admin caller; an unattended run with the switch on is not offered the propose tools.

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
fn the_switch_off_applies_at_once_for_an_admin_and_apply_needs_admin() {
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

    // A write token's turn only proposes, and its apply is refused.
    let write = api.token_of("phone", "write");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_d",
        "propose_agent_delete",
        json!({ "name": "helper" }),
    ));
    fake.push(AnthropicMessage::text("Proposed."));
    let events = api.chat_as(&write, None);
    let p = last(&events, "agent_proposal").clone();
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    assert_eq!(body["message"], "Apply agent changes in the web app.");
    assert!(api.agent_wcl("helper").is_some());

    // Restore the switch for the rest.
    api.anthropic(&fake, json!({ "agents": true }));
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
