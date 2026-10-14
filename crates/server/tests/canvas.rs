//! S1 checks for canvases (D15): a canvas the web client builds goes through the normal write
//! path, validates against `docs/vault-format.md`, and reads back exactly as it was sent.

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const BOARD: &str = r#"label = "Board"

field board {
  kind = "canvas"
}

field body {
  kind = "content"
}

template default {
  tab "Board" {
    slot "title"
    slot "board"
  }
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/board.wcl"), BOARD).unwrap();
        });
        let mut auth = HeaderMap::new();
        let bearer = format!("Bearer {}", server.admin_token());
        auth.insert(AUTHORIZATION, HeaderValue::from_str(&bearer).unwrap());
        let client = Client::builder().default_headers(auth).build().unwrap();
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

    /// A new board note holding `blocks`, as the web client creates one.
    fn board(&self, blocks: Value) -> Value {
        let res = self
            .post("/api/notes?type=board")
            .json(&json!({ "note": { "fields": { "title": "Plan" }, "blocks": blocks } }))
            .send()
            .unwrap();
        expect(res, 201)
    }

    /// A vault file without its `updated` line, which every save moves.
    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(path))
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .lines()
            .filter(|l| !l.starts_with("updated "))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "body: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

/// The `at` of every issue in a validation error.
fn issues(err: &Value) -> Vec<String> {
    err["issues"]
        .as_array()
        .unwrap_or_else(|| panic!("no issues: {err}"))
        .iter()
        .map(|i| s(&i["at"]).to_owned())
        .collect()
}

fn viewport(x: f64, y: f64, zoom: f64) -> Value {
    json!({ "kind": "viewport", "fields": { "x": x, "y": y, "zoom": zoom } })
}

/// Two cards of content blocks joined by an edge, the shape the canvas editor saves.
fn two_nodes_and_an_edge(label: Option<&str>) -> Value {
    let mut canvas = json!({
        "kind": "canvas",
        "fields": { "height": 640 },
        "blocks": [
            viewport(-120.5, 40.25, 0.75),
            {
                "kind": "node",
                "labels": ["idea"],
                "fields": { "kind": "card", "x": 0.0, "y": 0.0, "width": 240.0, "height": 120.0, "colour": "yellow" },
                "blocks": [
                    { "kind": "heading", "fields": { "level": 2, "text": "Idea" } },
                    { "kind": "paragraph", "fields": { "text": "An **idea**" } },
                    { "kind": "code", "fields": { "language": "sh", "source": "ls -la" } }
                ]
            },
            {
                "kind": "node",
                "labels": ["site"],
                "fields": { "kind": "card", "x": 320.5, "y": 10.25, "width": 240.0, "height": 120.0 },
                "blocks": [
                    {
                        "kind": "query_view",
                        "fields": { "query": "from board" },
                        "blocks": [{ "kind": "view", "labels": ["list"], "fields": { "kind": "list" } }]
                    }
                ]
            },
            {
                "kind": "edge",
                "labels": ["idea_to_site"],
                "fields": { "from": "idea", "to": "site", "label": "see", "colour": "red", "arrow": "start" }
            }
        ]
    });
    if let Some(l) = label {
        canvas["labels"] = json!([l]);
    }
    canvas
}

#[test]
fn two_nodes_and_an_edge_save_validate_and_reload_identically() {
    let api = Api::start();
    let board = two_nodes_and_an_edge(Some("board"));
    let content =
        json!({ "kind": "content", "labels": ["body"], "blocks": [two_nodes_and_an_edge(None)] });
    let note = api.board(json!([board, content]));
    let id = s(&note["id"]);

    let read = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
    let blocks = read["note"]["blocks"].as_array().unwrap();
    assert_eq!(blocks[0], board, "the canvas field reloaded differently");
    assert_eq!(
        blocks[1], content,
        "the content canvas reloaded differently"
    );

    let file = api.file(&format!("notes/board/{id}.wcl"));
    for line in [
        "canvas board {",
        "height = 640",
        "node idea {",
        "kind   = \"card\"",
        "paragraph {",
        "query_view {",
        "node site {",
        "edge idea_to_site {",
        "arrow  = \"start\"",
        "zoom = 0.75",
        "x      = 320.5",
    ] {
        assert!(file.contains(line), "missing {line:?} in {file}");
    }

    // Saving the canvas it read back changes nothing, and reads back the same again.
    let res = api
        .put(&format!("/api/notes/{id}"))
        .json(&json!({ "note": read["note"], "base_version": read["version"] }))
        .send()
        .unwrap();
    expect(res, 200);
    assert_eq!(api.file(&format!("notes/board/{id}.wcl")), file);

    // Through the canvas endpoint, the WCL it reads is the WCL it takes back unchanged.
    let wcl = expect(
        api.get(&format!("/api/notes/{id}/canvas/board"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(wcl["canvas"], board, "{wcl}");
    let res = api
        .put(&format!("/api/notes/{id}/canvas/board"))
        .json(&json!({ "wcl": wcl["wcl"], "base_version": wcl["version"] }))
        .send()
        .unwrap();
    let saved = expect(res, 200);
    assert_eq!(saved["wcl"], wcl["wcl"]);
    assert_eq!(api.file(&format!("notes/board/{id}.wcl")), file);
}

#[test]
fn a_canvas_that_breaks_the_format_is_refused_with_where() {
    let api = Api::start();
    let note = api.board(json!([two_nodes_and_an_edge(Some("board"))]));
    let id = s(&note["id"]);

    let node =
        |id: &str, fields: Value| json!({ "kind": "node", "labels": [id], "fields": fields });
    let text = |extra: Value| {
        let mut f = json!({ "kind": "card", "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 });
        for (k, v) in extra.as_object().unwrap() {
            f[k] = v.clone();
        }
        f
    };
    let cases = [
        (
            "an edge to a missing node",
            vec![
                viewport(0.0, 0.0, 1.0),
                node("a", text(json!({}))),
                json!({ "kind": "edge", "labels": ["e"], "fields": { "from": "a", "to": "gone" } }),
            ],
            r#"canvas "board" > edge "e".to"#,
        ),
        (
            "a zoom of zero",
            vec![viewport(0.0, 0.0, 0.0)],
            r#"canvas "board" > viewport.zoom"#,
        ),
        (
            "no viewport",
            vec![node("a", text(json!({})))],
            r#"canvas "board""#,
        ),
        (
            "a reused id",
            vec![
                viewport(0.0, 0.0, 1.0),
                node("a", text(json!({}))),
                node("a", text(json!({}))),
            ],
            r#"canvas "board" > node "a""#,
        ),
        (
            "a node in a missing group",
            vec![
                viewport(0.0, 0.0, 1.0),
                node("a", text(json!({ "group": "nope" }))),
            ],
            r#"canvas "board" > node "a".group"#,
        ),
        (
            "a node of no kind",
            vec![
                viewport(0.0, 0.0, 1.0),
                node(
                    "g",
                    json!({ "kind": "group", "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 }),
                ),
            ],
            r#"canvas "board" > node "g".kind"#,
        ),
        (
            "a card holding a canvas",
            vec![
                viewport(0.0, 0.0, 1.0),
                json!({ "kind": "node", "labels": ["c"], "fields": text(json!({})),
                        "blocks": [{ "kind": "canvas", "blocks": [viewport(0.0, 0.0, 1.0)] }] }),
            ],
            r#"canvas "board" > node "c" > canvas#1"#,
        ),
        (
            "a card's query view without its query",
            vec![
                viewport(0.0, 0.0, 1.0),
                json!({ "kind": "node", "labels": ["c"], "fields": text(json!({})),
                        "blocks": [{ "kind": "query_view", "blocks": [
                            { "kind": "view", "labels": ["list"], "fields": { "kind": "list" } }
                        ] }] }),
            ],
            r#"canvas "board" > node "c" > query_view#1.query"#,
        ),
        (
            "an arrow that is not one of the four",
            vec![
                viewport(0.0, 0.0, 1.0),
                node("a", text(json!({}))),
                json!({ "kind": "edge", "labels": ["e"], "fields": { "from": "a", "to": "a", "arrow": "sideways" } }),
            ],
            r#"canvas "board" > edge "e".arrow"#,
        ),
    ];
    let mut cases: Vec<(&str, Vec<Value>, &str, Value)> = cases
        .into_iter()
        .map(|(what, blocks, at)| (what, blocks, at, json!({})))
        .collect();
    for (what, height) in [("a canvas too short", 80), ("a canvas too tall", 9000)] {
        cases.push((
            what,
            vec![viewport(0.0, 0.0, 1.0)],
            r#"canvas "board".height"#,
            json!({ "height": height }),
        ));
    }
    for (what, blocks, at, fields) in cases {
        let canvas =
            json!({ "kind": "canvas", "labels": ["board"], "fields": fields, "blocks": blocks });
        let res = api
            .put(&format!("/api/notes/{id}"))
            .json(&json!({
                "note": { "fields": { "title": "Plan" }, "blocks": [canvas] },
                "base_version": note["version"],
            }))
            .send()
            .unwrap();
        let err = expect(res, 400);
        let at_list = issues(&err);
        assert!(
            at_list.iter().any(|a| a.ends_with(at)),
            "{what}: no issue at {at:?} in {at_list:?}"
        );
    }
    // Nothing was written.
    let read = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
    assert_eq!(read["version"], note["version"]);

    // A node of a kind cards replaced is not refused: it is written as the card it reads as.
    let old = json!({ "kind": "canvas", "labels": ["board"], "blocks": [
        viewport(0.0, 0.0, 1.0),
        node("q", json!({ "kind": "query", "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0,
                          "query": "from board", "view": "list" })),
    ] });
    let res = api
        .put(&format!("/api/notes/{id}"))
        .json(&json!({
            "note": { "fields": { "title": "Plan" }, "blocks": [old] },
            "base_version": note["version"],
        }))
        .send()
        .unwrap();
    expect(res, 200);
    let file = api.file(&format!("notes/board/{id}.wcl"));
    assert!(file.contains("kind   = \"card\""), "{file}");
    assert!(file.contains("query_view {"), "{file}");
    assert!(!file.contains("kind   = \"query\""), "{file}");
}

#[test]
fn a_canvas_on_a_template_validates_like_one_in_a_note() {
    let api = Api::start();
    let template = |query: &str| {
        format!(
            "template default {{\n  tab \"Board\" {{\n    slot \"title\"\n\n    canvas {{\n      height = 300\n\n      viewport {{\n        x    = 0.0\n        y    = 0.0\n        zoom = 1.0\n      }}\n\n      node open {{\n        kind   = \"card\"\n        x      = 0.0\n        y      = 0.0\n        width  = 300.0\n        height = 200.0\n\n        query_view {{\n          query = {query}\n\n          view cards {{\n            kind = \"cards\"\n          }}\n        }}\n      }}\n    }}\n  }}\n}}\n"
        )
    };
    let res = api
        .put("/api/types/board/templates/default")
        .json(&json!({ "wcl": template("\"from board where id != this\"") }))
        .send()
        .unwrap();
    expect(res, 200);
    let types = expect(api.get("/api/types").send().unwrap(), 200);
    let board = types["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "board")
        .unwrap();
    let items = &board["templates"][0]["tabs"][0]["items"];
    assert_eq!(items[1]["block"]["kind"], "canvas", "{items}");
    assert_eq!(items[1]["block"]["fields"]["height"], 300, "{items}");
    assert_eq!(items[1]["block"]["blocks"][1]["fields"]["kind"], "card");
    assert_eq!(
        items[1]["block"]["blocks"][1]["blocks"][0]["kind"],
        "query_view"
    );

    let res = api
        .put("/api/types/board/templates/default")
        .json(&json!({ "wcl": template("42") }))
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert!(
        issues(&err["failures"][0])
            .iter()
            .any(|a| a.ends_with(r#"node "open" > query_view#1.query"#)),
        "{err}"
    );

    // A template written before cards still loads: its query node reads as a card.
    let old = "template default {\n  tab \"Board\" {\n    slot \"title\"\n\n    canvas {\n      viewport {\n        x    = 0.0\n        y    = 0.0\n        zoom = 1.0\n      }\n\n      node open {\n        kind   = \"query\"\n        x      = 0.0\n        y      = 0.0\n        width  = 300.0\n        height = 200.0\n        view   = \"table\"\n        query  = \"from board\"\n      }\n    }\n  }\n}\n";
    let res = api
        .put("/api/types/board/templates/default")
        .json(&json!({ "wcl": old }))
        .send()
        .unwrap();
    expect(res, 200);
    let types = expect(api.get("/api/types").send().unwrap(), 200);
    let board = types["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "board")
        .unwrap();
    let node = &board["templates"][0]["tabs"][0]["items"][1]["block"]["blocks"][1];
    assert_eq!(node["fields"]["kind"], "card", "{node}");
    assert_eq!(node["blocks"][0]["kind"], "query_view", "{node}");
    assert_eq!(node["blocks"][0]["fields"]["query"], "from board", "{node}");
    assert_eq!(
        node["blocks"][0]["blocks"][0]["labels"][0], "table",
        "{node}"
    );
}
