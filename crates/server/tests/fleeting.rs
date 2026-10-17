//! S1 with S3 checks: the built-in fleeting note type behind the inbox.

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::TestServer;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn new(server: TestServer) -> Self {
        Api {
            server,
            client: Client::new(),
        }
    }

    fn request(&self, method: &str, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
    }

    fn send(&self, method: &str, path: &str) -> Response {
        self.request(method, path).send().unwrap()
    }

    fn post(&self, path: &str, body: Value) -> Response {
        self.request("POST", path).json(&body).send().unwrap()
    }

    fn put_type(&self, wcl: &str) -> Response {
        self.request("PUT", "/api/types/fleeting")
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn type_file(&self) -> String {
        std::fs::read_to_string(self.server.vault_path().join("types/fleeting.wcl")).unwrap()
    }
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

#[test]
fn every_new_vault_has_the_fleeting_type_marked_built_in() {
    let api = Api::new(TestServer::start());
    let file = api.type_file();
    assert!(file.contains("field processed"), "{file}");
    let types = expect(api.send("GET", "/api/types"), StatusCode::OK);
    let types = types["types"].as_array().unwrap();
    let fleeting = types.iter().find(|t| t["name"] == "fleeting").unwrap();
    assert_eq!(fleeting["label"], "Fleeting note");
    assert_eq!(fleeting["plural"], "Fleeting notes");
    assert_eq!(fleeting["icon"], "inbox");
    assert_eq!(fleeting["builtin"], true);
    let names: Vec<&str> = fleeting["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["body", "image", "processed"]);
    assert_eq!(fleeting["fields"][1]["kind"], "asset");
    assert_eq!(fleeting["fields"][1]["multiple"], true);
    assert_eq!(fleeting["fields"][2]["default"], false);
    let daily = types.iter().find(|t| t["name"] == "dailynote").unwrap();
    assert_eq!(daily["builtin"], true);
}

#[test]
fn a_vault_from_before_the_fleeting_type_gets_it_and_keeps_its_own_types() {
    // A vault made before the type existed: its own types, the daily note type, no fleeting type.
    const DAILY: &str = "label = \"Day\"\nplural = \"Days\"\n\nfield date {\n  kind     = \"date\"\n  required = true\n}\n\ntemplate default {\n  tab \"Day\" {\n    slot \"date\"\n  }\n}\n";
    const TASK: &str = "label = \"Task\"\nplural = \"Tasks\"\n\ntemplate default {\n  tab \"Task\" {\n    slot \"title\"\n  }\n}\n";
    let api = Api::new(TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(vault.join("types/dailynote.wcl"), DAILY).unwrap();
        std::fs::write(vault.join("types/task.wcl"), TASK).unwrap();
    }));
    assert!(api.type_file().contains("field processed"));
    let daily =
        std::fs::read_to_string(api.server.vault_path().join("types/dailynote.wcl")).unwrap();
    assert!(daily.contains("= \"Day\""), "{daily}");
    assert!(daily.contains("slot \"date\""), "{daily}");
    let ty = expect(api.send("GET", "/api/types/fleeting"), StatusCode::OK);
    assert!(
        ty["wcl"].as_str().unwrap().contains("Fleeting note"),
        "{ty}"
    );
}

#[test]
fn a_capture_waits_in_the_inbox_until_it_is_processed() {
    let api = Api::new(TestServer::start());
    let made = expect(
        api.post(
            "/api/notes?type=fleeting",
            json!({ "note": { "fields": { "title": "Call the plumber" }, "blocks": [
                { "kind": "content", "labels": ["body"], "blocks": [
                    { "kind": "paragraph", "fields": { "text": "Call the plumber about the leak" } }
                ] }
            ] } }),
        ),
        StatusCode::CREATED,
    );
    // A capture that leaves `processed` out gets its default, so it waits in the inbox.
    assert_eq!(made["note"]["fields"]["processed"], false, "{made}");
    let id = made["id"].as_str().unwrap().to_owned();
    let inbox = |api: &Api| {
        expect(
            api.post(
                "/api/query",
                json!({ "query": "from fleeting where !processed", "page_size": 10 }),
            ),
            StatusCode::OK,
        )
    };
    let waiting = inbox(&api);
    assert_eq!(waiting["total"], 1, "{waiting}");
    assert_eq!(waiting["rows"][0]["id"], id.as_str());

    let mut note = made["note"].clone();
    note["fields"]["processed"] = json!(true);
    expect(
        api.request("PUT", &format!("/api/notes/{id}"))
            .json(&json!({ "note": note, "base_version": made["version"] }))
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(inbox(&api)["total"], 0);
}

#[test]
fn the_fleeting_type_cannot_be_deleted() {
    let api = Api::new(TestServer::start());
    let body = expect(
        api.send("DELETE", "/api/types/fleeting"),
        StatusCode::CONFLICT,
    );
    assert_eq!(body["error"], "built_in_type");
    assert!(api.type_file().contains("field processed"));
}

#[test]
fn the_fleeting_type_can_be_customised_but_keeps_its_processed_bool() {
    let api = Api::new(TestServer::start());
    let base = api.type_file();

    let custom = base
        .replace(
            "field body {",
            "field source {\n  kind  = \"url\"\n  label = \"Source\"\n}\n\nfield body {",
        )
        .replace("colour      = \"yellow\"", "colour      = \"green\"");
    assert_ne!(custom, base);
    expect(api.put_type(&custom), StatusCode::OK);
    let customised = api.type_file();
    assert!(customised.contains("field source"), "{customised}");

    let start = customised.find("field processed").unwrap();
    let end = start + customised[start..].find("}\n").unwrap() + 2;
    let without = format!("{}{}", &customised[..start], &customised[end..]);
    let res = expect(api.put_type(&without), StatusCode::BAD_REQUEST);
    assert_eq!(res["error"], "invalid_type");

    let as_text = customised
        .replace("kind    = \"bool\"", "kind    = \"string\"")
        .replace("default = false", "default = \"no\"");
    assert_ne!(as_text, customised);
    expect(api.put_type(&as_text), StatusCode::BAD_REQUEST);

    assert_eq!(api.type_file(), customised);
}
