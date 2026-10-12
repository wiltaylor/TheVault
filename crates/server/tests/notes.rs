//! S1 with S3 checks: the notes API, its versions and 409s, history and trash, and the files the
//! writes leave in the vault folder.

use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, HeaderMap, HeaderValue, IF_MATCH,
};
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

field score {
  kind     = "int"
  computed = true
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

    /// Create a task from WCL text and return the JSON response.
    fn create(&self, wcl: &str) -> Value {
        let res = self
            .post("/api/notes?type=task")
            .header(CONTENT_TYPE, WCL)
            .body(wcl.to_owned())
            .send()
            .unwrap();
        expect(res, 201)
    }

    /// Replace a note with WCL text, sending `base` in `If-Match`.
    fn update(&self, id: &str, base: &str, wcl: &str) -> Response {
        self.put(&format!("/api/notes/{id}"))
            .header(CONTENT_TYPE, WCL)
            .header(IF_MATCH, format!("\"{base}\""))
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn file(&self, id: &str) -> Option<String> {
        std::fs::read_to_string(self.note_path(id)).ok()
    }

    fn note_path(&self, id: &str) -> std::path::PathBuf {
        self.server
            .vault_path()
            .join(format!("notes/task/{id}.wcl"))
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

/// The note's text without its `updated` line, which every write sets.
fn without_updated(wcl: &str) -> String {
    wcl.lines()
        .filter(|l| !l.starts_with("updated"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_utc_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 20 && b[4] == b'-' && b[10] == b'T' && b[19] == b'Z'
}

#[test]
fn create_read_update_delete_over_http() {
    let api = Api::start();

    // Create: the file appears, in canonical form, and equals the response.
    let created = api.create("title = \"Ship the importer\"\nestimate = 3\n");
    let id = s(&created["id"]).to_owned();
    let v1 = s(&created["version"]).to_owned();
    assert_eq!(created["type"], "task");
    let on_disk = api.file(&id).expect("the note file was written");
    assert_eq!(s(&created["wcl"]), on_disk);
    let lines: Vec<&str> = on_disk.lines().collect();
    assert_eq!(lines[0], "title    = \"Ship the importer\"");
    assert!(lines[1].starts_with("created  = \""), "{on_disk}");
    assert!(lines[2].starts_with("updated  = \""), "{on_disk}");
    assert_eq!(lines[3], "estimate = 3");
    assert_eq!(lines.len(), 4, "{on_disk}");

    // Read: as JSON, and as WCL text with the version in ETag.
    let read = expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 200);
    assert_eq!(read["wcl"], created["wcl"]);
    assert_eq!(read["version"], created["version"]);
    assert_eq!(read["note"]["fields"]["estimate"], 3);
    let res = api
        .get(&format!("/api/notes/{id}"))
        .header(ACCEPT, "text/plain")
        .send()
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()[ETAG].to_str().unwrap(), format!("\"{v1}\""));
    assert_eq!(res.text().unwrap(), on_disk);

    // Update: the file changes, the version moves, `created` stays.
    let updated = expect(
        api.update(&id, &v1, "title = \"Ship the importer\"\nestimate = 5\n"),
        200,
    );
    let v2 = s(&updated["version"]).to_owned();
    assert_ne!(v1, v2);
    let on_disk2 = api.file(&id).unwrap();
    assert_eq!(s(&updated["wcl"]), on_disk2);
    assert!(on_disk2.contains("estimate = 5"));
    assert_eq!(
        lines[1],
        on_disk2.lines().nth(1).unwrap(),
        "created changed"
    );

    // The listing by type shows it.
    let list = expect(api.get("/api/notes?type=task").send().unwrap(), 200);
    assert_eq!(list["notes"][0]["id"], id.as_str());
    assert_eq!(list["notes"][0]["title"], "Ship the importer");
    assert_eq!(list["notes"][0]["version"], v2.as_str());

    // Delete: the file leaves the vault for the trash.
    let deleted = expect(
        api.delete(&format!("/api/notes/{id}"))
            .header(IF_MATCH, format!("\"{v2}\""))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(deleted["wcl"], on_disk2.as_str());
    assert!(api.file(&id).is_none(), "the note file is still there");
    expect(api.get(&format!("/api/notes/{id}")).send().unwrap(), 404);
    let list = expect(api.get("/api/notes?type=task").send().unwrap(), 200);
    assert_eq!(list["notes"], json!([]));
    let trash = expect(api.get("/api/trash").send().unwrap(), 200);
    assert_eq!(trash["notes"][0]["id"], id.as_str());
    assert_eq!(trash["notes"][0]["type"], "task");
    assert_eq!(trash["notes"][0]["wcl"], on_disk2.as_str());

    // Restore from the trash: the file comes back with a new version.
    let restored = expect(
        api.post(&format!("/api/trash/{id}/restore"))
            .send()
            .unwrap(),
        200,
    );
    let on_disk3 = api.file(&id).expect("the restored file was written");
    assert_eq!(s(&restored["wcl"]), on_disk3);
    assert_eq!(without_updated(&on_disk3), without_updated(&on_disk2));
    assert_ne!(restored["version"], v2.as_str());
    let trash = expect(api.get("/api/trash").send().unwrap(), 200);
    assert_eq!(trash["notes"], json!([]));
}

#[test]
fn json_in_and_json_or_wcl_out() {
    let api = Api::start();
    let body = json!({
        "note": {
            "fields": { "estimate": 2, "title": "From JSON" },
            "blocks": [{
                "kind": "content",
                "labels": ["body"],
                "blocks": [{ "kind": "paragraph", "fields": { "text": "Hello **there**." } }]
            }]
        }
    });
    let res = api
        .post("/api/notes?type=task")
        .json(&body)
        .header(ACCEPT, "text/plain")
        .send()
        .unwrap();
    assert_eq!(res.status(), 201);
    let id = res.headers()["x-note-id"].to_str().unwrap().to_owned();
    let version = res.headers()[ETAG]
        .to_str()
        .unwrap()
        .trim_matches('"')
        .to_owned();
    let text = res.text().unwrap();
    let on_disk = api.file(&id).unwrap();
    assert_eq!(text, on_disk);
    assert!(
        on_disk.starts_with("title    = \"From JSON\"\n"),
        "{on_disk}"
    );
    assert!(
        on_disk.ends_with(
            "estimate = 2\n\ncontent body {\n  paragraph {\n    text = \"Hello **there**.\"\n  }\n}\n"
        ),
        "{on_disk}"
    );

    // A JSON update carries its base version in the body.
    let res = api
        .put(&format!("/api/notes/{id}"))
        .json(&json!({ "base_version": version, "wcl": "title = \"Renamed\"\n" }))
        .send()
        .unwrap();
    let updated = expect(res, 200);
    assert_eq!(s(&updated["wcl"]), api.file(&id).unwrap());
    assert_eq!(updated["note"]["fields"]["title"], "Renamed");
    assert!(updated["note"]["fields"].get("estimate").is_none());
    assert!(is_utc_datetime(s(&updated["note"]["fields"]["updated"])));
}

#[test]
fn messy_input_is_stored_canonically() {
    let api = Api::start();
    let created = api.create(
        "# a comment\nestimate=4   // trailing\ncontent body {\nparagraph { text = \"x\" }\n}\ncreated = \"1999-01-01T00:00:00Z\"\ntitle=\"Messy\"\n",
    );
    let id = s(&created["id"]);
    let on_disk = api.file(id).unwrap();
    assert_eq!(s(&created["wcl"]), on_disk);
    let created_at = s(&created["note"]["fields"]["created"]);
    assert_ne!(
        created_at, "1999-01-01T00:00:00Z",
        "the client set `created`"
    );
    assert!(is_utc_datetime(created_at));
    let updated_at = s(&created["note"]["fields"]["updated"]);
    assert_eq!(
        on_disk,
        format!(
            "title    = \"Messy\"\ncreated  = \"{created_at}\"\nupdated  = \"{updated_at}\"\nestimate = 4\n\ncontent body {{\n  paragraph {{\n    text = \"x\"\n  }}\n}}\n"
        )
    );
}

#[test]
fn a_stale_base_version_is_a_409_and_changes_nothing() {
    let api = Api::start();
    let created = api.create("title = \"Contested\"\nestimate = 1\n");
    let id = s(&created["id"]).to_owned();
    let v1 = s(&created["version"]).to_owned();
    let v2 = s(&expect(
        api.update(&id, &v1, "title = \"Contested\"\nestimate = 2\n"),
        200,
    )["version"])
    .to_owned();
    let before = api.file(&id).unwrap();

    // An update from someone still holding v1.
    let conflict = expect(
        api.update(&id, &v1, "title = \"Contested\"\nestimate = 99\n"),
        409,
    );
    assert_eq!(conflict["error"], "conflict");
    assert_eq!(conflict["id"], id.as_str());
    assert_eq!(conflict["current_version"], v2.as_str());
    assert_eq!(conflict["current"], before.as_str());
    assert_eq!(api.file(&id).unwrap(), before, "the file changed");

    // A delete from v1 is refused the same way, and the file stays.
    let conflict = expect(
        api.delete(&format!("/api/notes/{id}?base_version={v1}"))
            .send()
            .unwrap(),
        409,
    );
    assert_eq!(conflict["current_version"], v2.as_str());
    assert_eq!(api.file(&id).unwrap(), before);

    // A write without any base version is refused too.
    let res = api
        .put(&format!("/api/notes/{id}"))
        .header(CONTENT_TYPE, WCL)
        .body("title = \"No base\"\n")
        .send()
        .unwrap();
    assert_eq!(res.status(), 428);
    assert_eq!(api.file(&id).unwrap(), before);

    // History holds v1 and v2 only.
    let history = expect(
        api.get(&format!("/api/notes/{id}/history")).send().unwrap(),
        200,
    );
    assert_eq!(history["versions"].as_array().unwrap().len(), 2);
}

#[test]
fn an_expression_is_a_400_naming_the_item() {
    let api = Api::start();
    let res = api
        .post("/api/notes?type=task")
        .header(CONTENT_TYPE, WCL)
        .body("title = \"Sum\"\nestimate = 1 + 2\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["error"], "not_data_only");
    assert_eq!(err["item"], "estimate");
    assert!(s(&err["message"]).contains("estimate"), "{err}");

    let res = api
        .post("/api/notes?type=task")
        .header(CONTENT_TYPE, WCL)
        .body("import \"./other.wcl\"\ntitle = \"Imported\"\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["error"], "not_data_only");
    assert!(s(&err["item"]).contains("import"), "{err}");

    // An update with an expression leaves the existing file alone.
    let created = api.create("title = \"Plain\"\n");
    let id = s(&created["id"]).to_owned();
    let before = api.file(&id).unwrap();
    let err = expect(
        api.update(
            &id,
            s(&created["version"]),
            "title = \"Plain\"\nestimate = len(\"abc\")\n",
        ),
        400,
    );
    assert_eq!(err["item"], "estimate");
    assert_eq!(api.file(&id).unwrap(), before);

    let dir = api.server.vault_path().join("notes/task");
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
}

#[test]
fn a_note_that_breaks_its_schema_is_a_400_naming_the_field() {
    let api = Api::start();
    let res = api
        .post("/api/notes?type=task")
        .header(CONTENT_TYPE, WCL)
        .body("title = \"Bad\"\nestimate = \"three\"\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["error"], "invalid");
    assert_eq!(err["issues"][0]["at"], "estimate");

    // A computed field is written by handlers only (D3).
    let res = api
        .post("/api/notes?type=task")
        .header(CONTENT_TYPE, WCL)
        .body("title = \"Scored\"\nscore = 10\n")
        .send()
        .unwrap();
    let err = expect(res, 400);
    assert_eq!(err["issues"][0]["at"], "score");

    // An unknown type is not a type.
    let res = api
        .post("/api/notes?type=nope")
        .header(CONTENT_TYPE, WCL)
        .body("title = \"Lost\"\n")
        .send()
        .unwrap();
    assert_eq!(res.status(), 400);

    assert!(!api.server.vault_path().join("notes/task").exists());
}

#[test]
fn history_lists_every_version_and_restore_makes_a_new_one() {
    let api = Api::start();
    let created = api.create("title = \"Versioned\"\nestimate = 1\n");
    let id = s(&created["id"]).to_owned();
    let v1 = s(&created["version"]).to_owned();
    let v1_text = s(&created["wcl"]).to_owned();
    let v2 = s(&expect(
        api.update(&id, &v1, "title = \"Versioned again\"\nestimate = 2\n"),
        200,
    )["version"])
    .to_owned();

    let history = expect(
        api.get(&format!("/api/notes/{id}/history")).send().unwrap(),
        200,
    );
    let versions = history["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["version"], v1.as_str());
    assert_eq!(versions[1]["version"], v2.as_str());
    for v in versions {
        assert_eq!(v["author"]["kind"], "token", "{v}");
        assert_eq!(v["author"]["name"], testkit::ADMIN_TOKEN_NAME, "{v}");
        assert!(v["at"].is_i64(), "{v}");
    }
    assert_eq!(versions[1]["current"], true);
    assert_eq!(versions[0]["current"], false);

    // One old version, by token.
    let old = expect(
        api.get(&format!("/api/notes/{id}/history/{v1}"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(old["wcl"], v1_text.as_str());

    // Restoring with a stale base is a 409.
    let res = api
        .post(&format!("/api/notes/{id}/restore"))
        .json(&json!({ "version": v1, "base_version": v1 }))
        .send()
        .unwrap();
    assert_eq!(res.status(), 409);

    let restored = expect(
        api.post(&format!("/api/notes/{id}/restore"))
            .json(&json!({ "version": v1, "base_version": v2 }))
            .send()
            .unwrap(),
        200,
    );
    let v3 = s(&restored["version"]).to_owned();
    assert_ne!(v3, v1);
    assert_ne!(v3, v2);
    let on_disk = api.file(&id).unwrap();
    assert_eq!(s(&restored["wcl"]), on_disk);
    assert_eq!(without_updated(&on_disk), without_updated(&v1_text));

    let history = expect(
        api.get(&format!("/api/notes/{id}/history")).send().unwrap(),
        200,
    );
    let versions = history["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[2]["version"], v3.as_str());
    assert_eq!(versions[2]["current"], true);
}

#[test]
fn unknown_notes_and_versions_are_404() {
    let api = Api::start();
    let missing = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    expect(
        api.get(&format!("/api/notes/{missing}")).send().unwrap(),
        404,
    );
    assert_eq!(api.update(missing, "abc", "title = \"x\"\n").status(), 404);
    expect(
        api.post(&format!("/api/trash/{missing}/restore"))
            .send()
            .unwrap(),
        404,
    );
    let created = api.create("title = \"Here\"\n");
    let id = s(&created["id"]);
    expect(
        api.get(&format!("/api/notes/{id}/history/nope"))
            .send()
            .unwrap(),
        404,
    );
    expect(api.get("/api/notes/not-a-ulid").send().unwrap(), 404);
}
