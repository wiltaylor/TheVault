//! S1 with S3 checks: type create, change and delete, where a change is planned by one call and
//! applied by a second (D6), and the note files each step leaves in the vault folder.

use std::collections::BTreeMap;
use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{CONTENT_TYPE, IF_MATCH};
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

field legacy {
  kind = "string"
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

const WCL: &str = "text/plain; charset=utf-8";

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/task.wcl"), TASK_TYPE).unwrap();
}

/// `TASK_TYPE` with `block` added after its last field.
fn task_with(block: &str) -> String {
    TASK_TYPE.replace("template default", &format!("{block}\n\ntemplate default"))
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

    fn create_note(&self, type_name: &str, wcl: &str) -> String {
        let res = self
            .post(&format!("/api/notes?type={type_name}"))
            .header(CONTENT_TYPE, WCL)
            .body(wcl.to_owned())
            .send()
            .unwrap();
        s(&expect(res, 201)["id"]).to_owned()
    }

    /// Three tasks: one with every field set, one with only an estimate, one with a title only.
    fn three_tasks(&self) -> Vec<String> {
        vec![
            self.create_note(
                "task",
                "title = \"Full\"\nestimate = 3\nlegacy = \"old data\"\ncontent body {\n  paragraph {\n    text = \"Hello.\"\n  }\n}\n",
            ),
            self.create_note("task", "title = \"Estimated\"\nestimate = 1\n"),
            self.create_note("task", "title = \"Bare\"\n"),
        ]
    }

    /// Plan a change to `type_name`: the response to the first call.
    fn plan(&self, type_name: &str, wcl: &str, renames: &[(&str, &str)]) -> Response {
        let renames: BTreeMap<&str, &str> = renames.iter().copied().collect();
        self.put(&format!("/api/types/{type_name}"))
            .json(&json!({ "wcl": wcl, "renames": renames }))
            .send()
            .unwrap()
    }

    /// Apply a planned change: the second call, carrying the plan token.
    fn apply(&self, type_name: &str, wcl: &str, renames: &[(&str, &str)], plan: &str) -> Response {
        let renames: BTreeMap<&str, &str> = renames.iter().copied().collect();
        self.put(&format!("/api/types/{type_name}"))
            .json(&json!({ "wcl": wcl, "renames": renames, "plan": plan }))
            .send()
            .unwrap()
    }

    fn note_file(&self, type_name: &str, id: &str) -> String {
        std::fs::read_to_string(
            self.server
                .vault_path()
                .join(format!("notes/{type_name}/{id}.wcl")),
        )
        .unwrap_or_else(|e| panic!("note {id}: {e}"))
    }

    fn note_files(&self, ids: &[String]) -> Vec<String> {
        ids.iter().map(|id| self.note_file("task", id)).collect()
    }

    fn type_file(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.server.vault_path().join(format!("types/{name}.wcl"))).ok()
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

/// The ids a plan says it changes.
fn planned_ids(plan: &Value) -> Vec<String> {
    let mut ids: Vec<String> = plan["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| s(&c["id"]).to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(ids: &[String]) -> Vec<String> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids
}

/// The value of top-level field `name` in a note file, as written.
fn field<'a>(file: &'a str, name: &str) -> Option<&'a str> {
    file.lines()
        .filter_map(|l| l.split_once(" = "))
        .find(|(n, _)| n.trim_end() == name)
        .map(|(_, v)| v)
}

#[test]
fn a_new_required_field_without_a_default_is_rejected() {
    let api = Api::start();
    let ids = api.three_tasks();
    let before = api.note_files(&ids);

    let wcl = task_with("field owner {\n  kind = \"string\"\n  required = true\n}");
    let err = expect(api.plan("task", &wcl, &[]), 400);
    assert_eq!(err["error"], "invalid");
    assert_eq!(err["files"][0]["file"], "types/task.wcl");
    assert_eq!(err["files"][0]["issues"][0]["at"], "field owner.default");

    // A plan token cannot force it through either.
    let err = expect(api.apply("task", &wcl, &[], "anything"), 400);
    assert_eq!(err["error"], "invalid");

    assert_eq!(api.type_file("task").unwrap(), TASK_TYPE);
    assert_eq!(api.note_files(&ids), before);
}

#[test]
fn a_new_required_field_with_a_default_reaches_every_note_only_after_apply() {
    let api = Api::start();
    let ids = api.three_tasks();
    let before = api.note_files(&ids);
    let wcl =
        task_with("field owner {\n  kind = \"string\"\n  required = true\n  default = \"Wil\"\n}");

    // The plan lists every note and changes nothing.
    let plan = expect(api.plan("task", &wcl, &[]), 200);
    assert_eq!(plan["applied"], false);
    assert_eq!(plan["added"], json!(["owner"]));
    assert_eq!(planned_ids(&plan), sorted(&ids));
    for change in plan["changes"].as_array().unwrap() {
        assert_eq!(
            change["edits"],
            json!([{ "op": "default", "field": "owner", "value": "Wil" }]),
            "{change}"
        );
        assert!(change["version"].is_string(), "{change}");
    }
    assert_eq!(api.note_files(&ids), before, "planning changed a note");
    assert_eq!(api.type_file("task").unwrap(), TASK_TYPE);

    // Applying writes the default into every note and the field into the type file.
    let token = s(&plan["plan"]).to_owned();
    let applied = expect(api.apply("task", &wcl, &[], &token), 200);
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["notes"].as_array().unwrap().len(), 3);
    for (id, old) in ids.iter().zip(&before) {
        let file = api.note_file("task", id);
        assert_eq!(field(&file, "owner"), Some("\"Wil\""), "{file}");
        assert_eq!(field(&file, "created"), field(old, "created"));
        assert_eq!(field(&file, "estimate"), field(old, "estimate"));
        // The response's version is the note's new current version.
        let new_version = applied["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id.as_str())
            .unwrap()["version"]
            .clone();
        let read = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
        assert_eq!(read["version"], new_version);
        assert_eq!(s(&read["wcl"]), file);
    }
    let type_file = api.type_file("task").unwrap();
    assert!(type_file.contains("field owner {"), "{type_file}");
    assert!(type_file.contains("default  = \"Wil\""), "{type_file}");
}

#[test]
fn a_new_optional_field_leaves_every_note_byte_identical() {
    let api = Api::start();
    let ids = api.three_tasks();
    let before = api.note_files(&ids);
    let wcl = task_with("field owner {\n  kind = \"string\"\n  default = \"Wil\"\n}");

    let plan = expect(api.plan("task", &wcl, &[]), 200);
    assert_eq!(plan["changes"], json!([]));
    expect(api.apply("task", &wcl, &[], s(&plan["plan"])), 200);

    assert_eq!(api.note_files(&ids), before);
    assert!(api.type_file("task").unwrap().contains("field owner {"));
}

#[test]
fn renaming_a_field_moves_the_value_in_every_note() {
    let api = Api::start();
    let ids = api.three_tasks();
    let before = api.note_files(&ids);
    let wcl = TASK_TYPE
        .replace("field estimate", "field points")
        .replace("slot \"estimate\"", "slot \"points\"")
        .replace("field body", "field description");
    let renames = [("estimate", "points"), ("body", "description")];

    let plan = expect(api.plan("task", &wcl, &renames), 200);
    assert_eq!(
        plan["renamed"],
        json!([{ "from": "body", "to": "description" }, { "from": "estimate", "to": "points" }])
    );
    // The bare note holds neither field, so only two notes change.
    assert_eq!(planned_ids(&plan), sorted(&ids[..2]));
    assert_eq!(api.note_files(&ids), before);

    expect(api.apply("task", &wcl, &renames, s(&plan["plan"])), 200);
    let after = api.note_files(&ids);
    assert_eq!(field(&after[0], "points"), Some("3"), "{}", after[0]);
    assert!(field(&after[0], "estimate").is_none(), "{}", after[0]);
    assert!(
        after[0].ends_with("content description {\n  paragraph {\n    text = \"Hello.\"\n  }\n}\n"),
        "{}",
        after[0]
    );
    assert_eq!(field(&after[1], "points"), Some("1"), "{}", after[1]);
    assert_eq!(after[2], before[2], "the bare note changed");

    // The renamed notes validate under the new type: none is flagged.
    let flagged = expect(api.get("/api/flagged").send().unwrap(), 200);
    assert_eq!(flagged["notes"], json!([]));
}

#[test]
fn removing_a_field_keeps_the_data_and_flags_the_notes() {
    let api = Api::start();
    let ids = api.three_tasks();
    let before = api.note_files(&ids);
    let wcl = TASK_TYPE.replace("field legacy {\n  kind = \"string\"\n}\n\n", "");

    let plan = expect(api.plan("task", &wcl, &[]), 200);
    assert_eq!(plan["removed"], json!(["legacy"]));
    assert_eq!(plan["changes"], json!([]));
    assert_eq!(plan["flagged"], json!([ids[0]]));
    expect(api.apply("task", &wcl, &[], s(&plan["plan"])), 200);

    assert_eq!(api.note_files(&ids), before, "a note file changed");
    assert!(!api.type_file("task").unwrap().contains("legacy"));

    let flagged = expect(api.get("/api/flagged").send().unwrap(), 200);
    assert_eq!(
        flagged["notes"],
        json!([{
            "id": ids[0],
            "type": "task",
            "title": "Full",
            "version": flagged["notes"][0]["version"],
            "unknown": ["legacy"],
        }])
    );
    let list = expect(api.get("/api/notes?type=task").send().unwrap(), 200);
    for n in list["notes"].as_array().unwrap() {
        assert_eq!(n["flagged"], n["id"] == ids[0].as_str(), "{n}");
    }
}

#[test]
fn a_plan_applied_after_a_note_changed_fails_and_changes_nothing() {
    let api = Api::start();
    let ids = api.three_tasks();
    let wcl =
        task_with("field owner {\n  kind = \"string\"\n  required = true\n  default = \"Wil\"\n}");
    let plan = expect(api.plan("task", &wcl, &[]), 200);

    // Someone edits a note between the plan and the apply.
    let read = expect(
        api.get(&format!("/api/notes/{}", ids[1])).send().unwrap(),
        200,
    );
    let res = api
        .put(&format!("/api/notes/{}", ids[1]))
        .header(CONTENT_TYPE, WCL)
        .header(IF_MATCH, format!("\"{}\"", s(&read["version"])))
        .body("title = \"Estimated\"\nestimate = 8\n")
        .send()
        .unwrap();
    expect(res, 200);
    let before = api.note_files(&ids);

    let stale = expect(api.apply("task", &wcl, &[], s(&plan["plan"])), 409);
    assert_eq!(stale["error"], "plan_stale");
    assert_ne!(stale["plan"]["plan"], plan["plan"]);
    assert_eq!(planned_ids(&stale["plan"]), sorted(&ids));
    assert_eq!(api.note_files(&ids), before, "a stale plan changed a note");
    assert_eq!(api.type_file("task").unwrap(), TASK_TYPE);

    // So does a note created since the plan.
    let fresh = s(&stale["plan"]["plan"]).to_owned();
    api.create_note("task", "title = \"New\"\n");
    expect(api.apply("task", &wcl, &[], &fresh), 409);
    assert_eq!(api.type_file("task").unwrap(), TASK_TYPE);
}

#[test]
fn types_are_created_and_deleted() {
    let api = Api::start();
    let project = "label = \"Project\"\n\nfield code {\n  kind     = \"string\"\n  required = true\n}\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";

    let created = expect(
        api.post("/api/types")
            .json(&json!({ "name": "project", "wcl": project }))
            .send()
            .unwrap(),
        201,
    );
    assert_eq!(created["name"], "project");
    assert_eq!(api.type_file("project").as_deref(), Some(project));
    assert_eq!(s(&created["wcl"]), project);

    // The new type takes notes at once, and validates them.
    let id = api.create_note("project", "title = \"Vault\"\ncode = \"TV\"\n");
    let res = api
        .post("/api/notes?type=project")
        .header(CONTENT_TYPE, WCL)
        .body("title = \"No code\"\n")
        .send()
        .unwrap();
    assert_eq!(expect(res, 400)["issues"][0]["at"], "code");

    // A type that exists cannot be created again, and a broken one not at all.
    let res = api
        .post("/api/types")
        .json(&json!({ "name": "project", "wcl": project }))
        .send()
        .unwrap();
    assert_eq!(expect(res, 409)["error"], "type_exists");
    let res = api
        .post("/api/types")
        .json(&json!({ "name": "broken", "wcl": "label = 3\n" }))
        .send()
        .unwrap();
    assert_eq!(expect(res, 400)["error"], "invalid");
    assert!(api.type_file("broken").is_none());

    // A type with notes is not deleted.
    let in_use = expect(api.delete("/api/types/project").send().unwrap(), 409);
    assert_eq!(in_use["error"], "type_in_use");
    assert_eq!(in_use["notes"], json!([id]));
    assert!(api.type_file("project").is_some());

    // Once its notes are gone, it is.
    let read = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
    let res = api
        .delete(&format!("/api/notes/{id}"))
        .header(IF_MATCH, format!("\"{}\"", s(&read["version"])))
        .send()
        .unwrap();
    expect(res, 200);
    expect(api.delete("/api/types/project").send().unwrap(), 200);
    assert!(api.type_file("project").is_none());
    let res = api
        .post("/api/notes?type=project")
        .header(CONTENT_TYPE, WCL)
        .body("title = \"Orphan\"\ncode = \"X\"\n")
        .send()
        .unwrap();
    assert_eq!(expect(res, 400)["error"], "unknown_type");
    expect(api.delete("/api/types/project").send().unwrap(), 404);
    expect(api.plan("project", project, &[]), 404);
}

#[test]
fn a_change_that_would_break_existing_notes_is_refused() {
    let api = Api::start();
    let ids = api.three_tasks();
    let before = api.note_files(&ids);
    let wcl = TASK_TYPE.replace(
        "field estimate {\n  kind = \"int\"",
        "field estimate {\n  kind = \"string\"",
    );

    let err = expect(api.plan("task", &wcl, &[]), 400);
    assert_eq!(err["error"], "invalid");
    let mut files: Vec<String> = err["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| s(&f["file"]).to_owned())
        .collect();
    files.sort();
    let expected: Vec<String> = sorted(&ids[..2])
        .iter()
        .map(|id| format!("notes/task/{id}.wcl"))
        .collect();
    assert_eq!(files, expected);
    assert_eq!(err["files"][0]["issues"][0]["at"], "estimate");
    assert_eq!(api.note_files(&ids), before);
    assert_eq!(api.type_file("task").unwrap(), TASK_TYPE);
}
