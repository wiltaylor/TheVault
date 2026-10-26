//! S1 with the fake Anthropic provider: agents that manage skills (skills S9). `list_skills` and
//! `get_skill` read; `propose_skill_change`, `propose_skill_file_change`,
//! `propose_skill_file_delete` and `propose_skill_delete` check without writing and emit
//! `skill_proposal`; `POST /api/ai/apply` applies one through the skill writes, answers a stale
//! one 409 and a write-scope caller 403 `needs_admin`; an invalid `SKILL.md`, `SKILL.md` on the
//! file tool and a path outside the skill are refused; `confirm.skills` off applies at once for an
//! admin or write caller, a read caller still getting the proposal; an unattended run with the switch on is not offered the propose tools; `use_skill`
//! opens a skill created in the thread, outside its snapshot.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

const BRAND: &str =
    "---\nname: brand\ndescription: The house style for letters.\n---\n\nUse the logo.\n";

const PDF: &str = "---\nname: pdf-tools\ndescription: Fills PDF forms. Use when a PDF form arrives.\n---\n\n# PDF tools\n\nRun scripts/fill.py.\n";

/// An agent with the skill tools in its list, every confirmation on.
const CURATOR: &str = r#"label = "Curator"

permissions {
  tools = ["list_skills", "get_skill", "propose_skill_change", "propose_skill_file_change", "propose_skill_file_delete", "propose_skill_delete", "use_skill"]
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic, confirm: Value) -> Api {
        let server = TestServer::start_seeded(|_| {});
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.secret(&name, &value);
        }
        api.confirm(confirm);
        api
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

    /// Assistant's confirmation switches.
    fn confirm(&self, confirm: Value) {
        let res = self
            .client
            .put(self.server.url("/api/integrations/ai/settings"))
            .bearer_auth(self.token())
            .json(&json!({ "confirm": confirm }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn put_raw(&self, path: &str, content_type: &str, body: Vec<u8>) -> Response {
        self.client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", content_type)
            .body(body)
            .send()
            .unwrap()
    }

    fn put_agent(&self, name: &str, wcl: &str) {
        let res = self.put_raw(&format!("/api/agents/{name}"), "text/plain", wcl.into());
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn skill(&self, name: &str, md: &str) {
        let res = self.put_raw(
            &format!("/api/skills/{name}"),
            "text/markdown; charset=utf-8",
            md.into(),
        );
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn status(&self, path: &str) -> StatusCode {
        self.client
            .get(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .status()
    }

    fn bytes(&self, path: &str) -> Vec<u8> {
        let res = self
            .client
            .get(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{path}");
        res.bytes().unwrap().to_vec()
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

    fn chat_as(&self, token: &str, body: Value) -> Vec<(String, Value)> {
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

    /// One tool call by Assistant as the admin, then an answer: the turn's events.
    fn one(&self, fake: &FakeAnthropic, tool: &str, input: Value) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(&self.token(), json!({ "messages": [user("Do it.")] }))
    }

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

    /// Propose with `tool` and apply the proposal: what the apply answered.
    fn propose_and_apply(&self, fake: &FakeAnthropic, tool: &str, input: Value) -> (Value, Value) {
        let events = self.one(fake, tool, input);
        let p = last(&events, "skill_proposal").clone();
        assert_eq!(p["kind"], "skill");
        let (status, body) = self.apply(&p);
        assert_eq!(status, StatusCode::OK, "{body}");
        (p, body)
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
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

/// The tool result the model got back in the last request, as JSON, after checking its flag.
fn last_result(fake: &FakeAnthropic, is_error: bool) -> Value {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    let result = r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone();
    assert_eq!(result["is_error"], is_error, "{result}");
    serde_json::from_str(result["content"].as_str().unwrap()).unwrap()
}

fn tools_of(runs: &Value) -> Vec<String> {
    runs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

#[test]
fn skill_tools_list_read_and_propose_every_change_that_applies() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, json!({}));
    api.skill("brand", BRAND);

    // Assistant has every tool, so the skill tools are offered.
    api.one(&fake, "list_skills", json!({}));
    let tools: Vec<String> = fake.message_requests()[0].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    for t in [
        "list_skills",
        "get_skill",
        "propose_skill_change",
        "propose_skill_file_change",
        "propose_skill_file_delete",
        "propose_skill_delete",
    ] {
        assert!(tools.iter().any(|n| n == t), "{t} in {tools:?}");
    }
    let listed = last_result(&fake, false);
    assert_eq!(listed["skills"][0]["name"], "brand", "{listed}");
    assert_eq!(listed["skills"][0]["valid"], true);
    assert_eq!(listed["skills"][0]["offered"], true);

    api.one(&fake, "get_skill", json!({ "name": "brand" }));
    let got = last_result(&fake, false);
    assert_eq!(got["skill_md"], BRAND);
    assert_eq!(got["files"][0]["path"], "SKILL.md");
    assert!(got["format"].as_str().unwrap().contains("frontmatter"));

    // Create: proposed, nothing written until applied.
    let events = api.one(
        &fake,
        "propose_skill_change",
        json!({ "name": "pdf-tools", "skill_md": PDF }),
    );
    let p = last(&events, "skill_proposal").clone();
    assert_eq!(p["op"], "create", "{p}");
    assert_eq!(p["name"], "pdf-tools");
    assert!(p["before"].is_null());
    assert_eq!(p["after"], PDF);
    let result = last_result(&fake, false);
    assert_eq!(result["proposed"], true);
    assert!(
        result["message"].as_str().unwrap().contains("use_skill"),
        "{result}"
    );
    assert_eq!(api.status("/api/skills/pdf-tools"), StatusCode::NOT_FOUND);
    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        done,
        json!({ "op": "skill_create", "skill_name": "pdf-tools" })
    );
    assert_eq!(api.get("/api/skills/pdf-tools")["valid"], true);

    // Change SKILL.md: the diff's before is the text as stored.
    let changed = PDF.replace("Run scripts/fill.py.", "Run scripts/fill.py with the form.");
    let (p, done) = api.propose_and_apply(
        &fake,
        "propose_skill_change",
        json!({ "name": "pdf-tools", "skill_md": changed }),
    );
    assert_eq!(p["op"], "change");
    assert_eq!(p["before"], PDF);
    assert_eq!(done["op"], "skill_change");
    assert_eq!(
        api.bytes("/api/skills/pdf-tools/files/SKILL.md"),
        changed.as_bytes()
    );

    // A text file.
    let (p, done) = api.propose_and_apply(
        &fake,
        "propose_skill_file_change",
        json!({ "name": "pdf-tools", "path": "scripts/fill.py", "text": "print('filled')\n", "base64": null }),
    );
    assert_eq!(p["op"], "file");
    assert_eq!(p["binary"], false);
    assert!(p["before"].is_null());
    assert_eq!(p["after"], "print('filled')\n");
    assert_eq!(p["media_type"], "text/x-python");
    assert_eq!(done["op"], "skill_file");
    assert_eq!(done["path"], "scripts/fill.py");
    assert_eq!(
        api.bytes("/api/skills/pdf-tools/files/scripts/fill.py"),
        b"print('filled')\n"
    );

    // A binary file, as base64: shown by size and type.
    let blob = vec![0u8, 1, 2, 255, 254];
    let (p, done) = api.propose_and_apply(
        &fake,
        "propose_skill_file_change",
        json!({ "name": "pdf-tools", "path": "assets/blank.bin", "text": null, "base64": STANDARD.encode(&blob) }),
    );
    assert_eq!(p["binary"], true, "{p}");
    assert!(p["after"].is_null());
    assert_eq!(p["size"], 5);
    assert_eq!(p["media_type"], "application/octet-stream");
    assert_eq!(done["op"], "skill_file");
    assert_eq!(
        api.bytes("/api/skills/pdf-tools/files/assets/blank.bin"),
        blob
    );

    // Delete a folder of files.
    let (p, done) = api.propose_and_apply(
        &fake,
        "propose_skill_file_delete",
        json!({ "name": "pdf-tools", "path": "scripts" }),
    );
    assert_eq!(p["op"], "file_delete");
    assert_eq!(p["files"], json!(["scripts/fill.py"]));
    assert_eq!(done["op"], "skill_file_delete");
    assert_eq!(
        api.status("/api/skills/pdf-tools/files/scripts/fill.py"),
        StatusCode::NOT_FOUND
    );

    // Delete the skill.
    let (p, done) = api.propose_and_apply(
        &fake,
        "propose_skill_delete",
        json!({ "name": "pdf-tools" }),
    );
    assert_eq!(p["op"], "delete");
    assert_eq!(p["before"], changed);
    assert_eq!(done["op"], "skill_delete");
    assert_eq!(api.status("/api/skills/pdf-tools"), StatusCode::NOT_FOUND);

    // Each write is a skill.change entry, and each apply an ai.applied one.
    let log = api.get("/api/activity?kind=skill.change&limit=50");
    assert_eq!(log["items"].as_array().unwrap().len(), 7, "{log}");
    let log = api.get("/api/activity?kind=ai.applied&limit=50");
    assert_eq!(log["items"].as_array().unwrap().len(), 6, "{log}");
}

#[test]
fn bad_skill_changes_are_refused_with_their_issues() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, json!({}));
    api.skill("brand", BRAND);

    // A SKILL.md naming another skill, with no description.
    let events = api.one(
        &fake,
        "propose_skill_change",
        json!({ "name": "pdf-tools", "skill_md": "---\nname: other\n---\nBody\n" }),
    );
    assert!(find(&events, "skill_proposal").is_none(), "{events:?}");
    let r = last_result(&fake, true);
    let issues = r["issues"]["issues"].as_array().expect("issues");
    assert!(issues.iter().any(|i| i["at"] == "name"), "{r}");
    assert!(issues.iter().any(|i| i["at"] == "description"), "{r}");
    assert_eq!(api.status("/api/skills/pdf-tools"), StatusCode::NOT_FOUND);

    // SKILL.md goes through propose_skill_change.
    let events = api.one(
        &fake,
        "propose_skill_file_change",
        json!({ "name": "brand", "path": "SKILL.md", "text": BRAND, "base64": null }),
    );
    assert!(find(&events, "skill_proposal").is_none());
    let r = last_result(&fake, true);
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("propose_skill_change"),
        "{r}"
    );

    // A path outside the skill.
    for path in ["../agents/x.wcl", "/etc/passwd", "a/../../b"] {
        let events = api.one(
            &fake,
            "propose_skill_file_change",
            json!({ "name": "brand", "path": path, "text": "x", "base64": null }),
        );
        assert!(find(&events, "skill_proposal").is_none(), "{path}");
        last_result(&fake, true);
    }

    // Both text and base64, or neither.
    for (text, b64) in [(json!("x"), json!("eA==")), (Value::Null, Value::Null)] {
        api.one(
            &fake,
            "propose_skill_file_change",
            json!({ "name": "brand", "path": "a.txt", "text": text, "base64": b64 }),
        );
        let r = last_result(&fake, true);
        assert!(r["error"].as_str().unwrap().contains("exactly one"), "{r}");
    }

    // SKILL.md cannot be deleted on its own.
    api.one(
        &fake,
        "propose_skill_file_delete",
        json!({ "name": "brand", "path": "SKILL.md" }),
    );
    last_result(&fake, true);
}

#[test]
fn a_stale_apply_is_refused_and_writes_nothing() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, json!({}));
    api.skill("brand", BRAND);
    let events = api.one(
        &fake,
        "propose_skill_change",
        json!({ "name": "brand", "skill_md": BRAND.replace("Use the logo.", "Use the new logo.") }),
    );
    let p = last(&events, "skill_proposal").clone();
    let edited = BRAND.replace("Use the logo.", "Edited by hand.");
    api.skill("brand", &edited);
    let (status, body) = api.apply(&p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert_eq!(
        api.bytes("/api/skills/brand/files/SKILL.md"),
        edited.as_bytes()
    );

    // A file proposal for a new path goes stale when the path is written meanwhile.
    let events = api.one(
        &fake,
        "propose_skill_file_change",
        json!({ "name": "brand", "path": "notes.md", "text": "One\n", "base64": null }),
    );
    let p = last(&events, "skill_proposal").clone();
    let res = api.put_raw(
        "/api/skills/brand/files/notes.md",
        "application/octet-stream",
        b"Two\n".to_vec(),
    );
    assert!(res.status().is_success());
    let (status, _) = api.apply(&p);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(api.bytes("/api/skills/brand/files/notes.md"), b"Two\n");
}

#[test]
fn the_switch_off_applies_at_once_for_a_write_caller_and_apply_needs_admin() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, json!({ "skills": false }));
    let ai = api.get("/api/integrations/ai");
    assert_eq!(ai["confirm"]["skills"], false, "{ai}");
    assert_eq!(ai["confirm"]["agents"], true, "{ai}");

    let events = api.one(
        &fake,
        "propose_skill_change",
        json!({ "name": "pdf-tools", "skill_md": PDF }),
    );
    assert!(find(&events, "skill_proposal").is_none(), "{events:?}");
    let applied = last(&events, "applied");
    assert_eq!(applied["op"], "skill_create");
    assert_eq!(applied["skill_name"], "pdf-tools");
    assert_eq!(api.get("/api/skills/pdf-tools")["valid"], true);
    let r = last_result(&fake, false);
    assert_eq!(r["applied"], true);
    assert!(r["message"].as_str().unwrap().contains("use_skill"), "{r}");

    // A read token's turn only proposes, whatever the switch says, and a write token's apply of a
    // proposal no thread holds is refused.
    let read = api.token_of("reader", "read");
    let write = api.token_of("phone", "write");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_d",
        "propose_skill_delete",
        json!({ "name": "pdf-tools" }),
    ));
    fake.push(AnthropicMessage::text("Proposed."));
    let events = api.chat_as(&read, json!({ "messages": [user("Delete it.")] }));
    let p = last(&events, "skill_proposal").clone();
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    assert_eq!(body["message"], "Apply skill changes in the web app.");
    assert_eq!(api.status("/api/skills/pdf-tools"), StatusCode::OK);

    // A write token's turn, such as an app's, has the agent's own rights (agents G4): the
    // Assistant may change skills, so with the switch off the delete is made.
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_w",
        "propose_skill_delete",
        json!({ "name": "pdf-tools" }),
    ));
    fake.push(AnthropicMessage::text("Deleted."));
    let events = api.chat_as(&write, json!({ "messages": [user("Delete it.")] }));
    assert!(find(&events, "skill_proposal").is_none(), "{events:?}");
    assert_eq!(last(&events, "applied")["op"], "skill_delete");
    assert_eq!(api.status("/api/skills/pdf-tools"), StatusCode::NOT_FOUND);
}

#[test]
fn an_unattended_run_is_offered_the_propose_tools_only_with_the_switch_off() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, json!({}));
    api.put_agent("curator", CURATOR);
    let tools = tools_of(&api.get("/api/agents/curator/runs"));
    assert!(tools.iter().any(|t| t == "list_skills"), "{tools:?}");
    assert!(tools.iter().any(|t| t == "get_skill"), "{tools:?}");
    for t in [
        "propose_skill_change",
        "propose_skill_file_change",
        "propose_skill_file_delete",
        "propose_skill_delete",
    ] {
        assert!(!tools.iter().any(|n| n == t), "{t} in {tools:?}");
    }

    api.put_agent(
        "curator",
        &CURATOR.replace("\n}\n", "\n\n  confirm {\n    skills = false\n  }\n}\n"),
    );
    let tools = tools_of(&api.get("/api/agents/curator/runs"));
    for t in [
        "propose_skill_change",
        "propose_skill_file_change",
        "propose_skill_file_delete",
        "propose_skill_delete",
    ] {
        assert!(tools.iter().any(|n| n == t), "{t} in {tools:?}");
    }
}

#[test]
fn use_skill_opens_a_skill_created_in_the_thread_outside_its_snapshot() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, json!({}));
    api.skill("brand", BRAND);
    api.put_agent(
        "curator",
        &CURATOR.replace("\n}\n", "\n\n  confirm {\n    skills = false\n  }\n}\n"),
    );
    let thread = api.get_thread("curator");
    let body =
        |text: &str| json!({ "agent": "curator", "thread": thread, "messages": [user(text)] });

    // The first turn creates the skill at once: its snapshot lists only brand.
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_c",
        "propose_skill_change",
        json!({ "name": "pdf-tools", "skill_md": PDF }),
    ));
    fake.push(AnthropicMessage::text("Created."));
    let events = api.chat_as(&api.token(), body("Make a PDF skill."));
    assert_eq!(last(&events, "applied")["op"], "skill_create");
    let system = |i: usize| {
        fake.message_requests()[i].json()["system"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let first = system(0);
    assert!(first.contains("- `brand`:"), "{first}");
    assert!(!first.contains("pdf-tools"));

    // The next turn keeps the snapshot, and use_skill opens the new skill all the same.
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_u",
        "use_skill",
        json!({ "name": "pdf-tools" }),
    ));
    fake.push(AnthropicMessage::text("Using it."));
    api.chat_as(&api.token(), body("Use it."));
    let r = last_result(&fake, false);
    assert!(
        r["instructions"].as_str().unwrap().contains("# PDF tools"),
        "{r}"
    );
    let n = fake.message_requests().len();
    assert_eq!(system(n - 1), first);
}

impl Api {
    fn get_thread(&self, agent: &str) -> String {
        let res = self
            .client
            .post(self.server.url(&format!("/api/agents/{agent}/threads")))
            .bearer_auth(self.token())
            .json(&json!({}))
            .send()
            .unwrap();
        assert!(res.status().is_success());
        let v: Value = res.json().unwrap();
        v["id"].as_str().unwrap().to_owned()
    }
}
