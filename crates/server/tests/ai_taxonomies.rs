//! S1 with the fake Anthropic provider: the chat's taxonomy tools (ai D31). `list_taxonomies`,
//! `get_taxonomy` and `taxonomy_format` read; `propose_taxonomy_change` and
//! `propose_taxonomy_delete` check without writing and emit `taxonomy_proposal`; `POST
//! /api/ai/apply` applies one, renames moving the notes' terms, and answers a write-scope caller
//! 403 `needs_admin`; `confirm.taxonomies` off applies at once for an admin caller.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic, TestServer};

const TASK: &str = r#"label = "Task"

field stage {
  kind     = "taxonomy"
  taxonomy = "stage"
}

template default {
  tab "Main" {
    slot "title"
    slot "stage"
  }
}
"#;

const STAGE: &str = r#"label       = "Stage"
description = "Where a task is"

term "Todo"

term "Doing" {
  term "Blocked"
}

term "Done"
"#;

const SHADE: &str = r#"label = "Shade"

term "Light"

term "Dark"
"#;

const MODEL: &str = "claude-opus-5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(|vault| {
            for dir in ["types", "taxonomies"] {
                std::fs::create_dir_all(vault.join(dir)).unwrap();
            }
            std::fs::write(vault.join("types/task.wcl"), TASK).unwrap();
            std::fs::write(vault.join("taxonomies/stage.wcl"), STAGE).unwrap();
            std::fs::write(vault.join("taxonomies/shade.wcl"), SHADE).unwrap();
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

    /// The fake as the default provider, with every confirmation switch as stored.
    fn anthropic(&self, fake: &FakeAnthropic, confirm: Value) {
        self.secret("ai_anthropic_api_key", fake.api_key());
        self.secret(
            "ai_settings",
            &json!({
                "default_provider": "anthropic",
                "models": { "anthropic": MODEL },
                "endpoints": { "anthropic": fake.base_url() },
            })
            .to_string(),
        );
        // The chat's switches are the built-in Assistant's (agents G6).
        let mut all = json!({
            "notes": true, "archive": true, "delete": true,
            "types": true, "downloads": true, "taxonomies": true, "agents": true,
        });
        for (k, v) in confirm.as_object().into_iter().flatten() {
            all[k] = v.clone();
        }
        let res = self
            .client
            .put(self.server.url("/api/integrations/ai/settings"))
            .bearer_auth(self.token())
            .json(&json!({ "confirm": all }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn task(&self, title: &str, stage: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/notes?type=task"))
            .bearer_auth(self.token())
            .json(&json!({ "note": { "fields": { "title": title, "stage": stage } } }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED, "{}", res.text().unwrap());
        let v: Value = res.json().unwrap();
        v["id"].as_str().unwrap().to_owned()
    }

    fn stage_of(&self, id: &str) -> Value {
        let v: Value = self
            .client
            .get(self.server.url(&format!("/api/notes/{id}")))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .json()
            .unwrap();
        v["note"]["fields"]["stage"].clone()
    }

    /// `taxonomies/<name>.wcl` as the taxonomies API reads it, or `None` when there is none.
    fn taxonomy_wcl(&self, name: &str) -> Option<String> {
        let res = self
            .client
            .get(self.server.url(&format!("/api/taxonomies/{name}")))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        if !res.status().is_success() {
            return None;
        }
        let v: Value = res.json().unwrap();
        Some(v["wcl"].as_str().unwrap().to_owned())
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

    fn chat_as(&self, token: &str, messages: Value) -> Vec<(String, Value)> {
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(token)
            .json(&json!({ "messages": messages }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    /// One tool call as `token`, then an answer: the turn's events.
    fn one_as(
        &self,
        token: &str,
        fake: &FakeAnthropic,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(token, json!([user("Do it.")]))
    }

    fn one(&self, fake: &FakeAnthropic, tool: &str, input: Value) -> Vec<(String, Value)> {
        self.one_as(&self.token(), fake, tool, input)
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

/// The event names, `usage` left out and each run of `text` folded into one.
fn shape(events: &[(String, Value)]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for (n, _) in events {
        if n == "usage" || (n == "text" && out.last() == Some(&"text")) {
            continue;
        }
        out.push(n);
    }
    out
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

/// The tool result the model got back in request `request`, as JSON, after checking its flag.
fn result_in(fake: &FakeAnthropic, request: usize, is_error: bool) -> Value {
    let r = fake.message_requests()[request].json();
    let result = r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone();
    assert_eq!(result["is_error"], is_error, "{result}");
    serde_json::from_str(result["content"].as_str().unwrap()).unwrap()
}

const RENAMED: &str = r#"label       = "Stage"
description = "Where a task is"

term "Open"

term "Doing" {
  term "Blocked"
}

term "Done"
"#;

#[test]
fn taxonomy_tools_list_read_and_a_change_with_a_rename_moves_the_note_after_apply() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let id = api.task("Paint", "Todo");
    let before = api.taxonomy_wcl("stage").unwrap();
    fake.push(AnthropicMessage::new().tool_use("toolu_l", "list_taxonomies", json!({})));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_g",
        "get_taxonomy",
        json!({ "name": "stage" }),
    ));
    fake.push(AnthropicMessage::new().tool_use("toolu_f", "taxonomy_format", json!({})));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "propose_taxonomy_change",
        json!({ "name": "stage", "wcl": RENAMED, "renames": [{ "from": "Todo", "to": "Open" }] }),
    ));
    fake.push(AnthropicMessage::text("I proposed renaming Todo."));
    let events = api.chat_as(&api.token(), json!([user("Call Todo Open.")]));
    assert_eq!(
        shape(&events),
        [
            "tool",
            "tool_result",
            "tool",
            "tool_result",
            "tool",
            "tool_result",
            "tool",
            "taxonomy_proposal",
            "tool_result",
            "text",
            "done"
        ],
        "{events:?}"
    );

    // list_taxonomies: each with its label, openness, built-in mark and term count.
    let listed = result_in(&fake, 1, false);
    let all = listed["taxonomies"].as_array().unwrap();
    let stage = all.iter().find(|t| t["name"] == "stage").unwrap();
    assert_eq!(stage["label"], "Stage");
    assert_eq!(stage["description"], "Where a task is");
    assert_eq!(stage["open"], false);
    assert_eq!(stage["builtin"], false);
    assert_eq!(stage["terms"], 4);
    assert!(all.iter().any(|t| t["builtin"] == true), "{listed}");

    // get_taxonomy: the stored WCL and the terms as paths.
    let got = result_in(&fake, 2, false);
    assert_eq!(got["wcl"], before.as_str());
    assert_eq!(
        got["terms"],
        json!(["Todo", "Doing", "Doing/Blocked", "Done"])
    );

    // taxonomy_format: the taxonomy-file part of docs/vault-format.md.
    let format = result_in(&fake, 3, false);
    let format = format["format"].as_str().unwrap();
    assert!(format.starts_with("## Taxonomy files"), "{format}");

    let p = last(&events, "taxonomy_proposal").clone();
    assert_eq!(p["id"], "toolu_t");
    assert_eq!(p["kind"], "taxonomy");
    assert_eq!(p["op"], "change");
    assert_eq!(p["name"], "stage");
    assert_eq!(p["exists"], true);
    assert_eq!(p["before"], before.as_str());
    assert!(
        p["after"].as_str().unwrap().contains("term \"Open\""),
        "{p}"
    );
    assert_eq!(p["renames"], json!([{ "from": "Todo", "to": "Open" }]));
    assert_eq!(p["affected"], 1);
    assert_eq!(p["issues"], json!([]));

    // The model is told it awaits the user; nothing was written.
    let told = result_in(&fake, 4, false);
    assert!(
        told["message"]
            .as_str()
            .unwrap()
            .contains("awaits the user")
    );
    assert_eq!(api.taxonomy_wcl("stage").unwrap(), before);
    assert_eq!(api.stage_of(&id), "Todo");

    // A write-scope caller cannot apply it.
    let writer = api.token_of("writer", "write");
    let (status, err) = api.apply_as(&writer, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["error"], "needs_admin");
    assert_eq!(err["message"], "Apply taxonomy changes in the web app.");
    assert_eq!(api.stage_of(&id), "Todo");

    // Applied: the file changes and the note's term moves with the rename.
    let (status, done) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        done,
        json!({ "op": "taxonomy_change", "taxonomy_name": "stage", "notes": 1 })
    );
    assert!(api.taxonomy_wcl("stage").unwrap().contains("term \"Open\""));
    assert_eq!(api.stage_of(&id), "Open");

    // Its before is no longer the file.
    let (status, err) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["error"], "stale");
}

#[test]
fn a_new_taxonomy_name_proposes_a_new_taxonomy() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let size = "label = \"Size\"\n\nterm \"Small\"\n\nterm \"Large\"\n";
    let events = api.one(
        &fake,
        "propose_taxonomy_change",
        json!({ "name": "size", "wcl": size, "renames": [] }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "taxonomy_proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "taxonomy_proposal").clone();
    assert_eq!(p["op"], "create");
    assert_eq!(p["exists"], false);
    assert_eq!(p["before"], Value::Null);
    assert_eq!(p["affected"], 0);
    assert_eq!(api.taxonomy_wcl("size"), None);

    let (status, done) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "taxonomy_create");
    assert!(api.taxonomy_wcl("size").unwrap().contains("term \"Large\""));
}

#[test]
fn invalid_wcl_and_a_dropped_held_term_are_errors_and_propose_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    api.task("Paint", "Todo");
    let events = api.one(
        &fake,
        "propose_taxonomy_change",
        json!({ "name": "stage", "wcl": "term {", "renames": [] }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    let content = result_in(&fake, 1, true);
    assert!(
        content["message"]
            .as_str()
            .unwrap()
            .contains("not proposed"),
        "{content}"
    );

    // Dropping Todo, which a note holds, without a rename.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let events = api.one(
        &fake,
        "propose_taxonomy_change",
        json!({ "name": "stage", "wcl": RENAMED, "renames": [] }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    let content = result_in(&fake, 1, true);
    assert_eq!(content["issues"]["error"], "term_in_use", "{content}");
    assert_eq!(content["issues"]["notes"][0]["title"], "Paint", "{content}");
    assert!(api.taxonomy_wcl("stage").unwrap().contains("term \"Todo\""));
}

#[test]
fn an_unused_taxonomy_is_proposed_for_delete_and_one_in_use_is_refused() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let events = api.one(&fake, "propose_taxonomy_delete", json!({ "name": "shade" }));
    assert_eq!(
        shape(&events),
        ["tool", "taxonomy_proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "taxonomy_proposal").clone();
    assert_eq!(p["kind"], "taxonomy");
    assert_eq!(p["op"], "delete");
    assert_eq!(p["name"], "shade");
    assert_eq!(p["before"], SHADE);
    assert!(api.taxonomy_wcl("shade").is_some());
    let (status, done) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        done,
        json!({ "op": "taxonomy_delete", "taxonomy_name": "shade", "notes": 0 })
    );
    assert_eq!(api.taxonomy_wcl("shade"), None);

    // stage is used by the task type's field.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    let events = api.one(&fake, "propose_taxonomy_delete", json!({ "name": "stage" }));
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    let content = result_in(&fake, 1, true);
    assert_eq!(content["issues"]["error"], "taxonomy_in_use", "{content}");
    assert!(content.to_string().contains("types/task.wcl"), "{content}");
    assert!(api.taxonomy_wcl("stage").is_some());
}

#[test]
fn with_taxonomy_confirmation_off_an_admin_change_applies_at_once_and_a_writer_still_proposes() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({}));
    // The switch is on by default and takes a partial change.
    let res = api
        .client
        .put(api.server.url("/api/integrations/ai/settings"))
        .bearer_auth(api.token())
        .json(&json!({ "confirm": { "taxonomies": false } }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: Value = res.json().unwrap();
    assert_eq!(v["confirm"]["taxonomies"], false);
    assert_eq!(v["confirm"]["types"], true);

    let id = api.task("Paint", "Todo");
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "taxonomies": false }));
    let events = api.one(
        &fake,
        "propose_taxonomy_change",
        json!({ "name": "stage", "wcl": RENAMED, "renames": [{ "from": "Todo", "to": "Open" }] }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    assert_eq!(a["id"], "toolu_x");
    assert_eq!(a["op"], "taxonomy_change");
    assert_eq!(a["taxonomy_name"], "stage");
    assert_eq!(api.stage_of(&id), "Open");
    let told = result_in(&fake, 1, false);
    assert_eq!(told["applied"], true);

    // A delete goes at once too.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "taxonomies": false }));
    let events = api.one(&fake, "propose_taxonomy_delete", json!({ "name": "shade" }));
    assert_eq!(last(&events, "applied")["op"], "taxonomy_delete");
    assert_eq!(api.taxonomy_wcl("shade"), None);

    // A write-scope caller lacks admin, so it gets a proposal whatever the switch says.
    let writer = api.token_of("writer", "write");
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "taxonomies": false }));
    let size = "label = \"Size\"\n\nterm \"Small\"\n";
    let events = api.one_as(
        &writer,
        &fake,
        "propose_taxonomy_change",
        json!({ "name": "size", "wcl": size, "renames": [] }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "taxonomy_proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(api.taxonomy_wcl("size"), None);
}
