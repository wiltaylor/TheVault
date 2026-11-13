//! S1 with the fake Anthropic provider: agents propose scheduled tasks (TV-71, S3 of TV-32).
//!
//! `propose_task_change` and `propose_task_delete` always wait for the user's Apply, with every
//! confirmation switch off too. A change is checked as saving the task checks it, its script must
//! compile as a task, and a file with `read_paths` or `write_paths` is refused, at the proposal
//! and again at Apply. The proposal carries the schedule in words, the next three runs, and
//! warnings for any host, for secrets with hosts and for a secret that does not exist. Apply
//! needs admin, or the copy waiting in the thread of a call from a note; a task changed since
//! gives 409 stale; once applied the task runs on its schedule; a delete keeps its run history.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

/// 03:00 UTC on 2026-01-05, 06 and 07: 03:00 in London in January.
const NEXT: [i64; 3] = [1_767_582_000_000, 1_767_668_400_000, 1_767_754_800_000];

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

const STAMP: &str = r#"fn run() -> Result[(), string] {
    Ok(())
}

fn call(params: Value) -> Value {
    params
}
"#;

const NIGHTLY: &str = "label = \"Nightly\"\nschedule = \"0 3 * * *\"\ntimezone = \"Europe/London\"\nscript = \"stamp\"\nallow_domains = [\"api.example.com\"]\nsecrets = [\"api_key\"]\n";

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

/// Every confirmation switch off.
fn all_off() -> Value {
    json!({
        "notes": false, "archive": false, "delete": false, "types": false, "downloads": false,
        "taxonomies": false, "agents": false, "skills": false, "tools": false,
    })
}

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
            api.server.seed_secret(&name, &value);
        }
        api.ok("PUT", "/api/scripts/stamp", json!({ "source": STAMP }));
        api.ok("PUT", "/api/secrets/api_key", json!({ "value": "k" }));
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

    fn call(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
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

    fn put_task(&self, name: &str, text: &str) {
        self.put_text(&format!("/api/tasks/{name}"), text);
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

    /// The task's file, or `None` when there is no such task.
    fn task(&self, name: &str) -> Option<Value> {
        let (status, v) = self.call("GET", &format!("/api/tasks/{name}"), &self.token(), None);
        status.is_success().then_some(v)
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

    /// A proposal of `propose_task_change`, which must be made.
    fn propose(&self, fake: &FakeAnthropic, name: &str, wcl: &str, script: Option<&str>) -> Value {
        let events = self.one(
            fake,
            "propose_task_change",
            json!({ "name": name, "wcl": wcl, "script_source": script }),
        );
        assert!(find(&events, "applied").is_none(), "{events:?}");
        find(&events, "task_proposal")
            .unwrap_or_else(|| panic!("{events:?} {:?}", last_result(fake)))
            .clone()
    }

    /// A `propose_task_change` that is refused: no proposal, and the error the model reads.
    fn refused(&self, fake: &FakeAnthropic, name: &str, wcl: &str, script: Option<&str>) -> Value {
        let events = self.one(
            fake,
            "propose_task_change",
            json!({ "name": name, "wcl": wcl, "script_source": script }),
        );
        assert!(find(&events, "task_proposal").is_none(), "{events:?}");
        let (err, said) = last_result(fake);
        assert!(err, "{said}");
        said
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
    fn call_note(&self, agent: &str, _call: &str) {
        let login = self
            .client
            .post(self.server.url("/api/login"))
            .json(&json!({ "password": testkit::PASSWORD }))
            .send()
            .unwrap();
        assert!(login.status().is_success());
        let cookie = login.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let saved: Value = self
            .client
            .post(self.server.url("/api/notes?type=memo"))
            .header("cookie", cookie)
            .json(&json!({ "note": { "fields": { "title": "Ask" } } }))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap();
        let issued: Value = self
            .client
            .post(self.server.url("/api/agents/calls"))
            .header("cookie", cookie)
            .json(&json!({ "note": saved["id"], "agent": agent }))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap();
        let call = issued["call"].as_str().unwrap();
        let text = format!("[@Keeper schedule it.](agent:{agent}?call={call})");
        let response = self
            .client
            .put(
                self.server
                    .url(&format!("/api/notes/{}", saved["id"].as_str().unwrap())),
            )
            .header("cookie", cookie)
            .json(&json!({ "base_version": saved["version"], "note": {
                "fields": { "title": "Ask" },
                "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
                    { "kind": "paragraph", "fields": { "text": text } },
                ] }],
            } }))
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().unwrap()
        );
    }

    fn advance(&self, by: &str) -> Value {
        self.ok("POST", "/api/test/clock", json!({ "advance": by }))
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

/// The warnings of a proposal.
fn warnings(p: &Value) -> Vec<String> {
    p["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_new_task_waits_for_apply_with_every_switch_off_then_runs_on_its_schedule() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "confirm": all_off() }),
    );

    // A proposal, never made at once, with the schedule in words, its next runs and its grants.
    let p = api.propose(&fake, "nightly", NIGHTLY, None);
    assert_eq!(p["kind"], "task");
    assert_eq!(p["op"], "create");
    assert_eq!(p["exists"], false);
    assert!(p["before"].is_null());
    assert_eq!(p["proposer"], "assistant");
    assert_eq!(p["script"], "stamp");
    assert_eq!(p["script_exists"], true);
    assert!(p["script_after"].is_null());
    let task = &p["task"];
    assert_eq!(task["schedule"], "0 3 * * *");
    assert_eq!(task["timezone"], "Europe/London");
    assert!(
        task["schedule_text"]
            .as_str()
            .unwrap()
            .ends_with("(Europe/London)"),
        "{task}"
    );
    assert_eq!(task["allow_domains"], json!(["api.example.com"]));
    assert_eq!(task["secrets"], json!(["api_key"]));
    assert_eq!(task["paused"], false);
    assert_eq!(p["next_runs"], json!(NEXT));
    // Secrets with hosts: a warning that the task could send them out (TV-32 R4).
    let w = warnings(&p);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("may read the secrets api_key and reach api.example.com"),
        "{w:?}"
    );
    let (err, said) = last_result(&fake);
    assert!(!err && said["proposed"] == true, "{said}");
    assert_eq!(said["next_runs"], json!(NEXT));
    assert!(api.task("nightly").is_none());

    // A write token cannot apply a body it sends; the admin can, once.
    let write = api.token_of("phone", "write");
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    assert!(api.task("nightly").is_none());
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "op": "task_create", "task_name": "nightly", "script_name": null, "next_run": NEXT[0] })
    );
    let got = api.task("nightly").unwrap();
    assert_eq!(got["next_run"], NEXT[0], "{got}");
    // Applied again: the task exists now, so it is stale and nothing is written.
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");

    // The activity log names the apply and the agent that proposed it.
    let activity = api.get("/api/activity?kind=ai.applied");
    let entry = activity["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["summary"] == "Applied: create task nightly")
        .unwrap_or_else(|| panic!("{activity}"));
    assert_eq!(entry["detail"]["proposer"], "assistant", "{entry}");

    // The new task runs on its schedule (TV-32 Q5).
    let ran = api.advance("PT3H");
    let runs = ran["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "{ran}");
    assert_eq!(runs[0]["task"], "nightly");
    assert_eq!(runs[0]["outcome"], "succeeded", "{ran}");

    // The same file: nothing to apply.
    let after = p["after"].as_str().unwrap();
    let events = api.one(
        &fake,
        "propose_task_change",
        json!({ "name": "nightly", "wcl": after, "script_source": null }),
    );
    assert!(find(&events, "task_proposal").is_none());
    assert_eq!(last_result(&fake).1["proposed"], false);

    // get_task reads the file, the task, its next runs, its script and the vocabulary.
    api.one(&fake, "get_task", json!({ "name": "nightly" }));
    let (err, got) = last_result(&fake);
    assert!(!err, "{got}");
    assert_eq!(got["wcl"], after);
    assert_eq!(got["task"]["script"], "stamp");
    assert_eq!(got["script_source"], STAMP);
    assert_eq!(got["next_runs"].as_array().unwrap().len(), 3, "{got}");
    assert!(
        got["format"].as_str().unwrap().contains("read_paths"),
        "{got}"
    );
    // A proposal that does not write the script still shows it, and goes stale when the script
    // changes before Apply (TV-71 review F3).
    let p = api.propose(&fake, "nightly", &format!("{after}paused = true\n"), None);
    assert_eq!(p["script_before"], STAMP);
    assert!(p["script_after"].is_null());
    let edited = format!("{STAMP}\nfn extra() -> int {{\n    1\n}}\n");
    api.ok("PUT", "/api/scripts/stamp", json!({ "source": edited }));
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert_eq!(api.task("nightly").unwrap()["paused"], false);

    api.one(&fake, "get_task", json!({ "name": "nope" }));
    let (err, got) = last_result(&fake);
    assert!(
        err && got["error"].as_str().unwrap().contains("nightly"),
        "{got}"
    );
}

#[test]
fn internal_secrets_are_refused_in_task_proposals_and_apply() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let p = api.propose(&fake, "nightly", NIGHTLY, Some(STAMP));
    for name in [
        "ai_accounts",
        "ai_account_chatgpt_test",
        "gitsync_ssh_key",
        "brave_api_key",
        "google_client_secret",
    ] {
        let wcl = NIGHTLY.replace("api_key", name);
        let said = api.refused(&fake, "nightly", &wcl, Some(STAMP));
        assert!(
            said.to_string().contains("reserved for the server"),
            "{said}"
        );
        let mut old = p.clone();
        old["after"] = json!(wcl);
        let (status, body) = api.apply_as(&api.token(), &old);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.to_string().contains("reserved for the server"),
            "{body}"
        );
        assert!(api.task("nightly").is_none());
    }
}

#[test]
fn fs_paths_are_refused_and_any_host_or_a_missing_secret_warns() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);

    // read_paths or write_paths from an agent: refused, nothing written (TV-32 Q3).
    for extra in ["read_paths = [\"/\"]\n", "write_paths = [\"/srv/out\"]\n"] {
        let said = api.refused(&fake, "mover", &format!("{NIGHTLY}{extra}"), None);
        assert!(
            said["error"]
                .as_str()
                .unwrap()
                .contains("may not propose read_paths or write_paths"),
            "{said}"
        );
        assert!(api.task("mover").is_none());
    }
    // A new source for the shared script, proposed while nothing with paths runs it.
    let changed = format!("{STAMP}\nfn extra() -> int {{\n    1\n}}\n");
    let sharer = api.propose(&fake, "sharer", NIGHTLY, Some(&changed));
    assert_eq!(sharer["script_after"], changed.as_str());

    // A task the user gave paths cannot be proposed with them either.
    api.put_task("mover", &format!("{NIGHTLY}read_paths = [\"/srv/in\"]\n"));
    let said = api.refused(
        &fake,
        "mover",
        &format!("{NIGHTLY}read_paths = [\"/srv/in\"]\npaused = true\n"),
        None,
    );
    assert!(
        said["error"]
            .as_str()
            .unwrap()
            .contains("only the user changes"),
        "{said}"
    );

    // Nor may an agent change the code that task runs (TV-71 review F1): a new source for its
    // script is refused, naming the task and its folders; the same task without a source is not.
    let said = api.refused(&fake, "other", NIGHTLY, Some(&changed));
    let why = said["error"].as_str().unwrap();
    assert!(why.contains("task mover (reads /srv/in,"), "{said}");
    assert!(why.contains("may not change script \"stamp\""), "{said}");
    api.propose(&fake, "other", NIGHTLY, None);
    // The proposal made before the paths were given is refused at Apply.
    let (status, body) = api.apply_as(&api.token(), &sharer);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "not_permitted");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
    assert!(api.task("sharer").is_none());

    // An admin applying a body with paths in it: refused at Apply too.
    let p = api.propose(&fake, "lister", NIGHTLY, None);
    let mut forged = p.clone();
    forged["after"] = json!(format!("{NIGHTLY}read_paths = [\"/\"]\n"));
    let (status, body) = api.apply_as(&api.token(), &forged);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "not_permitted");
    assert!(api.task("lister").is_none());

    // Any host is allowed with a warning (TV-32 Q4); a secret that does not exist warns too.
    let open = NIGHTLY
        .replace("[\"api.example.com\"]", "[\"*\"]")
        .replace("[\"api_key\"]", "[\"api_key\", \"missing_key\"]");
    let p = api.propose(&fake, "open", &open, None);
    let w = warnings(&p);
    assert_eq!(w.len(), 3, "{w:?}");
    assert!(
        w[0].starts_with("The task open may reach any host"),
        "{w:?}"
    );
    assert!(w[0].contains("local network"), "{w:?}");
    assert!(w[1].contains("and reach any host"), "{w:?}");
    assert!(w[2].contains("There is no secret missing_key yet"), "{w:?}");

    // A paused task names no next runs.
    let p = api.propose(&fake, "later", &format!("{NIGHTLY}paused = true\n"), None);
    assert_eq!(p["next_runs"], json!([]));
    assert_eq!(p["task"]["paused"], true);
}

#[test]
fn a_task_with_its_script_is_written_in_one_apply_and_changes_and_deletes_are_checked() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let digest_task = "schedule = \"0 7 * * 1-5\"\nscript = \"digest\"\n";
    let digest = "fn run() -> Result[(), string] {\n    Ok(())\n}\n";

    // A missing script with no source, a source that does not compile, and a function that
    // takes arguments: refused, with why.
    let said = api.refused(&fake, "digest", digest_task, None);
    assert!(said.to_string().contains("there is no script"), "{said}");
    let said = api.refused(&fake, "digest", digest_task, Some("fn run( {"));
    assert!(!said["problems"].as_array().unwrap().is_empty(), "{said}");
    let said = api.refused(
        &fake,
        "caller",
        "schedule = \"0 7 * * *\"\nscript = \"stamp\"\nfunction = \"call\"\n",
        None,
    );
    assert!(
        said["problems"][0]
            .as_str()
            .unwrap()
            .contains("takes 1 arguments"),
        "{said}"
    );
    assert!(api.task("digest").is_none());
    assert!(api.script("digest").is_none());

    // A new task with a new script: both written on Apply.
    let p = api.propose(&fake, "digest", digest_task, Some(digest));
    assert_eq!(p["script"], "digest");
    assert_eq!(p["script_exists"], false);
    assert!(p["script_before"].is_null());
    assert_eq!(p["script_after"], digest);
    assert!(api.script("digest").is_none());
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "task_create");
    assert_eq!(body["script_name"], "digest");
    assert_eq!(api.script("digest").as_deref(), Some(digest));
    assert!(api.task("digest").is_some());

    // A change, then the user edits the task first: stale, nothing written.
    let p = api.propose(
        &fake,
        "digest",
        "schedule = \"0 8 * * 1-5\"\nscript = \"digest\"\n",
        None,
    );
    assert_eq!(p["op"], "change");
    assert_eq!(p["exists"], true);
    assert!(p["before"].as_str().unwrap().contains("0 7 * * 1-5"));
    api.put_task(
        "digest",
        "schedule = \"0 9 * * 1-5\"\nscript = \"digest\"\n",
    );
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert_eq!(api.task("digest").unwrap()["schedule"], "0 9 * * 1-5");

    // A proposal made before another task started to run the script: Apply re-checks what runs
    // it under the write lock, and finds it changed (TV-71 review F2).
    let early = api.propose(
        &fake,
        "digest",
        "schedule = \"0 10 * * 1-5\"\nscript = \"digest\"\n",
        None,
    );
    assert_eq!(early["runs_as"], json!([]));

    // A change of a shared script lists what else runs it.
    api.put_task(
        "other",
        "schedule = \"0 1 * * *\"\nscript = \"digest\"\nallow_domains = [\"*\"]\n",
    );
    let (status, body) = api.apply_as(&api.token(), &early);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("what runs the script digest changed"),
        "{body}"
    );
    assert_eq!(api.task("digest").unwrap()["schedule"], "0 9 * * 1-5");
    let changed = format!("{digest}\nfn extra() -> int {{\n    1\n}}\n");
    let refused = api.refused(
        &fake,
        "digest",
        "schedule = \"0 9 * * 1-5\"\nscript = \"digest\"\n",
        Some(&changed),
    );
    assert!(refused.to_string().contains("task other (hosts *)"));
    // File-only changes still show shared roles, without a source-change warning.
    let p = api.propose(
        &fake,
        "digest",
        "schedule = \"0 10 * * 1-5\"\nscript = \"digest\"\n",
        None,
    );
    assert_eq!(p["runs_as"].as_array().unwrap().len(), 1, "{p}");
    assert_eq!(p["runs_as"][0]["name"], "other");
    let w = warnings(&p);
    assert!(w.is_empty(), "{w:?}");

    // A delete keeps the script and the run history.
    let ran = api.ok("POST", "/api/tasks/digest/run?wait=true", json!({}));
    assert_eq!(ran["outcome"], "succeeded", "{ran}");
    let events = api.one(&fake, "propose_task_delete", json!({ "name": "digest" }));
    let p = find(&events, "task_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["op"], "delete");
    assert!(p["before"].as_str().unwrap().contains("0 9 * * 1-5"));
    assert!(api.task("digest").is_some());
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "task_delete");
    assert!(api.task("digest").is_none());
    assert_eq!(api.script("digest").as_deref(), Some(digest));
    let runs = api.get("/api/tasks/digest/runs");
    assert_eq!(runs["runs"].as_array().unwrap().len(), 1, "{runs}");
    // A missing task names those there are.
    api.one(&fake, "propose_task_delete", json!({ "name": "digest" }));
    let (err, said) = last_result(&fake);
    assert!(
        err && said["error"].as_str().unwrap().contains("other"),
        "{said}"
    );
}

#[test]
fn a_call_from_a_note_leaves_a_task_proposal_that_a_phone_applies_from_the_thread() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.put_text("/api/agents/keeper", KEEPER);

    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "propose_task_change",
        json!({ "name": "nightly", "wcl": NIGHTLY, "script_source": null }),
    ));
    fake.push(AnthropicMessage::text("The task waits for you."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
    api.advance("PT1S");
    let first = fake.message_requests()[0].json();
    assert!(
        first["tools"]
            .to_string()
            .contains("\"propose_task_change\"")
    );

    // Every switch is off, yet the task waits in the thread.
    assert!(api.task("nightly").is_none());
    let threads = api.threads("keeper");
    assert_eq!(threads.len(), 1, "{threads:?}");
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
    assert_eq!(stored["event"], "task_proposal");

    // A write token applies the copy the thread holds, not a body it changed.
    let write = api.token_of("phone", "write");
    let mut forged = stored["data"].clone();
    forged["after"] = json!(format!("{NIGHTLY}read_paths = [\"/\"]\n"));
    let (status, body) = api.apply_as(&write, &forged);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = api.apply_as(&api.token(), &stored["data"]);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "task_create");
    let got = api.task("nightly").unwrap();
    assert_eq!(got["read_paths"], json!([]), "{got}");
    assert_eq!(api.threads("keeper")[0]["pending"], 0);
}

/// An agent whose trigger runs when a memo is created, with every switch off (TV-71 review F4).
const WATCHER: &str = r#"label = "Watcher"

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

trigger opened {
  on     = ["create"]
  type   = "memo"
  prompt = "Look after the memo."
}
"#;

#[test]
fn a_trigger_run_is_not_offered_the_task_proposals_and_a_call_is_refused_without_a_proposal() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.put_text("/api/agents/watcher", WATCHER);
    // The model calls the tool although the run does not offer it.
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_w",
        "propose_task_change",
        json!({ "name": "nightly", "wcl": NIGHTLY, "script_source": null }),
    ));
    fake.push(AnthropicMessage::text("Needs the user."));
    api.ok(
        "POST",
        "/api/notes?type=memo",
        json!({ "note": { "fields": { "title": "Roof" } } }),
    );
    api.advance("PT31S");
    let requests = fake.message_requests();
    assert_eq!(requests.len(), 2);
    let offered = requests[0].json()["tools"].to_string();
    assert!(!offered.contains("\"propose_task_change\""), "{offered}");
    assert!(!offered.contains("\"propose_task_delete\""), "{offered}");
    assert!(offered.contains("\"get_task\""), "{offered}");
    // The call is refused before any check, as a tool the run does not have, so it never
    // reaches the HELD_REFUSAL of a proposal that would wait: nothing waits, nothing is written.
    let (err, said) = last_result(&fake);
    assert!(err, "{said}");
    assert_eq!(
        said["error"], "there is no tool named \"propose_task_change\"",
        "{said}"
    );
    let threads = api.threads("watcher");
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(threads[0]["pending"], 0, "{threads:?}");
    assert!(api.task("nightly").is_none());
}
