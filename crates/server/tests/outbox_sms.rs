//! S1: an Outbox SMS sent through the phone (outbox X6, X14). Marking the item ready queues
//! `send_sms` for the phone with the item's id and records the command's id; the phone's result
//! for the command records the send with the phone's id of the message, or fails the item with
//! the phone's error; and once the sent copy is linked, by that id or, for an older app that
//! reports none, by number, text and time, the item leaves the Outbox for good. One whose copy
//! never comes is removed 24 hours after the send.

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

    /// Whether note `id` is gone for good: not in the vault and not in the trash.
    fn gone(&self, id: &str) -> bool {
        let status = self
            .req(&self.admin, "GET", &format!("/api/notes/{id}"))
            .send()
            .unwrap()
            .status()
            .as_u16();
        let trash = expect(
            self.req(&self.admin, "GET", "/api/trash").send().unwrap(),
            200,
        );
        let trashed = trash["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == id);
        status == 404 && !trashed
    }

    /// The phone's capture linking `copy` as item `id`'s result, as it syncs to the server: the
    /// item written with the phone's token.
    fn link_as_phone(&self, phone: &Phone, id: &str, copy: &str) {
        let mut note = self.note(id);
        note["note"]["fields"]["result"] = json!(copy);
        expect(
            self.req(&phone.client, "PUT", &format!("/api/notes/{id}"))
                .json(&json!({ "note": note["note"], "base_version": note["version"] }))
                .send()
                .unwrap(),
            200,
        );
    }

    /// Move the test clock on by `by`, an ISO 8601 duration, which runs the scheduler's tick.
    fn advance(&self, by: &str) {
        expect(
            self.req(&self.admin, "POST", "/api/test/clock")
                .json(&json!({ "advance": by }))
                .send()
                .unwrap(),
            200,
        );
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

/// An SMS item the user marked ready, so it sends at once, with `extra` fields.
fn sms(extra: Value, body: &[&str]) -> Value {
    let mut fields = json!({
        "title": "SMS to Ann",
        "kind": "sms",
        "status": "ready",
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
fn a_ready_sms_waits_for_the_phone_and_leaves_the_outbox_once_sent_with_its_copy() {
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
    // Sent, with its copy in the vault: the item is gone for good, the copy a regular message.
    assert!(api.gone(&id));
    assert_eq!(api.fields(&copy)["device_ref"], "sms:4412");
    api.settle();

    // A copy that is not there yet is left to the phone's capture: the item waits, sending.
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
    assert_eq!(f["status"], "sending", "{f}");
    assert_eq!(f["sent_ref"], "sms:4413");
    assert_eq!(f["sent_at"], NOW);
    assert!(f.get("result").is_none_or(Value::is_null), "{f}");
    // The capture links the copy on the phone, which syncs it: the server removes the item.
    let copy = api.copy(&phone, "sms:4413", NUMBER, "Running late", NOW);
    api.link_as_phone(&phone, &later, &copy);
    api.settle();
    assert!(api.gone(&later));
    assert!(api.note(&copy)["note"]["fields"]["title"] == "Running late");
}

#[test]
fn a_sent_sms_whose_copy_never_comes_is_removed_after_a_day() {
    let api = Api::start();
    let phone = api.phone();
    let id = api.create(&api.admin, "outbox", sms(json!({}), &["Bins out"]));
    api.settle();
    let cid = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Another item still waits for the phone: no send recorded, so it is never swept.
    let waiting = api.create(&api.admin, "outbox", sms(json!({}), &["Later"]));
    api.settle();
    api.report(
        &phone,
        &cid,
        json!({ "status": "done", "result": { "device_ref": "sms:9" } }),
    );
    assert_eq!(api.fields(&id)["status"], "sending");

    api.advance("PT23H");
    api.settle();
    assert_eq!(api.fields(&id)["status"], "sending");
    api.advance("PT2H");
    api.settle();
    assert!(api.gone(&id));
    assert_eq!(api.fields(&waiting)["status"], "sending");
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

    // Retry: the user marks it ready again, and a new command goes to the phone.
    api.edit(&id, json!({ "status": "ready" }));
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
    assert_eq!(f["status"], "sending", "{f}");
    assert_eq!(f["sent_ref"], "sms:77");
    assert_eq!(f["sent_at"], NOW);
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
fn an_older_apps_result_without_device_ref_links_the_copy_by_number_text_and_time() {
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
    // The copy found, the item leaves the Outbox.
    assert!(api.gone(&id));
    assert_eq!(api.fields(&copy)["title"], "Bins out tonight");

    // An older app that reports no result at all: sent, with no copy to link yet.
    let other = api.create(&api.admin, "outbox", sms(json!({}), &["No copy of this"]));
    api.settle();
    let cid = api.commands(&phone, "?status=pending")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    api.report(&phone, &cid, json!({ "status": "done" }));
    let f = api.fields(&other);
    assert_eq!(f["status"], "sending", "{f}");
    assert_eq!(f["sent_at"], NOW);
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
    api.edit(&id, json!({ "status": "draft" }));
    api.report(
        &phone,
        &cid,
        json!({ "status": "done", "result": { "device_ref": "sms:5" } }),
    );
    let f = api.fields(&id);
    assert_eq!(f["status"], "draft", "{f}");
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
