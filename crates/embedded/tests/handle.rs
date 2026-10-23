//! `Embedded::handle` driven directly, as a Tauri custom scheme handler drives it (D17, D18, D26).

use std::path::Path;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{Request, Response, StatusCode, header};
use embedded::{Config, Embedded};
use serde_json::{Value, json};
use tempfile::TempDir;

const TEMPLATE: &str = r#"
template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// An article counts the words of its title on every update, through a `before_update` handler.
const ARTICLE: &str = r#"label = "Article"

field words {
  kind     = "int"
  computed = true
}

handler "before_update" {
  script   = "count"
  function = "count_words"
}
"#;

const COUNT: &str = r#"use vault

fn count_words(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    let title = fields.get("title").unwrap().as_string().unwrap()
    fields["words"] = Value::Int(title.split(" ").len())
    Ok(Value::Map(#{ "fields": Value::Map(fields) }))
}
"#;

/// A report is marked as reviewed by an `after_update` handler on the home server.
const REPORT: &str = r#"label = "Report"

field review {
  kind = "string"
}

field body {
  kind = "content"
}

handler "after_create" {
  script   = "review"
  function = "after_update"
}

handler "after_update" {
  script = "review"
}
"#;

const REVIEW: &str = r#"use vault

fn after_update(note: Value) -> Result[Value, string] {
    let id = note.get("id").unwrap().as_string().unwrap()
    let version = note.get("version").unwrap().as_string().unwrap()
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    if fields.contains_key("review") {
        Ok(Value::Null)
    } else {
        fields["review"] = Value::String("seen")
        vault::update(id, Value::Map(#{ "fields": Value::Map(fields) }), version)
    }
}
"#;

const NOOP: &str = "fn run() -> Result[string, string] {\n    Ok(\"ran\")\n}\n";

fn seed(vault: &Path) {
    let write = |path: &str, body: &str| {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    write("types/article.wcl", &format!("{ARTICLE}{TEMPLATE}"));
    write("types/report.wcl", &format!("{REPORT}{TEMPLATE}"));
    write("scripts/count.ws", COUNT);
    write("scripts/review.ws", REVIEW);
    write("scripts/noop.ws", NOOP);
    write(
        "tasks/noop.wcl",
        "schedule = \"* * * * *\"\nscript = \"noop\"\n",
    );
}

struct Device {
    server: Embedded,
    _vault: TempDir,
    _web: TempDir,
}

fn device() -> Device {
    let vault = tempfile::tempdir().unwrap();
    seed(vault.path());
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>TheVault</title>",
    )
    .unwrap();
    std::fs::create_dir_all(web.path().join("assets")).unwrap();
    std::fs::write(web.path().join("assets/app-abc123.js"), "console.log(1)").unwrap();
    let server = Embedded::open(Config {
        vault: vault.path().to_owned(),
        web: Some(web.path().to_owned()),
        secret_key: None,
    })
    .unwrap();
    Device {
        server,
        _vault: vault,
        _web: web,
    }
}

async fn bytes(res: Response<Body>) -> Bytes {
    axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
}

impl Device {
    async fn send(&self, req: Request<Body>) -> Response<Body> {
        self.server.handle(req).await
    }

    /// Sends `method path` with a JSON body, or none, and expects `status`.
    async fn json(&self, method: &str, path: &str, body: Option<Value>, status: u16) -> Value {
        let req = Request::builder().method(method).uri(path);
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = self.send(req).await;
        let got = res.status().as_u16();
        let body = bytes(res).await;
        let text = String::from_utf8_lossy(&body);
        assert_eq!(got, status, "{method} {path}: {text}");
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    }

    async fn get(&self, path: &str) -> Value {
        self.json("GET", path, None, 200).await
    }

    async fn create(&self, type_name: &str, fields: Value) -> Value {
        let body = json!({ "type": type_name, "note": { "fields": fields } });
        self.json("POST", "/api/notes", Some(body), 201).await
    }

    async fn update(&self, id: &str, fields: Value) -> Value {
        let base = self.get(&format!("/api/notes/{id}")).await["version"].clone();
        let body = json!({ "base_version": base, "note": { "fields": fields } });
        self.json("PUT", &format!("/api/notes/{id}"), Some(body), 200)
            .await
    }

    async fn history(&self, id: &str) -> Vec<Value> {
        let page = self.get(&format!("/api/notes/{id}/history")).await;
        page["versions"].as_array().cloned().unwrap_or_default()
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

#[tokio::test]
async fn creates_reads_and_updates_notes_as_the_owner_with_no_session() {
    let d = device();
    let note = d.create("article", json!({ "title": "Field notes" })).await;
    let id = s(&note["id"]).to_owned();

    let read = d.get(&format!("/api/notes/{id}")).await;
    assert_eq!(read["note"]["fields"]["title"], "Field notes", "{read}");

    // The before_update handler runs on the device's write.
    let saved = d
        .update(&id, json!({ "title": "Field notes from the river" }))
        .await;
    assert_eq!(saved["note"]["fields"]["words"], 5, "{saved}");
    let read = d.get(&format!("/api/notes/{id}")).await;
    assert_eq!(
        read["note"]["fields"]["title"],
        "Field notes from the river"
    );
    assert_eq!(read["version"], saved["version"]);

    // Admin routes on the vault's files answer too: the caller is the owner. Tokens and secrets
    // are the home server's, which a device token cannot reach (D2).
    d.get("/api/device-files").await;
    let refused = d.json("GET", "/api/tokens", None, 403).await;
    assert_eq!(refused["error"], "needs_web_app", "{refused}");

    // A missing note is a 404, not a login prompt.
    d.json("GET", "/api/notes/01JZZZZZZZZZZZZZZZZZZZZZZZ", None, 404)
        .await;
}

#[tokio::test]
async fn never_empties_the_trash_for_the_device() {
    let d = device();
    let note = d.create("article", json!({ "title": "Old draft" })).await;
    let id = s(&note["id"]).to_owned();
    let base = s(&note["version"]).to_owned();
    d.json(
        "DELETE",
        &format!("/api/notes/{id}?base_version={base}"),
        None,
        200,
    )
    .await;

    // Only the signed-in user empties the trash (ai D21): the app cannot tell the user from its
    // device token, so both purges are left to the web app and nothing is purged.
    let refused = d.json("POST", "/api/trash/empty", None, 403).await;
    assert_eq!(refused["error"], "needs_web_app", "{refused}");
    let refused = d
        .json("DELETE", &format!("/api/trash/{id}"), None, 403)
        .await;
    assert_eq!(refused["error"], "needs_web_app", "{refused}");
    let trash = d.get("/api/trash").await;
    assert_eq!(trash["notes"][0]["id"], id.as_str(), "{trash}");

    // Restoring stays local.
    d.json("POST", &format!("/api/trash/{id}/restore"), None, 200)
        .await;
}

#[tokio::test]
async fn searches_the_local_index() {
    let d = device();
    let note = d
        .create("article", json!({ "title": "Quokka sightings" }))
        .await;
    d.create("article", json!({ "title": "Unrelated" })).await;

    let page = d.get("/api/search?q=quokka").await;
    assert_eq!(page["total"], 1, "{page}");
    assert_eq!(page["rows"][0]["id"], note["id"], "{page}");
    let page = d.get("/api/search?q=wombat").await;
    assert_eq!(page["total"], 0, "{page}");
}

#[tokio::test]
async fn uploads_an_asset_from_a_streamed_body_and_serves_a_range_as_206() {
    let d = device();
    let file: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let chunks: Vec<Result<Bytes, std::io::Error>> = file
        .chunks(16 * 1024)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    let req = Request::post("/api/assets?filename=report.pdf")
        .header(header::CONTENT_TYPE, "application/pdf")
        .body(Body::from_stream(futures_util::stream::iter(chunks)))
        .unwrap();
    let res = d.send(req).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let asset: Value = serde_json::from_slice(&bytes(res).await).unwrap();
    assert_eq!(asset["size"], file.len(), "{asset}");
    let path = format!("/api/assets/{}", s(&asset["id"]));

    let req = Request::get(&path)
        .header(header::RANGE, "bytes=1000-1999")
        .body(Body::empty())
        .unwrap();
    let res = d.send(req).await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()[header::CONTENT_RANGE],
        format!("bytes 1000-1999/{}", file.len())
    );
    assert_eq!(res.headers()[header::ACCEPT_RANGES], "bytes");
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/pdf");
    assert_eq!(&bytes(res).await[..], &file[1000..2000]);

    // pdf.js asks for the tail to find the cross-reference table.
    let req = Request::get(&path)
        .header(header::RANGE, "bytes=-100")
        .body(Body::empty())
        .unwrap();
    let res = d.send(req).await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(&bytes(res).await[..], &file[file.len() - 100..]);

    let res = d
        .send(Request::get(&path).body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(&bytes(res).await[..], &file[..]);

    let req = Request::get(&path)
        .header(header::RANGE, format!("bytes={}-", file.len()))
        .body(Body::empty())
        .unwrap();
    let res = d.send(req).await;
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
}

#[tokio::test]
async fn serves_the_web_client_with_the_spa_fallback() {
    let d = device();
    let d = &d;
    let get = |path: &'static str| async move {
        d.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    };

    let res = get("/").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert!(String::from_utf8_lossy(&bytes(res).await).contains("<title>TheVault"));

    let res = get("/assets/app-abc123.js").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(&bytes(res).await[..], b"console.log(1)");

    // A client route gets index.html; a missing file does not.
    let res = get("/notes/article/01JABC").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(String::from_utf8_lossy(&bytes(res).await).contains("<title>TheVault"));
    assert_eq!(
        get("/assets/missing.js").await.status(),
        StatusCode::NOT_FOUND
    );

    // An unknown /api path is a JSON 404, never the client.
    let res = get("/api/nothing-here").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(!String::from_utf8_lossy(&bytes(res).await).contains("<title>"));
}

/// The names of this process's threads.
fn thread_names() -> Vec<String> {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
        .map(|n| n.trim().to_owned())
        .collect()
}

#[tokio::test]
async fn runs_no_scheduler_and_no_after_handlers() {
    let d = device();
    let note = d.create("report", json!({ "title": "Q3" })).await;
    let id = s(&note["id"]).to_owned();
    d.update(&id, json!({ "title": "Q3 final" })).await;

    // On the home server the after_* handlers would add a version marking the report reviewed,
    // authored by the handler. Here every version is the owner's.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let history = d.history(&id).await;
    assert!(!history.is_empty());
    for v in &history {
        assert_eq!(
            v["author"],
            json!({ "kind": "session", "name": "owner" }),
            "{history:?}"
        );
    }
    let read = d.get(&format!("/api/notes/{id}")).await;
    assert!(read["note"]["fields"].get("review").is_none(), "{read}");

    let names = thread_names();
    assert!(!names.iter().any(|n| n == "scheduler"), "{names:?}");
    assert!(!names.iter().any(|n| n == "handler-queue"), "{names:?}");

    // The every-minute task is listed, beside the shipped ones. Its runs, and running it now,
    // are the home server's, and this device is not paired with one.
    let tasks = d.get("/api/tasks").await;
    let listed = tasks["tasks"].as_array().unwrap();
    assert!(listed.iter().any(|t| t["name"] == "noop"), "{tasks}");
    let runs = d.json("GET", "/api/tasks/noop/runs", None, 503).await;
    assert_eq!(runs["error"], "not_paired", "{runs}");
    let refused = d.json("POST", "/api/tasks/noop/run", None, 503).await;
    assert_eq!(refused["error"], "not_paired", "{refused}");
}

#[tokio::test]
async fn runs_no_agent_triggers() {
    let vault = tempfile::tempdir().unwrap();
    seed(vault.path());
    std::fs::create_dir_all(vault.path().join("agents")).unwrap();
    std::fs::write(
        vault.path().join("agents/watcher.wcl"),
        "label = \"Watcher\"\n\ntrigger every_minute {\n  schedule = \"* * * * *\"\n  prompt = \"Look.\"\n}\n\ntrigger new_report {\n  on = [\"create\", \"update\"]\n  type = \"report\"\n  prompt = \"Read it.\"\n}\n\ntrigger asked {\n  mention = true\n}\n",
    )
    .unwrap();
    let server = Embedded::open(Config {
        vault: vault.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    let d = Device {
        server,
        _vault: vault,
        _web: tempfile::tempdir().unwrap(),
    };
    let note = d.create("report", json!({ "title": "Q3" })).await;
    d.update(s(&note["id"]), json!({ "title": "Q3 final" }))
        .await;
    // A call of the agent committed in a note is the home server's to answer too (agents G19).
    let call = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
    let text = format!("[@Watcher read this](agent:watcher?call={call})");
    let body = json!({ "type": "report", "note": {
        "fields": { "title": "Q4" },
        "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
            { "kind": "paragraph", "fields": { "text": text } },
        ] }],
    } });
    let called = d.json("POST", "/api/notes", Some(body), 201).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let read = d.get(&format!("/api/notes/{}", s(&called["id"]))).await;
    assert_eq!(
        read["note"]["blocks"][0]["blocks"][0]["fields"]["text"], text,
        "{read}"
    );
    assert_eq!(d.server.writer().state().agent_call(call).unwrap(), None);
    let asked = d
        .json("GET", &format!("/api/agents/calls/{call}"), None, 503)
        .await;
    assert_eq!(asked["error"], "not_paired", "{asked}");

    // The triggers are the home server's: their runs are read there, and nothing ran here or
    // started a thread.
    let runs = d.json("GET", "/api/agents/watcher/runs", None, 503).await;
    assert_eq!(runs["error"], "not_paired", "{runs}");
    let w = d.server.writer();
    for t in ["every_minute", "new_report", "asked"] {
        assert!(w.state().agent_runs("watcher", t, 20).unwrap().is_empty());
    }
    let threads = w.list(Some(writer::AGENT_THREAD_TYPE)).unwrap();
    assert!(threads.is_empty(), "{threads:?}");
    let names = thread_names();
    assert!(!names.iter().any(|n| n == "agent-runs"), "{names:?}");
}
