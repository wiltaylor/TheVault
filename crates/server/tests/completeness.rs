//! S1 with S3 checks: assets, batches with dry runs, upserts, templates, taxonomy hierarchy edits,
//! scripts, canvases, schema inspection and the endpoint listing (D17, D32).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const STATUS: &str = r#"label = "Status"

term "Inbox"

term "Active" {
  term "In progress"

  term "Blocked"
}

term "Done"
"#;

const TASK: &str = r#"label = "Task"

field ext_id {
  kind = "string"
}

field estimate {
  kind = "int"
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

field cover {
  kind = "asset"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
  }
}
"#;

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

fn seed(vault: &Path) {
    for (dir, name, body) in [
        ("taxonomies", "status", STATUS),
        ("types", "task", TASK),
        ("types", "board", BOARD),
    ] {
        std::fs::create_dir_all(vault.join(dir)).unwrap();
        std::fs::write(vault.join(dir).join(format!("{name}.wcl")), body).unwrap();
    }
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(seed);
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

    fn delete(&self, path: &str) -> RequestBuilder {
        self.client.delete(self.server.url(path))
    }

    fn batch(&self, dry_run: bool, ops: &Value) -> Response {
        self.post("/api/batch")
            .json(&json!({ "dry_run": dry_run, "ops": ops }))
            .send()
            .unwrap()
    }

    fn read(&self, id: &str) -> Value {
        expect(self.get(&format!("/api/notes/{id}")).send().unwrap(), 200)
    }

    /// Every vault file outside `.vault/`, with its bytes.
    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let path = e.path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if rel == ".vault" {
                    continue;
                }
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.server.vault_path(), self.server.vault_path(), &mut out);
        out
    }

    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(path))
            .unwrap_or_else(|e| panic!("{path}: {e}"))
    }
}

fn client(token: &str) -> Client {
    let mut auth = HeaderMap::new();
    auth.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(auth).build().unwrap()
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

/// The parts of a batch report a dry run and the real run must agree on.
fn outcomes(report: &Value) -> Vec<(String, String, String)> {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            let error = i["error"]["error"].as_str().unwrap_or("").to_owned();
            (s(&i["op"]).to_owned(), s(&i["result"]).to_owned(), error)
        })
        .collect()
}

fn task(ext_id: &str, title: &str, estimate: i64) -> Value {
    json!({
        "op": "upsert",
        "type": "task",
        "key": "ext_id",
        "note": { "fields": { "title": title, "ext_id": ext_id, "estimate": estimate } },
    })
}

#[test]
fn an_uploaded_asset_is_stored_under_its_ulid_and_downloads_byte_identical() {
    let api = Api::start();
    let bytes: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    let res = api
        .post("/api/assets?filename=sample%20export.bin")
        .body(bytes.clone())
        .send()
        .unwrap();
    let asset = expect(res, 201);
    let id = s(&asset["id"]);
    assert_eq!(id.len(), 26, "{asset}");
    assert_eq!(asset["size"], 70_000);
    assert_eq!(s(&asset["path"]), format!("assets/{id}/sample export.bin"));
    let on_disk = std::fs::read(
        api.server
            .vault_path()
            .join(format!("assets/{id}/sample export.bin")),
    )
    .unwrap();
    assert_eq!(on_disk, bytes);

    let res = api.get(&format!("/api/assets/{id}")).send().unwrap();
    assert_eq!(res.status(), 200);
    let csp = res.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("sandbox"), "{csp}");
    assert_eq!(res.bytes().unwrap().to_vec(), bytes);

    let list = expect(api.get("/api/assets").send().unwrap(), 200);
    assert_eq!(list["assets"][0]["id"], id);

    // A note naming the asset holds it in place.
    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "With cover", "cover": id } } }))
        .send()
        .unwrap();
    let note = expect(res, 201);
    let res = api.delete(&format!("/api/assets/{id}")).send().unwrap();
    let err = expect(res, 409);
    assert_eq!(err["error"], "asset_in_use");
    assert_eq!(err["by"][0], format!("notes/task/{}.wcl", s(&note["id"])));

    let res = api
        .delete(&format!("/api/notes/{}", s(&note["id"])))
        .header("if-match", s(&note["version"]))
        .send()
        .unwrap();
    expect(res, 200);
    expect(
        api.delete(&format!("/api/assets/{id}")).send().unwrap(),
        200,
    );
    assert!(
        !api.server
            .vault_path()
            .join(format!("assets/{id}"))
            .exists()
    );
    expect(api.get(&format!("/api/assets/{id}")).send().unwrap(), 404);

    for bad in ["a%2Fb", "..", ""] {
        let res = api
            .post(&format!("/api/assets?filename={bad}"))
            .body("x")
            .send()
            .unwrap();
        expect(res, 400);
    }
}

#[test]
fn a_dry_run_reports_what_the_real_run_does_and_changes_no_file() {
    let api = Api::start();
    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "Old", "status": "Inbox" } } }))
        .send()
        .unwrap();
    let old = expect(res, 201);
    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "Doomed" } } }))
        .send()
        .unwrap();
    let doomed = expect(res, 201);

    let ops = json!([
        { "op": "create", "type": "task", "note": { "fields": { "title": "New", "status": "Active/Blocked" } } },
        { "op": "update", "id": old["id"], "base_version": old["version"],
          "wcl": "title = \"Old, edited\"\nstatus = \"Done\"\n" },
        { "op": "delete", "id": doomed["id"], "base_version": doomed["version"] },
        task("EXT-1", "Imported", 3),
    ]);
    let before = api.files();
    let dry = expect(api.batch(true, &ops), 200);
    assert_eq!(api.files(), before, "a dry run changed files");
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["committed"], false);
    assert!(dry["items"][0]["version"].is_null());
    assert!(s(&dry["items"][1]["wcl"]).contains("Old, edited"));

    let real = expect(api.batch(false, &ops), 200);
    assert_eq!(real["committed"], true);
    assert_eq!(outcomes(&dry), outcomes(&real));
    assert_eq!(
        outcomes(&real)
            .iter()
            .map(|(_, r, _)| r.as_str())
            .collect::<Vec<_>>(),
        ["created", "updated", "deleted", "created"]
    );
    // What the dry run said the update would write is what the real run wrote, but for the
    // `updated` stamp.
    let strip = |t: &str| {
        t.lines()
            .filter(|l| !l.starts_with("updated"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        strip(s(&dry["items"][1]["wcl"])),
        strip(&api.file(&format!("notes/task/{}.wcl", s(&old["id"]))))
    );
    let new_id = s(&real["items"][0]["id"]);
    assert!(
        api.file(&format!("notes/task/{new_id}.wcl"))
            .contains("Active/Blocked")
    );
    assert!(
        !api.server
            .vault_path()
            .join(format!("notes/task/{}.wcl", s(&doomed["id"])))
            .exists()
    );
}

#[test]
fn a_failing_batch_commits_nothing_and_its_dry_run_fails_the_same_way() {
    let api = Api::start();
    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "Keep" } } }))
        .send()
        .unwrap();
    let keep = expect(res, 201);
    let ops = json!([
        task("EXT-1", "Fine", 1),
        { "op": "create", "type": "task", "note": { "fields": { "title": "Bad", "status": "Nope" } } },
        { "op": "update", "id": keep["id"], "base_version": "stale", "wcl": "title = \"x\"\n" },
        { "op": "create", "type": "nosuch", "wcl": "title = \"x\"\n" },
        task("EXT-1", "Twice", 2),
    ]);
    let before = api.files();
    let dry = expect(api.batch(true, &ops), 422);
    let real = expect(api.batch(false, &ops), 422);
    assert_eq!(api.files(), before, "a failed batch changed files");
    assert_eq!(real["committed"], false);
    assert_eq!(outcomes(&dry), outcomes(&real));
    let errors: Vec<String> = outcomes(&real).into_iter().map(|(_, _, e)| e).collect();
    assert_eq!(
        errors,
        ["", "invalid", "conflict", "unknown_type", "bad_request"]
    );
    assert_eq!(real["items"][0]["result"], "created");
    assert!(real["items"][0]["version"].is_null());
}

#[test]
fn upserting_the_same_batch_twice_creates_once_and_then_changes_nothing() {
    let api = Api::start();
    let ops = json!([
        task("EXT-1", "First", 1),
        task("EXT-2", "Second", 2),
        task("EXT-3", "Third", 3),
    ]);
    let first = expect(api.batch(false, &ops), 200);
    let results: Vec<String> = outcomes(&first).into_iter().map(|(_, r, _)| r).collect();
    assert_eq!(results, ["created", "created", "created"]);
    let files = api.files();
    let notes = expect(api.get("/api/notes?type=task").send().unwrap(), 200);
    assert_eq!(notes["notes"].as_array().unwrap().len(), 3);

    let dry = expect(api.batch(true, &ops), 200);
    let second = expect(api.batch(false, &ops), 200);
    for report in [&dry, &second] {
        let results: Vec<String> = outcomes(report).into_iter().map(|(_, r, _)| r).collect();
        assert_eq!(results, ["unchanged", "unchanged", "unchanged"]);
    }
    assert_eq!(api.files(), files, "the second upsert changed files");
    for n in 0..3 {
        let item = &second["items"][n];
        assert_eq!(item["id"], first["items"][n]["id"]);
        assert_eq!(item["version"], first["items"][n]["version"]);
        let history = expect(
            api.get(&format!("/api/notes/{}/history", s(&item["id"])))
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(history["versions"].as_array().unwrap().len(), 1);
    }

    // A changed value updates the matched note and keeps its other fields.
    let res = api
        .post("/api/upsert")
        .json(&json!({ "type": "task", "key": "ext_id",
                       "note": { "fields": { "ext_id": "EXT-2", "title": "Second", "estimate": 5 } } }))
        .send()
        .unwrap();
    let one = expect(res, 200);
    assert_eq!(one["items"][0]["result"], "updated");
    assert_eq!(one["items"][0]["id"], first["items"][1]["id"]);
    let note = api.read(s(&one["items"][0]["id"]));
    assert_eq!(note["note"]["fields"]["estimate"], 5);
    assert_eq!(note["note"]["fields"]["ext_id"], "EXT-2");

    // A key that is not a single scalar field is refused.
    let res = api
        .post("/api/upsert")
        .json(&json!({ "type": "task", "key": "status",
                       "note": { "fields": { "title": "x", "status": "Done" } } }))
        .send()
        .unwrap();
    let err = expect(res, 422);
    assert_eq!(err["items"][0]["error"]["error"], "bad_request");
}

#[test]
fn schema_inspection_lists_a_type_created_through_the_api() {
    let api = Api::start();
    let wcl = r#"label = "Person"
description = "Someone I know."

field email {
  kind     = "string"
  required = true
  default  = "unknown@example.com"
}

field born {
  kind = "date"
}

field role {
  kind    = "select"
  options = ["friend", "colleague"]
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
  multiple = true
}

field manager {
  kind      = "reference"
  target    = "person"
  on_delete = "restrict"
}

template default {
  tab "Main" {
    slot "title"
    slot "email"
  }
}

template card {
  tab "Card" {
    slot "title"
  }
}
"#;
    let res = api
        .put("/api/types/person")
        .header(CONTENT_TYPE, "text/plain")
        .body(wcl)
        .send()
        .unwrap();
    expect(res, 200);

    let schema = expect(api.get("/api/schema").send().unwrap(), 200);
    let person = schema["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "person")
        .unwrap_or_else(|| panic!("no person type in {schema}"));
    assert_eq!(person["label"], "Person");
    assert_eq!(person["description"], "Someone I know.");
    let fields: Vec<(&str, &str)> = person["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (s(&f["name"]), s(&f["kind"])))
        .collect();
    assert_eq!(
        fields,
        [
            ("email", "string"),
            ("born", "date"),
            ("role", "select"),
            ("status", "taxonomy"),
            ("manager", "reference"),
        ]
    );
    let field = |n: &str| {
        person["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == n)
            .unwrap()
    };
    assert_eq!(field("email")["required"], true);
    assert_eq!(field("email")["default"], "unknown@example.com");
    assert_eq!(field("role")["options"], json!(["friend", "colleague"]));
    assert_eq!(field("status")["taxonomy"], "status");
    assert_eq!(field("status")["multiple"], true);
    assert_eq!(field("manager")["target"], "person");
    assert_eq!(field("manager")["relation"], "one_way");
    assert_eq!(field("manager")["on_delete"], "restrict");
    let templates: Vec<&str> = person["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| s(&t["name"]))
        .collect();
    assert_eq!(templates, ["default", "card"]);
    assert_eq!(
        person["templates"][0]["tabs"][0]["items"][1]["slot"],
        "email"
    );
    let status = &schema["taxonomies"][0];
    assert_eq!(status["name"], "status");
    assert_eq!(
        status["terms"][1]["children"][0]["path"],
        "Active/In progress"
    );
}

#[test]
fn the_endpoint_listing_covers_every_registered_route() {
    let api = Api::start();
    let doc = expect(api.get("/api/openapi.json").send().unwrap(), 200);
    assert_eq!(doc["openapi"], "3.1.0");
    let mut listed = BTreeSet::new();
    for (path, ops) in doc["paths"].as_object().unwrap() {
        for method in ops.as_object().unwrap().keys() {
            listed.insert((method.to_ascii_uppercase(), path.clone()));
        }
    }
    let registered: BTreeSet<(String, String)> = api::routes()
        .into_iter()
        .map(|(m, p)| (m.to_string(), p.to_owned()))
        .collect();
    assert_eq!(listed, registered);
    assert_eq!(doc["paths"]["/api/batch"]["post"]["x-scope"], "write");
    assert_eq!(
        doc["paths"]["/api/taxonomies/{name}"]["put"]["x-scope"],
        "admin"
    );
    assert_eq!(doc["paths"]["/api/health"]["get"]["security"], json!([]));

    // Every listed route is served: none answers the router's 404 or 405.
    for (method, path) in &listed {
        let concrete = path
            .split('/')
            .map(|seg| if seg.starts_with('{') { "x" } else { seg })
            .collect::<Vec<_>>()
            .join("/");
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let res = api
            .client
            .request(method.clone(), api.server.url(&concrete))
            .header(CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .unwrap();
        let status = res.status().as_u16();
        let body = res.text().unwrap();
        assert!(
            status != 405 && !(status == 404 && body.is_empty()),
            "{method} {concrete}: {status} {body}"
        );
    }
}

#[test]
fn templates_are_read_replaced_and_deleted_inside_the_type_file() {
    let api = Api::start();
    // The type has notes; templates change none of them, so this is allowed.
    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "T" } } }))
        .send()
        .unwrap();
    expect(res, 201);

    let list = expect(api.get("/api/types/task/templates").send().unwrap(), 200);
    assert_eq!(list["templates"][0]["name"], "default");
    let one = expect(
        api.get("/api/types/task/templates/default").send().unwrap(),
        200,
    );
    assert!(s(&one["wcl"]).starts_with("template default {"), "{one}");

    let card =
        "template card {\n  tab \"Card\" {\n    slot \"title\"\n    slot \"estimate\"\n  }\n}\n";
    let res = api
        .put("/api/types/task/templates/card")
        .header(CONTENT_TYPE, "text/plain")
        .body(card)
        .send()
        .unwrap();
    let put = expect(res, 200);
    assert!(s(&put["wcl"]).contains("slot \"estimate\""), "{put}");
    let file = api.file("types/task.wcl");
    assert!(file.contains("template card {"), "{file}");
    assert!(file.contains("field ext_id {"), "{file}");

    // A template placing a field the type does not have is refused.
    let res = api
        .put("/api/types/task/templates/card")
        .json(&json!({ "wcl": "template card {\n  tab \"C\" {\n    slot \"nope\"\n  }\n}\n" }))
        .send()
        .unwrap();
    assert_eq!(expect(res, 400)["error"], "invalid_type");
    // `{{name}}` in a fixed block's text must name `title` or a field of the type.
    let res = api
        .put("/api/types/task/templates/card")
        .json(&json!({ "wcl": "template card {\n  tab \"C\" {\n    paragraph {\n      text = \"Due {{nope}}\"\n    }\n  }\n}\n" }))
        .send()
        .unwrap();
    let refused = expect(res, 400);
    assert_eq!(refused["error"], "invalid_type");
    assert!(
        refused
            .to_string()
            .contains("`{{nope}}` names no field of the type"),
        "{refused}"
    );
    let res = api
        .put("/api/types/task/templates/card")
        .json(&json!({ "wcl": "template card {\n  tab \"C\" {\n    paragraph {\n      text = \"{{title}}: {{estimate}} hours\"\n    }\n  }\n}\n" }))
        .send()
        .unwrap();
    expect(res, 200);
    // The label must match the path.
    let res = api
        .put("/api/types/task/templates/card")
        .json(&json!({ "wcl": "template other {\n  tab \"C\" {\n    slot \"title\"\n  }\n}\n" }))
        .send()
        .unwrap();
    expect(res, 400);

    expect(
        api.delete("/api/types/task/templates/card").send().unwrap(),
        200,
    );
    assert!(!api.file("types/task.wcl").contains("template card"));
    expect(
        api.delete("/api/types/task/templates/default")
            .send()
            .unwrap(),
        400,
    );
    expect(
        api.get("/api/types/task/templates/card").send().unwrap(),
        404,
    );
}

#[test]
fn a_taxonomy_edit_renames_and_moves_terms_in_every_note() {
    let api = Api::start();
    let mut ids = BTreeMap::new();
    for status in ["Active", "Active/In progress", "Active/Blocked", "Done"] {
        let res = api
            .post("/api/notes?type=task")
            .json(&json!({ "note": { "fields": { "title": status, "status": status } } }))
            .send()
            .unwrap();
        ids.insert(status, s(&expect(res, 201)["id"]).to_owned());
    }
    let untouched = api.file(&format!("notes/task/{}.wcl", ids["Done"]));

    // "Active" becomes "Doing", and "Blocked" moves to the top level.
    let new = "label = \"Status\"\n\nterm \"Inbox\"\n\nterm \"Doing\" {\n  term \"In progress\"\n}\n\nterm \"Blocked\"\n\nterm \"Done\"\n";
    let res = api
        .put("/api/taxonomies/status")
        .json(&json!({ "wcl": new, "renames": { "Active": "Doing", "Active/Blocked": "Blocked" } }))
        .send()
        .unwrap();
    let out = expect(res, 200);
    assert_eq!(out["notes"].as_array().unwrap().len(), 3, "{out}");
    let status_of = |id: &str| api.read(id)["note"]["fields"]["status"].clone();
    assert_eq!(status_of(&ids["Active"]), "Doing");
    assert_eq!(status_of(&ids["Active/In progress"]), "Doing/In progress");
    assert_eq!(status_of(&ids["Active/Blocked"]), "Blocked");
    assert_eq!(
        api.file(&format!("notes/task/{}.wcl", ids["Done"])),
        untouched
    );
    assert!(api.file("taxonomies/status.wcl").contains("term \"Doing\""));

    // Dropping a term a note holds is refused and changes nothing.
    let before = api.files();
    let res = api
        .put("/api/taxonomies/status")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Status\"\n\nterm \"Done\"\n")
        .send()
        .unwrap();
    let err = expect(res, 409);
    assert_eq!(err["error"], "term_in_use");
    assert_eq!(api.files(), before);

    // A taxonomy a field uses cannot be deleted; an unused one can.
    expect(api.delete("/api/taxonomies/status").send().unwrap(), 409);
    let res = api
        .put("/api/taxonomies/area")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Area\"\n\nterm \"Home\"\n")
        .send()
        .unwrap();
    expect(res, 200);
    let list = expect(api.get("/api/taxonomies").send().unwrap(), 200);
    assert_eq!(list["taxonomies"].as_array().unwrap().len(), 2);
    expect(api.delete("/api/taxonomies/area").send().unwrap(), 200);
    assert!(!api.server.vault_path().join("taxonomies/area.wcl").exists());
}

#[test]
fn term_colours_round_trip_and_an_unknown_colour_is_refused() {
    let api = Api::start();
    let wcl = "label = \"Area\"\n\nterm \"Plant\" {\n  colour = \"green\"\n\n  term \"Electrical\" {\n    term \"Wiring\" {\n      colour = \"gray\"\n    }\n  }\n}\n\nterm \"Office\"\n";
    let res = api
        .put("/api/taxonomies/area")
        .header(CONTENT_TYPE, "text/plain")
        .body(wcl)
        .send()
        .unwrap();
    expect(res, 200);
    assert_eq!(api.file("taxonomies/area.wcl"), wcl);
    let read = expect(api.get("/api/taxonomies/area").send().unwrap(), 200);
    let plant = &read["terms"][0];
    assert_eq!(plant["colour"], "green");
    assert_eq!(plant["children"][0]["colour"], Value::Null);
    assert_eq!(plant["children"][0]["children"][0]["colour"], "gray");
    assert_eq!(read["terms"][1]["colour"], Value::Null);
    let schema = expect(api.get("/api/schema").send().unwrap(), 200);
    let area = schema["taxonomies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "area")
        .unwrap();
    assert_eq!(area["terms"][0]["colour"], "green");

    // An unknown colour is a validation error naming the term and the palette, and writes nothing.
    let before = api.files();
    let res = api
        .put("/api/taxonomies/area")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Area\"\n\nterm \"Plant\" {\n  colour = \"teal\"\n}\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    let text = err.to_string();
    assert!(text.contains("term \\\"Plant\\\""), "{text}");
    assert!(text.contains("is not a palette colour"), "{text}");
    assert_eq!(api.files(), before);
}

#[test]
fn scripts_are_stored_as_sent_and_kept_while_a_handler_names_them() {
    let api = Api::start();
    let source = "fn run() {\n    // digest\n}\n";
    let res = api
        .put("/api/scripts/digest")
        .header(CONTENT_TYPE, "text/plain")
        .body(source)
        .send()
        .unwrap();
    expect(res, 200);
    assert_eq!(api.file("scripts/digest.ws"), source);
    let read = expect(api.get("/api/scripts/digest").send().unwrap(), 200);
    assert_eq!(read["source"], source);
    let list = expect(api.get("/api/scripts").send().unwrap(), 200);
    assert_eq!(list["scripts"], json!(["digest"]));

    let res = api
        .put("/api/types/note")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Note\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n\nhandler \"after_create\" {\n  script = \"digest\"\n}\n")
        .send()
        .unwrap();
    expect(res, 200);
    let err = expect(api.delete("/api/scripts/digest").send().unwrap(), 409);
    assert_eq!(err["error"], "script_in_use");
    expect(
        api.put("/api/scripts/Bad-Name").body("x").send().unwrap(),
        400,
    );
    expect(api.get("/api/scripts/missing").send().unwrap(), 404);
}

#[test]
fn a_script_that_does_not_compile_is_stored_and_its_problems_reported() {
    let api = Api::start();
    let put = |name: &str, source: &str| {
        expect(
            api.put(&format!("/api/scripts/{name}"))
                .json(&json!({ "source": source }))
                .send()
                .unwrap(),
            200,
        )
    };

    let fine = put("fine", "fn run() { }\n");
    assert_eq!(fine["problems"], json!([]));

    let broken = put("broken", "fn run( {\n");
    assert_eq!(api.file("scripts/broken.ws"), "fn run( {\n");
    let problems = broken["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1, "{broken}");
    assert_eq!(problems[0]["role"], "task");
    assert!(
        problems[0]["message"]
            .as_str()
            .unwrap()
            .contains("does not compile"),
        "{broken}"
    );
    let read = expect(api.get("/api/scripts/broken").send().unwrap(), 200);
    assert_eq!(read["problems"], broken["problems"]);

    // A before_* handler cannot reach HTTP, so a script one names that uses it is a problem there.
    let source = "use http\nfn before_update(note: Value) -> Result[Value, string] { Ok(note) }\n";
    put("gate", source);
    let res = api
        .put("/api/types/gated")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Gated\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n\nhandler \"before_update\" {\n  script = \"gate\"\n}\n")
        .send()
        .unwrap();
    expect(res, 200);
    let read = expect(api.get("/api/scripts/gate").send().unwrap(), 200);
    let problems = read["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1, "{read}");
    assert_eq!(problems[0]["role"], "before_handler");
}

#[test]
fn a_note_canvas_is_read_and_replaced_at_its_version() {
    let api = Api::start();
    let wcl = "title = \"Plan\"\n\ncanvas board {\n  viewport {\n    x    = 0.0\n    y    = 0.0\n    zoom = 1.0\n  }\n}\n\ncontent body {\n  paragraph {\n    text = \"Intro\"\n  }\n\n  canvas {\n    viewport {\n      x    = 5.0\n      y    = 5.0\n      zoom = 2.0\n    }\n  }\n}\n";
    let res = api
        .post("/api/notes?type=board")
        .header(CONTENT_TYPE, "text/plain")
        .body(wcl)
        .send()
        .unwrap();
    let note = expect(res, 201);
    let id = s(&note["id"]);

    let board = expect(
        api.get(&format!("/api/notes/{id}/canvas/board"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(board["version"], note["version"]);
    assert!(s(&board["wcl"]).starts_with("canvas board {"), "{board}");
    let inner = expect(
        api.get(&format!("/api/notes/{id}/canvas/body.1"))
            .send()
            .unwrap(),
        200,
    );
    assert!(s(&inner["wcl"]).contains("zoom = 2.0"), "{inner}");
    expect(
        api.get(&format!("/api/notes/{id}/canvas/body.2"))
            .send()
            .unwrap(),
        404,
    );

    let new = "canvas {\n  viewport {\n    x    = 1.0\n    y    = 2.0\n    zoom = 0.5\n  }\n\n  node goal {\n    kind   = \"text\"\n    x      = 0.0\n    y      = 0.0\n    width  = 100.0\n    height = 50.0\n    text   = \"Goal\"\n  }\n}\n";
    let res = api
        .put(&format!("/api/notes/{id}/canvas/board"))
        .json(&json!({ "wcl": new, "base_version": note["version"] }))
        .send()
        .unwrap();
    let saved = expect(res, 200);
    assert_ne!(saved["version"], note["version"]);
    let file = api.file(&format!("notes/board/{id}.wcl"));
    assert!(file.contains("node goal {"), "{file}");
    assert!(
        file.contains("zoom = 2.0"),
        "the content canvas changed: {file}"
    );

    // The old version is stale now.
    let res = api
        .put(&format!("/api/notes/{id}/canvas/board"))
        .json(&json!({ "wcl": new, "base_version": note["version"] }))
        .send()
        .unwrap();
    assert_eq!(expect(res, 409)["error"], "conflict");

    let res = api
        .put(&format!("/api/notes/{id}/canvas/body.1"))
        .json(&json!({ "wcl": new, "base_version": saved["version"] }))
        .send()
        .unwrap();
    expect(res, 200);
    let file = api.file(&format!("notes/board/{id}.wcl"));
    assert!(!file.contains("zoom = 2.0"), "{file}");
}

#[test]
fn a_write_token_can_batch_and_upload_but_not_change_definitions() {
    let api = Api::start();
    let res = api
        .post("/api/tokens")
        .json(&json!({ "name": "agent", "scope": "write" }))
        .send()
        .unwrap();
    let token = expect(res, 201);
    let agent = client(s(&token["secret"]));
    let res = agent
        .post(api.server.url("/api/batch"))
        .json(&json!({ "ops": [task("EXT-9", "From agent", 1)] }))
        .send()
        .unwrap();
    expect(res, 200);
    let res = agent
        .post(api.server.url("/api/assets?filename=a.txt"))
        .body("hi")
        .send()
        .unwrap();
    expect(res, 201);
    expect(
        agent.get(api.server.url("/api/schema")).send().unwrap(),
        200,
    );
    for path in [
        "/api/taxonomies/status",
        "/api/scripts/x",
        "/api/types/task/templates/card",
    ] {
        let res = agent.put(api.server.url(path)).body("x").send().unwrap();
        expect(res, 403);
    }
}

#[test]
fn a_type_with_notes_is_not_deleted_and_the_refusal_counts_them() {
    let api = Api::start();
    let mut ids = Vec::new();
    for title in ["One", "Two"] {
        let res = api
            .post("/api/notes?type=task")
            .json(&json!({ "note": { "fields": { "title": title } } }))
            .send()
            .unwrap();
        ids.push(expect(res, 201));
    }
    let before = api.files();
    let err = expect(api.delete("/api/types/task").send().unwrap(), 409);
    assert_eq!(err["error"], "type_in_use");
    assert_eq!(err["count"], 2, "{err}");
    assert_eq!(err["notes"].as_array().unwrap().len(), 2, "{err}");
    assert!(s(&err["message"]).contains("2 notes"), "{err}");
    assert_eq!(api.files(), before);

    for note in &ids {
        let res = api
            .delete(&format!("/api/notes/{}", s(&note["id"])))
            .header("if-match", s(&note["version"]))
            .send()
            .unwrap();
        expect(res, 200);
    }
    let out = expect(api.delete("/api/types/task").send().unwrap(), 200);
    assert_eq!(out["deleted"], true);
    assert!(!api.server.vault_path().join("types/task.wcl").exists());
    let types = expect(api.get("/api/types").send().unwrap(), 200);
    assert!(
        types["types"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "task"),
        "{types}"
    );
    expect(api.delete("/api/types/task").send().unwrap(), 404);
}

#[test]
fn a_taxonomy_in_use_is_not_deleted_and_the_refusal_lists_the_notes() {
    let api = Api::start();
    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "Finish", "status": "Done" } } }))
        .send()
        .unwrap();
    let note = expect(res, 201);
    let id = s(&note["id"]);
    expect(
        api.post("/api/notes?type=task")
            .json(&json!({ "note": { "fields": { "title": "No status" } } }))
            .send()
            .unwrap(),
        201,
    );

    let before = api.files();
    let err = expect(api.delete("/api/taxonomies/status").send().unwrap(), 409);
    assert_eq!(err["error"], "taxonomy_in_use");
    let by: Vec<&str> = err["by"].as_array().unwrap().iter().map(s).collect();
    assert!(by.contains(&"types/task.wcl (field status)"), "{err}");
    let notes = err["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{err}");
    assert_eq!(notes[0]["id"], id);
    assert_eq!(notes[0]["type"], "task");
    assert_eq!(notes[0]["title"], "Finish");
    assert_eq!(api.files(), before);

    // Dropping the term the note holds is refused the same way.
    let res = api
        .put("/api/taxonomies/status")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Status\"\n\nterm \"Inbox\"\n")
        .send()
        .unwrap();
    let err = expect(res, 409);
    assert_eq!(err["error"], "term_in_use");
    assert_eq!(err["notes"][0]["id"], id, "{err}");
    assert_eq!(err["notes"][0]["title"], "Finish", "{err}");
    assert_eq!(api.files(), before);

    // A term no note holds can go.
    let res = api
        .put("/api/taxonomies/status")
        .header(CONTENT_TYPE, "text/plain")
        .body("label = \"Status\"\n\nterm \"Done\"\n")
        .send()
        .unwrap();
    expect(res, 200);
    expect(api.delete("/api/taxonomies/nope").send().unwrap(), 404);
}

#[test]
fn an_asset_notes_name_is_deleted_with_unlink_and_every_reference_goes() {
    let api = Api::start();
    let upload = |name: &str| {
        let res = api
            .post(&format!("/api/assets?filename={name}"))
            .body("bytes")
            .send()
            .unwrap();
        s(&expect(res, 201)["id"]).to_owned()
    };
    let id = upload("photo.png");
    let keep = upload("other.png");

    let res = api
        .post("/api/notes?type=task")
        .json(&json!({ "note": { "fields": { "title": "Covered", "cover": id } } }))
        .send()
        .unwrap();
    let task = expect(res, 201);
    let board_wcl = format!(
        "title = \"Board\"\n\ncanvas board {{\n  viewport {{\n    x    = 0.0\n    y    = 0.0\n    zoom = 1.0\n  }}\n\n  node pic {{\n    kind   = \"asset\"\n    x      = 0.0\n    y      = 0.0\n    width  = 100.0\n    height = 100.0\n    asset  = \"{id}\"\n  }}\n\n  node goal {{\n    kind   = \"text\"\n    x      = 200.0\n    y      = 0.0\n    width  = 100.0\n    height = 50.0\n    text   = \"Goal\"\n  }}\n\n  edge pic_goal {{\n    from = \"pic\"\n    to   = \"goal\"\n  }}\n}}\n\ncontent body {{\n  paragraph {{\n    text = \"Intro\"\n  }}\n\n  image {{\n    asset = \"{id}\"\n    alt   = \"Photo\"\n  }}\n\n  asset {{\n    asset = \"{id}\"\n  }}\n\n  image {{\n    asset = \"{keep}\"\n  }}\n}}\n"
    );
    let res = api
        .post("/api/notes?type=board")
        .header(CONTENT_TYPE, "text/plain")
        .body(board_wcl)
        .send()
        .unwrap();
    let board = expect(res, 201);
    let (task_id, board_id) = (s(&task["id"]).to_owned(), s(&board["id"]).to_owned());

    // Without unlink the delete is refused, listing the notes by title.
    let before = api.files();
    let err = expect(
        api.delete(&format!("/api/assets/{id}")).send().unwrap(),
        409,
    );
    assert_eq!(err["error"], "asset_in_use");
    let mut titles: Vec<&str> = err["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| s(&n["title"]))
        .collect();
    titles.sort();
    assert_eq!(titles, ["Board", "Covered"], "{err}");
    assert_eq!(api.files(), before);

    // With unlink the references go in the same commit as the asset.
    let res = api
        .delete(&format!("/api/assets/{id}?unlink=true"))
        .send()
        .unwrap();
    let out = expect(res, 200);
    assert_eq!(out["deleted"], true);
    assert_eq!(out["notes"].as_array().unwrap().len(), 2, "{out}");
    assert!(
        !api.server
            .vault_path()
            .join(format!("assets/{id}"))
            .exists()
    );
    expect(api.get(&format!("/api/assets/{id}")).send().unwrap(), 404);

    let task_now = api.read(&task_id);
    assert!(
        task_now["note"]["fields"].get("cover").is_none(),
        "{task_now}"
    );
    assert_ne!(task_now["version"], task["version"]);
    let board_file = api.file(&format!("notes/board/{board_id}.wcl"));
    assert!(!board_file.contains(&id), "{board_file}");
    assert!(!board_file.contains("node pic"), "{board_file}");
    assert!(!board_file.contains("edge pic_goal"), "{board_file}");
    assert!(board_file.contains("node goal"), "{board_file}");
    assert!(board_file.contains(&keep), "{board_file}");
    assert!(board_file.contains("text = \"Intro\""), "{board_file}");
    let history = expect(
        api.get(&format!("/api/notes/{board_id}/history"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(history["versions"].as_array().unwrap().len(), 2);

    // An asset a type file names is kept even with unlink: a type is not a note.
    let pinned = upload("floor.png");
    let room = format!(
        "label = \"Room\"\n\nfield desk {{\n  kind  = \"location\"\n  map   = \"image\"\n  image = \"{pinned}\"\n}}\n\ntemplate default {{\n  tab \"Main\" {{\n    slot \"title\"\n  }}\n}}\n"
    );
    expect(
        api.put("/api/types/room")
            .header(CONTENT_TYPE, "text/plain")
            .body(room)
            .send()
            .unwrap(),
        200,
    );
    let before = api.files();
    let err = expect(
        api.delete(&format!("/api/assets/{pinned}?unlink=true"))
            .send()
            .unwrap(),
        409,
    );
    assert_eq!(err["error"], "asset_in_use");
    assert_eq!(err["by"][0], "types/room.wcl", "{err}");
    assert_eq!(api.files(), before);

    // An asset nothing names is deleted with or without unlink.
    let lone = upload("lone.png");
    expect(
        api.delete(&format!("/api/assets/{lone}?unlink=true"))
            .send()
            .unwrap(),
        200,
    );
    expect(
        api.delete(&format!("/api/assets/{lone}")).send().unwrap(),
        404,
    );
}
