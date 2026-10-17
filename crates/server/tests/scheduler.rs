//! S1 with S7: scheduled tasks on a server whose clock only the test moves, calling S7's
//! stand-in HTTP server (D10).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::{Reply, StandIn, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";
const START_MS: i64 = 1_767_573_000_000;
const MINUTE: i64 = 60_000;

/// `START` moved by `minutes`, in milliseconds since the epoch, as the API reports times.
fn at(minutes: i64) -> Value {
    json!(START_MS + minutes * MINUTE)
}

const ITEM: &str = r#"label = "Item"

field estimate {
  kind = "int"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate"
  }
}
"#;

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/item.wcl"), ITEM).unwrap();
}

struct Api {
    server: TestServer,
    client: Client,
}

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

impl Api {
    fn start() -> Api {
        let server =
            TestServer::start_with_args(seed, &[OsStr::new("--test-clock"), OsStr::new(START)]);
        let client = client(server.admin_token());
        Api { server, client }
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.client.get(self.server.url(path))
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.client.post(self.server.url(path))
    }

    fn put(&self, path: &str) -> RequestBuilder {
        self.client.put(self.server.url(path))
    }

    fn script(&self, name: &str, source: &str) {
        expect(
            self.put(&format!("/api/scripts/{name}"))
                .json(&json!({ "source": source }))
                .send()
                .unwrap(),
            200,
        );
    }

    fn task(&self, name: &str, wcl: &str) -> Value {
        expect(
            self.put(&format!("/api/tasks/{name}"))
                .body(wcl.to_owned())
                .send()
                .unwrap(),
            200,
        )
    }

    /// Move the test clock forward and return the runs that the move started.
    fn advance(&self, by: &str) -> Vec<Value> {
        let out = expect(
            self.post("/api/test/clock")
                .json(&json!({ "advance": by }))
                .send()
                .unwrap(),
            200,
        );
        out["runs"].as_array().unwrap().clone()
    }

    fn runs(&self, task: &str) -> Vec<Value> {
        let out = expect(
            self.get(&format!("/api/tasks/{task}/runs")).send().unwrap(),
            200,
        );
        out["runs"].as_array().unwrap().clone()
    }

    fn trigger(&self, task: &str) -> Value {
        expect(
            self.post(&format!("/api/tasks/{task}/run")).send().unwrap(),
            200,
        )
    }
}

#[test]
fn advancing_the_clock_past_a_cron_time_runs_the_task_exactly_once() {
    let api = Api::start();
    api.script("hourly", "use vault\nfn run() { vault::log(\"tick\") }\n");
    let task = api.task("hourly", "schedule = \"0 * * * *\"\nscript = \"hourly\"\n");
    assert_eq!(task["next_run"], at(30));

    // 00:59: not yet.
    assert!(api.advance("PT29M").is_empty());
    assert!(api.runs("hourly").is_empty());

    // 01:00: once.
    let ran = api.advance("PT1M");
    assert_eq!(ran.len(), 1, "{ran:?}");
    assert_eq!(ran[0]["outcome"], json!("succeeded"));
    assert_eq!(ran[0]["attempt"], json!(1));
    assert_eq!(ran[0]["started"], at(30));
    assert_eq!(ran[0]["log"], json!("tick\n"));

    // Moving within the same hour, or ticking again at the same time, runs nothing more.
    assert!(api.advance("PT10M").is_empty());
    assert!(api.advance("PT0S").is_empty());

    // Jumping past five firings at once runs the task once, not five times.
    let ran = api.advance("PT5H");
    assert_eq!(ran.len(), 1, "{ran:?}");
    assert_eq!(api.runs("hourly").len(), 2);
    let status = expect(api.get("/api/tasks/hourly").send().unwrap(), 200);
    assert_eq!(status["next_run"], at(390));
    assert_eq!(status["latest_run"]["outcome"], json!("succeeded"));
    assert_eq!(status["failing"], json!(false));
}

#[test]
fn a_task_calls_the_stand_in_with_a_secret_header_and_updates_a_note() {
    let api = Api::start();
    let stand_in = StandIn::start();
    stand_in.route("/greeting", Reply::text(200, "Fetched title"));

    let note = expect(
        api.post("/api/notes")
            .json(&json!({ "type": "item", "note": { "fields": { "title": "Before", "estimate": 1 } } }))
            .send()
            .unwrap(),
        201,
    );
    let id = note["id"].as_str().unwrap();

    expect(
        api.put("/api/secrets/greeting_key")
            .json(&json!({ "value": "s3cret-value" }))
            .send()
            .unwrap(),
        200,
    );
    api.script(
        "fetch",
        &format!(
            r#"use vault
use http
use secrets
fn run() -> Result[string, string] {{
    let key = secrets::get("greeting_key")?
    let r = http::get("{url}", #{{ "x-api-key": key }})?
    if r.status != 200 {{
        return Err(fmt("the API answered {{}}", r.status))
    }}
    let note = vault::get("{id}")?
    let version = note.get("version").unwrap().as_string().unwrap()
    let body = Value::Map(#{{
        "fields": Value::Map(#{{
            "title": Value::String(r.body),
            "estimate": Value::Int(2),
        }}),
    }})
    vault::update("{id}", body, version)?
    vault::log("updated")
    Ok("done")
}}
"#,
            url = stand_in.url("/greeting"),
        ),
    );
    api.task(
        "fetch",
        &format!(
            "schedule = \"0 0 1 1 *\"\nscript = \"fetch\"\nallow_domains = [\"{}\"]\nsecrets = [\"greeting_key\"]\n",
            stand_in.host()
        ),
    );

    let run = api.trigger("fetch");
    assert_eq!(run["outcome"], json!("succeeded"), "{run}");
    assert_eq!(run["log"], json!("updated\n"));

    let seen = stand_in.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].header("x-api-key"), Some("s3cret-value"));

    let after = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
    assert_eq!(after["note"]["fields"]["title"], json!("Fetched title"));
    assert_eq!(after["note"]["fields"]["estimate"], json!(2));
    let history = expect(
        api.get(&format!("/api/notes/{id}/history")).send().unwrap(),
        200,
    );
    let authors: Vec<&Value> = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| &v["author"])
        .collect();
    assert!(
        authors.iter().any(|a| a.to_string().contains("fetch")),
        "{history}"
    );
}

#[test]
fn a_task_calling_a_domain_off_its_allowlist_fails_and_the_run_says_why() {
    let api = Api::start();
    let stand_in = StandIn::start();
    stand_in.route("/data", Reply::text(200, "secret data"));
    api.script(
        "leak",
        &format!(
            "use http\nfn run() -> Result[int, string] {{ Ok(http::get(\"{}\", #{{}})?.status) }}\n",
            stand_in.url("/data")
        ),
    );
    api.task(
        "leak",
        "schedule = \"0 0 1 1 *\"\nscript = \"leak\"\nallow_domains = [\"api.example.com\"]\nretries = 0\n",
    );

    let run = api.trigger("leak");
    assert_eq!(run["outcome"], json!("failed"));
    let message = run["message"].as_str().unwrap();
    assert!(
        message.contains("\"127.0.0.1\" is not on this script's allowlist"),
        "{message}"
    );
    assert!(stand_in.requests().is_empty());

    let runs = api.runs("leak");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["message"], json!(message));
    // No retries were asked for, so none come.
    assert!(api.advance("P1D").is_empty());
}

#[test]
fn a_failing_task_retries_with_increasing_delays_and_the_latest_run_shows_failed() {
    let api = Api::start();
    api.script(
        "flaky",
        "fn run() -> Result[int, string] { Err(\"the upstream is down\") }\n",
    );
    api.task(
        "flaky",
        "schedule = \"0 0 1 1 *\"\nscript = \"flaky\"\nretries = 2\nretry_delay = \"PT1M\"\n",
    );

    let first = api.trigger("flaky");
    assert_eq!(first["outcome"], json!("failed"));
    assert_eq!(first["message"], json!("the upstream is down"));
    let status = expect(api.get("/api/tasks/flaky").send().unwrap(), 200);
    assert_eq!(
        status["retry"],
        json!({ "attempt": 2, "at": START_MS + MINUTE })
    );

    // The first retry waits one minute.
    assert!(api.advance("PT59S").is_empty());
    let second = api.advance("PT1S");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0]["attempt"], json!(2));
    assert_eq!(second[0]["started"], at(1));

    // The second waits two.
    assert!(api.advance("PT1M59S").is_empty());
    let third = api.advance("PT1S");
    assert_eq!(third.len(), 1);
    assert_eq!(third[0]["attempt"], json!(3));
    assert_eq!(third[0]["started"], at(3));

    // Two retries were allowed, so there is no fourth try.
    assert!(api.advance("PT1H").is_empty());

    let runs = api.runs("flaky");
    let attempts: Vec<&Value> = runs.iter().map(|r| &r["attempt"]).collect();
    assert_eq!(attempts, [&json!(3), &json!(2), &json!(1)]);
    assert!(runs.iter().all(|r| r["outcome"] == json!("failed")));

    let status = expect(api.get("/api/tasks/flaky").send().unwrap(), 200);
    assert_eq!(status["failing"], json!(true));
    assert_eq!(status["latest_run"]["outcome"], json!("failed"));
    assert_eq!(status["latest_run"]["attempt"], json!(3));
    assert_eq!(status["retry"], Value::Null);
    let list = expect(api.get("/api/tasks").send().unwrap(), 200);
    assert_eq!(list["tasks"][0]["name"], json!("flaky"));
    assert_eq!(list["tasks"][0]["failing"], json!(true));
}

#[test]
fn the_secrets_api_never_returns_a_secret_value() {
    let api = Api::start();
    const VALUE: &str = "hunter2-do-not-show";
    let put = api
        .put("/api/secrets/webhook")
        .json(&json!({ "value": VALUE }))
        .send()
        .unwrap();
    let put = put.text().unwrap();
    assert!(!put.contains(VALUE), "{put}");

    let mut bodies = vec![put];
    for path in ["/api/secrets", "/api/secrets/webhook"] {
        let res = api.get(path).send().unwrap();
        assert_eq!(res.status().as_u16(), 200);
        bodies.push(res.text().unwrap());
    }
    let list: Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(list["secrets"][0]["name"], json!("webhook"));
    let one: Value = serde_json::from_str(&bodies[2]).unwrap();
    assert_eq!(one["name"], json!("webhook"));
    assert!(one.get("value").is_none());
    for body in &bodies {
        assert!(!body.contains(VALUE), "{body}");
    }

    // Replacing works without showing either value.
    let res = api
        .put("/api/secrets/webhook")
        .json(&json!({ "value": "another" }))
        .send()
        .unwrap();
    assert!(!res.text().unwrap().contains("another"));

    // Secrets are admin-only, even to list.
    let reader = expect(
        api.post("/api/tokens")
            .json(&json!({ "name": "reader", "scope": "read" }))
            .send()
            .unwrap(),
        201,
    );
    let reader = client(reader["secret"].as_str().unwrap());
    let res = reader.get(api.server.url("/api/secrets")).send().unwrap();
    assert_eq!(res.status().as_u16(), 403);

    expect(
        api.client
            .delete(api.server.url("/api/secrets/webhook"))
            .send()
            .unwrap(),
        200,
    );
    let res = api.get("/api/secrets/webhook").send().unwrap();
    assert_eq!(res.status().as_u16(), 404);
}

#[test]
fn task_files_are_checked_and_changing_tasks_needs_admin() {
    let api = Api::start();
    api.script("noop", "fn run() { }\n");

    let res = api
        .put("/api/tasks/bad")
        .body("schedule = \"every day\"\nscript = \"missing\"\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["error"], json!("invalid"));
    assert_eq!(err["issues"][0]["at"], json!("schedule"));

    let res = api
        .put("/api/tasks/bad")
        .body("schedule = \"0 * * * *\"\nscript = \"missing\"\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["issues"][0]["at"], json!("script"));
    assert!(!api.server.vault_path().join("tasks/bad.wcl").exists());

    let task = api.task(
        "noop",
        "schedule=\"*/5 * * * *\"\nscript=\"noop\"\npaused=true\n",
    );
    assert_eq!(task["paused"], json!(true));
    assert_eq!(task["next_run"], Value::Null);
    let on_disk = std::fs::read_to_string(api.server.vault_path().join("tasks/noop.wcl")).unwrap();
    assert_eq!(on_disk, task["wcl"].as_str().unwrap());

    // A paused task never fires on its schedule, but can be triggered by hand.
    assert!(api.advance("PT1H").is_empty());
    assert_eq!(api.trigger("noop")["outcome"], json!("succeeded"));

    // A script a task names cannot be deleted.
    let res = api
        .client
        .delete(api.server.url("/api/scripts/noop"))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 409);

    let writer = expect(
        api.post("/api/tokens")
            .json(&json!({ "name": "writer", "scope": "write" }))
            .send()
            .unwrap(),
        201,
    );
    let writer = client(writer["secret"].as_str().unwrap());
    let res = writer
        .put(api.server.url("/api/tasks/noop"))
        .body("schedule=\"* * * * *\"\nscript=\"noop\"\n")
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 403);
    let res = writer
        .post(api.server.url("/api/tasks/noop/run"))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 403);
    // Reading run history needs only read.
    let res = writer
        .get(api.server.url("/api/tasks/noop/runs"))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);

    expect(
        api.client
            .delete(api.server.url("/api/tasks/noop"))
            .send()
            .unwrap(),
        200,
    );
    let res = api.get("/api/tasks/noop").send().unwrap();
    assert_eq!(res.status().as_u16(), 404);
    // Only the shipped, paused Gmail sync is left.
    let left: Vec<Value> = expect(api.get("/api/tasks").send().unwrap(), 200)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].clone())
        .collect();
    assert_eq!(left, [json!("google_gmail")]);
}

/// Debug builds are the test builds: they accept `--test-clock`. A release build refuses to
/// start with it. Run this test with `--release` to check the release side.
#[test]
fn only_test_builds_accept_the_test_clock() {
    let vault = tempfile::tempdir().unwrap();
    testkit::seed_credentials(vault.path());
    let mut child = Command::new(testkit::server_binary())
        .arg("--vault")
        .arg(vault.path())
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--test-clock")
        .arg(START)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    if cfg!(debug_assertions) {
        // It starts and prints its address.
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().unwrap()),
            &mut line,
        )
        .unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(line.contains("listen"), "{line:?}");
    } else {
        let out = child.wait_with_output().unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("--test-clock is accepted only by test builds"),
            "{stderr}"
        );
    }
}

#[test]
fn the_test_clock_endpoint_exists_only_with_the_flag() {
    let plain = TestServer::start_seeded(seed);
    let res = client(plain.admin_token())
        .post(plain.url("/api/test/clock"))
        .json(&json!({ "advance": "PT1M" }))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);

    let api = Api::start();
    let now = expect(api.get("/api/test/clock").send().unwrap(), 200);
    assert_eq!(now["now"], json!(START_MS));
    let out = expect(
        api.post("/api/test/clock")
            .json(&json!({ "set": "2026-02-01T00:00:00Z" }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(out["now"], json!(1_769_904_000_000_i64));
}

#[test]
fn the_cron_preview_describes_a_schedule_and_lists_the_times_the_scheduler_uses() {
    let api = Api::start();
    let res = api
        .get("/api/cron")
        .query(&[
            ("schedule", "0 7 * * 1-5"),
            ("timezone", "Australia/Melbourne"),
            ("count", "5"),
        ])
        .send()
        .unwrap();
    let preview = expect(res, 200);
    assert_eq!(
        preview["text"],
        json!("Weekdays at 07:00 (Australia/Melbourne)")
    );
    let next = preview["next"].as_array().unwrap();
    assert_eq!(next.len(), 5);
    // Monday 2026-01-05 00:30 UTC is 11:30 in Melbourne (AEDT, +11), so the first run is
    // Tuesday 07:00 there: Monday 20:00 UTC. Then each weekday, skipping the weekend.
    assert_eq!(next[0], json!(1_767_643_200_000_i64));
    assert_eq!(next[3], json!(1_767_643_200_000_i64 + 3 * 24 * 60 * MINUTE));
    assert_eq!(next[4], json!(1_767_643_200_000_i64 + 6 * 24 * 60 * MINUTE));

    // The task with the same schedule fires first at the same time.
    api.script("digest", "fn run() { }\n");
    let task = api.task(
        "digest",
        "schedule = \"0 7 * * 1-5\"\ntimezone = \"Australia/Melbourne\"\nscript = \"digest\"\n",
    );
    assert_eq!(task["next_run"], next[0]);
    assert_eq!(
        task["schedule_text"],
        json!("Weekdays at 07:00 (Australia/Melbourne)")
    );

    // What the scheduler refuses, the preview refuses with the same issues.
    let res = api
        .get("/api/cron")
        .query(&[("schedule", "61 * * * *"), ("timezone", "Mars/Olympus")])
        .send()
        .unwrap();
    let err = expect(res, 400);
    let at: Vec<&Value> = err["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| &i["at"])
        .collect();
    assert_eq!(at, [&json!("schedule"), &json!("timezone")], "{err}");
}

#[test]
fn a_scheduled_run_fetches_from_any_host_writes_a_file_and_changes_notes() {
    let api = Api::start();
    let stand_in = StandIn::start();
    stand_in.route("/weather", Reply::text(200, "Sunny"));
    let out = tempfile::tempdir().unwrap();
    let out_dir = std::fs::canonicalize(out.path()).unwrap();

    let note = expect(
        api.post("/api/notes")
            .json(&json!({ "type": "item", "note": { "fields": { "title": "Counter", "estimate": 1 } } }))
            .send()
            .unwrap(),
        201,
    );
    let id = note["id"].as_str().unwrap();
    api.script(
        "weather",
        &r#"use vault
use http
use fs
fn run() -> Result[string, string] {
    let r = http::get("$URL", #{})?
    fs::write_text(fs::join("$OUT", "weather.txt"), r.body)?
    let made = vault::create("item", Value::Map(#{
        "fields": Value::Map(#{ "title": Value::String("Weather: " + r.body) }),
    }))?
    vault::patch("$ID", Value::Map(#{ "estimate": Value::Int(2) }), "")?
    vault::log("made " + made.get("id").unwrap().as_string().unwrap())
    Ok("done")
}
"#
        .replace("$URL", &stand_in.url("/weather"))
        .replace("$OUT", &out_dir.to_string_lossy())
        .replace("$ID", id),
    );
    let task = api.task(
        "weather",
        &format!(
            "schedule = \"0 * * * *\"\nscript = \"weather\"\nallow_domains = [\"*\"]\nwrite_paths = [\"{}\"]\nretries = 0\n",
            out_dir.display()
        ),
    );
    assert_eq!(task["allow_domains"], json!(["*"]));
    assert_eq!(task["write_paths"], json!([out_dir.to_string_lossy()]));
    // The file is stored canonical, and putting it back changes nothing.
    let wcl = task["wcl"].as_str().unwrap().to_owned();
    let on_disk =
        std::fs::read_to_string(api.server.vault_path().join("tasks/weather.wcl")).unwrap();
    assert_eq!(on_disk, wcl);
    assert_eq!(api.task("weather", &wcl)["wcl"], json!(wcl));

    // 01:00: the schedule fires.
    let ran = api.advance("PT30M");
    assert_eq!(ran.len(), 1, "{ran:?}");
    assert_eq!(ran[0]["outcome"], json!("succeeded"), "{}", ran[0]);
    assert_eq!(
        std::fs::read_to_string(out_dir.join("weather.txt")).unwrap(),
        "Sunny"
    );
    let rows = expect(
        api.get("/api/query")
            .query(&[("q", "from item where title == \"Weather: Sunny\"")])
            .send()
            .unwrap(),
        200,
    );
    let made = rows["rows"][0]["id"].as_str().unwrap().to_owned();
    let counter = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
    assert_eq!(counter["note"]["fields"]["estimate"], json!(2));
    assert_eq!(counter["note"]["fields"]["title"], json!("Counter"));

    // The changes are recorded as the task's.
    let history = expect(
        api.get(&format!("/api/notes/{made}/history"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(
        history["versions"][0]["author"],
        json!({ "kind": "task", "name": "weather" })
    );

    // The run history keeps the run with its log.
    let runs = api.runs("weather");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["log"], json!(format!("made {made}\n")));
    assert!(runs[0]["ended"].as_i64().unwrap() >= runs[0]["started"].as_i64().unwrap());
}

#[test]
fn a_task_cannot_be_given_file_access_to_the_vault() {
    let api = Api::start();
    api.script("noop", "fn run() { }\n");
    let vault = std::fs::canonicalize(api.server.vault_path()).unwrap();
    for (field, path) in [
        ("write_paths", vault.join("notes")),
        ("read_paths", vault.clone()),
    ] {
        let res = api
            .put("/api/tasks/noop")
            .body(format!(
                "schedule = \"0 * * * *\"\nscript = \"noop\"\n{field} = [\"{}\"]\n",
                path.display()
            ))
            .send()
            .unwrap();
        let err = expect(res, 400);
        assert_eq!(err["issues"][0]["at"], json!(field), "{err}");
        assert!(
            err["issues"][0]["message"]
                .as_str()
                .unwrap()
                .contains("inside the vault"),
            "{err}"
        );
    }
    let res = api
        .put("/api/tasks/noop")
        .body("schedule = \"0 * * * *\"\nscript = \"noop\"\nread_paths = [\"relative/dir\"]\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["issues"][0]["at"], json!("read_paths"), "{err}");
}
