//! S1 with the fake Anthropic provider: agents and wscript (script-tools T1 to T7, T10).
//!
//! `run_script` reports compile and run-time errors, results and logs, updates many notes in one
//! run, reaches any host and every secret, and is offered only to an agent with `run_code`,
//! which an agent cannot grant without an admin. Script tools are parsed and checked, offered
//! with their schema when valid and allowed, called with a params map, bounded by their hosts
//! and secrets, proposed and deleted under the `tools` switch and admin scope, and managed through
//! `/api/tools`. A skill's tools are offered after `use_skill`, named `<skill>__<tool>`, and run
//! the skill's own scripts. Every run is an `ai.script` activity entry with the version that ran.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, Reply, StandIn, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

const ITEM: &str = r#"label = "Item"

field done {
  kind = "bool"
}

template default {
  tab "Main" {
    slot "title"
    slot "done"
  }
}
"#;

/// An agent that may run its own code.
const SCRIPTER: &str =
    "label = \"Scripter\"\n\npermissions {\n  scripts {\n    run_code = true\n  }\n}\n";

const GREET_SCRIPT: &str = r#"use vault

fn run(params: Value) -> Result[Value, string] {
    let who = params.get("who").unwrap().as_string().unwrap()
    let loud = match params.get("loud") {
        Some(v) => v.as_bool().unwrap_or(false),
        None => false,
    }
    vault::log("greeting " + who)
    if who == "nobody" {
        return Err("there is nobody to greet")
    }
    let text = if loud { "HELLO, " + who } else { "Hello, " + who }
    Ok(Value::Map(#{ "text": Value::String(text) }))
}
"#;

const GREET: &str = r#"description = "Greet someone by name."
script = "greet"
function = "run"

param who {
  kind = "string"
  description = "Who to greet."
  required = true
}

param loud {
  kind = "bool"
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

fn sha(text: &str) -> String {
    writer::content_hash(text)
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/item.wcl"), ITEM).unwrap();
        });
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

    fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, self.server.url(path))
            .bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn get(&self, path: &str) -> Value {
        let (status, v) = self.send(reqwest::Method::GET, path, &self.token(), None);
        assert!(status.is_success(), "{path}: {v}");
        v
    }

    fn put_text(&self, path: &str, text: &str) -> (StatusCode, Value) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(text.to_owned())
            .send()
            .unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn agent(&self, name: &str, wcl: &str) {
        let (status, v) = self.put_text(&format!("/api/agents/{name}"), wcl);
        assert!(status.is_success(), "{v}");
    }

    fn tool(&self, name: &str, wcl: &str, script: Option<&str>) -> Value {
        let mut body = json!({ "wcl": wcl });
        if let Some(s) = script {
            body["script_source"] = json!(s);
        }
        let (status, v) = self.send(
            reqwest::Method::PUT,
            &format!("/api/tools/{name}"),
            &self.token(),
            Some(body),
        );
        assert!(status.is_success(), "{v}");
        v
    }

    fn confirm(&self, confirm: Value) {
        let (status, v) = self.send(
            reqwest::Method::PUT,
            "/api/integrations/ai/settings",
            &self.token(),
            Some(json!({ "confirm": confirm })),
        );
        assert!(status.is_success(), "{v}");
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let (status, v) = self.send(
            reqwest::Method::POST,
            "/api/tokens",
            &self.token(),
            Some(json!({ "name": name, "scope": scope })),
        );
        assert_eq!(status, StatusCode::CREATED, "{v}");
        v["secret"].as_str().unwrap().to_owned()
    }

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

    /// One round of tool calls by `agent` as the admin, then an answer: the turn's events.
    fn calls(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        calls: &[(&str, Value)],
    ) -> Vec<(String, Value)> {
        let mut m = AnthropicMessage::new();
        for (i, (tool, input)) in calls.iter().enumerate() {
            m = m.tool_use(&format!("toolu_{i}"), tool, input.clone());
        }
        fake.push(m);
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(&self.token(), agent)
    }

    fn one(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        self.calls(fake, agent, &[(tool, input)])
    }

    fn create_item(&self, title: &str) -> String {
        let (status, v) = self.send(
            reqwest::Method::POST,
            "/api/notes",
            &self.token(),
            Some(json!({ "type": "item", "note": { "fields": { "title": title } } })),
        );
        assert!(status.is_success(), "{v}");
        v["id"].as_str().unwrap().to_owned()
    }

    fn apply_as(&self, token: &str, change: &Value) -> (StatusCode, Value) {
        self.send(
            reqwest::Method::POST,
            "/api/ai/apply",
            token,
            Some(change.clone()),
        )
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

/// The tool results the model got back in the last request, by call id, as JSON with the flag.
fn results(fake: &FakeAnthropic) -> Vec<(bool, Value)> {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    r["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "tool_result")
        .map(|c| {
            (
                c["is_error"] == true,
                serde_json::from_str(c["content"].as_str().unwrap()).unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn last_result(fake: &FakeAnthropic) -> (bool, Value) {
    results(fake).pop().unwrap()
}

/// The tool names a request offered.
fn offered(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

fn tool_of<'a>(request: &'a Value, name: &str) -> Option<&'a Value> {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
}

#[test]
fn run_script_answers_results_logs_compile_and_runtime_errors() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);

    api.calls(
        &fake,
        Some("scripter"),
        &[
            (
                "run_script",
                json!({ "code": "use vault\nfn main() -> int {\n    vault::log(\"counting\")\n    let n = 0\n    for i in [1, 2, 3] { n = n + i }\n    n\n}\n", "function": null, "args": null }),
            ),
            (
                "run_script",
                json!({ "code": "fn twice(x: Value, s: Value) -> string { s.as_string().unwrap() + fmt(\"{}\", x.as_int().unwrap() * 2) }", "function": "twice", "args": "[21, \"=\"]" }),
            ),
            ("run_script", json!({ "code": "fn main() -> int { undefined_thing }", "function": null, "args": null })),
            ("run_script", json!({ "code": "fn main() -> Result[int, string] { Err(\"no luck\") }", "function": null, "args": null })),
            ("run_script", json!({ "code": "fn main() -> int { let n = 0\n while true { n = n + 1 }\n n }", "function": null, "args": null })),
        ],
    );
    let r = results(&fake);
    assert_eq!(r.len(), 5);

    let (err, ok) = &r[0];
    assert!(!err, "{ok}");
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["result"], 6);
    assert_eq!(ok["log"], "counting\n");
    assert!(ok["instructions"].as_u64().unwrap() > 0, "{ok}");
    assert!(ok["duration_ms"].is_u64(), "{ok}");
    assert_eq!(ok["script"]["kind"], "code");

    assert_eq!(r[1].1["result"], "=42", "{}", r[1].1);

    let (err, compile) = &r[2];
    assert!(err);
    assert_eq!(compile["kind"], "compile", "{compile}");
    assert!(
        compile["error"]
            .as_str()
            .unwrap()
            .contains("does not compile"),
        "{compile}"
    );

    let (err, returned) = &r[3];
    assert!(err);
    assert_eq!(returned["kind"], "returned");
    assert_eq!(returned["error"], "no luck");

    // A runaway loop stops at the instruction limit, and says so.
    let (err, runaway) = &r[4];
    assert!(err);
    assert_eq!(runaway["kind"], "instruction_limit", "{runaway}");
    assert!(
        runaway["message"].as_str().unwrap().contains("50 million"),
        "{runaway}"
    );

    // Every run is an ai.script entry with the hash of the code that ran.
    let log = api.get("/api/activity?kind=ai.script&limit=50");
    let items = log["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{log}");
    let first = items
        .iter()
        .find(|e| e["detail"]["outcome"] == "ok" && e["detail"]["log"] == "counting\n")
        .unwrap_or_else(|| panic!("{log}"));
    assert_eq!(first["detail"]["tool"], "run_script");
    assert_eq!(first["detail"]["agent"], "scripter");
    assert_eq!(first["detail"]["script"]["kind"], "code");
    assert_eq!(
        first["detail"]["script"]["hash"].as_str().unwrap().len(),
        64
    );
    assert!(first["detail"]["turn"].is_string());
    assert!(first["detail"]["instructions"].as_u64().unwrap() > 0);
    assert!(
        items
            .iter()
            .any(|e| e["detail"]["error_kind"] == "instruction_limit")
    );
}

#[test]
fn run_script_updates_many_notes_in_one_run_as_the_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);
    for i in 0..40 {
        api.create_item(&format!("Item {i}"));
    }
    let code = r#"use vault

fn main(dry_run: Value) -> Result[int, string] {
    let dry_run = dry_run.as_bool().unwrap_or(true)
    let changed = 0
    let cursor = ""
    loop {
        let page = vault::query_page("from item where !done", 15, cursor)?
        let rows = page.get("rows").unwrap().as_list().unwrap()
        for row in rows {
            let id = row.get("id").unwrap().as_string().unwrap()
            if !dry_run {
                let saved = vault::patch(id, Value::Map(#{ "done": Value::Bool(true) }), "")?
            }
            changed = changed + 1
        }
        match page.get("next").unwrap().as_string() {
            Some(next) => {
                if dry_run { cursor = next } else { cursor = "" }
            }
            None => break,
        }
    }
    Ok(changed)
}
"#;
    api.calls(
        &fake,
        Some("scripter"),
        &[
            (
                "run_script",
                json!({ "code": code, "function": null, "args": "[true]" }),
            ),
            (
                "run_script",
                json!({ "code": code, "function": null, "args": "[false]" }),
            ),
        ],
    );
    let r = results(&fake);
    assert_eq!(r[0].1["result"], 40, "{}", r[0].1);
    assert_eq!(r[1].1["result"], 40, "{}", r[1].1);
    let open = api.get("/api/query?q=from%20item%20where%20!done");
    assert_eq!(open["total"], 0, "{open}");
    // The changes are the agent's, recorded in history.
    let any = api.get("/api/query?q=from%20item%20limit%201");
    let id = any["rows"][0]["id"].as_str().unwrap();
    let history = api.get(&format!("/api/notes/{id}/history"));
    let latest = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["current"] == true)
        .unwrap();
    assert_eq!(
        latest["author"],
        json!({ "kind": "agent", "name": "scripter" }),
        "{history}"
    );
}

#[test]
fn run_script_reaches_any_host_and_reads_every_secret() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);
    api.secret("weather_key", "k-123");
    let web = StandIn::start();
    web.route("/forecast", Reply::text(200, "sunny"));
    let code = r#"use http
use secrets

fn main(url: Value) -> Result[string, string] {
    let key = secrets::get("weather_key")?
    let r = http::get(url.as_string().unwrap(), #{ "authorization": key })?
    Ok(r.body)
}
"#;
    api.one(
        &fake,
        Some("scripter"),
        "run_script",
        json!({ "code": code, "function": null, "args": json!([web.url("/forecast")]).to_string() }),
    );
    let (err, out) = last_result(&fake);
    assert!(!err, "{out}");
    assert_eq!(out["result"], "sunny");
    let req = &web.requests()[0];
    assert_eq!(req.header("authorization"), Some("k-123"));
}

#[test]
fn run_script_is_offered_only_with_run_code_and_never_self_granted() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);
    api.agent(
        "limited",
        "label = \"Limited\"\n\npermissions {\n  tools = [\"get_note\"]\n\n  scripts {\n    run_code = true\n  }\n}\n",
    );
    // A manager that may change agents without asking, but cannot run code.
    api.agent(
        "manager",
        "label = \"Manager\"\n\npermissions {\n  confirm {\n    agents = false\n  }\n}\n",
    );
    // A boss that runs code and changes agents without asking.
    api.agent(
        "boss",
        "label = \"Boss\"\n\npermissions {\n  confirm {\n    agents = false\n  }\n\n  scripts {\n    run_code = true\n  }\n}\n",
    );

    for (agent, offers) in [
        (None, false),
        (Some("scripter"), true),
        (Some("limited"), false),
        (Some("manager"), false),
    ] {
        fake.push(AnthropicMessage::text("Hi."));
        api.chat_as(&api.token(), agent);
        let request = fake.message_requests().last().unwrap().json();
        assert_eq!(
            offered(&request).contains(&"run_script".to_owned()),
            offers,
            "{agent:?}"
        );
    }

    // A script's changes are made at once, so a `read` caller's turn is offered no script, and a
    // `write` caller's (an app's) has the agent's own scope, as its switches do.
    for (scope, offers) in [("read", false), ("write", true)] {
        let token = api.token_of(&format!("caller_{scope}"), scope);
        fake.push(AnthropicMessage::text("Hi."));
        api.chat_as(&token, Some("scripter"));
        let request = fake.message_requests().last().unwrap().json();
        assert_eq!(
            offered(&request).contains(&"run_script".to_owned()),
            offers,
            "{scope}"
        );
    }

    // The manager cannot turn it on for anyone, itself included.
    let grant = "label = \"Helper\"\n\npermissions {\n  scripts {\n    run_code = true\n  }\n}\n";
    api.one(
        &fake,
        Some("manager"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": grant }),
    );
    let (err, out) = last_result(&fake);
    assert!(err, "{out}");
    assert!(
        out["error"].as_str().unwrap().contains("Run wscript code"),
        "{out}"
    );
    api.one(
        &fake,
        Some("manager"),
        "propose_agent_change",
        json!({ "name": "manager", "wcl": "label = \"Manager\"\n\npermissions {\n  scripts {\n    run_code = true\n  }\n}\n" }),
    );
    assert!(last_result(&fake).0);
    assert_eq!(
        api.get("/api/agents/manager")["permissions"]["scripts"]["run_code"],
        false
    );

    // The boss has it, so it may propose it, but with its agents switch off the change still
    // waits for an Apply, which only an admin makes.
    let events = api.one(
        &fake,
        Some("boss"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": grant }),
    );
    assert!(
        !events
            .iter()
            .any(|(n, v)| n == "applied" && v["op"] == "agent_create"),
        "{events:?}"
    );
    let p = find(&events, "agent_proposal").unwrap().clone();
    assert_eq!(p["grants_run_code"], true, "{p}");
    let (status, _) = api.send(
        reqwest::Method::GET,
        "/api/agents/helper",
        &api.token(),
        None,
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    let write = api.token_of("phone", "write");
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        api.get("/api/agents/helper")["permissions"]["scripts"]["run_code"],
        true
    );
}

#[test]
fn tools_are_checked_listed_offered_and_called_with_their_params() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let put = api.tool("greet", GREET, Some(GREET_SCRIPT));
    assert_eq!(put["valid"], true, "{put}");
    assert_eq!(put["script_source"], GREET_SCRIPT);
    // A tool whose script is missing, one whose function takes the wrong arguments, and one whose
    // script does not compile are kept but not valid.
    let missing = api.tool(
        "missing",
        "description = \"Nothing.\"\nscript = \"nowhere\"\nfunction = \"run\"\n",
        None,
    );
    assert_eq!(missing["valid"], false);
    assert!(
        missing["problems"][0]
            .as_str()
            .unwrap()
            .contains("no script"),
        "{missing}"
    );
    let arity = api.tool(
        "arity",
        "description = \"Two args.\"\nscript = \"two\"\nfunction = \"run\"\n",
        Some("fn run(a: Value, b: Value) -> int { 1 }\n"),
    );
    assert!(
        arity["problems"][0].as_str().unwrap().contains("takes one"),
        "{arity}"
    );
    let broken = api.tool(
        "broken",
        "description = \"Broken.\"\nscript = \"broken\"\nfunction = \"run\"\n",
        Some("fn run(p: Value) -> int { nope }\n"),
    );
    assert_eq!(broken["valid"], false);
    // Bad vocabulary and a built-in name are refused outright.
    let (status, body) = api.put_text("/api/tools/odd", "script = \"greet\"\ncolour = \"red\"\n");
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = api.put_text("/api/tools/get_note", GREET);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("server has itself"), "{body}");

    let listed = api.get("/api/tools");
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["arity", "broken", "greet", "missing"]);
    let one = api.get("/api/tools/greet");
    assert_eq!(one["tool"]["params"][0]["name"], "who");
    assert_eq!(one["script"], "greet");

    // Only the valid tool is offered, with a strict schema.
    let events = api.calls(
        &fake,
        None,
        &[
            ("greet", json!({ "who": "Ann", "loud": null })),
            ("greet", json!({ "who": "Bo", "loud": true })),
            ("greet", json!({ "who": "nobody", "loud": null })),
            ("greet", json!({ "loud": 3 })),
            ("broken", json!({})),
        ],
    );
    let first = fake.message_requests()[0].json();
    let spec = tool_of(&first, "greet").unwrap_or_else(|| panic!("{first}"));
    assert_eq!(spec["description"], "Greet someone by name.");
    assert_eq!(spec["input_schema"]["required"], json!(["who", "loud"]));
    assert_eq!(
        spec["input_schema"]["properties"]["loud"]["type"],
        json!(["boolean", "null"])
    );
    for name in ["broken", "missing", "arity"] {
        assert!(tool_of(&first, name).is_none(), "{name}");
    }
    // With a script tool to call, the Assistant's instructions point it at the wscript skill.
    let system = first["system"].as_str().unwrap();
    assert!(system.contains("## Scripts"), "{system}");
    assert!(system.contains("wscript skill"), "{system}");
    let r = results(&fake);
    assert_eq!(
        r[0].1["result"],
        json!({ "text": "Hello, Ann" }),
        "{}",
        r[0].1
    );
    assert_eq!(r[0].1["log"], "greeting Ann\n");
    assert_eq!(r[1].1["result"]["text"], "HELLO, Bo");
    assert!(r[2].0);
    assert_eq!(r[2].1["error"], "there is nobody to greet");
    // A call that does not fit the schema is refused before the script runs.
    assert!(r[3].0);
    assert!(
        r[3].1["error"].as_str().unwrap().contains("who"),
        "{}",
        r[3].1
    );
    assert!(r[4].0);
    assert!(
        r[4].1["error"].as_str().unwrap().contains("no tool"),
        "{}",
        r[4].1
    );
    assert!(find(&events, "tool_result").is_some());

    // Each run of a vault tool is logged with its script's hash and the generation it was read
    // at: the three that ran, not the call refused before it ran nor the tool not offered.
    let log = api.get("/api/activity?kind=ai.script&limit=50");
    let items = log["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{log}");
    let entry = &items
        .iter()
        .find(|e| e["detail"]["outcome"] == "ok")
        .unwrap()["detail"];
    assert_eq!(entry["tool"], "greet");
    assert_eq!(entry["script"]["kind"], "vault");
    assert_eq!(entry["script"]["script"], "greet");
    assert_eq!(entry["script"]["hash"], sha(GREET_SCRIPT));
    assert_eq!(
        entry["script"]["tool_hash"],
        sha(api.get("/api/tools/greet")["wcl"].as_str().unwrap())
    );
    assert!(
        entry["script"]["generation"].as_u64().unwrap() > 0,
        "{entry}"
    );

    // The route runs a tool too.
    let (status, ran) = api.send(
        reqwest::Method::POST,
        "/api/tools/greet/run",
        &api.token(),
        Some(json!({ "params": { "who": "Cy" } })),
    );
    assert_eq!(status, StatusCode::OK, "{ran}");
    assert_eq!(ran["ok"], true);
    assert_eq!(ran["result"]["text"], "Hello, Cy");
    assert_eq!(ran["script"]["hash"], sha(GREET_SCRIPT));
    // Writing and running need admin; reading does not.
    let read = api.token_of("reader", "read");
    let (status, _) = api.send(reqwest::Method::GET, "/api/tools", &read, None);
    assert_eq!(status, StatusCode::OK);
    for (method, path, body) in [
        (
            reqwest::Method::POST,
            "/api/tools/greet/run",
            Some(json!({ "params": {} })),
        ),
        (
            reqwest::Method::PUT,
            "/api/tools/greet",
            Some(json!({ "wcl": GREET })),
        ),
        (reqwest::Method::DELETE, "/api/tools/greet", None),
    ] {
        let write = api.token_of(&format!("w{}", path.len() + method.as_str().len()), "write");
        let (status, _) = api.send(method.clone(), path, &write, body);
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }

    // A script a tool names cannot be deleted from under it.
    let (status, body) = api.send(
        reqwest::Method::DELETE,
        "/api/scripts/greet",
        &api.token(),
        None,
    );
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = api.send(
        reqwest::Method::DELETE,
        "/api/tools/greet",
        &api.token(),
        None,
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["script_deleted"], "greet");
    let (status, _) = api.send(
        reqwest::Method::GET,
        "/api/scripts/greet",
        &api.token(),
        None,
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn a_tool_reaches_only_its_hosts_and_secrets_and_the_allowlist_offers_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let web = StandIn::start();
    web.route("/data", Reply::text(200, "fetched"));
    api.secret("granted_key", "g-1");
    api.secret("other_key", "o-1");
    let script = r#"use http
use secrets

fn run(params: Value) -> Result[Value, string] {
    let what = params.get("what").unwrap().as_string().unwrap()
    if what == "http" {
        let url = params.get("url").unwrap().as_string().unwrap()
        let r = http::get(url, #{})?
        return Ok(Value::String(r.body))
    }
    Ok(Value::String(secrets::get(what)?))
}
"#;
    let wcl = format!(
        "description = \"Fetch.\"\nscript = \"fetch\"\nfunction = \"run\"\nallow_domains = [\"{}\"]\nsecrets = [\"granted_key\"]\ntimeout = 10\n\nparam what {{\n  kind = \"string\"\n  required = true\n}}\n\nparam url {{\n  kind = \"string\"\n}}\n",
        web.host()
    );
    api.tool("fetch", &wcl, Some(script));
    api.calls(
        &fake,
        None,
        &[
            ("fetch", json!({ "what": "http", "url": web.url("/data") })),
            (
                "fetch",
                json!({ "what": "http", "url": "http://example.invalid/data" }),
            ),
            ("fetch", json!({ "what": "granted_key", "url": null })),
            ("fetch", json!({ "what": "other_key", "url": null })),
        ],
    );
    let r = results(&fake);
    assert_eq!(r[0].1["result"], "fetched", "{}", r[0].1);
    assert!(r[1].0);
    assert!(
        r[1].1["error"].as_str().unwrap().contains("allowlist"),
        "{}",
        r[1].1
    );
    assert_eq!(r[2].1["result"], "g-1");
    assert!(r[3].0);
    assert!(
        r[3].1["error"].as_str().unwrap().contains("not granted"),
        "{}",
        r[3].1
    );

    // An agent with a tools list is offered only the tools it names.
    api.agent(
        "plain",
        "label = \"Plain\"\n\npermissions {\n  tools = [\"get_note\"]\n}\n",
    );
    api.agent(
        "fetcher",
        "label = \"Fetcher\"\n\npermissions {\n  tools = [\"get_note\", \"fetch\"]\n}\n",
    );
    for (agent, offers) in [("plain", false), ("fetcher", true)] {
        fake.push(AnthropicMessage::text("Hi."));
        api.chat_as(&api.token(), Some(agent));
        let request = fake.message_requests().last().unwrap().json();
        assert_eq!(
            offered(&request).contains(&"fetch".to_owned()),
            offers,
            "{agent}"
        );
    }
    // The agents list names the vault's tools for the tools picker.
    let agents = api.get("/api/agents");
    assert!(
        agents["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "fetch"),
        "{agents}"
    );
    assert!(
        agents["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "run_script")
    );
    // An agent file may name a tool that does not exist only as a skill's.
    let (status, body) = api.put_text(
        "/api/agents/odd",
        "permissions {\n  tools = [\"no_such_tool\"]\n}\n",
    );
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = api.put_text(
        "/api/agents/skilled",
        "permissions {\n  tools = [\"use_skill\", \"brand-kit__fill\"]\n}\n",
    );
    assert!(status.is_success(), "{body}");
}

#[test]
fn tool_changes_are_proposed_or_made_by_the_switch_and_admin_scope() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);

    // The switch on: a proposal with the WCL and the script, nothing written until applied.
    let events = api.one(
        &fake,
        None,
        "propose_tool_change",
        json!({ "name": "greet", "wcl": GREET, "script_source": GREET_SCRIPT }),
    );
    let p = find(&events, "tool_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["kind"], "tool");
    assert_eq!(p["op"], "create");
    assert!(p["before"].is_null());
    assert!(p["after"].as_str().unwrap().contains("Greet someone"));
    assert_eq!(p["script"], "greet");
    assert_eq!(p["script_after"], GREET_SCRIPT);
    assert!(p["script_before"].is_null());
    assert_eq!(p["problems"], json!([]));
    let (status, _) = api.send(reqwest::Method::GET, "/api/tools/greet", &api.token(), None);
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Applying needs admin.
    let write = api.token_of("phone", "write");
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "op": "tool_create", "tool_name": "greet", "script_name": "greet" })
    );
    assert_eq!(api.get("/api/tools/greet")["valid"], true);
    // A stale proposal is refused.
    let (status, _) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT);

    // get_tool and list_tools read.
    api.one(&fake, None, "get_tool", json!({ "name": "greet" }));
    let (_, got) = last_result(&fake);
    assert_eq!(got["script_source"], GREET_SCRIPT);
    assert!(got["format"].as_str().unwrap().contains("param <name>"));
    api.one(&fake, None, "list_tools", json!({}));
    assert_eq!(last_result(&fake).1["tools"][0]["name"], "greet");

    // A bad file is refused with its issues.
    api.one(
        &fake,
        None,
        "propose_tool_change",
        json!({ "name": "bad", "wcl": "colour = 1\n", "script_source": null }),
    );
    assert!(last_result(&fake).0);

    // The switch off, as the admin: made at once.
    api.confirm(json!({ "tools": false }));
    let changed = GREET.replace("Greet someone by name.", "Say hello.");
    let events = api.one(
        &fake,
        None,
        "propose_tool_change",
        json!({ "name": "greet", "wcl": changed, "script_source": null }),
    );
    let applied = find(&events, "applied").unwrap_or_else(|| panic!("{events:?}"));
    assert_eq!(applied["op"], "tool_change");
    assert_eq!(
        api.get("/api/tools/greet")["tool"]["description"],
        "Say hello."
    );

    // The switch off, but a caller with only `read`: still a proposal. (A `write` caller's turn has
    // the agent's own scope, which for the Assistant is `admin`.)
    let read = api.token_of("reader", "read");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_w",
        "propose_tool_delete",
        json!({ "name": "greet" }),
    ));
    fake.push(AnthropicMessage::text("Done."));
    let events = api.chat_as(&read, None);
    let p = find(&events, "tool_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["op"], "delete");
    assert_eq!(p["script_delete"], true);
    assert_eq!(api.get("/api/tools/greet")["valid"], true);

    // A task naming the script keeps it when the tool goes.
    let (status, body) = api.put_text(
        "/api/tasks/nightly",
        "schedule = \"0 0 1 1 *\"\nscript = \"greet\"\npaused = true\n",
    );
    assert!(status.is_success(), "{body}");
    let events = api.one(
        &fake,
        None,
        "propose_tool_delete",
        json!({ "name": "greet" }),
    );
    let applied = find(&events, "applied").unwrap_or_else(|| panic!("{events:?}"));
    assert_eq!(applied["op"], "tool_delete");
    assert!(applied["script_name"].is_null(), "{applied}");
    let (status, _) = api.send(reqwest::Method::GET, "/api/tools/greet", &api.token(), None);
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = api.send(
        reqwest::Method::GET,
        "/api/scripts/greet",
        &api.token(),
        None,
    );
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn a_skills_tools_are_offered_after_use_skill_and_run_its_own_scripts() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let md = "---\nname: brand-kit\ndescription: The house style.\n---\n\nUse the stamp tool.\n";
    let res = api
        .client
        .put(api.server.url("/api/skills/brand-kit"))
        .bearer_auth(api.token())
        .header("content-type", "text/markdown; charset=utf-8")
        .body(md)
        .send()
        .unwrap();
    assert!(res.status().is_success());
    let stamp = "description = \"Stamp a title with the house style.\"\nscript = \"stamp\"\nfunction = \"run\"\n\nparam title {\n  kind = \"string\"\n  required = true\n}\n";
    let script = "fn run(params: Value) -> string {\n    \"** \" + params.get(\"title\").unwrap().as_string().unwrap() + \" **\"\n}\n";
    for (path, body) in [("tools/stamp.wcl", stamp), ("scripts/stamp.ws", script)] {
        let res = api
            .client
            .put(
                api.server
                    .url(&format!("/api/skills/brand-kit/files/{path}")),
            )
            .bearer_auth(api.token())
            .body(body.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }
    // A vault script of the same name is not the one the skill's tool runs.
    api.tool(
        "stamp",
        "description = \"The vault's stamp.\"\nscript = \"stamp\"\nfunction = \"run\"\n",
        Some("fn run(params: Value) -> string { \"vault\" }\n"),
    );

    fake.push(AnthropicMessage::new().tool_use(
        "toolu_a",
        "use_skill",
        json!({ "name": "brand-kit" }),
    ));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_b",
        "brand-kit__stamp",
        json!({ "title": "Q3" }),
    ));
    fake.push(AnthropicMessage::text("Done."));
    api.chat_as(&api.token(), None);
    let requests = fake.message_requests();
    let first = requests[0].json();
    assert!(
        tool_of(&first, "brand-kit__stamp").is_none(),
        "not before use_skill"
    );
    assert!(tool_of(&first, "stamp").is_some());
    let second = requests[1].json();
    let used: Value = serde_json::from_str(
        second["messages"].as_array().unwrap().last().unwrap()["content"][0]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(used["tools"], json!(["brand-kit__stamp"]), "{used}");
    let spec = tool_of(&second, "brand-kit__stamp").unwrap_or_else(|| panic!("{second}"));
    assert_eq!(spec["description"], "Stamp a title with the house style.");
    let (err, out) = last_result(&fake);
    assert!(!err, "{out}");
    assert_eq!(out["result"], "** Q3 **");
    assert_eq!(out["script"]["kind"], "skill");
    assert_eq!(out["script"]["skill"], "brand-kit");
    assert_eq!(out["script"]["path"], "scripts/stamp.ws");
    assert_eq!(out["script"]["hash"], sha(script));

    let log = api.get("/api/activity?kind=ai.script&limit=50");
    let entry = &log["items"][0]["detail"];
    assert_eq!(entry["tool"], "brand-kit__stamp", "{log}");
    assert_eq!(entry["script"]["skill"], "brand-kit");
    assert_eq!(entry["script"]["hash"], sha(script));
}
