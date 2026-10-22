//! S1: notifications through the API, from scripts (scheduled and handlers) and from the server
//! when a scheduled script fails for good.

use std::ffi::OsStr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const TEMPLATE: &str = r#"
template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TASK: &str = r#"label = "Task"

field done {
  kind = "bool"
}

field due {
  kind = "date"
}
"#;

/// An invoice notifies the user once it is created.
const INVOICE: &str = r#"label = "Invoice"

handler "after_create" {
  script = "filed"
}
"#;

const FILED: &str = r#"use vault

fn after_create(note: Value) -> Result[(), string] {
    let id = note.get("id").unwrap().as_string().unwrap()
    vault::notify("Invoice filed", "It needs a category.", id)?
    Ok(())
}
"#;

/// A memo's `before_create` handler tries to notify, which is refused inside the write.
const MEMO: &str = r#"label = "Memo"

handler "before_create" {
  script = "quiet"
}
"#;

const QUIET: &str = r#"use vault

fn before_create(note: Value) -> Result[Value, string] {
    match vault::notify("from inside the write", "", "") {
        Ok(_) => Err("vault::notify ran inside the write"),
        Err(_) => Ok(Value::Null),
    }
}
"#;

/// The example from `docs/scripts.md`, "Notifying the user".
const DIGEST: &str = r#"use vault

fn run() -> Result[(), string] {
    let page = vault::query_page("from task where !done && due < today()", 1, "")?
    let overdue = page.get("total").unwrap().as_int().unwrap()
    if overdue == 0 {
        return Ok(())
    }
    let day = vault::daily("")?
    let id = day.get("id").unwrap().as_string().unwrap()
    vault::notify(fmt("{} tasks are overdue", overdue), "See today's note.", id)?
    Ok(())
}
"#;

fn seed(vault: &Path) {
    for (path, body) in [
        ("types/task.wcl", format!("{TASK}{TEMPLATE}")),
        ("types/invoice.wcl", format!("{INVOICE}{TEMPLATE}")),
        ("types/memo.wcl", format!("{MEMO}{TEMPLATE}")),
        ("scripts/filed.ws", FILED.to_owned()),
        ("scripts/quiet.ws", QUIET.to_owned()),
        ("scripts/digest.ws", DIGEST.to_owned()),
        (
            "scripts/flaky.ws",
            "fn run() -> Result[int, string] { Err(\"the upstream is down\\nmore detail\") }\n"
                .to_owned(),
        ),
    ] {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
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

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server =
            TestServer::start_with_args(seed, &[OsStr::new("--test-clock"), OsStr::new(START)]);
        let client = client(server.admin_token());
        Api { server, client }
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
    }

    fn create_note(&self, type_name: &str, fields: Value) -> Value {
        expect(
            self.req("POST", &format!("/api/notes?type={type_name}"))
                .json(&json!({ "note": { "fields": fields } }))
                .send()
                .unwrap(),
            201,
        )
    }

    fn notify(&self, body: Value) -> Response {
        self.req("POST", "/api/notifications")
            .json(&body)
            .send()
            .unwrap()
    }

    fn list(&self, query: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notifications{query}"))
                .send()
                .unwrap(),
            200,
        )
    }

    fn token(&self, name: &str, scope: &str) -> Client {
        let t = expect(
            self.req("POST", "/api/tokens")
                .json(&json!({ "name": name, "scope": scope }))
                .send()
                .unwrap(),
            201,
        );
        client(t["secret"].as_str().unwrap())
    }

    /// Wait for the list to hold `n` notifications.
    fn wait_for(&self, n: usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let list = self.list("");
            if list["items"].as_array().unwrap().len() >= n {
                return list;
            }
            assert!(Instant::now() < deadline, "no notification came: {list}");
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn titles(list: &Value) -> Vec<&str> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect()
}

#[test]
fn notifications_are_created_listed_paged_read_and_removed() {
    let api = Api::start();
    let task = api.create_note("task", json!({ "title": "Pay rent" }));
    let empty = api.list("");
    assert_eq!(empty["items"], json!([]));
    assert_eq!(empty["unread"], 0);

    let first = expect(
        api.notify(json!({ "title": "Rent is due", "body": "Pay it today.", "note": task["id"] })),
        201,
    );
    assert_eq!(first["note"], task["id"]);
    assert_eq!(first["note_type"], "task");
    assert_eq!(first["note_title"], "Pay rent");
    assert_eq!(first["read"], Value::Null);
    assert!(first["source"].as_str().unwrap().starts_with("token:"));
    let second = expect(
        api.notify(json!({ "title": "Backup done", "url": "/settings/scheduled/backup" })),
        201,
    );
    assert_eq!(second["url"], "/settings/scheduled/backup");
    assert_eq!(second["note"], Value::Null);
    expect(api.notify(json!({ "title": "Third" })), 201);

    let all = api.list("");
    assert_eq!(titles(&all), ["Third", "Backup done", "Rent is due"]);
    assert_eq!(all["unread"], 3);
    assert_eq!(all["next"], Value::Null);
    let page = api.list("?limit=2");
    assert_eq!(titles(&page), ["Third", "Backup done"]);
    let rest = api.list(&format!(
        "?limit=2&cursor={}",
        page["next"].as_str().unwrap()
    ));
    assert_eq!(titles(&rest), ["Rent is due"]);
    assert_eq!(rest["next"], Value::Null);

    let read = expect(
        api.req(
            "POST",
            &format!("/api/notifications/{}/read", first["id"].as_str().unwrap()),
        )
        .send()
        .unwrap(),
        200,
    );
    assert!(read["read"].is_i64());
    assert_eq!(read["unread"], 2);
    assert_eq!(titles(&api.list("?unread=true")), ["Third", "Backup done"]);

    let gone = expect(
        api.req(
            "DELETE",
            &format!("/api/notifications/{}", second["id"].as_str().unwrap()),
        )
        .send()
        .unwrap(),
        200,
    );
    assert_eq!(gone["unread"], 1);
    expect(
        api.req(
            "DELETE",
            &format!("/api/notifications/{}", second["id"].as_str().unwrap()),
        )
        .send()
        .unwrap(),
        404,
    );
    expect(
        api.req("POST", "/api/notifications/nope/read")
            .send()
            .unwrap(),
        404,
    );

    let cleared = expect(api.req("DELETE", "/api/notifications").send().unwrap(), 200);
    assert_eq!(cleared["cleared"], 1);
    assert_eq!(titles(&api.list("")), ["Third"]);
    let marked = expect(
        api.req("POST", "/api/notifications/read-all")
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(marked["marked"], 1);
    assert_eq!(api.list("")["unread"], 0);
}

#[test]
fn a_notification_is_checked_and_needs_write_to_make() {
    let api = Api::start();
    let task = api.create_note("task", json!({ "title": "Doomed" }));
    expect(
        api.req(
            "DELETE",
            &format!(
                "/api/notes/{}?base_version={}",
                task["id"].as_str().unwrap(),
                task["version"].as_str().unwrap()
            ),
        )
        .send()
        .unwrap(),
        200,
    );
    let trashed = expect(api.notify(json!({ "title": "x", "note": task["id"] })), 400);
    assert!(
        trashed["message"].as_str().unwrap().contains("trash"),
        "{trashed}"
    );
    let missing = expect(
        api.notify(json!({ "title": "x", "note": "01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ" })),
        400,
    );
    assert!(missing["message"].as_str().unwrap().contains("no note"));
    expect(api.notify(json!({ "title": "  " })), 400);
    expect(api.notify(json!({ "title": "x".repeat(201) })), 400);
    expect(
        api.notify(json!({ "title": "x", "body": "y".repeat(2001) })),
        400,
    );
    expect(
        api.notify(json!({ "title": "x", "url": "https://example.com" })),
        400,
    );
    expect(api.notify(json!({ "title": "x", "colour": "red" })), 422);

    let reader = api.token("reader", "read");
    expect(
        reader
            .get(api.server.url("/api/notifications"))
            .send()
            .unwrap(),
        200,
    );
    expect(
        reader
            .post(api.server.url("/api/notifications"))
            .json(&json!({ "title": "x" }))
            .send()
            .unwrap(),
        403,
    );
    let agent = api.token("agent", "write");
    let made = expect(
        agent
            .post(api.server.url("/api/notifications"))
            .json(&json!({ "title": "From the agent" }))
            .send()
            .unwrap(),
        201,
    );
    assert_eq!(made["source"], "token:agent");
    assert_eq!(titles(&api.list("")), ["From the agent"]);
}

#[test]
fn handlers_notify_after_a_write_but_never_inside_one() {
    let api = Api::start();
    let invoice = api.create_note("invoice", json!({ "title": "March" }));
    let list = api.wait_for(1);
    let n = &list["items"][0];
    assert_eq!(n["title"], "Invoice filed");
    assert_eq!(n["body"], "It needs a category.");
    assert_eq!(n["note"], invoice["id"]);
    assert_eq!(n["source"], "handler:invoice.after_create");

    // A before_* handler's notify is refused, in a dry run and in a real write alike.
    let dry = expect(
        api.req("POST", "/api/batch")
            .json(&json!({ "dry_run": true, "ops": [
                { "op": "create", "type": "memo", "note": { "fields": { "title": "Dry" } } },
                { "op": "create", "type": "invoice", "note": { "fields": { "title": "Dry" } } },
            ] }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(dry["ok"], true, "{dry}");
    api.create_note("memo", json!({ "title": "Real" }));
    // Let any job the writes could have queued run before counting.
    thread::sleep(Duration::from_millis(500));
    assert_eq!(titles(&api.list("")), ["Invoice filed"]);
}

#[test]
fn a_scheduled_script_notifies_with_its_task_as_the_source() {
    let api = Api::start();
    api.create_note(
        "task",
        json!({ "title": "Late", "done": false, "due": "2025-12-01" }),
    );
    expect(
        api.req("PUT", "/api/tasks/digest")
            .body("label = \"Morning digest\"\nschedule = \"0 7 * * *\"\nscript = \"digest\"\nretries = 0\n")
            .send()
            .unwrap(),
        200,
    );
    let run = expect(
        api.req("POST", "/api/tasks/digest/run?wait=true")
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(run["outcome"], "succeeded", "{run}");
    let list = api.list("");
    assert_eq!(titles(&list), ["1 tasks are overdue"]);
    let n = &list["items"][0];
    assert_eq!(n["source"], "task:digest");
    assert_eq!(n["note_type"], "dailynote");
}

#[test]
fn a_scheduled_script_that_fails_for_good_notifies_once() {
    let api = Api::start();
    expect(
        api.req("PUT", "/api/tasks/flaky")
            .body("label = \"Morning digest\"\nschedule = \"0 0 1 1 *\"\nscript = \"flaky\"\nretries = 1\nretry_delay = \"PT1M\"\n")
            .send()
            .unwrap(),
        200,
    );
    let first = expect(
        api.req("POST", "/api/tasks/flaky/run?wait=true")
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(first["outcome"], "failed");
    // A retry is waiting, so the user hears nothing yet.
    assert_eq!(api.list("")["items"], json!([]));

    let advance = |by: &str| {
        expect(
            api.req("POST", "/api/test/clock")
                .json(&json!({ "advance": by }))
                .send()
                .unwrap(),
            200,
        )
    };
    let retried = advance("PT1M");
    assert_eq!(retried["runs"][0]["attempt"], 2);
    assert_eq!(retried["runs"][0]["outcome"], "failed");
    advance("PT2H");

    let list = api.list("");
    assert_eq!(
        titles(&list),
        ["Scheduled script \u{2018}Morning digest\u{2019} failed: the upstream is down"]
    );
    let n = &list["items"][0];
    assert_eq!(n["source"], "system");
    assert_eq!(n["url"], "/settings/scheduled/flaky");
    assert_eq!(n["body"], "more detail");
    assert_eq!(list["unread"], 1);
}
