//! S1 with the fake Anthropic provider: agents run the vault's saved scripts and tasks by name.
//!
//! `run_saved_script`, `run_task`, `list_scripts` and `list_tasks` are offered only to an agent
//! with `permissions.scripts.run_saved` or `run_code`, which implies it, and never to a `read`
//! caller's turn. A saved script's function runs with the call's arguments as the agent, reaching
//! the web only when the agent may read web pages and secrets only when it may run its own code.
//! A task runs as its Run now button runs it, with its own capabilities: a short one answers its
//! outcome and log, a long one that it started. An agent cannot grant the switch without an
//! admin. Every run is an `ai.script` activity entry with the version that ran.

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

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

/// An agent that may run saved scripts and tasks, and reads web pages (the default).
const SAVER: &str =
    "label = \"Saver\"\n\npermissions {\n  scripts {\n    run_saved = true\n  }\n}\n";

/// The same without web pages.
const OFFLINE: &str = "label = \"Offline\"\n\npermissions {\n  web {\n    read = false\n  }\n\n  scripts {\n    run_saved = true\n  }\n}\n";

/// An agent that may run its own code, which implies the saved scripts and tasks.
const SCRIPTER: &str =
    "label = \"Scripter\"\n\npermissions {\n  scripts {\n    run_code = true\n  }\n}\n";

const SAVED_TOOLS: [&str; 4] = ["list_scripts", "run_saved_script", "list_tasks", "run_task"];

/// A saved script with functions to call: one that marks a note, one that fetches a URL, one
/// that reads a secret.
const HELPERS: &str = r#"use vault
use http
use secrets

fn mark(id: Value, done: Value) -> Result[string, string] {
    let id = id.as_string().unwrap()
    let saved = vault::patch(id, Value::Map(#{ "done": done }), "")?
    vault::log("marked " + id)
    Ok(id)
}

fn fetch(url: Value) -> Result[string, string] {
    let r = http::get(url.as_string().unwrap(), #{})?
    Ok(r.body)
}

fn key() -> Result[string, string] {
    secrets::get("weather_key")
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

fn sha(text: &str) -> String {
    writer::content_hash(text)
}

fn seed(vault: &std::path::Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/item.wcl"), ITEM).unwrap();
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        Api::start_env(fake, &[])
    }

    fn start_env(fake: &FakeAnthropic, env: &[(&str, &str)]) -> Api {
        let server = TestServer::start_with_env(seed, env);
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
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

    fn put_text(&self, path: &str, text: &str) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(text.to_owned())
            .send()
            .unwrap();
        let status = res.status();
        let body = res.text().unwrap();
        assert!(status.is_success(), "{path}: {body}");
    }

    fn secret(&self, name: &str, value: &str) {
        let (status, v) = self.send(
            reqwest::Method::PUT,
            &format!("/api/secrets/{name}"),
            &self.token(),
            Some(json!({ "value": value })),
        );
        assert!(status.is_success(), "{v}");
    }

    fn agent(&self, name: &str, wcl: &str) {
        self.put_text(&format!("/api/agents/{name}"), wcl);
    }

    fn script(&self, name: &str, source: &str) {
        let (status, v) = self.send(
            reqwest::Method::PUT,
            &format!("/api/scripts/{name}"),
            &self.token(),
            Some(json!({ "source": source })),
        );
        assert!(status.is_success(), "{v}");
    }

    fn task(&self, name: &str, wcl: &str) {
        self.put_text(&format!("/api/tasks/{name}"), wcl);
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
        let mut body = json!({ "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Do it." }] }] });
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

    /// The tool names the last request of a turn by `agent`, as `token`, offered.
    fn offered_to(&self, fake: &FakeAnthropic, token: &str, agent: Option<&str>) -> Vec<String> {
        fake.push(AnthropicMessage::text("Hi."));
        self.chat_as(token, agent);
        let request = fake.message_requests().last().unwrap().json();
        request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
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

/// The tool results the model got back in the last request, as JSON with the error flag.
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

#[test]
fn saved_scripts_and_tasks_are_offered_only_with_run_saved_or_run_code() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("saver", SAVER);
    api.agent("scripter", SCRIPTER);
    api.agent(
        "limited",
        "label = \"Limited\"\n\npermissions {\n  tools = [\"get_note\", \"run_task\"]\n\n  scripts {\n    run_saved = true\n  }\n}\n",
    );
    let admin = api.token();
    for (agent, expected) in [
        (None, &[][..]),
        (Some("saver"), &SAVED_TOOLS[..]),
        (Some("scripter"), &SAVED_TOOLS[..]),
        (Some("limited"), &["run_task"][..]),
    ] {
        let offered = api.offered_to(&fake, &admin, agent);
        let got: Vec<&str> = SAVED_TOOLS
            .iter()
            .copied()
            .filter(|t| offered.iter().any(|o| o == t))
            .collect();
        assert_eq!(got, expected, "{agent:?}");
        // run_saved alone is not run_code.
        if agent == Some("saver") {
            assert!(!offered.contains(&"run_script".to_owned()));
        }
    }
    // A run's changes are made at once, so a `read` caller's turn is offered none; a `write`
    // caller's (an app's) has the agent's own scope.
    for (scope, offers) in [("read", false), ("write", true)] {
        let token = api.token_of(&format!("caller_{scope}"), scope);
        let offered = api.offered_to(&fake, &token, Some("saver"));
        assert_eq!(
            offered.contains(&"run_saved_script".to_owned()),
            offers,
            "{scope}"
        );
        assert_eq!(offered.contains(&"run_task".to_owned()), offers, "{scope}");
    }
    // The instructions say what the tools are for.
    api.offered_to(&fake, &admin, Some("saver"));
    let request = fake.message_requests().last().unwrap().json();
    let system = request["system"].as_str().unwrap();
    assert!(system.contains("run_saved_script"), "{system}");
    // The agent screen reads the switch back.
    let saver = api.get("/api/agents/saver");
    assert_eq!(
        saver["permissions"]["scripts"]["run_saved"], true,
        "{saver}"
    );
    assert_eq!(saver["permissions"]["scripts"]["run_code"], false);
}

#[test]
fn run_saved_script_calls_a_function_with_its_args_as_the_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("saver", SAVER);
    api.agent("offline", OFFLINE);
    api.agent("scripter", SCRIPTER);
    api.script("helpers", HELPERS);
    api.secret("weather_key", "k-123");
    let id = api.create_item("Paint the fence");
    let web = StandIn::start();
    web.route("/forecast", Reply::text(200, "sunny"));
    let url = web.url("/forecast");

    api.calls(
        &fake,
        Some("saver"),
        &[
            ("list_scripts", json!({})),
            (
                "run_saved_script",
                json!({ "script": "helpers", "function": "mark", "args": json!([id, true]).to_string() }),
            ),
            (
                "run_saved_script",
                json!({ "script": "helpers", "function": "fetch", "args": json!([url]).to_string() }),
            ),
            (
                "run_saved_script",
                json!({ "script": "helpers", "function": "key", "args": null }),
            ),
            (
                "run_saved_script",
                json!({ "script": "nowhere", "function": "run", "args": null }),
            ),
            (
                "run_saved_script",
                json!({ "script": "helpers", "function": "absent", "args": null }),
            ),
        ],
    );
    let r = results(&fake);
    let listed = &r[0].1["scripts"];
    let helpers = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "helpers")
        .unwrap_or_else(|| panic!("{listed}"));
    assert_eq!(
        helpers["functions"],
        json!([
            { "name": "fetch", "args": 1 },
            { "name": "key", "args": 0 },
            { "name": "mark", "args": 2 },
        ]),
        "{helpers}"
    );
    // The function ran with both arguments, as the agent.
    assert!(!r[1].0, "{}", r[1].1);
    assert_eq!(r[1].1["result"], json!(id));
    assert_eq!(r[1].1["log"], format!("marked {id}\n"));
    assert_eq!(r[1].1["script"]["kind"], "vault");
    assert_eq!(r[1].1["script"]["script"], "helpers");
    assert_eq!(r[1].1["script"]["hash"], sha(HELPERS));
    let note = api.get(&format!("/api/notes/{id}"));
    assert_eq!(note["note"]["fields"]["done"], true, "{note}");
    let history = api.get(&format!("/api/notes/{id}/history"));
    let latest = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["current"] == true)
        .unwrap();
    assert_eq!(
        latest["author"],
        json!({ "kind": "agent", "name": "saver" }),
        "{history}"
    );
    // The agent reads web pages, so its script reaches the web; it cannot run its own code, so
    // its script reads no secret.
    assert!(!r[2].0, "{}", r[2].1);
    assert_eq!(r[2].1["result"], "sunny");
    assert!(r[3].0, "{}", r[3].1);
    assert!(r[4].0, "{}", r[4].1);
    assert_eq!(r[4].1["kind"], "not_found");
    assert!(r[5].0, "{}", r[5].1);
    assert_eq!(r[5].1["kind"], "entry", "{}", r[5].1);

    // Without web pages, the script reaches no host.
    let before = web.requests().len();
    api.calls(
        &fake,
        Some("offline"),
        &[(
            "run_saved_script",
            json!({ "script": "helpers", "function": "fetch", "args": json!([url]).to_string() }),
        )],
    );
    let r = results(&fake);
    assert!(r[0].0, "{}", r[0].1);
    assert_eq!(web.requests().len(), before);

    // With Run wscript code, it reads every secret, as run_script does.
    api.calls(
        &fake,
        Some("scripter"),
        &[(
            "run_saved_script",
            json!({ "script": "helpers", "function": "key", "args": null }),
        )],
    );
    let r = results(&fake);
    assert!(!r[0].0, "{}", r[0].1);
    assert_eq!(r[0].1["result"], "k-123");

    // Each run is logged with the script's hash and the generation it was read at.
    let log = api.get("/api/activity?kind=ai.script&limit=50");
    let items = log["items"].as_array().unwrap();
    let entry = &items
        .iter()
        .find(|e| e["detail"]["outcome"] == "ok" && e["detail"]["log"] != "")
        .unwrap_or_else(|| panic!("{log}"))["detail"];
    assert_eq!(entry["tool"], "run_saved_script");
    assert_eq!(entry["script"]["kind"], "vault");
    assert_eq!(entry["script"]["script"], "helpers");
    assert_eq!(entry["script"]["hash"], sha(HELPERS));
    assert!(
        entry["script"]["generation"].as_u64().unwrap() > 0,
        "{entry}"
    );
    assert!(entry["ms"].is_u64(), "{entry}");
}

#[test]
fn run_task_runs_it_as_its_run_now_button_does_and_answers_the_log() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // The agent reads no web page, but the task reaches the host it declares.
    api.agent("offline", OFFLINE);
    let web = StandIn::start();
    web.route("/ping", Reply::text(200, "pong"));
    let tally = format!(
        "use vault\nuse http\n\nfn run() -> Result[string, string] {{\n    let r = http::get(\"{}\", #{{}})?\n    vault::log(\"tallied \" + r.body)\n    Ok(\"done\")\n}}\n",
        web.url("/ping")
    );
    api.script("tally", &tally);
    api.task(
        "tally",
        &format!(
            "label = \"Tally\"\nschedule = \"0 0 1 1 *\"\nscript = \"tally\"\nallow_domains = [\"{}\"]\n",
            web.host()
        ),
    );
    api.script(
        "broken",
        "fn run() -> Result[string, string] { Err(\"no luck\") }\n",
    );
    api.task(
        "broken",
        "schedule = \"0 0 1 1 *\"\nscript = \"broken\"\nretries = 0\n",
    );

    api.calls(
        &fake,
        Some("offline"),
        &[
            ("run_task", json!({ "name": "tally" })),
            ("run_task", json!({ "name": "broken" })),
            ("run_task", json!({ "name": "nothing" })),
        ],
    );
    let r = results(&fake);
    assert!(!r[0].0, "{}", r[0].1);
    assert_eq!(r[0].1["started"], false, "{}", r[0].1);
    assert_eq!(r[0].1["result"]["outcome"], "succeeded", "{}", r[0].1);
    assert_eq!(r[0].1["log"], "tallied pong\n", "{}", r[0].1);
    let run_id = r[0].1["run_id"].as_i64().unwrap();
    assert!(r[1].0, "{}", r[1].1);
    assert_eq!(r[1].1["kind"], "task_failed");
    assert!(
        r[1].1["error"].as_str().unwrap().contains("no luck"),
        "{}",
        r[1].1
    );
    assert!(r[2].0, "{}", r[2].1);
    assert_eq!(r[2].1["kind"], "not_found");

    // It is the task's own run, in its history.
    let runs = api.get("/api/tasks/tally/runs");
    assert_eq!(runs["runs"][0]["id"], run_id, "{runs}");
    assert_eq!(runs["runs"][0]["outcome"], "succeeded");

    // list_tasks reads it back.
    api.calls(&fake, Some("offline"), &[("list_tasks", json!({}))]);
    let r = results(&fake);
    let tasks = r[0].1["tasks"].as_array().unwrap();
    let t = tasks.iter().find(|t| t["name"] == "tally").unwrap();
    assert_eq!(t["label"], "Tally");
    assert_eq!(t["script"], "tally");
    assert_eq!(t["latest_run"]["outcome"], "succeeded", "{t}");
    assert!(t["latest_run"].get("log").is_none(), "{t}");
    let b = tasks.iter().find(|t| t["name"] == "broken").unwrap();
    assert_eq!(b["latest_run"]["outcome"], "failed", "{b}");

    // The run is logged with the task and the version of its script.
    let log = api.get("/api/activity?kind=ai.script&limit=50");
    let entry = &log["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["detail"]["outcome"] == "ok")
        .unwrap_or_else(|| panic!("{log}"))["detail"];
    assert_eq!(entry["tool"], "run_task");
    assert_eq!(entry["script"]["kind"], "vault");
    assert_eq!(entry["script"]["task"], "tally");
    assert_eq!(entry["script"]["script"], "tally");
    assert_eq!(entry["script"]["hash"], sha(&tally));
    assert!(
        entry["script"]["generation"].as_u64().unwrap() > 0,
        "{entry}"
    );
}

#[test]
fn a_long_task_answers_that_it_started() {
    let fake = FakeAnthropic::start();
    let api = Api::start_env(&fake, &[("VAULT_TEST_RUN_TASK_WAIT_MS", "500")]);
    api.agent("saver", SAVER);
    let web = StandIn::start();
    web.route(
        "/slow",
        Reply::text(200, "late").delay(Duration::from_secs(3)),
    );
    api.script(
        "slow",
        &format!(
            "use vault\nuse http\n\nfn run() -> Result[string, string] {{\n    let r = http::get(\"{}\", #{{}})?\n    vault::log(r.body)\n    Ok(\"done\")\n}}\n",
            web.url("/slow")
        ),
    );
    api.task(
        "slow",
        &format!(
            "schedule = \"0 0 1 1 *\"\nscript = \"slow\"\nallow_domains = [\"{}\"]\n",
            web.host()
        ),
    );
    let events = api.calls(
        &fake,
        Some("saver"),
        &[("run_task", json!({ "name": "slow" }))],
    );
    assert!(find(&events, "tool_result").is_some());
    let r = results(&fake);
    assert!(!r[0].0, "{}", r[0].1);
    assert_eq!(r[0].1["started"], true, "{}", r[0].1);
    assert_eq!(r[0].1["result"]["started"], true, "{}", r[0].1);
    let id = r[0].1["run_id"].as_i64().unwrap();
    // A second start while it runs is refused, as the route refuses it.
    api.calls(
        &fake,
        Some("saver"),
        &[("run_task", json!({ "name": "slow" }))],
    );
    let r = results(&fake);
    assert!(r[0].0, "{}", r[0].1);
    assert_eq!(r[0].1["kind"], "running", "{}", r[0].1);
    // It goes on and ends.
    let began = Instant::now();
    loop {
        let run = api.get(&format!("/api/tasks/slow/runs/{id}"));
        if run["outcome"] == "succeeded" {
            assert_eq!(run["log"], "late\n");
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(30), "{run}");
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = api.get("/api/activity?kind=ai.script&limit=50");
    let outcomes: Vec<&Value> = log["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["detail"]["outcome"])
        .collect();
    assert!(outcomes.contains(&&json!("started")), "{log}");
}

#[test]
fn run_saved_is_never_granted_without_an_admin() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // A manager that may change agents without asking, but runs no saved script.
    api.agent(
        "manager",
        "label = \"Manager\"\n\npermissions {\n  confirm {\n    agents = false\n  }\n}\n",
    );
    // A boss that runs them and changes agents without asking.
    api.agent(
        "boss",
        "label = \"Boss\"\n\npermissions {\n  confirm {\n    agents = false\n  }\n\n  scripts {\n    run_saved = true\n  }\n}\n",
    );
    let grant = "label = \"Helper\"\n\npermissions {\n  scripts {\n    run_saved = true\n  }\n}\n";
    api.calls(
        &fake,
        Some("manager"),
        &[(
            "propose_agent_change",
            json!({ "name": "helper", "wcl": grant }),
        )],
    );
    let r = results(&fake);
    assert!(r[0].0, "{}", r[0].1);
    assert!(
        r[0].1["error"]
            .as_str()
            .unwrap()
            .contains("Run saved scripts and tasks"),
        "{}",
        r[0].1
    );
    // The boss has it, so it may propose it, but the change waits for an admin's Apply.
    let events = api.calls(
        &fake,
        Some("boss"),
        &[(
            "propose_agent_change",
            json!({ "name": "helper", "wcl": grant }),
        )],
    );
    assert!(
        !events
            .iter()
            .any(|(n, v)| n == "applied" && v["op"] == "agent_create"),
        "{events:?}"
    );
    let p = find(&events, "agent_proposal").unwrap().clone();
    assert_eq!(p["grants_run_saved"], true, "{p}");
    assert_eq!(p["grants_run_code"], false, "{p}");
    let (status, _) = api.send(
        reqwest::Method::GET,
        "/api/agents/helper",
        &api.token(),
        None,
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    let write = api.token_of("phone", "write");
    let (status, body) = api.send(
        reqwest::Method::POST,
        "/api/ai/apply",
        &write,
        Some(p.clone()),
    );
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    let (status, body) = api.send(
        reqwest::Method::POST,
        "/api/ai/apply",
        &api.token(),
        Some(p),
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        api.get("/api/agents/helper")["permissions"]["scripts"]["run_saved"],
        true
    );
}
