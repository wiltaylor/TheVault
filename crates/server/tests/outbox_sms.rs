//! S1: an Outbox SMS sent through the phone (outbox X6). Approving the item queues `send_sms`
//! for the phone with the item's id and records the command's id; the phone's result for the
//! command marks the item sent with the phone's id of the message, or failed with the phone's
//! error; and the sent copy is linked as the item's `result`, by that id or, for an older app
//! that reports none, by number, text and time.

use std::ffi::OsStr;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::{PASSWORD, TestServer};

/// Monday 2026-01-05, half past midnight UTC: the server's clock, which stays there.
const NOW: &str = "2026-01-05T00:30:00Z";

const NUMBER: &str = "+61400000002";

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

struct Api {
    server: TestServer,
    admin: Client,
}

struct Phone {
    id: String,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server =
            TestServer::start_with_args(|_| {}, &[OsStr::new("--test-clock"), OsStr::new(NOW)]);
        let admin = client(server.admin_token());
        Api { server, admin }
    }

    fn req(&self, c: &Client, method: &str, path: &str) -> RequestBuilder {
        c.request(method.parse().unwrap(), self.server.url(path))
    }

    /// Pair an Android phone named `name` with `settings`.
    fn pair(&self, name: &str, settings: Value) -> Phone {
        let paired = expect(
            Client::new()
                .post(self.server.url("/api/devices/pair"))
                .json(&json!({ "password": PASSWORD, "name": name, "platform": "android" }))
                .send()
                .unwrap(),
            201,
        );
        let id = paired["device_id"].as_str().unwrap().to_owned();
        expect(
            self.req(&self.admin, "PATCH", &format!("/api/devices/{id}"))
                .json(&json!({ "settings": settings }))
                .send()
                .unwrap(),
            200,
        );
        Phone {
            id,
            client: client(paired["token"].as_str().unwrap()),
        }
    }

    /// A phone that may send SMS.
    fn phone(&self) -> Phone {
        self.pair(
            "Pixel",
            json!({ "commands": ["send_sms"], "sms_send": true }),
        )
    }

    fn create(&self, c: &Client, type_name: &str, note: Value) -> String {
        let made = expect(
            self.req(c, "POST", &format!("/api/notes?type={type_name}"))
                .json(&json!({ "note": note }))
                .send()
                .unwrap(),
            201,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req(&self.admin, "GET", &format!("/api/notes/{id}"))
                .send()
                .unwrap(),
            200,
        )
    }

    fn fields(&self, id: &str) -> Value {
        self.note(id)["note"]["fields"].clone()
    }

    /// Change fields of note `id` as the user would.
    fn edit(&self, id: &str, changes: Value) {
        let mut note = self.note(id);
        for (k, v) in changes.as_object().unwrap() {
            note["note"]["fields"][k] = v.clone();
        }
        expect(
            self.req(&self.admin, "PUT", &format!("/api/notes/{id}"))
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

    fn commands(&self, phone: &Phone, query: &str) -> Vec<Value> {
        let out = expect(
            self.req(
                &phone.client,
                "GET",
                &format!("/api/devices/{}/commands{query}", phone.id),
            )
            .send()
            .unwrap(),
            200,
        );
        out["commands"].as_array().unwrap().clone()
    }

    /// The phone reporting on command `cid`.
    fn report(&self, phone: &Phone, cid: &str, body: Value) -> Value {
        expect(
            self.req(
                &phone.client,
                "POST",
                &format!("/api/devices/{}/commands/{cid}/result", phone.id),
            )
            .json(&body)
            .send()
            .unwrap(),
            200,
        )
    }

    /// The sent copy the phone's capture makes, as it reaches the server by sync: a `message`
    /// note written with the phone's token.
    fn copy(&self, phone: &Phone, device_ref: &str, to: &str, body: &str, sent: &str) -> String {
        self.create(
            &phone.client,
            "message",
            json!({
                "fields": {
                    "title": body,
                    "channel": "sms",
                    "direction": "out",
                    "to_raw": [to],
                    "sent": sent,
                    "device_ref": device_ref,
                    "partial": false,
                },
                "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [paragraph(body)] }],
            }),
        )
    }

    fn notifications(&self) -> Vec<Value> {
        let out = expect(
            self.req(&self.admin, "GET", "/api/notifications")
                .send()
                .unwrap(),
            200,
        );
        out["items"].as_array().unwrap().clone()
    }
}

fn paragraph(text: &str) -> Value {
    json!({ "kind": "paragraph", "fields": { "text": text } })
}

/// An SMS item the user approved, so it sends at once, with `extra` fields.
fn sms(extra: Value, body: &[&str]) -> Value {
    let mut fields = json!({
        "title": "SMS to Ann",
        "kind": "sms",
        "status": "approved",
        "phone": NUMBER,
    });
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    let blocks: Vec<Value> = body.iter().map(|t| paragraph(t)).collect();
    json!({
        "fields": fields,
        "blocks": [{ "kind": "content", "labels": ["body"], "blocks": blocks }],
    })
}

#[test]
fn an_approved_sms_waits_for_the_phone_and_its_result_marks_it_sent_with_the_copy_linked() {
    let api = Api::start();
    let phone = api.phone();
    let id = api.create(
        &api.admin,
        "outbox",
        sms(json!({}), &["Bins out **tonight**", "Thanks!"]),
    );
    api.settle();

    // The phone has one send_sms, naming the item; the item waits on it.
    let pending = api.commands(&phone, "?status=pending");
    assert_eq!(pending.len(), 1, "{pending:#?}");
    let command = &pending[0];
    let cid = command["id"].as_str().unwrap();
    assert_eq!(command["name"], "send_sms");
    assert_eq!(
        command["payload"],
        json!({ "to": NUMBER, "body": "Bins out tonight\nThanks!", "outbox": id })
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "sending", "{f}");
    assert_eq!(f["sent_ref"], cid);
    assert!(f.get("sent_at").is_none_or(Value::is_null), "{f}");

    // Taken, the item still waits.
    api.report(&phone, cid, json!({ "status": "delivered" }));
    assert_eq!(api.fields(&id)["status"], "sending");

    // The phone's copy of the message reached the server before the result did.
    let copy = api.copy(&phone, "sms:4412", NUMBER, "Bins out tonight\nThanks!", NOW);
    api.report(
        &phone,
        cid,
        json!({ "status": "done", "result": { "device_ref": "sms:4412" } }),
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "sent", "{f}");
    assert_eq!(f["sent_ref"], "sms:4412");
    assert_eq!(f["sent_at"], NOW);
    assert_eq!(f["result"], copy.as_str());
    assert!(f.get("error").is_none_or(Value::is_null), "{f}");
    api.settle();

    // A copy that is not there yet is left to the phone's capture: the item is still sent.
    let later = api.create(&api.admin, "outbox", sms(json!({}), &["Running late"]));
    api.settle();
    let cid = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    api.report(
        &phone,
        &cid,
        json!({ "status": "done", "result": { "device_ref": "sms:4413" } }),
    );
    let f = api.fields(&later);
    assert_eq!(f["status"], "sent", "{f}");
    assert_eq!(f["sent_ref"], "sms:4413");
    assert!(f.get("result").is_none_or(Value::is_null), "{f}");
}

#[test]
fn the_phones_error_fails_the_item_and_a_retry_queues_a_new_command() {
    let api = Api::start();
    let phone = api.phone();
    let id = api.create(&api.admin, "outbox", sms(json!({}), &["Bins out"]));
    api.settle();
    let first = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    api.report(
        &phone,
        &first,
        json!({ "status": "failed", "result": "SmsManager result code 4 (no service)" }),
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "failed", "{f}");
    assert_eq!(
        f["error"],
        "The phone could not send the SMS: SmsManager result code 4 (no service)"
    );
    assert_eq!(f["sent_ref"], first.as_str());
    let bell = api.notifications();
    assert!(
        bell.iter()
            .any(|n| n["title"] == "Could not send SMS to Ann"),
        "{bell:#?}"
    );

    // Retry: the user approves it again, and a new command goes to the phone.
    api.edit(&id, json!({ "status": "approved" }));
    api.settle();
    let pending = api.commands(&phone, "?status=pending");
    assert_eq!(pending.len(), 1, "{pending:#?}");
    let second = pending[0]["id"].as_str().unwrap();
    assert_ne!(second, first);
    let f = api.fields(&id);
    assert_eq!(f["status"], "sending", "{f}");
    assert_eq!(f["sent_ref"], second);
    api.report(
        &phone,
        second,
        json!({ "status": "done", "result": { "device_ref": "sms:77" } }),
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "sent", "{f}");
    assert_eq!(f["sent_ref"], "sms:77");
    assert!(f.get("error").is_none_or(Value::is_null), "{f}");
}

#[test]
fn without_sms_sending_on_the_item_fails_with_the_setting_to_turn_on() {
    let api = Api::start();

    // No phone has SMS sending on.
    let off = api.pair("Pixel", json!({ "commands": ["send_sms"] }));
    let id = api.create(&api.admin, "outbox", sms(json!({}), &["Bins out"]));
    api.settle();
    let f = api.fields(&id);
    assert_eq!(f["status"], "failed", "{f}");
    assert_eq!(
        f["error"],
        "Turn on SMS sending for a phone in Settings > Devices."
    );

    // The item names the phone whose switch is off.
    let id = api.create(
        &api.admin,
        "outbox",
        sms(json!({ "device": off.id }), &["Bins out"]),
    );
    api.settle();
    let f = api.fields(&id);
    assert_eq!(f["status"], "failed", "{f}");
    assert_eq!(
        f["error"],
        "Turn on SMS sending for Pixel in Settings > Devices."
    );
    assert!(api.commands(&off, "").is_empty());

    // With two phones sending, an item naming neither cannot choose; naming one sends from it.
    let a = api.pair(
        "Pixel 8",
        json!({ "commands": ["send_sms"], "sms_send": true }),
    );
    let b = api.pair(
        "Pixel 9",
        json!({ "commands": ["send_sms"], "sms_send": true }),
    );
    let id = api.create(&api.admin, "outbox", sms(json!({}), &["Bins out"]));
    api.settle();
    let f = api.fields(&id);
    assert_eq!(f["status"], "failed", "{f}");
    assert!(
        f["error"]
            .as_str()
            .unwrap()
            .contains("2 phones have SMS sending on"),
        "{f}"
    );
    let id = api.create(
        &api.admin,
        "outbox",
        sms(json!({ "device": b.id }), &["Bins out"]),
    );
    api.settle();
    assert_eq!(api.fields(&id)["status"], "sending");
    assert!(api.commands(&a, "").is_empty());
    assert_eq!(api.commands(&b, "?status=pending").len(), 1);
}

#[test]
fn an_older_apps_result_without_device_ref_marks_it_sent_and_links_the_copy_by_number_text_and_time()
 {
    let api = Api::start();
    let phone = api.phone();
    let id = api.create(
        &api.admin,
        "outbox",
        sms(json!({ "phone": "0400 000 002" }), &["Bins out, *tonight*"]),
    );
    api.settle();
    let cid = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Three outgoing SMS: another text to the number, the same text twenty minutes before, and
    // the copy.
    api.copy(
        &phone,
        "sms:10",
        NUMBER,
        "Something else",
        "2026-01-05T00:29:00Z",
    );
    api.copy(
        &phone,
        "sms:11",
        NUMBER,
        "Bins out, tonight",
        "2026-01-05T00:10:00Z",
    );
    api.copy(
        &phone,
        "sms:12",
        "+61400000009",
        "Bins out, tonight",
        "2026-01-05T00:29:30Z",
    );
    let copy = api.copy(
        &phone,
        "sms:13",
        NUMBER,
        "Bins out tonight",
        "2026-01-05T11:29:40+11:00",
    );

    api.report(
        &phone,
        &cid,
        json!({ "status": "done", "result": { "device_ref": null } }),
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "sent", "{f}");
    assert_eq!(f["sent_ref"], cid.as_str());
    assert_eq!(f["sent_at"], NOW);
    assert_eq!(f["result"], copy.as_str());

    // An older app that reports no result at all: sent, with no copy to link.
    let other = api.create(&api.admin, "outbox", sms(json!({}), &["No copy of this"]));
    api.settle();
    let cid = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    api.report(&phone, &cid, json!({ "status": "done" }));
    let f = api.fields(&other);
    assert_eq!(f["status"], "sent", "{f}");
    assert!(f.get("result").is_none_or(Value::is_null), "{f}");
}

#[test]
fn a_result_for_an_item_that_moved_on_is_left_alone() {
    let api = Api::start();
    let phone = api.phone();
    let id = api.create(&api.admin, "outbox", sms(json!({}), &["Bins out"]));
    api.settle();
    let cid = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The user gave up on it before the phone answered.
    api.edit(&id, json!({ "status": "rejected" }));
    api.report(
        &phone,
        &cid,
        json!({ "status": "done", "result": { "device_ref": "sms:5" } }),
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "rejected", "{f}");
    assert_eq!(f["sent_ref"], cid.as_str());

    // A send_sms a script queued, naming no item, changes no note.
    let other = api.create(
        &api.admin,
        "outbox",
        sms(json!({ "status": "draft" }), &["Draft"]),
    );
    api.settle();
    assert!(api.commands(&phone, "?status=pending").is_empty());
    assert_eq!(api.fields(&other)["status"], "draft");
}
