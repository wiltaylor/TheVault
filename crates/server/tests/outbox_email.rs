//! S1 with the fake Google server: the shipped `outbox` type's `outbox_send` handler sending a
//! ready email item through Gmail, and the sent copy coming back as a regular `email` note, after
//! which the item leaves the Outbox for good (outbox X2, X3, X4, X14).

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::fake_google::{
    DEFAULT_ACCOUNT, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN, SendRoute,
    SentMessage,
};
use testkit::{FakeGoogle, TestServer};

const SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";
const RECONNECT: &str = "Reconnect Google in Settings > Integrations to send email.";

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

struct Api {
    server: TestServer,
    client: Client,
    google: FakeGoogle,
}

impl Api {
    /// A server whose `outbox` handlers and `google_gmail` task reach `google`, with the Google
    /// secrets set and `google_settings` holding `settings` and the fake's endpoints.
    fn start_with(settings: Value) -> Api {
        let google = FakeGoogle::start();
        let server = TestServer::start();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap();
        let api = Api {
            server,
            client,
            google,
        };
        let mut settings = settings;
        settings["endpoints"] = api.google.endpoints();
        for (name, value) in [
            ("google_client_id", DEFAULT_CLIENT_ID.to_owned()),
            ("google_client_secret", DEFAULT_CLIENT_SECRET.to_owned()),
            ("google_refresh_token", DEFAULT_REFRESH_TOKEN.to_owned()),
            ("google_account", DEFAULT_ACCOUNT.to_owned()),
            ("google_settings", settings.to_string()),
        ] {
            api.put_secret(name, &value);
        }
        // The shipped type and task, with the fake's host allowed as the Google tests do.
        let allow = format!("allow_domains = [\"{}\", ", api.google.host());
        let outbox = api.file("types/outbox.wcl");
        assert_eq!(outbox.matches("allow_domains = [").count(), 3);
        expect(
            api.req("PUT", "/api/types/outbox")
                .body(outbox.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        let task = api.file("tasks/google_gmail.wcl");
        expect(
            api.req("PUT", "/api/tasks/google_gmail")
                .body(task.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        api
    }

    /// Connected with the scope to send.
    fn start() -> Api {
        Api::start_with(json!({ "scopes": [SEND_SCOPE] }))
    }

    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(path)).unwrap()
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
    }

    fn put_secret(&self, name: &str, value: &str) {
        expect(
            self.req("PUT", &format!("/api/secrets/{name}"))
                .json(&json!({ "value": value }))
                .send()
                .unwrap(),
            200,
        );
    }

    fn upload(&self, name: &str, bytes: Vec<u8>) -> String {
        let made = expect(
            self.req("POST", &format!("/api/assets?filename={name}"))
                .body(bytes)
                .send()
                .unwrap(),
            201,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn create(&self, type_name: &str, note: Value) -> String {
        let made = expect(
            self.req("POST", &format!("/api/notes?type={type_name}"))
                .json(&json!({ "note": note }))
                .send()
                .unwrap(),
            201,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notes/{id}")).send().unwrap(),
            200,
        )
    }

    fn fields(&self, id: &str) -> Value {
        self.note(id)["note"]["fields"].clone()
    }

    /// Whether note `id` is gone for good: not in the vault and not in the trash.
    fn gone(&self, id: &str) -> bool {
        let status = self
            .req("GET", &format!("/api/notes/{id}"))
            .send()
            .unwrap()
            .status()
            .as_u16();
        let trash = expect(self.req("GET", "/api/trash").send().unwrap(), 200);
        let trashed = trash["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == id);
        status == 404 && !trashed
    }

    /// The Outbox activity entries of item `id`, oldest first, as their kinds.
    fn outbox_activity(&self, id: &str) -> Vec<String> {
        let page = expect(
            self.req("GET", "/api/activity?category=outbox&limit=200")
                .send()
                .unwrap(),
            200,
        );
        let mut kinds: Vec<String> = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["subject"] == id)
            .map(|e| e["kind"].as_str().unwrap().to_owned())
            .collect();
        kinds.reverse();
        kinds
    }

    /// Change fields of note `id` as the user would.
    fn edit(&self, id: &str, changes: Value) {
        let mut note = self.note(id);
        for (k, v) in changes.as_object().unwrap() {
            note["note"]["fields"][k] = v.clone();
        }
        expect(
            self.req("PUT", &format!("/api/notes/{id}"))
                .json(&json!({ "note": note["note"], "base_version": note["version"] }))
                .send()
                .unwrap(),
            200,
        );
    }

    /// Let every queued handler run finish, and fail on any that failed.
    fn settle(&self) {
        let failed = self.server.settle_handlers();
        assert!(failed.is_empty(), "{failed:#?}");
    }

    fn query(&self, text: &str) -> Vec<Value> {
        let out = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": text, "page_size": 1000 }))
                .send()
                .unwrap(),
            200,
        );
        out["rows"].as_array().unwrap().clone()
    }

    /// The email notes of Gmail message `gmail_id`.
    fn copies(&self, gmail_id: &str) -> Vec<Value> {
        self.query(&format!(
            "from email include archived where gmail_ref == \"{gmail_id}\""
        ))
    }

    fn run_gmail(&self) {
        let run = expect(
            self.req("POST", "/api/tasks/google_gmail/run?wait=true")
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run:#}");
        self.settle();
    }

    /// The one message Gmail was asked to send.
    fn sent(&self) -> SentMessage {
        let sent = self.google.sent_messages();
        assert_eq!(sent.len(), 1, "{sent:#?}");
        sent.into_iter().next().unwrap()
    }
}

fn paragraph(text: &str) -> Value {
    json!({ "kind": "paragraph", "fields": { "text": text } })
}

/// An email item, marked ready by the user so it sends at once, with `extra` fields.
fn email(extra: Value, body: Vec<Value>) -> Value {
    let mut fields = json!({
        "title": "Lunch",
        "kind": "email",
        "status": "ready",
        "to": ["Ann Example <ann@example.com>"],
        "subject": "Lunch on Friday?",
    });
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    json!({
        "fields": fields,
        "blocks": [{ "kind": "content", "labels": ["body"], "blocks": body }],
    })
}

fn bytes(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31) ^ seed).collect()
}

#[test]
fn a_ready_email_goes_out_as_rfc822_and_leaves_the_outbox_once_its_copy_comes_back() {
    let api = Api::start();
    let pdf = bytes(3000, 7);
    let asset = api.upload("menu.pdf", pdf.clone());
    let id = api.create(
        "outbox",
        email(
            json!({
                "cc": ["bob@example.com"],
                "bcc": ["carol@example.com", "dan@example.com"],
                "attachments": [asset],
            }),
            vec![
                paragraph("Are you free for **lunch** on Friday?"),
                json!({
                    "kind": "list",
                    "fields": { "style": "bullet" },
                    "blocks": [
                        { "kind": "item", "fields": { "text": "The usual place" } },
                        { "kind": "item", "fields": { "text": "At noon" } },
                    ],
                }),
            ],
        ),
    );
    api.settle();

    let sent = api.sent();
    assert_eq!(sent.route, SendRoute::Raw);
    assert_eq!(sent.thread_id, None);
    assert_eq!(sent.header("From"), Some(DEFAULT_ACCOUNT));
    assert_eq!(sent.header("To"), Some("Ann Example <ann@example.com>"));
    assert_eq!(sent.header("Cc"), Some("bob@example.com"));
    assert_eq!(
        sent.header("Bcc"),
        Some("carol@example.com, dan@example.com")
    );
    assert_eq!(sent.header("Subject"), Some("Lunch on Friday?"));
    assert_eq!(sent.header("MIME-Version"), Some("1.0"));
    assert!(
        sent.header("Content-Type")
            .unwrap()
            .starts_with("multipart/mixed"),
        "{}",
        sent.raw
    );
    assert!(sent.header("In-Reply-To").is_none());
    let text = sent.text.clone().unwrap();
    assert!(text.contains("Are you free for lunch on Friday?"), "{text}");
    assert!(text.contains("- The usual place\r\n- At noon"), "{text}");
    let html = sent.html.clone().unwrap();
    assert!(
        html.contains("<p>Are you free for <strong>lunch</strong> on Friday?</p>"),
        "{html}"
    );
    assert!(html.contains("<li>At noon</li>"), "{html}");
    assert_eq!(
        sent.attachments,
        [("menu.pdf".to_owned(), "application/pdf".to_owned(), pdf)]
    );
    // Every line of the message is short, as MIME needs.
    assert!(sent.raw.split("\r\n").all(|l| l.len() <= 998));

    // The copy is imported at once, and the item, sent, leaves the Outbox for good: the copy is
    // a regular email note.
    let copies = api.copies(&sent.id);
    assert_eq!(copies.len(), 1, "{copies:#?}");
    let copy = api.fields(copies[0]["id"].as_str().unwrap());
    assert_eq!(copy["title"], "Lunch on Friday?");
    assert_eq!(copy["from_raw"], DEFAULT_ACCOUNT);
    assert_eq!(copy["labels"], json!(["system/SENT"]));
    assert!(api.gone(&id));
    assert!(api.query("from outbox include archived").is_empty());
    // The log keeps the send.
    assert_eq!(
        api.outbox_activity(&id),
        ["outbox.ready", "outbox.sending", "outbox.sent"]
    );

    // The Gmail task, whose labels do not include SENT, finds nothing more to import.
    api.run_gmail();
    assert_eq!(api.copies(&sent.id).len(), 1);
    assert_eq!(api.google.sent_messages().len(), 1);
}

#[test]
fn a_reply_carries_the_threading_headers_and_thread() {
    let api = Api::start();
    let original = api.google.add_message(json!({
        "thread": "t-lunch",
        "labels": ["INBOX"],
        "from": "Ann Example <ann@example.com>",
        "to": [DEFAULT_ACCOUNT],
        "subject": "Lunch?",
        "text": "Shall we?",
        "message_id": "<lunch-2@mail.example.com>",
        "references": "<lunch-1@mail.example.com>",
    }));
    let replied = api.create(
        "email",
        json!({ "fields": { "title": "Lunch?", "gmail_ref": original } }),
    );
    api.create(
        "outbox",
        email(
            json!({ "subject": "Re: Lunch?", "in_reply_to": replied }),
            vec![paragraph("Yes, Friday.")],
        ),
    );
    api.settle();

    let sent = api.sent();
    assert_eq!(
        sent.header("In-Reply-To"),
        Some("<lunch-2@mail.example.com>")
    );
    assert_eq!(
        sent.header("References"),
        Some("<lunch-1@mail.example.com> <lunch-2@mail.example.com>")
    );
    assert_eq!(sent.thread_id.as_deref(), Some("t-lunch"));
    // A message with no attachments is the two bodies alone.
    assert!(
        sent.header("Content-Type")
            .unwrap()
            .starts_with("multipart/alternative"),
        "{}",
        sent.raw
    );
}

#[test]
fn a_table_goes_as_a_markdown_table_in_the_text_part_and_a_table_in_the_html() {
    let api = Api::start();
    api.create(
        "outbox",
        email(
            json!({}),
            vec![json!({
                "kind": "table",
                "fields": { "header": true, "align": ["left", "right"] },
                "blocks": [
                    { "kind": "row", "fields": { "cells": ["Dish", "Price"] } },
                    { "kind": "row", "fields": { "cells": ["**Soup** | bread", "9"] } },
                    { "kind": "row", "fields": { "cells": ["Tea"] } },
                ],
            })],
        ),
    );
    api.settle();

    let sent = api.sent();
    let text = sent.text.clone().unwrap();
    assert!(
        text.contains("| Dish | Price |\r\n| --- | ---: |\r\n| Soup \\| bread | 9 |\r\n| Tea |  |"),
        "{text}"
    );
    let html = sent.html.clone().unwrap();
    assert!(
        html.contains(
            "<table><tr><th>Dish</th><th style=\"text-align:right\">Price</th></tr><tr><td><strong>Soup</strong> | bread</td><td style=\"text-align:right\">9</td></tr>"
        ),
        "{html}"
    );
}

#[test]
fn headers_that_are_not_ascii_go_as_encoded_words() {
    let api = Api::start();
    let id = api.create(
        "outbox",
        email(
            json!({ "to": ["Zoë Ñandú <zoe@example.com>"], "subject": "Café at 1?" }),
            vec![paragraph("Ça va ?")],
        ),
    );
    api.settle();
    let sent = api.sent();
    let subject = sent.header("Subject").unwrap();
    assert!(subject.starts_with("=?UTF-8?B?"), "{subject}");
    assert!(
        sent.header("To").unwrap().ends_with(" <zoe@example.com>"),
        "{}",
        sent.raw
    );
    assert!(sent.raw.is_ascii());
    assert_eq!(sent.text.as_deref(), Some("Ça va ?\r\n"));
    // The copy decodes them again.
    let copies = api.copies(&sent.id);
    assert_eq!(
        api.fields(copies[0]["id"].as_str().unwrap())["title"],
        "Café at 1?"
    );
    assert!(api.gone(&id));
}

#[test]
fn a_message_over_4_5_mb_goes_through_the_upload_endpoint() {
    let api = Api::start();
    let big = bytes(5 * 1024 * 1024, 3);
    let asset = api.upload("scan.png", big.clone());
    let id = api.create(
        "outbox",
        email(
            json!({ "attachments": [asset] }),
            vec![paragraph("The scan.")],
        ),
    );
    api.settle();

    let sent = api.sent();
    assert_eq!(sent.route, SendRoute::Upload);
    let uploads = api
        .google
        .requests_to("/upload/gmail/v1/users/me/messages/send");
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].query("uploadType").as_deref(), Some("media"));
    assert_eq!(uploads[0].header("content-type"), Some("message/rfc822"));
    assert!(
        api.google
            .requests_to("/gmail/v1/users/me/messages/send")
            .is_empty()
    );
    assert_eq!(
        sent.attachments,
        [("scan.png".to_owned(), "image/png".to_owned(), big)]
    );
    assert!(api.gone(&id));
}

#[test]
fn attachments_over_25_mb_are_refused_before_sending() {
    let api = Api::start();
    let a = api.upload("one.bin", bytes(13 * 1024 * 1024, 1));
    let b = api.upload("two.bin", bytes(13 * 1024 * 1024, 2));
    let id = api.create(
        "outbox",
        email(json!({ "attachments": [a, b] }), vec![paragraph("Both.")]),
    );
    api.settle();

    let f = api.fields(&id);
    assert_eq!(f["status"], "failed", "{f}");
    let error = f["error"].as_str().unwrap();
    assert!(error.contains("over the 25 MB Gmail sends"), "{error}");
    assert!(api.google.sent_messages().is_empty());

    // The user removes one and retries: failed -> ready.
    api.edit(&id, json!({ "attachments": [a], "status": "ready" }));
    api.settle();
    assert!(api.gone(&id));
    assert_eq!(api.sent().attachments.len(), 1);
}

#[test]
fn without_the_gmail_send_scope_the_item_fails_with_the_reconnect_message() {
    let api = Api::start_with(json!({}));
    let status = expect(
        api.req("GET", "/api/integrations/google").send().unwrap(),
        200,
    );
    assert_eq!(status["can_send"], false, "{status}");
    let id = api.create("outbox", email(json!({}), vec![paragraph("Hi.")]));
    api.settle();
    let f = api.fields(&id);
    assert_eq!(f["status"], "failed");
    assert_eq!(f["error"], RECONNECT);
    assert!(api.google.sent_messages().is_empty());
    assert!(api.google.requests_to("/token").is_empty());

    api.put_secret(
        "google_settings",
        &json!({ "scopes": [SEND_SCOPE], "endpoints": api.google.endpoints() }).to_string(),
    );
    let status = expect(
        api.req("GET", "/api/integrations/google").send().unwrap(),
        200,
    );
    assert_eq!(status["can_send"], true, "{status}");
}

#[test]
fn a_gmail_error_fails_the_item_with_its_message() {
    let api = Api::start();
    api.google.set_send_error(Some((500, "Backend Error")));
    let id = api.create("outbox", email(json!({}), vec![paragraph("Hi.")]));
    api.settle();
    let f = api.fields(&id);
    assert_eq!(f["status"], "failed");
    assert_eq!(
        f["error"],
        "Gmail answered 500 to messages.send: Backend Error"
    );
    assert!(f.get("sent_ref").is_none_or(Value::is_null), "{f}");

    // A failed item stays in the Outbox, with its error, until the user retries it.
    assert_eq!(api.outbox_activity(&id).last().unwrap(), "outbox.failed");
    api.google.set_send_error(None);
    api.edit(&id, json!({ "status": "ready" }));
    api.settle();
    assert!(api.gone(&id));
}

#[test]
fn an_item_without_recipients_fails_and_a_draft_is_not_sent() {
    let api = Api::start();
    let draft = api.create(
        "outbox",
        email(json!({ "status": "draft" }), vec![paragraph("Hi.")]),
    );
    api.edit(&draft, json!({ "subject": "Lunch, Friday?" }));
    api.settle();
    assert!(api.google.sent_messages().is_empty());
    assert_eq!(api.fields(&draft)["status"], "draft");

    let none = api.create("outbox", email(json!({ "to": [] }), vec![paragraph("Hi.")]));
    api.settle();
    let f = api.fields(&none);
    assert_eq!(f["status"], "failed");
    assert!(
        f["error"].as_str().unwrap().contains("no recipients"),
        "{f}"
    );
}

#[test]
fn the_gmail_task_imports_a_sent_copy_once_whatever_the_label_settings() {
    let api = Api::start_with(json!({ "scopes": [SEND_SCOPE], "gmail_labels": ["INBOX"] }));
    let copy = api.google.add_message(json!({
        "labels": ["SENT"],
        "at": 1_790_000_000_000_i64,
        "from": DEFAULT_ACCOUNT,
        "to": ["ann@example.com"],
        "subject": "Sent earlier",
        "text": "Sent while the copy could not be fetched.",
    }));
    // An item sent before its copy arrived: the user records the send, which runs no copy.
    let id = api.create(
        "outbox",
        json!({ "fields": {
            "title": "Sent earlier", "kind": "email", "status": "sending", "sent_ref": copy,
            "sent_at": "2026-10-23T10:00:00Z", "to": ["ann@example.com"],
        } }),
    );
    api.settle();
    assert!(api.copies(&copy).is_empty());
    assert_eq!(api.fields(&id)["status"], "sending");

    api.run_gmail();
    let copies = api.copies(&copy);
    assert_eq!(copies.len(), 1, "{copies:#?}");
    // Linked, the item leaves the Outbox.
    assert!(api.gone(&id));
    api.run_gmail();
    assert_eq!(api.copies(&copy).len(), 1);
    // The label settings are untouched: INBOX mail only.
    assert!(api.google.sent_messages().is_empty());
}
