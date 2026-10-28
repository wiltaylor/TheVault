//! S1: what the web client reads to lay out notes, and the server serving the built client.

use std::ffi::OsStr;
use std::path::Path;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label  = "Task"
plural = "Tasks"
icon   = "check"

field status {
  kind    = "select"
  label   = "Status"
  options = ["todo", "done"]
  default = "todo"
}

field estimate {
  kind = "int"
}

field body {
  kind = "content"
}

template default {
  tab "Overview" {
    slot "title"
    slot "status"
    slot "body"
  }

  tab "Details" {
    slot "estimate" {
      label = "Hours"
    }
    paragraph {
      text = "Fixed text."
    }
  }
}
"#;

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/task.wcl"), TASK_TYPE).unwrap();
}

fn get_json(server: &TestServer, path: &str) -> (u16, Value) {
    let res = Client::new()
        .get(server.url(path))
        .header(AUTHORIZATION, format!("Bearer {}", server.admin_token()))
        .send()
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().unwrap_or(Value::Null))
}

#[test]
fn lists_every_type_with_its_fields_and_templates() {
    let server = TestServer::start_seeded(seed);
    let (status, body) = get_json(&server, "/api/types");
    assert_eq!(status, 200, "{body}");
    let types = body["types"].as_array().unwrap();
    let mut names: Vec<&str> = types.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    // The seeded type, and the built-in types every vault has.
    // `person` comes with the phone and Google types, which reference it.
    assert_eq!(
        names,
        [
            "agent_memory",
            "agent_thread",
            "call",
            "conversation",
            "dailynote",
            "email",
            "email_thread",
            "event",
            "event_series",
            "fleeting",
            "message",
            "monthlynote",
            "outbox",
            "person",
            "quarterlynote",
            "task",
            "weeklynote",
            "yearlynote"
        ]
    );
    let task = types.iter().find(|t| t["name"] == "task").unwrap();
    assert_eq!(task["name"], "task");
    assert_eq!(task["plural"], "Tasks");
    assert_eq!(task["icon"], "check");
    assert_eq!(
        task["fields"][0],
        json!({
            "name": "status", "label": "Status", "description": null, "kind": "select",
            "required": false, "computed": false, "hidden": false, "multiple": false,
            "compound": false,
            "default": "todo", "options": ["todo", "done"],
        })
    );
    assert_eq!(task["fields"][1]["label"], "estimate");
    assert_eq!(task["fields"][2]["compound"], true);
    let tabs = &task["templates"][0]["tabs"];
    assert_eq!(task["templates"][0]["name"], "default");
    assert_eq!(tabs[0]["title"], "Overview");
    assert_eq!(
        tabs[0]["items"][1],
        json!({ "slot": "status", "label": null })
    );
    assert_eq!(
        tabs[1]["items"][0],
        json!({ "slot": "estimate", "label": "Hours" })
    );
    assert_eq!(
        tabs[1]["items"][1],
        json!({ "block": { "kind": "paragraph", "fields": { "text": "Fixed text." } } })
    );
}

#[test]
fn listing_types_needs_credentials() {
    let server = TestServer::start_seeded(seed);
    let res = Client::new().get(server.url("/api/types")).send().unwrap();
    assert_eq!(res.status().as_u16(), 401);
}

fn web_server() -> (TestServer, tempfile::TempDir) {
    let web = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(web.path().join("assets")).unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>shell</title>",
    )
    .unwrap();
    std::fs::write(
        web.path().join("sw.js"),
        "self.addEventListener('fetch', () => {});",
    )
    .unwrap();
    std::fs::write(web.path().join("assets/app-abc123.js"), "console.log(1)").unwrap();
    let server = TestServer::start_with_args(seed, &[OsStr::new("--web"), web.path().as_os_str()]);
    (server, web)
}

#[test]
fn serves_the_web_client_without_credentials() {
    let (server, _web) = web_server();
    let client = Client::new();
    let get = |path: &str| client.get(server.url(path)).send().unwrap();

    let index = get("/");
    assert_eq!(index.status().as_u16(), 200);
    assert_eq!(index.headers()[CONTENT_TYPE], "text/html; charset=utf-8");
    assert_eq!(index.headers()[CACHE_CONTROL], "no-cache");
    assert!(index.text().unwrap().contains("shell"));

    // A client route that names no file loads the shell.
    let route = get("/types/task");
    assert_eq!(route.status().as_u16(), 200);
    assert!(route.text().unwrap().contains("shell"));

    let sw = get("/sw.js");
    assert_eq!(sw.headers()[CONTENT_TYPE], "text/javascript; charset=utf-8");
    assert_eq!(sw.headers()[CACHE_CONTROL], "no-cache");

    let asset = get("/assets/app-abc123.js");
    assert_eq!(asset.status().as_u16(), 200);
    assert!(
        asset.headers()[CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable")
    );

    assert_eq!(get("/missing.js").status().as_u16(), 404);
}

#[test]
fn api_paths_never_fall_back_to_the_web_client() {
    let (server, _web) = web_server();
    let client = Client::new();
    // An existing route still checks credentials.
    let res = client.get(server.url("/api/types")).send().unwrap();
    assert_eq!(res.status().as_u16(), 401);
    // An unknown /api path is a JSON 404, not the shell.
    let res = client
        .get(server.url("/api/nothing-here"))
        .header(AUTHORIZATION, format!("Bearer {}", server.admin_token()))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);
    let body: Value = res.json().unwrap();
    assert_eq!(body["error"], "not_found");
}
