//! S1: archiving notes through the API. An archived note keeps working by id, but listings,
//! queries, search and the link graph leave it out unless the request asks for archived notes.

use std::path::Path;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label = "Task"

field parent {
  kind   = "reference"
  target = "task"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "parent"
    slot "body"
  }
}
"#;

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/task.wcl"), TASK_TYPE).unwrap();
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(seed);
        let mut auth = HeaderMap::new();
        auth.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder().default_headers(auth).build().unwrap();
        Api { server, client }
    }

    fn create(&self, wcl: &str) -> Value {
        let res = self
            .client
            .post(self.server.url("/api/notes?type=task"))
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(wcl.to_owned())
            .send()
            .unwrap();
        expect(res, 201)
    }

    fn post(&self, path: &str) -> Response {
        self.client.post(self.server.url(path)).send().unwrap()
    }

    fn post_json(&self, path: &str, body: Value) -> Response {
        self.client
            .post(self.server.url(path))
            .json(&body)
            .send()
            .unwrap()
    }

    fn get(&self, path: &str) -> Response {
        self.client.get(self.server.url(path)).send().unwrap()
    }

    fn archive(&self, note: &Value) -> Value {
        let path = format!(
            "/api/notes/{}/archive?base_version={}",
            s(&note["id"]),
            s(&note["version"])
        );
        expect(self.post(&path), 200)
    }
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "unexpected status, body: {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: body is not JSON: {text}"))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("{v} is not a string"))
}

fn titles(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {v}"))
        .iter()
        .map(|r| s(&r["title"]).to_owned())
        .collect()
}

#[test]
fn archive_and_unarchive_are_versioned_writes() {
    let api = Api::start();
    let note = api.create("title = \"Old plan\"\n");
    let id = s(&note["id"]).to_owned();

    // Like any write, the base version is required and must be current.
    expect(api.post(&format!("/api/notes/{id}/archive")), 428);
    let stale = expect(
        api.post(&format!("/api/notes/{id}/archive?base_version=0000")),
        409,
    );
    assert_eq!(stale["error"], "conflict", "{stale}");

    let archived = api.archive(&note);
    assert_eq!(archived["event"], "update");
    assert_eq!(archived["note"]["fields"]["archived"], true);
    assert!(s(&archived["wcl"]).contains("\narchived = true\n"));
    let read = expect(api.get(&format!("/api/notes/{id}")), 200);
    assert_eq!(read["note"]["fields"]["archived"], true);
    let history = expect(api.get(&format!("/api/notes/{id}/history")), 200);
    assert_eq!(history["versions"].as_array().unwrap().len(), 2);

    // A save that leaves the flag out keeps the note archived.
    let saved = expect(
        api.client
            .put(api.server.url(&format!("/api/notes/{id}")))
            .json(&json!({
                "note": { "fields": { "title": "Old plan, edited" } },
                "base_version": archived["version"],
            }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(saved["note"]["fields"]["archived"], true);

    let back = expect(
        api.client
            .post(api.server.url(&format!("/api/notes/{id}/unarchive")))
            .header("If-Match", format!("\"{}\"", s(&saved["version"])))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(back["note"]["fields"].get("archived"), None, "{back}");
    let history = expect(api.get(&format!("/api/notes/{id}/history")), 200);
    assert_eq!(history["versions"].as_array().unwrap().len(), 4);
}

#[test]
fn listings_queries_and_search_leave_archived_notes_out_unless_asked() {
    let api = Api::start();
    let old = api.create("title = \"Shelved roadmap\"\n");
    api.create("title = \"Live roadmap\"\n");
    api.archive(&old);

    let listed = |q: &str| titles(&expect(api.get(&format!("/api/notes{q}")), 200), "notes");
    assert_eq!(listed("?type=task"), ["Live roadmap"]);
    assert_eq!(listed("?type=task&archived=only"), ["Shelved roadmap"]);
    assert_eq!(listed("?archived=include").len(), 2);
    let only = expect(api.get("/api/notes?archived=only"), 200);
    assert_eq!(only["notes"][0]["archived"], true);

    let search = |q: &str| titles(&expect(api.get(&format!("/api/search?q={q}")), 200), "rows");
    assert_eq!(search("roadmap"), ["Live roadmap"]);
    assert_eq!(search("roadmap&archived=only"), ["Shelved roadmap"]);
    assert_eq!(search("roadmap&archived=include").len(), 2);
    assert_eq!(search("shelved"), Vec::<String>::new());
    let bad = expect(api.get("/api/search?q=roadmap&archived=yes"), 400);
    assert!(
        s(&bad["message"]).contains("exclude, include or only"),
        "{bad}"
    );

    let query = |body: Value| titles(&expect(api.post_json("/api/query", body), 200), "rows");
    assert_eq!(
        query(json!({ "query": "from task sort title" })),
        ["Live roadmap"]
    );
    assert_eq!(
        query(json!({ "query": "from task where archived" })),
        ["Shelved roadmap"]
    );
    assert_eq!(
        query(json!({ "query": "from task include archived sort title" })),
        ["Live roadmap", "Shelved roadmap"]
    );
    assert_eq!(
        query(json!({ "query": "from task sort title", "archived": "include" })),
        ["Live roadmap", "Shelved roadmap"]
    );
    assert_eq!(
        query(json!({ "query": "from task", "archived": "only" })),
        ["Shelved roadmap"]
    );
    let got = expect(
        api.get("/api/query?q=from%20task%20search%20%22roadmap%22&archived=only"),
        200,
    );
    assert_eq!(titles(&got, "rows"), ["Shelved roadmap"]);

    // The builder model carries the clause.
    let parsed = expect(
        api.post_json(
            "/api/query/parse",
            json!({ "query": "from  task include archived" }),
        ),
        200,
    );
    assert_eq!(parsed["text"], "from task include archived");
    assert_eq!(parsed["model"]["include_archived"], true);
    let text = expect(
        api.post_json("/api/query/text", json!({ "model": parsed["model"] })),
        200,
    );
    assert_eq!(text["text"], "from task include archived");
}

#[test]
fn links_to_an_archived_note_still_work_but_the_graph_leaves_it_out() {
    let api = Api::start();
    let target = api.create("title = \"Archived target\"\n");
    let tid = s(&target["id"]).to_owned();
    let source = api.create(&format!(
        "title = \"Source\"\nparent = \"{tid}\"\n\ncontent body {{\n  paragraph {{\n    text = \"See [[{tid}]]\"\n  }}\n}}\n"
    ));
    let sid = s(&source["id"]).to_owned();
    let graph = expect(api.get(&format!("/api/notes/{sid}/graph")), 200);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);

    api.archive(&target);
    let graph = expect(api.get(&format!("/api/notes/{sid}/graph")), 200);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 1, "{graph}");
    // The archived note opens by id and has a graph of its own.
    expect(api.get(&format!("/api/notes/{tid}")), 200);
    let own = expect(api.get(&format!("/api/notes/{tid}/graph")), 200);
    assert_eq!(own["nodes"].as_array().unwrap().len(), 2, "{own}");
    // A reference to it still validates.
    let saved = expect(
        api.client
            .put(api.server.url(&format!("/api/notes/{sid}")))
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .header("If-Match", format!("\"{}\"", s(&source["version"])))
            .body(format!("title = \"Source again\"\nparent = \"{tid}\"\n"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(saved["note"]["fields"]["parent"], tid.as_str());
}

#[test]
fn an_archived_daily_note_is_still_the_note_for_its_day() {
    let api = Api::start();
    let day = expect(api.post("/api/daily/2026-10-13"), 201);
    let archived = api.archive(&day);
    let again = expect(api.post("/api/daily/2026-10-13"), 200);
    assert_eq!(again["id"], day["id"]);
    assert_eq!(again["created"], false);
    assert_eq!(again["note"]["fields"]["archived"], true);
    assert_eq!(again["version"], archived["version"]);
    expect(api.get("/api/daily/2026-10-13"), 200);
}

#[test]
fn a_type_cannot_define_a_field_called_archived() {
    let api = Api::start();
    let res = api
        .client
        .put(api.server.url("/api/types/thing"))
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(
            "label = \"Thing\"\nfield archived {\n  kind = \"bool\"\n}\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n",
        )
        .send()
        .unwrap();
    let body = expect(res, 400);
    assert!(body.to_string().contains("reserved"), "{body}");
}
