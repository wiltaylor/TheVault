//! S1: the built-in `message`, `call` and `conversation` types, and the `person` fields they
//! rely on (D15, D24).

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::TestServer;

const PHONE_TYPES: [&str; 3] = ["message", "call", "conversation"];

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

    fn put_type(&self, name: &str, wcl: &str) -> Response {
        self.request("PUT", &format!("/api/types/{name}"))
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn type_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(format!("types/{name}.wcl"))).unwrap()
    }

    fn types(&self) -> Vec<Value> {
        let body = expect(self.send("GET", "/api/types"), StatusCode::OK);
        body["types"].as_array().unwrap().clone()
    }

    fn create(&self, type_name: &str, note: Value) -> String {
        let made = expect(
            self.post(
                &format!("/api/notes?type={type_name}"),
                json!({ "note": note }),
            ),
            StatusCode::CREATED,
        );
        made["id"].as_str().unwrap().to_owned()
    }
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn find<'a>(types: &'a [Value], name: &str) -> &'a Value {
    types
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no type {name}"))
}

fn field<'a>(ty: &'a Value, name: &str) -> &'a Value {
    ty["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("{} has no field {name}", ty["name"]))
}

fn field_names(ty: &Value) -> Vec<&str> {
    ty["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect()
}

#[test]
fn a_new_vault_has_the_phone_types_with_the_fields_the_spec_lists() {
    let api = Api::new(TestServer::start());
    let types = api.types();

    let message = find(&types, "message");
    assert_eq!(message["builtin"], true);
    assert_eq!(
        field_names(message),
        [
            "channel",
            "direction",
            "from",
            "to",
            "from_raw",
            "to_raw",
            "conversation",
            "sent",
            "body",
            "attachments",
            "partial",
            "device_ref"
        ]
    );
    assert_eq!(
        field(message, "channel")["options"],
        json!(["sms", "mms", "rcs"])
    );
    assert_eq!(field(message, "direction")["options"], json!(["in", "out"]));
    assert_eq!(field(message, "from")["target"], "person");
    assert_eq!(field(message, "from")["multiple"], false);
    assert_eq!(field(message, "to")["target"], "person");
    assert_eq!(field(message, "to")["multiple"], true);
    assert_eq!(field(message, "from_raw")["kind"], "string");
    assert_eq!(field(message, "to_raw")["kind"], "list");
    assert_eq!(field(message, "to_raw")["items"], "string");
    assert_eq!(field(message, "conversation")["target"], "conversation");
    assert_eq!(field(message, "sent")["kind"], "datetime");
    assert_eq!(field(message, "body")["kind"], "content");
    assert_eq!(field(message, "attachments")["kind"], "asset");
    assert_eq!(field(message, "attachments")["multiple"], true);
    assert_eq!(field(message, "partial")["kind"], "bool");
    assert_eq!(field(message, "device_ref")["kind"], "string");

    let call = find(&types, "call");
    assert_eq!(call["builtin"], true);
    assert_eq!(
        field_names(call),
        [
            "direction",
            "person",
            "number",
            "started",
            "duration",
            "device_ref"
        ]
    );
    assert_eq!(
        field(call, "direction")["options"],
        json!(["in", "out", "missed"])
    );
    assert_eq!(field(call, "person")["target"], "person");
    assert_eq!(field(call, "number")["kind"], "string");
    assert_eq!(field(call, "started")["kind"], "datetime");
    assert_eq!(field(call, "duration")["kind"], "int");

    let conversation = find(&types, "conversation");
    assert_eq!(conversation["builtin"], true);
    assert_eq!(
        field_names(conversation),
        ["participants", "channel", "thread_ref"]
    );
    assert_eq!(field(conversation, "participants")["target"], "person");
    assert_eq!(field(conversation, "participants")["multiple"], true);
    assert_eq!(field(conversation, "channel")["kind"], "select");
    assert_eq!(field(conversation, "thread_ref")["kind"], "string");

    // The types reference `person`, so a vault without the starter pack gets one.
    let person = find(&types, "person");
    assert_eq!(person["builtin"], false);
    assert_eq!(field(person, "phones")["kind"], "list");
    assert_eq!(field(person, "phones")["items"], "string");
    assert_eq!(field(person, "emails")["items"], "string");
    assert_eq!(field(person, "addresses")["items"], "string");
    assert_eq!(field(person, "websites")["items"], "url");
    assert_eq!(field(person, "photo")["kind"], "asset");
    assert_eq!(field(person, "device_contacts")["kind"], "dict");
    assert_eq!(field(person, "device_contacts")["hidden"], true);
    assert_eq!(field(person, "phones")["hidden"], false);
}

#[test]
fn an_existing_vault_gets_the_phone_types_and_keeps_its_own_person_with_the_phone_fields() {
    // A vault from before the phone types: the older built-ins and its own `person`.
    const PERSON: &str = "label = \"Contact\"\n\nfield email {\n  kind = \"string\"\n}\n\ntemplate default {\n  tab \"Contact\" {\n    slot \"title\"\n  }\n}\n";
    let api = Api::new(TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(
            vault.join("types/dailynote.wcl"),
            writer::DEFAULT_DAILY_TYPE,
        )
        .unwrap();
        std::fs::write(
            vault.join("types/fleeting.wcl"),
            writer::DEFAULT_FLEETING_TYPE,
        )
        .unwrap();
        std::fs::write(vault.join("types/person.wcl"), PERSON).unwrap();
    }));
    let types = api.types();
    for name in PHONE_TYPES {
        assert_eq!(find(&types, name)["builtin"], true, "{name}");
        assert!(api.type_file(name).contains("label"), "{name}");
    }
    // Its own person, with its single `email` moved into `emails` by the upgrade step, the
    // fields the device scripts write added, and nothing else changed.
    let person = api.type_file("person");
    assert!(person.starts_with("label = \"Contact\"\n"), "{person}");
    assert!(person.contains("tab \"Contact\""), "{person}");
    let person = find(&types, "person");
    assert_eq!(
        field_names(person),
        [
            "emails",
            "email_labels",
            "phones",
            "phone_labels",
            "addresses",
            "address_labels",
            "organisation",
            "job_title",
            "nickname",
            "birthday",
            "websites",
            "contact_note",
            "photo",
            "device_contacts",
            "body"
        ]
    );
}

#[test]
fn the_phone_types_cannot_be_deleted() {
    let api = Api::new(TestServer::start());
    for name in PHONE_TYPES {
        let before = api.type_file(name);
        let body = expect(
            api.send("DELETE", &format!("/api/types/{name}")),
            StatusCode::CONFLICT,
        );
        assert_eq!(body["error"], "built_in_type", "{name}");
        assert_eq!(api.type_file(name), before);
    }
}

#[test]
fn the_phone_types_can_be_customised() {
    let api = Api::new(TestServer::start());
    for name in PHONE_TYPES {
        let base = api.type_file(name);
        let custom = base.replacen(
            "template default {",
            "field note {\n  kind  = \"string\"\n  label = \"Note\"\n}\n\ntemplate default {",
            1,
        );
        assert_ne!(custom, base);
        expect(api.put_type(name, &custom), StatusCode::OK);
        assert!(api.type_file(name).contains("field note"), "{name}");
        let ty = expect(
            api.send("GET", &format!("/api/types/{name}")),
            StatusCode::OK,
        );
        assert!(ty["wcl"].as_str().unwrap().contains("field note"), "{ty}");
    }
}

#[test]
fn a_conversation_page_queries_its_messages() {
    let api = Api::new(TestServer::start());
    let alex = api.create(
        "person",
        json!({ "fields": { "title": "Alex", "phones": ["+61400000001"] } }),
    );
    let thread = api.create(
        "conversation",
        json!({ "fields": { "title": "Alex", "participants": [alex], "channel": "sms" } }),
    );
    let other = api.create(
        "conversation",
        json!({ "fields": { "title": "Someone else", "channel": "sms" } }),
    );
    for (title, sent, conversation) in [
        ("See you soon", "2026-10-15T09:05:00Z", &thread),
        ("Running late", "2026-10-15T09:00:00Z", &thread),
        ("Unrelated", "2026-10-15T09:02:00Z", &other),
    ] {
        api.create(
            "message",
            json!({ "fields": {
                "title": title, "channel": "sms", "direction": "in", "from": alex,
                "from_raw": "+61400000001", "conversation": conversation, "sent": sent,
            } }),
        );
    }

    let types = api.types();
    let query = find(&types, "conversation")["templates"][0]["tabs"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|i| {
            let b = &i["block"];
            (b["kind"] == "query_view").then(|| b["fields"]["query"].as_str().unwrap().to_owned())
        })
        .expect("the conversation page has a query view");
    let res = expect(
        api.post(
            "/api/query",
            json!({ "query": query, "this": thread, "page_size": 10 }),
        ),
        StatusCode::OK,
    );
    let titles: Vec<&str> = res["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Running late", "See you soon"], "{res}");
}

/// A message's pictures show on its page and its card: both templates of the message type have an
/// image block showing the `attachments` field, and the page keeps the field's slot, where every
/// attachment, a picture or not, is a file.
#[test]
fn a_message_shows_the_images_in_its_attachments() {
    let api = Api::new(TestServer::start());
    let types = api.types();
    let message = find(&types, "message");
    let items = |template: &str| -> Vec<Value> {
        message["templates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == template)
            .unwrap_or_else(|| panic!("no {template} template: {message}"))["tabs"][0]["items"]
            .as_array()
            .unwrap()
            .clone()
    };
    for template in ["default", "card"] {
        let items = items(template);
        assert!(
            items
                .iter()
                .any(|i| i["block"]["kind"] == "image"
                    && i["block"]["fields"]["field"] == "attachments"),
            "the {template} template shows the attachments' images: {items:?}"
        );
    }
    assert!(
        items("default").iter().any(|i| i["slot"] == "attachments"),
        "the page keeps the attachments slot"
    );
}
