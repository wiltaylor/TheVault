//! S1: the Google sync's built-in types `email`, `email_thread` and `event`, and the
//! `gmail_label` taxonomy (google-sync D4, D14, D15, D17). Its settings are secrets, with no
//! note type (google-sync D21).

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};
use testkit::TestServer;

const GOOGLE_TYPES: [&str; 3] = ["email", "email_thread", "event"];

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn new() -> Self {
        Api {
            server: TestServer::start(),
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

    fn types(&self) -> Vec<Value> {
        let body = expect(self.send("GET", "/api/types"), StatusCode::OK);
        body["types"].as_array().unwrap().clone()
    }

    fn create(&self, type_name: &str, fields: Value) -> String {
        let made = expect(
            self.post(
                &format!("/api/notes?type={type_name}"),
                json!({ "note": { "fields": fields } }),
            ),
            StatusCode::CREATED,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn create_note(&self, type_name: &str, note: Value) -> String {
        let made = expect(
            self.post(
                &format!("/api/notes?type={type_name}"),
                json!({ "note": note }),
            ),
            StatusCode::CREATED,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    /// The fields of note `id`'s dict `name`, `null` when it has none.
    fn dict(&self, id: &str, name: &str) -> Value {
        let note = expect(
            self.send("GET", &format!("/api/notes/{id}")),
            StatusCode::OK,
        );
        note["note"]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["kind"] == "dict" && b["labels"] == json!([name]))
            .map_or(Value::Null, |b| b["fields"].clone())
    }

    fn fields(&self, id: &str) -> Value {
        expect(
            self.send("GET", &format!("/api/notes/{id}")),
            StatusCode::OK,
        )["note"]["fields"]
            .clone()
    }
}

/// A dict field `name` holding `fields`, as a note block.
fn dict(name: &str, fields: Value) -> Value {
    json!({ "kind": "dict", "labels": [name], "fields": fields })
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

/// Every slot of every tab of template `name` in order, those inside layout blocks such as
/// `columns` included, with `<kind>` for each block a tab holds directly.
fn template_items(ty: &Value, name: &str) -> Vec<String> {
    // A slot inside a block is a block of kind `slot` labelled with the field.
    fn walk(v: &Value, out: &mut Vec<String>) {
        if v["kind"] == "slot" {
            if let Some(field) = v["labels"][0].as_str() {
                out.push(field.to_owned());
            }
            return;
        }
        for b in v["blocks"].as_array().into_iter().flatten() {
            walk(b, out);
        }
    }
    let template = ty["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("{} has no {name} template", ty["name"]));
    let mut out = Vec::new();
    for tab in template["tabs"].as_array().unwrap() {
        for item in tab["items"].as_array().unwrap() {
            if let Some(slot) = item["slot"].as_str() {
                out.push(slot.to_owned());
            } else if let Some(kind) = item["block"]["kind"].as_str() {
                if kind != "columns" {
                    out.push(format!("<{kind}>"));
                }
                walk(&item["block"], &mut out);
            }
        }
    }
    out
}

#[test]
fn a_new_vault_has_the_google_types_with_the_fields_the_spec_lists() {
    let api = Api::new();
    let types = api.types();
    for name in GOOGLE_TYPES {
        assert_eq!(find(&types, name)["builtin"], true, "{name}");
    }
    assert!(
        types.iter().all(|t| t["name"] != "google_sync"),
        "a new vault has no google_sync type"
    );

    let email = find(&types, "email");
    assert_eq!(
        field_names(email),
        [
            "from",
            "from_raw",
            "to",
            "to_raw",
            "cc",
            "cc_raw",
            "sent",
            "thread",
            "labels",
            "snippet",
            "body",
            "html",
            "attachments",
            "processed",
            "gmail_ref"
        ]
    );
    for (name, target, multiple) in [
        ("from", "person", false),
        ("to", "person", true),
        ("cc", "person", true),
        ("thread", "email_thread", false),
    ] {
        assert_eq!(field(email, name)["kind"], "reference", "{name}");
        assert_eq!(field(email, name)["target"], target, "{name}");
        assert_eq!(field(email, name)["multiple"], multiple, "{name}");
    }
    assert_eq!(field(email, "from_raw")["kind"], "string");
    for name in ["to_raw", "cc_raw"] {
        assert_eq!(field(email, name)["kind"], "list");
        assert_eq!(field(email, name)["items"], "string");
    }
    assert_eq!(field(email, "sent")["kind"], "datetime");
    assert_eq!(field(email, "labels")["kind"], "taxonomy");
    assert_eq!(field(email, "labels")["taxonomy"], "gmail_label");
    assert_eq!(field(email, "labels")["multiple"], true);
    assert_eq!(field(email, "snippet")["kind"], "string");
    assert_eq!(field(email, "body")["kind"], "content");
    assert_eq!(field(email, "html")["kind"], "asset");
    assert_eq!(field(email, "html")["multiple"], false);
    assert_eq!(field(email, "attachments")["kind"], "asset");
    assert_eq!(field(email, "attachments")["multiple"], true);
    assert_eq!(field(email, "gmail_ref")["kind"], "string");
    assert_eq!(field(email, "gmail_ref")["hidden"], true);
    assert_eq!(field(email, "processed")["kind"], "bool");
    assert_eq!(field(email, "processed")["label"], "Processed");
    assert_eq!(field(email, "processed")["default"], false);
    assert_eq!(field(email, "processed")["required"], false);
    assert_eq!(field(email, "processed")["computed"], false);

    let thread = find(&types, "email_thread");
    assert_eq!(field_names(thread), ["participants", "thread_ref"]);
    assert_eq!(field(thread, "participants")["target"], "person");
    assert_eq!(field(thread, "participants")["multiple"], true);
    assert_eq!(field(thread, "thread_ref")["hidden"], true);

    let event = find(&types, "event");
    assert_eq!(
        field_names(event),
        [
            "when",
            "calendar",
            "calendar_name",
            "location",
            "description",
            "attendees",
            "attendees_raw",
            "organiser",
            "status",
            "meet_url",
            "series",
            "google_ref",
            "etag",
            "snapshot",
            "send_updates"
        ]
    );
    assert_eq!(field(event, "when")["kind"], "date_range");
    assert_eq!(field(event, "when")["of"], "date_or_datetime");
    assert_eq!(field(event, "calendar")["kind"], "string");
    assert_eq!(field(event, "calendar_name")["kind"], "string");
    assert_eq!(field(event, "calendar_name")["computed"], true);
    assert_eq!(field(event, "location")["kind"], "string");
    assert_eq!(field(event, "description")["kind"], "content");
    assert_eq!(field(event, "attendees")["target"], "person");
    assert_eq!(field(event, "attendees")["multiple"], true);
    assert_eq!(field(event, "attendees_raw")["items"], "string");
    assert_eq!(field(event, "organiser")["kind"], "string");
    assert_eq!(
        field(event, "status")["options"],
        json!(["confirmed", "tentative", "cancelled"])
    );
    assert_eq!(field(event, "meet_url")["kind"], "url");
    assert_eq!(field(event, "series")["kind"], "string");
    assert_eq!(field(event, "series")["computed"], true);
    for name in ["google_ref", "etag", "snapshot", "send_updates"] {
        assert_eq!(field(event, name)["hidden"], true, "{name}");
    }
    assert_eq!(field(event, "snapshot")["kind"], "dict");
    assert_eq!(field(event, "send_updates")["kind"], "string");
    // The write-back ticket declares the handler; this one ships none.
    assert!(
        event["handlers"].as_array().is_none_or(Vec::is_empty),
        "{event}"
    );
}

#[test]
fn the_google_templates_resolve_with_their_slots() {
    let api = Api::new();
    let types = api.types();
    let email = find(&types, "email");
    let page = template_items(email, "default");
    // Processed sits beside the sender.
    let from = page.iter().position(|s| s == "from").unwrap();
    assert_eq!(page[from + 1], "processed", "{page:?}");
    for slot in [
        "from",
        "to",
        "cc",
        "sent",
        "thread",
        "labels",
        "body",
        "attachments",
    ] {
        assert!(
            page.contains(&slot.to_owned()),
            "email page lacks {slot}: {page:?}"
        );
    }
    assert_eq!(
        template_items(email, "card"),
        ["title", "from_raw", "processed", "sent", "snippet"]
    );
    let thread = find(&types, "email_thread");
    assert_eq!(
        template_items(thread, "default"),
        ["participants", "<query_view>"]
    );
    let event = find(&types, "event");
    let page = template_items(event, "default");
    for slot in [
        "when",
        "series",
        "calendar",
        "status",
        "attendees",
        "description",
    ] {
        assert!(
            page.contains(&slot.to_owned()),
            "event page lacks {slot}: {page:?}"
        );
    }
    assert_eq!(template_items(event, "card"), ["title", "when", "location"]);
}

#[test]
fn a_thread_page_queries_its_emails_oldest_first() {
    let api = Api::new();
    let alex = api.create(
        "person",
        json!({ "title": "Alex", "emails": ["alex@example.com"] }),
    );
    let thread = api.create(
        "email_thread",
        json!({ "title": "Lunch", "participants": [alex], "thread_ref": "t1" }),
    );
    let other = api.create(
        "email_thread",
        json!({ "title": "Other", "thread_ref": "t2" }),
    );
    for (title, sent, t) in [
        ("Re: Lunch", "2026-10-15T09:05:00Z", &thread),
        ("Lunch", "2026-10-15T09:00:00Z", &thread),
        ("Unrelated", "2026-10-15T09:02:00Z", &other),
    ] {
        api.create(
            "email",
            json!({
                "title": title, "from": alex, "from_raw": "Alex <alex@example.com>",
                "thread": t, "sent": sent, "labels": ["system/INBOX"],
            }),
        );
    }
    let types = api.types();
    let query = find(&types, "email_thread")["templates"][0]["tabs"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|i| {
            let b = &i["block"];
            (b["kind"] == "query_view").then(|| b["fields"]["query"].as_str().unwrap().to_owned())
        })
        .expect("the thread page has a query view");
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
    assert_eq!(titles, ["Lunch", "Re: Lunch"], "{res}");
}

#[test]
fn the_types_take_what_the_syncs_will_write() {
    let api = Api::new();
    // An email's Gmail labels: a system one, and a new nested one under a new parent.
    let email = api.create(
        "email",
        json!({ "title": "Receipt", "labels": ["system/STARRED", "Shopping"], "gmail_ref": "m1" }),
    );
    api.create(
        "email",
        json!({ "title": "Order", "labels": ["Shopping/Orders"] }),
    );
    let fields = api.fields(&email);
    assert_eq!(
        fields["labels"],
        json!(["system/STARRED", "Shopping"]),
        "{fields}"
    );
    let taxonomy =
        std::fs::read_to_string(api.server.vault_path().join("taxonomies/gmail_label.wcl"))
            .unwrap();
    assert!(taxonomy.contains("term \"Orders\""), "{taxonomy}");

    // An all-day event holds dates and a timed one datetimes.
    let day = api.create(
        "event",
        json!({
            "title": "Holiday", "calendar": "primary", "status": "confirmed",
            "when": { "start": "2026-10-20", "end": "2026-10-21" },
        }),
    );
    let timed = api.create_note(
        "event",
        json!({
            "fields": {
                "title": "Standup", "meet_url": "https://meet.google.com/abc-defg-hij",
                "when": { "start": "2026-10-20T09:30:00+11:00", "end": "2026-10-20T10:00:00+11:00" },
                "google_ref": "e1", "etag": "\"3\"",
            },
            "blocks": [dict("snapshot", json!({ "title": "Standup" }))],
        }),
    );
    assert_eq!(api.fields(&day)["when"]["start"], "2026-10-20");
    assert_eq!(api.dict(&timed, "snapshot")["title"], "Standup");
    // A client cannot write the computed series.
    let res = api.post(
        "/api/notes?type=event",
        json!({ "note": { "fields": { "title": "Weekly", "series": "RRULE:FREQ=WEEKLY" } } }),
    );
    assert!(res.status().is_client_error(), "{}", res.text().unwrap());
}

#[test]
fn the_google_types_cannot_be_deleted() {
    let api = Api::new();
    for name in GOOGLE_TYPES {
        let body = expect(
            api.send("DELETE", &format!("/api/types/{name}")),
            StatusCode::CONFLICT,
        );
        assert_eq!(body["error"], "built_in_type", "{name}");
    }
}
