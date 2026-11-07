//! S1 with the fake Anthropic provider: agents propose saved scripts (TV-70, S2 of TV-32).
//!
//! `propose_script_change` and `propose_script_delete` always wait for the user's Apply, with
//! every confirmation switch off too. A change must compile in every way the script runs; a delete
//! is refused while something runs the script. The proposal carries the "Runs as" list, each
//! handler, task and tool with its grants, and a warning for a run that reaches any host, which
//! the tool proposal carries too. Apply needs admin, or the copy waiting in the thread of a call
//! from a note; a script changed since gives 409 stale; a discard writes nothing.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MEMO: &str = r#"label = "Memo"

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
  }
}
"#;

/// The type whose handler runs the shared script, reaching any host.
const ITEM: &str = r#"label = "Item"

handler "after_update" {
  script        = "stamp"
  function      = "after_update"
  allow_domains = ["*"]
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const STAMP: &str = r#"fn run() -> Result[(), string] {
    Ok(())
}

fn after_update(note: Value) -> Result[(), string] {
    Ok(())
}

fn call(params: Value) -> Value {
    params
}
"#;

const NIGHTLY: &str = "schedule = \"0 3 * * *\"\ntimezone = \"Europe/London\"\nscript = \"stamp\"\nallow_domains = [\"api.example.com\"]\nsecrets = [\"api_key\"]\npaused = true\n";

const ECHO: &str = "description = \"Echo the parameters.\"\nscript = \"stamp\"\nfunction = \"call\"\nallow_domains = [\"echo.example.com\"]\n";

/// Every confirmation switch off.
fn all_off() -> Value {
    json!({
        "notes": false, "archive": false, "delete": false, "types": false, "downloads": false,
        "taxonomies": false, "agents": false, "skills": false, "tools": false,
    })
}

/// An agent called from notes, with every switch off.
const KEEPER: &str = r#"label = "Keeper"

permissions {
  confirm {
    notes      = false
    archive    = false
    delete     = false
    types      = false
    downloads  = false
    taxonomies = false
    agents     = false
    skills     = false
    tools      = false
  }
}

trigger asked {
  mention = true
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start_with_args(
            |vault| {
                std::fs::create_dir_all(vault.join("types")).unwrap();
                std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
            },
            &[OsStr::new("--test-clock"), OsStr::new(START)],
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.ok("PUT", &format!("/api/secrets/{name}"), json!({ "value": value }));
        }
        api
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn send(&self, method: &str, path: &str, token: &str, body: Option<Value>) -> Response {
        let url = self.server.url(path);
        let req = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            m => panic!("{m}"),
        }
        .bearer_auth(token);
        match body {
            Some(b) => req.json(&b),
            None => req,
        }
        .send()
        .unwrap()
    }

    fn call(&self, method: &str, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
        let res = self.send(method, path, token, body);
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, v) = self.call(method, path, &self.token(), Some(body));
        assert!(status.is_success(), "{method} {path}: {v}");
        v
    }

    fn get(&self, path: &str) -> Value {
        let (status, v) = self.call("GET", path, &self.token(), None);
        assert!(status.is_success(), "GET {path}: {v}");
        v
    }

    fn put_text(&self, path: &str, text: &str) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(text.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{path}: {}", res.text().unwrap());
    }

    fn script(&self, name: &str) -> Option<String> {
        let (status, v) = self.call("GET", &format!("/api/scripts/{name}"), &self.token(), None);
        status
            .is_success()
            .then(|| v["source"].as_str().unwrap().to_owned())
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let (status, v) = self.call(
            "POST",
            "/api/tokens",
            &self.token(),
            Some(json!({ "name": name, "scope": scope })),
        );
        assert_eq!(status, StatusCode::CREATED, "{v}");
        v["secret"].as_str().unwrap().to_owned()
    }

    /// The shared script and what runs it: a handler of type item, task nightly and tool echo.
    fn seed_users(&self) {
        self.ok("PUT", "/api/scripts/stamp", json!({ "source": STAMP }));
        self.put_text("/api/types/item", ITEM);
        self.put_text("/api/tasks/nightly", NIGHTLY);
        self.ok("PUT", "/api/tools/echo", json!({ "wcl": ECHO }));
    }

    /// One tool call by the Assistant as the admin, then an answer: the turn's events.
    fn one(&self, fake: &FakeAnthropic, tool: &str, input: Value) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_0", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let res = self.send(
            "POST",
            "/api/ai/chat",
            &self.token(),
            Some(json!({ "messages": [user("Do it.")] })),
        );
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    fn apply_as(&self, token: &str, change: &Value) -> (StatusCode, Value) {
        self.call("POST", "/api/ai/apply", token, Some(change.clone()))
    }

    fn threads(&self, agent: &str) -> Vec<Value> {
        self.get(&format!("/api/agents/{agent}/threads"))["threads"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// A memo whose body calls `agent` with call id `call`.
    fn call_note(&self, agent: &str, call: &str) {
        let text = format!("[@Keeper tidy the script.](agent:{agent}?call={call})");
        self.ok(
            "POST",
            "/api/notes?type=memo",
            json!({ "note": {
                "fields": { "title": "Ask" },
                "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
                    { "kind": "paragraph", "fields": { "text": text } },
                ] }],
            } }),
        );
    }

    fn advance(&self, by: &str) {
        self.ok("POST", "/api/test/clock", json!({ "advance": by }));
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn read_events(res: Response) -> Vec<(String, Value)> {
    use std::io::{BufRead, BufReader};
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

/// The last tool result the model got back, as JSON with its error flag.
fn last_result(fake: &FakeAnthropic) -> (bool, Value) {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    let c = r["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|c| c["type"] == "tool_result")
        .unwrap()
        .clone();
    (
        c["is_error"] == true,
        serde_json::from_str(c["content"].as_str().unwrap()).unwrap_or(Value::Null),
    )
}

/// The `kind` and `name` of each entry of a "Runs as" list.
fn users(runs_as: &Value) -> Vec<(String, String)> {
    runs_as
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["kind"].as_str().unwrap().to_owned(),
                u["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pair(kind: &str, name: &str) -> (String, String) {
    (kind.to_owned(), name.to_owned())
}

#[test]
fn a_script_change_waits_for_apply_with_every_switch_off_and_shows_what_runs_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    api.ok("PUT", "/api/integrations/ai/settings", json!({ "confirm": all_off() }));
    let changed = STAMP.replace("Ok(())\n}\n\nfn after", "let x = 1\n    Ok(())\n}\n\nfn after");

    // get_script reads the source and what runs it.
    api.one(&fake, "get_script", json!({ "name": "stamp" }));
    let (err, got) = last_result(&fake);
    assert!(!err, "{got}");
    assert_eq!(got["source"], STAMP);
    assert_eq!(got["problems"], json!([]));
    assert_eq!(
        users(&got["runs_as"]),
        [pair("handler", "item"), pair("task", "nightly"), pair("tool", "echo")]
    );

    // A proposal, never made at once, with its diff, its users and their grants.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    assert!(find(&events, "applied").is_none(), "{events:?}");
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["kind"], "script");
    assert_eq!(p["op"], "change");
    assert_eq!(p["exists"], true);
    assert_eq!(p["before"], STAMP);
    assert_eq!(p["after"], changed.as_str());
    assert_eq!(p["proposer"], "assistant");
    assert_eq!(
        users(&p["runs_as"]),
        [pair("handler", "item"), pair("task", "nightly"), pair("tool", "echo")]
    );
    let handler = &p["runs_as"][0];
    assert_eq!(handler["event"], "after_update");
    assert_eq!(handler["allow_domains"], json!(["*"]));
    assert_eq!(handler["file"], "types/item.wcl");
    let task = &p["runs_as"][1];
    assert_eq!(task["schedule"], "0 3 * * *");
    assert_eq!(task["timezone"], "Europe/London");
    assert_eq!(task["secrets"], json!(["api_key"]));
    assert_eq!(task["allow_domains"], json!(["api.example.com"]));
    assert_eq!(task["paused"], true);
    assert_eq!(p["runs_as"][2]["allow_domains"], json!(["echo.example.com"]));
    // Any host is allowed, with a warning (TV-32 Q4).
    let warnings = p["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{p}");
    let w = warnings[0].as_str().unwrap();
    assert!(w.starts_with("The after_update handler of type item may reach any host"), "{w}");
    assert!(w.contains("local network"), "{w}");
    let (err, said) = last_result(&fake);
    assert!(!err && said["proposed"] == true, "{said}");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));

    // A write token cannot apply a body it sends; the admin can, once.
    let write = api.token_of("phone", "write");
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "op": "script_change", "script_name": "stamp" }));
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
    // The script changed since: stale, nothing written.
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    let activity = api.get("/api/activity?kind=ai.applied");
    assert!(activity.to_string().contains("Applied: change script stamp"), "{activity}");

    // The same source: nothing to apply.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    assert!(find(&events, "script_proposal").is_none());
    assert_eq!(last_result(&fake).1["proposed"], false);

    // A source that does not compile is not proposed, and says why.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": "fn run( {" }),
    );
    assert!(find(&events, "script_proposal").is_none(), "{events:?}");
    let (err, said) = last_result(&fake);
    assert!(err, "{said}");
    assert!(!said["problems"].as_array().unwrap().is_empty(), "{said}");
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));

    // A new script: created on Apply.
    let fresh = "fn run() -> Result[(), string] {\n    Ok(())\n}\n";
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "fresh", "source": fresh }),
    );
    let p = find(&events, "script_proposal").unwrap().clone();
    assert_eq!(p["op"], "create");
    assert!(p["before"].is_null());
    assert_eq!(p["runs_as"], json!([]));
    assert_eq!(api.script("fresh"), None);
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "script_create");
    assert_eq!(api.script("fresh").as_deref(), Some(fresh));

    // A script in use cannot be deleted; one nothing runs is, on Apply.
    let events = api.one(&fake, "propose_script_delete", json!({ "name": "stamp" }));
    assert!(find(&events, "script_proposal").is_none(), "{events:?}");
    let (err, said) = last_result(&fake);
    assert!(err, "{said}");
    assert_eq!(users(&said["runs_as"]).len(), 3, "{said}");
    let events = api.one(&fake, "propose_script_delete", json!({ "name": "fresh" }));
    let p = find(&events, "script_proposal").unwrap().clone();
    assert_eq!(p["op"], "delete");
    assert_eq!(p["before"], fresh);
    assert_eq!(api.script("fresh").as_deref(), Some(fresh));
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "op": "script_delete", "script_name": "fresh" }));
    assert_eq!(api.script("fresh"), None);
    // A missing script names those there are.
    api.one(&fake, "propose_script_delete", json!({ "name": "fresh" }));
    let (err, said) = last_result(&fake);
    assert!(err && said["error"].as_str().unwrap().contains("stamp"), "{said}");
}

#[test]
fn a_tool_proposal_shows_what_else_runs_its_script() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    let open = ECHO.replace("[\"echo.example.com\"]", "[\"*\"]");
    let events = api.one(
        &fake,
        "propose_tool_change",
        json!({ "name": "echo", "wcl": open, "script_source": null }),
    );
    let p = find(&events, "tool_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    // The tool itself is left out of its own list.
    assert_eq!(
        users(&p["runs_as"]),
        [pair("handler", "item"), pair("task", "nightly")]
    );
    let warnings: Vec<&str> = p["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 2, "{p}");
    assert!(warnings[0].starts_with("The tool echo may reach any host"), "{warnings:?}");
    assert!(warnings[1].starts_with("The after_update handler of type item"), "{warnings:?}");
    // A delete that keeps the script lists what keeps it.
    let events = api.one(&fake, "propose_tool_delete", json!({ "name": "echo" }));
    let p = find(&events, "tool_proposal").unwrap().clone();
    assert_eq!(p["script_delete"], false);
    assert_eq!(users(&p["runs_as"]).len(), 2, "{p}");
}

#[test]
fn a_call_from_a_note_leaves_a_script_proposal_that_a_phone_applies_or_discards_from_the_thread() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok("PUT", "/api/scripts/stamp", json!({ "source": STAMP }));
    api.put_text("/api/agents/keeper", KEEPER);
    let changed = format!("{STAMP}\nfn extra() -> int {{\n    1\n}}\n");

    fake.push(AnthropicMessage::new().tool_use(
        "toolu_s",
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    ));
    fake.push(AnthropicMessage::text("The change waits for you."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
    api.advance("PT1S");
    let first = fake.message_requests()[0].json();
    assert!(first["tools"].to_string().contains("\"propose_script_change\""));

    // Every switch is off, yet the change waits in the thread.
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
    let threads = api.threads("keeper");
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(threads[0]["origin"], "mention");
    assert_eq!(threads[0]["pending"], 1, "{threads:?}");
    let thread = threads[0]["id"].as_str().unwrap().to_owned();
    let read = api.get(&format!("/api/agents/keeper/threads/{thread}"));
    let stored = read["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["status"] == "pending")
        .unwrap_or_else(|| panic!("{read}"))
        .clone();
    assert_eq!(stored["event"], "script_proposal");

    // A write token applies the copy the thread holds, not a body it changed.
    let write = api.token_of("phone", "write");
    let mut forged = stored["data"].clone();
    forged["after"] = json!("fn run() -> Result[(), string] {\n    Err(\"forged\")\n}\n");
    let (status, body) = api.apply_as(&write, &forged);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "script_change");
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
    assert_eq!(api.threads("keeper")[0]["pending"], 0);

    // A second call's proposal, discarded from the phone: nothing written.
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "propose_script_change",
        json!({ "name": "stamp", "source": STAMP }),
    ));
    fake.push(AnthropicMessage::text("Waiting."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC");
    api.advance("PT1S");
    let pending = api
        .threads("keeper")
        .into_iter()
        .find(|t| t["pending"] == 1)
        .unwrap_or_else(|| panic!("{:?}", api.threads("keeper")));
    let thread = pending["id"].as_str().unwrap().to_owned();
    let (status, body) = api.call(
        "POST",
        "/api/ai/discard",
        &write,
        Some(json!({ "id": "toolu_t", "thread": thread })),
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pending"], 0, "{body}");
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
}
