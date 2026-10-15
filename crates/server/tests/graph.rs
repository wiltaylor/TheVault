//! S1 with S3 checks: `GET /api/notes/{id}/graph`, the link neighbourhood the right sidebar draws.
//! Mentions and reference fields both link; placeholder links and notes in the trash do not.

use std::collections::BTreeSet;
use std::path::Path;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, IF_MATCH};
use serde_json::Value;
use testkit::TestServer;

const TEMPLATE: &str = r#"
template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const BODY: &str = r#"
field body {
  kind = "content"
}
"#;

/// A project holds its tasks, two-way with `task.project`.
const PROJECT: &str = r#"label = "Project"

field tasks {
  kind     = "reference"
  target   = "task"
  relation = "two_way"
  inverse  = "project"
  multiple = true
}
"#;

const TASK: &str = r#"label = "Task"

field project {
  kind     = "reference"
  target   = "project"
  relation = "two_way"
  inverse  = "tasks"
}
"#;

/// A comment points one way at a project.
const COMMENT: &str = r#"label = "Comment"

field about {
  kind   = "reference"
  target = "project"
}
"#;

const DOC: &str = "label = \"Doc\"\n";

const WCL: &str = "text/plain; charset=utf-8";

fn seed(vault: &Path) {
    let types = vault.join("types");
    std::fs::create_dir_all(&types).unwrap();
    for (name, body) in [
        ("project", PROJECT),
        ("task", TASK),
        ("comment", COMMENT),
        ("doc", DOC),
    ] {
        std::fs::write(
            types.join(format!("{name}.wcl")),
            format!("{body}{BODY}{TEMPLATE}"),
        )
        .unwrap();
    }
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

    fn create(&self, type_name: &str, wcl: &str) -> String {
        let res = self
            .client
            .post(self.server.url(&format!("/api/notes?type={type_name}")))
            .header(CONTENT_TYPE, WCL)
            .body(wcl.to_owned())
            .send()
            .unwrap();
        s(&expect(res, 201)["id"]).to_owned()
    }

    fn version(&self, id: &str) -> String {
        let res = self
            .client
            .get(self.server.url(&format!("/api/notes/{id}")))
            .send()
            .unwrap();
        s(&expect(res, 200)["version"]).to_owned()
    }

    fn update(&self, id: &str, wcl: &str) {
        let base = self.version(id);
        let res = self
            .client
            .put(self.server.url(&format!("/api/notes/{id}")))
            .header(CONTENT_TYPE, WCL)
            .header(IF_MATCH, format!("\"{base}\""))
            .body(wcl.to_owned())
            .send()
            .unwrap();
        expect(res, 200);
    }

    fn delete(&self, id: &str) {
        let base = self.version(id);
        let res = self
            .client
            .delete(self.server.url(&format!("/api/notes/{id}")))
            .header(IF_MATCH, format!("\"{base}\""))
            .send()
            .unwrap();
        expect(res, 200);
    }

    fn graph(&self, id: &str, query: &str) -> Response {
        self.client
            .get(self.server.url(&format!("/api/notes/{id}/graph{query}")))
            .send()
            .unwrap()
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

/// A note's WCL: a title, extra fields, and one paragraph of `text` in its body.
fn note(title: &str, fields: &str, text: &str) -> String {
    format!(
        "title = {}\n{fields}\ncontent body {{\n  paragraph {{\n    text = {}\n  }}\n}}\n",
        serde_json::to_string(title).unwrap(),
        serde_json::to_string(text).unwrap()
    )
}

/// Every node as `(id, direction)`.
fn nodes(g: &Value) -> BTreeSet<(String, String)> {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (s(&n["id"]).to_owned(), s(&n["direction"]).to_owned()))
        .collect()
}

/// Every edge as `(from, to, via, field)`.
fn edges(g: &Value) -> Vec<(String, String, String, String)> {
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                s(&e["from"]).to_owned(),
                s(&e["to"]).to_owned(),
                s(&e["via"]).to_owned(),
                s(&e["field"]).to_owned(),
            )
        })
        .collect()
}

fn edge(from: &str, to: &str, via: &str, field: &str) -> (String, String, String, String) {
    (from.into(), to.into(), via.into(), field.into())
}

#[test]
fn the_graph_holds_links_both_ways_and_among_neighbours() {
    let api = Api::start();
    let doc = api.create("doc", &note("Design", "", "plain"));
    // The project mentions the doc twice, and a placeholder that is not a link.
    let project = api.create(
        "project",
        &note(
            "Pump",
            "",
            &format!("See [[{doc}]], again [[{doc}|the design]], later [[+doc|Not yet]]."),
        ),
    );
    let comment = api.create(
        "comment",
        &note("Thoughts", &format!("about = \"{project}\""), "x"),
    );
    // A link between two neighbours.
    api.update(&doc, &note("Design", "", &format!("Re [[{comment}]]")));
    // Two-way: the project gains the task in `tasks`.
    let task = api.create(
        "task",
        &note("Fix pump", &format!("project = \"{project}\""), ""),
    );
    // A note in the trash links to nothing.
    let gone = api.create("doc", &note("Old", "", &format!("[[{project}]]")));
    api.delete(&gone);

    let g = expect(api.graph(&project, ""), 200);
    assert_eq!(g["id"], project.as_str());
    assert_eq!(g["truncated"], 0);
    assert_eq!(
        g["nodes"][0]["id"],
        project.as_str(),
        "the centre comes first"
    );
    assert_eq!(g["nodes"][0]["title"], "Pump");
    assert_eq!(g["nodes"][0]["type"], "project");
    assert_eq!(
        nodes(&g),
        BTreeSet::from([
            (project.clone(), "center".to_owned()),
            (doc.clone(), "out".to_owned()),
            (comment.clone(), "in".to_owned()),
            (task.clone(), "both".to_owned()),
        ])
    );
    let mut want = vec![
        edge(&project, &doc, "mention", "body"),
        edge(&comment, &project, "field", "about"),
        edge(&doc, &comment, "mention", "body"),
        edge(&project, &task, "field", "tasks"),
        edge(&task, &project, "field", "project"),
    ];
    want.sort();
    assert_eq!(edges(&g), want);

    // Seen from the doc, the comment links back and the project links in.
    let g = expect(api.graph(&doc, ""), 200);
    assert_eq!(
        nodes(&g),
        BTreeSet::from([
            (doc.clone(), "center".to_owned()),
            (project.clone(), "in".to_owned()),
            (comment.clone(), "out".to_owned()),
        ])
    );
    // The comment's link to the project is between two neighbours, so it is drawn too.
    assert!(edges(&g).contains(&edge(&comment, &project, "field", "about")));

    // A cap keeps the most recently updated neighbours and counts the rest.
    let g = expect(api.graph(&project, "?limit=2"), 200);
    assert_eq!(g["truncated"], 1);
    assert_eq!(g["nodes"].as_array().unwrap().len(), 3);
    let kept: BTreeSet<String> = nodes(&g).into_iter().map(|(id, _)| id).collect();
    for (from, to, _, _) in edges(&g) {
        assert!(kept.contains(&from) && kept.contains(&to), "{from} -> {to}");
    }
    let updated: Vec<String> = g["nodes"].as_array().unwrap()[1..]
        .iter()
        .map(|n| s(&n["updated"]).to_owned())
        .collect();
    assert!(updated[0] >= updated[1], "newest first: {updated:?}");
}

#[test]
fn a_note_with_no_links_has_an_empty_graph_and_an_unknown_one_is_404() {
    let api = Api::start();
    let alone = api.create("doc", &note("Alone", "", "no links [[+doc|Someday]]"));
    let g = expect(api.graph(&alone, ""), 200);
    assert_eq!(g["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(g["edges"].as_array().unwrap().len(), 0);
    assert_eq!(g["truncated"], 0);

    let res = api.graph("01ARZ3NDEKTSV4RRFFQ69G5FAV", "");
    assert_eq!(expect(res, 404)["error"], "not_found");

    // A trashed note's graph is gone with it.
    api.delete(&alone);
    assert_eq!(api.graph(&alone, "").status().as_u16(), 404);
}
