//! S1: queries, the builder's parse endpoint and full-text search over notes created through the
//! API (D8, D16, D33).

use std::path::Path;

use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

field body {
  kind = "content"
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
    std::fs::write(vault.join("types/task.wcl"), TASK_TYPE).unwrap();
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        Api {
            server: TestServer::start_seeded(seed),
            client: Client::new(),
        }
    }

    /// Create a task from WCL text and return its id.
    fn create(&self, wcl: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/notes?type=task"))
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(wcl.to_owned())
            .send()
            .unwrap();
        s(&expect(res, 201)["id"]).to_owned()
    }

    fn post(&self, path: &str, body: Value) -> Response {
        self.client
            .post(self.server.url(path))
            .json(&body)
            .send()
            .unwrap()
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Response {
        self.client
            .get(self.server.url(path))
            .query(query)
            .send()
            .unwrap()
    }
}

/// Assert the status and return the JSON body.
fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "unexpected status, body: {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: body is not JSON: {text}"))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("{v} is not a string"))
}

fn ids(page: &Value) -> Vec<String> {
    page["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows in {page}"))
        .iter()
        .map(|r| s(&r["id"]).to_owned())
        .collect()
}

#[test]
fn a_query_returns_api_created_notes_a_page_at_a_time() {
    let api = Api::start();
    let mut made = Vec::new();
    for n in 1..=5 {
        made.push(api.create(&format!("title = \"Task {n}\"\nestimate = {n}\n")));
    }
    api.create("title = \"Small\"\nestimate = 0\n");

    let text = "from task where estimate > 0 sort estimate desc";
    let first = expect(
        api.post("/api/query", json!({ "query": text, "page_size": 2 })),
        200,
    );
    assert_eq!(first["total"], 5);
    assert_eq!(ids(&first), [made[4].clone(), made[3].clone()]);
    assert_eq!(first["rows"][0]["title"], "Task 5");
    assert_eq!(first["rows"][0]["type"], "task");
    assert_eq!(first["rows"][0]["fields"]["estimate"], 5);

    let mut seen = ids(&first);
    let mut cursor = first["next"].clone();
    let mut pages = 1;
    while let Some(c) = cursor.as_str() {
        let page = expect(
            api.post(
                "/api/query",
                json!({ "query": text, "page_size": 2, "cursor": c }),
            ),
            200,
        );
        seen.extend(ids(&page));
        cursor = page["next"].clone();
        pages += 1;
    }
    assert_eq!(pages, 3);
    let mut want = made.clone();
    want.reverse();
    assert_eq!(seen, want);

    // The same query over GET, with the cursor of the first page.
    let second = expect(
        api.get(
            "/api/query",
            &[
                ("q", text),
                ("page_size", "2"),
                ("cursor", s(&first["next"])),
            ],
        ),
        200,
    );
    assert_eq!(ids(&second), [made[2].clone(), made[1].clone()]);
}

#[test]
fn a_query_with_a_syntax_error_is_400_with_line_and_column() {
    let api = Api::start();
    let body = expect(
        api.post(
            "/api/query",
            json!({ "query": "from task\nwhere estimate >" }),
        ),
        400,
    );
    assert_eq!(body["error"], "query_syntax", "{body}");
    assert_eq!(body["line"], 2, "{body}");
    assert!(body["column"].as_u64().is_some_and(|c| c > 1), "{body}");
    assert!(!s(&body["message"]).is_empty());

    let body = expect(api.get("/api/query", &[("q", "from task sort")]), 400);
    assert_eq!(body["error"], "query_syntax", "{body}");
    assert_eq!(body["line"], 1, "{body}");
}

#[test]
fn a_query_using_this_needs_a_note_id() {
    let api = Api::start();
    let id = api.create("title = \"Holder\"\nestimate = 1\n");
    api.create("title = \"Other\"\nestimate = 2\n");

    let text = "from task where id == this";
    let body = expect(api.post("/api/query", json!({ "query": text })), 400);
    assert_eq!(body["error"], "no_this", "{body}");
    assert!(s(&body["message"]).contains("this"), "{body}");

    let page = expect(
        api.post("/api/query", json!({ "query": text, "this": id })),
        200,
    );
    assert_eq!(ids(&page), [id]);
}

#[test]
fn other_query_failures_are_400() {
    let api = Api::start();
    let body = expect(api.post("/api/query", json!({ "query": "from nope" })), 400);
    assert_eq!(body["error"], "query", "{body}");
    let body = expect(
        api.post(
            "/api/query",
            json!({ "query": "from task", "cursor": "zz" }),
        ),
        400,
    );
    assert_eq!(body["error"], "bad_cursor", "{body}");
    let body = expect(api.post("/api/query", json!({ "q": "from task" })), 400);
    assert_eq!(body["error"], "bad_request", "{body}");
}

#[test]
fn search_finds_a_note_by_a_word_in_its_content() {
    let api = Api::start();
    let id = api.create(
        "title = \"Notes\"\ncontent body {\nparagraph { text = \"the quokka escaped again\" }\n}\n",
    );
    api.create("title = \"Unrelated\"\ncontent body {\nparagraph { text = \"nothing here\" }\n}\n");

    let page = expect(api.get("/api/search", &[("q", "quokka")]), 200);
    assert_eq!(ids(&page), std::slice::from_ref(&id), "{page}");
    assert_eq!(page["total"], 1);
    assert_eq!(page["rows"][0]["title"], "Notes");

    let none = expect(api.get("/api/search", &[("q", "wombat")]), 200);
    assert_eq!(none["total"], 0);

    // The query language's `search` clause reaches the same index.
    let page = expect(
        api.post(
            "/api/query",
            json!({ "query": "from task search \"quokka\"" }),
        ),
        200,
    );
    assert_eq!(ids(&page), [id]);

    let body = expect(api.get("/api/search", &[]), 400);
    assert_eq!(body["error"], "bad_request", "{body}");
}

#[test]
fn parse_returns_the_builder_model_or_the_error() {
    let api = Api::start();
    let text = "from task where estimate > 2 sort estimate desc";
    let body = expect(api.post("/api/query/parse", json!({ "query": text })), 200);
    assert_eq!(body["text"], text, "{body}");
    assert_eq!(body["model"]["from"], json!(["task"]), "{body}");
    assert_eq!(body["model"]["sort"][0]["field"], "estimate", "{body}");
    assert!(body["builder_error"].is_null(), "{body}");

    // The model converts back to the same text.
    let back = expect(
        api.post("/api/query/text", json!({ "model": body["model"] })),
        200,
    );
    assert_eq!(back["text"], text);

    // A valid query the builder cannot show has no model, and says why.
    let body = expect(
        api.post(
            "/api/query/parse",
            json!({ "query": "from task where estimate * 2 > 3" }),
        ),
        200,
    );
    assert!(body["model"].is_null(), "{body}");
    assert!(s(&body["builder_error"]).contains("builder"), "{body}");

    let body = expect(
        api.post("/api/query/parse", json!({ "query": "from task where (" })),
        400,
    );
    assert_eq!(body["error"], "query_syntax", "{body}");
    assert_eq!(body["line"], 1);
    assert!(body["column"].is_u64());

    let body = expect(
        api.post("/api/query/text", json!({ "model": { "from": [] } })),
        400,
    );
    assert_eq!(body["error"], "bad_request", "{body}");
}
