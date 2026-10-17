//! S4: the default device scripts run against a fake phone and canned device events (D5, D6, D15,
//! D16, D23, D24, D25).

use std::sync::{Arc, Mutex};

use embedded::device::{DeviceConfig, DeviceData, DeviceRuntime, EventStatus};
use embedded::{Config, Embedded};
use script::Vault;
use serde_json::{Value as Json, json};
use state::Author;
use tempfile::TempDir;
use writer::{Input, Writer};

const DEVICE: &str = "01PHONEDEVICE0000000000000";

/// The key of the device's link in a person's `device_contacts`.
const SLOT: &str = "device_01PHONEDEVICE0000000000000";

/// A phone with a contact list, that records every change and message it is asked for.
#[derive(Default)]
struct Phone {
    contacts: Mutex<Vec<Json>>,
    actions: Mutex<Vec<String>>,
}

impl Phone {
    fn actions(&self) -> Vec<String> {
        self.actions.lock().unwrap().clone()
    }

    fn act(&self, what: String) {
        self.actions.lock().unwrap().push(what);
    }
}

impl DeviceData for Phone {
    fn sms_list(&self, _filter: &Json) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn sms_get(&self, device_ref: &str) -> Result<Json, String> {
        Err(format!("no SMS {device_ref}"))
    }

    fn calls_list(&self, _filter: &Json) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn contacts_list(&self, filter: &Json) -> Result<Vec<Json>, String> {
        let name = filter["name"].as_str();
        Ok(self
            .contacts
            .lock()
            .unwrap()
            .iter()
            .filter(|c| name.is_none_or(|n| c["name"] == n))
            .cloned()
            .collect())
    }

    fn contacts_get(&self, contact_id: &str) -> Result<Json, String> {
        self.contacts
            .lock()
            .unwrap()
            .iter()
            .find(|c| c["contact_id"] == contact_id)
            .cloned()
            .ok_or_else(|| format!("no contact {contact_id}"))
    }

    fn contacts_upsert(&self, contact: &Json) -> Result<Json, String> {
        self.act(format!("upsert {contact}"));
        let id = contact["contact_id"].as_str().unwrap_or("c-new").to_owned();
        Ok(json!({ "contact_id": id }))
    }

    fn contacts_delete(&self, contact_id: &str) -> Result<(), String> {
        self.act(format!("delete {contact_id}"));
        Ok(())
    }

    fn sms_send(&self, to: &str, body: &str) -> Result<Json, String> {
        self.act(format!("sms {to}: {body}"));
        Ok(json!({ "device_ref": "sms:900" }))
    }

    fn show_notification(&self, title: &str, body: &str, note: &str) -> Result<(), String> {
        self.act(format!("notify {title} / {body} / {note}"));
        Ok(())
    }
}

struct Device {
    _dir: TempDir,
    server: Embedded,
    runtime: DeviceRuntime,
    phone: Arc<Phone>,
}

impl Device {
    fn open() -> Device {
        let dir = TempDir::new().unwrap();
        let server = Embedded::open(Config {
            vault: dir.path().to_owned(),
            web: None,
            secret_key: None,
        })
        .unwrap();
        let phone = Arc::new(Phone::default());
        let runtime = server
            .device(DeviceConfig {
                id: DEVICE.to_owned(),
                platform: "android".to_owned(),
                data: phone.clone(),
            })
            .unwrap();
        Device {
            _dir: dir,
            server,
            runtime,
            phone,
        }
    }

    fn writer(&self) -> &Arc<Writer> {
        self.server.writer()
    }

    fn create(&self, type_name: &str, fields: Json) -> String {
        self.writer()
            .create(
                type_name,
                &Input::Json(json!({ "fields": fields })),
                &Author::Session("test".into()),
            )
            .unwrap()
            .id
    }

    fn query(&self, q: &str) -> Vec<Json> {
        Vault::query(&**self.writer(), q, 10_000).unwrap()
    }

    fn fields(&self, id: &str) -> Json {
        Vault::get(&**self.writer(), id).unwrap()["note"]["fields"].clone()
    }

    /// The person's `device_contacts`, or null when it has none.
    fn links(&self, id: &str) -> Json {
        let note = Vault::get(&**self.writer(), id).unwrap();
        note["note"]["blocks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|b| b["kind"] == "dict" && b["labels"] == json!(["device_contacts"]))
            .map_or(Json::Null, |b| b["fields"].clone())
    }

    /// Queue `event` and run the queue; every event in the pass must end `status`.
    fn send_expect(&self, event: &str, data: Json, status: EventStatus) {
        self.runtime
            .enqueue(event, data, 1_760_000_000_000)
            .unwrap();
        let ran = self.runtime.run_pending().unwrap();
        assert!(!ran.is_empty());
        for e in ran {
            assert_eq!(e.status, status, "{e:?}");
        }
    }

    fn send(&self, event: &str, data: Json) {
        self.send_expect(event, data, EventStatus::Done);
    }

    fn settings(&self, settings: Json) {
        self.runtime.set_settings(&settings).unwrap();
    }
}

fn sms(device_ref: &str, number: Option<&str>, raw: &str, thread: &str, body: &str) -> Json {
    json!({
        "device_ref": device_ref,
        "thread_ref": thread,
        "number": number,
        "number_raw": raw,
        "contact": null,
        "body": body,
        "sent": "2026-10-20T09:30:00+11:00",
    })
}

fn body_text(fields_note: &Json) -> Vec<String> {
    fields_note["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|b| b["blocks"].as_array().cloned().unwrap_or_default())
        .map(|p| p["fields"]["text"].as_str().unwrap_or("").to_owned())
        .collect()
}

#[test]
fn sms_become_messages_in_a_conversation_linked_to_the_person_with_the_number() {
    let d = Device::open();
    let alex = d.create(
        "person",
        json!({ "title": "Alex Doe", "phones": ["+61400000001"] }),
    );

    d.send(
        "sms_received",
        sms(
            "sms:1",
            Some("+61400000001"),
            "0400 000 001",
            "t1",
            "Dinner at 7?\nBring wine",
        ),
    );
    let messages = d.query("from message");
    assert_eq!(messages.len(), 1, "{messages:?}");
    let m = &messages[0];
    assert_eq!(m["title"], "Dinner at 7?");
    let f = &m["fields"];
    assert_eq!(f["channel"], "sms");
    assert_eq!(f["direction"], "in");
    assert_eq!(f["from"], alex.as_str());
    assert_eq!(f["from_raw"], "+61400000001");
    assert_eq!(f["device_ref"], "sms:1");
    assert_eq!(f["partial"], false);
    let note = Vault::get(&**d.writer(), m["id"].as_str().unwrap()).unwrap();
    assert_eq!(body_text(&note["note"]), ["Dinner at 7?", "Bring wine"]);

    let conversations = d.query("from conversation");
    assert_eq!(conversations.len(), 1);
    let c = &conversations[0];
    assert_eq!(f["conversation"], c["id"]);
    assert_eq!(c["fields"]["thread_ref"], "t1");
    assert_eq!(c["fields"]["channel"], "sms");
    assert_eq!(c["fields"]["participants"], json!([alex]));

    // The same SMS again, after a restart or a catch-up, is not captured twice.
    d.send(
        "sms_received",
        sms(
            "sms:1",
            Some("+61400000001"),
            "0400 000 001",
            "t1",
            "Dinner at 7?\nBring wine",
        ),
    );
    assert_eq!(d.query("from message").len(), 1);

    // A reply sent in the same thread joins the conversation, to Alex.
    d.send(
        "sms_sent",
        sms("sms:2", Some("+61400000001"), "0400 000 001", "t1", "Sure"),
    );
    let sent = d.query("from message where device_ref == \"sms:2\"");
    let f = &sent[0]["fields"];
    assert_eq!(f["direction"], "out");
    assert_eq!(f["to"], json!([alex]));
    assert_eq!(f["to_raw"], json!(["+61400000001"]));
    assert_eq!(f["conversation"], c["id"]);
    assert_eq!(d.query("from conversation").len(), 1);
}

#[test]
fn an_unknown_number_creates_no_person() {
    let d = Device::open();
    d.create(
        "person",
        json!({ "title": "Alex Doe", "phones": ["+61400000001"] }),
    );
    d.send(
        "sms_received",
        sms(
            "sms:7",
            Some("+61499999999"),
            "0499 999 999",
            "t9",
            "Parcel waiting",
        ),
    );
    d.send(
        "sms_received",
        sms("sms:8", None, "AUSPOST", "t10", "Delivered"),
    );
    assert_eq!(d.query("from person").len(), 1);
    let m = d.query("from message where device_ref == \"sms:7\"");
    assert_eq!(m[0]["fields"]["from_raw"], "+61499999999");
    assert!(m[0]["fields"].get("from").is_none(), "{m:?}");
    let short = d.query("from message where device_ref == \"sms:8\"");
    assert_eq!(short[0]["fields"]["from_raw"], "AUSPOST");
    assert!(short[0]["fields"].get("from").is_none());
    let threads = d.query("from conversation where thread_ref == \"t9\"");
    assert_eq!(threads[0]["fields"]["participants"], json!([]));
}

#[test]
fn a_group_mms_links_every_known_recipient_and_keeps_its_attachments() {
    let d = Device::open();
    let alex = d.create(
        "person",
        json!({ "title": "Alex Doe", "phones": ["+61400000001"] }),
    );
    let sam = d.create(
        "person",
        json!({ "title": "Sam Lee", "phones": ["+61400000002"] }),
    );
    let photo = d
        .writer()
        .put_asset("beach.jpg", b"\xff\xd8\xff".to_vec())
        .unwrap()
        .id;
    d.send(
        "mms_received",
        json!({
            "device_ref": "mms:87",
            "thread_ref": "t4",
            "number": "+61400000001",
            "number_raw": "0400 000 001",
            "contact": { "id": "c1", "name": "Alex" },
            "to": [
                { "number": "+61400000002", "number_raw": "0400 000 002", "contact": null },
                { "number": "+61400000003", "number_raw": "0400 000 003", "contact": null },
            ],
            "subject": "",
            "body": "Look at this",
            "attachments": [{ "asset": photo, "filename": "beach.jpg", "mime": "image/jpeg", "size": 3 }],
            "sent": "2026-10-20T10:00:00+11:00",
        }),
    );
    let m = d.query("from message");
    assert_eq!(m.len(), 1);
    let f = &m[0]["fields"];
    assert_eq!(f["channel"], "mms");
    assert_eq!(f["from"], alex.as_str());
    assert_eq!(f["to"], json!([sam]));
    assert_eq!(f["to_raw"], json!(["+61400000002", "+61400000003"]));
    assert_eq!(f["attachments"], json!([photo]));
    let c = d.query("from conversation");
    assert_eq!(c[0]["fields"]["channel"], "mms");
    assert_eq!(c[0]["fields"]["participants"], json!([alex, sam]));
    assert_eq!(d.query("from person").len(), 2);
}

/// An MMS with a picture and a contact card keeps both, in order, in `attachments`, whose images
/// the message type's templates show; the contact card stays a file.
#[test]
fn an_mms_keeps_its_image_and_its_other_parts_in_attachments() {
    let d = Device::open();
    let photo = d
        .writer()
        .put_asset("image000001.jpg", b"\xff\xd8\xff".to_vec())
        .unwrap()
        .id;
    let card = d
        .writer()
        .put_asset("contact.vcf", b"BEGIN:VCARD".to_vec())
        .unwrap()
        .id;
    d.send(
        "mms_received",
        json!({
            "device_ref": "mms:90",
            "thread_ref": "t9",
            "number": "+61400000009",
            "number_raw": "0400 000 009",
            "contact": null,
            "to": [],
            "subject": "",
            "body": "",
            "attachments": [
                { "asset": photo, "filename": "image000001.jpg", "mime": "image/jpeg", "size": 3 },
                { "asset": card, "filename": "contact.vcf", "mime": "text/x-vcard", "size": 11 },
            ],
            "sent": "2026-10-20T10:00:00+11:00",
        }),
    );
    let m = d.query("from message");
    assert_eq!(m.len(), 1);
    let f = &m[0]["fields"];
    assert_eq!(f["attachments"], json!([photo, card]));
    assert_eq!(m[0]["title"], "MMS from +61400000009");
    // The templates that show the image are checked in crates/server/tests/phone_types.rs.
}

#[test]
fn rcs_senders_match_by_name_or_by_the_phone_contact_and_are_partial() {
    let d = Device::open();
    let alex = d.create(
        "person",
        json!({ "title": "Alex Doe", "phones": ["+61400000001"] }),
    );
    let sam = d.create(
        "person",
        json!({ "title": "Samuel Lee", "phones": ["+61400000002"] }),
    );
    // The phone knows Samuel as "Sammy".
    d.phone.contacts.lock().unwrap().push(json!({
        "contact_id": "c2",
        "name": "Sammy",
        "phones": ["+61400000002"],
        "emails": [],
    }));
    let rcs = |key: &str, sender: &str, body: &str| {
        json!({
            "device_ref": key,
            "thread_ref": format!("conv-{sender}"),
            "sender": sender,
            "conversation": null,
            "body": body,
            "sent": "2026-10-20T11:00:00+11:00",
            "partial": true,
        })
    };
    d.send("rcs_received", rcs("n1", "Alex Doe", "On my way"));
    d.send("rcs_received", rcs("n2", "Sammy", "Running late"));
    d.send("rcs_received", rcs("n3", "Stranger", "Hello"));
    d.send("rcs_received", rcs("n1", "Alex Doe", "On my way"));

    let by_ref = |r: &str| d.query(&format!("from message where device_ref == \"{r}\""));
    let a = by_ref("n1");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["fields"]["from"], alex.as_str());
    assert_eq!(a[0]["fields"]["channel"], "rcs");
    assert_eq!(a[0]["fields"]["partial"], true);
    assert_eq!(a[0]["fields"]["from_raw"], "Alex Doe");
    assert_eq!(by_ref("n2")[0]["fields"]["from"], sam.as_str());
    let stranger = by_ref("n3");
    assert!(stranger[0]["fields"].get("from").is_none());
    assert_eq!(stranger[0]["fields"]["from_raw"], "Stranger");
    assert_eq!(d.query("from message").len(), 3);
    assert_eq!(d.query("from person").len(), 2);
    assert_eq!(
        d.query("from conversation where channel == \"rcs\"").len(),
        3
    );
}

#[test]
fn calls_become_call_notes_once_each() {
    let d = Device::open();
    let alex = d.create(
        "person",
        json!({ "title": "Alex Doe", "phones": ["+61400000001"] }),
    );
    let call = |key: &str, direction: &str, number: &str, duration: i64| {
        json!({
            "device_ref": key,
            "direction": direction,
            "number": number,
            "number_raw": number,
            "contact": null,
            "started": "2026-10-20T12:00:00+11:00",
            "duration": duration,
        })
    };
    d.send("call_ended", call("call:1", "in", "+61400000001", 125));
    d.send("call_ended", call("call:2", "missed", "+61488888888", 0));
    d.send("call_ended", call("call:1", "in", "+61400000001", 125));

    let calls = d.query("from call sort title asc");
    assert_eq!(calls.len(), 2, "{calls:?}");
    let from_alex = &calls[0];
    assert_eq!(from_alex["title"], "Call from Alex Doe");
    assert_eq!(from_alex["fields"]["person"], alex.as_str());
    assert_eq!(from_alex["fields"]["duration"], 125);
    assert_eq!(from_alex["fields"]["direction"], "in");
    assert_eq!(from_alex["fields"]["number"], "+61400000001");
    let missed = &calls[1];
    assert_eq!(missed["title"], "Missed call from +61488888888");
    assert!(missed["fields"].get("person").is_none());
    assert_eq!(d.query("from person").len(), 1);
}

#[test]
fn the_exclusion_list_and_the_skip_pattern_drop_events_before_any_handler() {
    let d = Device::open();
    d.settings(json!({
        "exclusions": [
            { "kind": "number", "value": "+61 400 000 005" },
            { "kind": "thread", "value": "secret-thread" },
            { "kind": "contact", "value": "Dr Who" },
        ],
    }));
    d.send_expect(
        "sms_received",
        sms("sms:20", Some("+61400000005"), "0400000005", "t20", "hi"),
        EventStatus::Skipped,
    );
    d.send_expect(
        "sms_received",
        sms(
            "sms:21",
            Some("+61400000006"),
            "0400000006",
            "secret-thread",
            "hi",
        ),
        EventStatus::Skipped,
    );
    d.send_expect(
        "call_ended",
        json!({
            "device_ref": "call:20", "direction": "in", "number": "+61400000005",
            "number_raw": "0400000005", "contact": null,
            "started": "2026-10-20T12:00:00+11:00", "duration": 3,
        }),
        EventStatus::Skipped,
    );
    d.send_expect(
        "rcs_received",
        json!({
            "device_ref": "n20", "thread_ref": "x", "sender": "Dr Who", "conversation": null,
            "body": "hello", "sent": "2026-10-20T11:00:00+11:00", "partial": true,
        }),
        EventStatus::Skipped,
    );
    // The default skip pattern drops one-time codes.
    d.send_expect(
        "sms_received",
        sms(
            "sms:22",
            Some("+61400000007"),
            "0400000007",
            "t22",
            "Your verification code is 482913",
        ),
        EventStatus::Skipped,
    );
    assert!(d.query("from message").is_empty());
    assert!(d.query("from call").is_empty());
    let skipped = d.runtime.events().unwrap();
    assert_eq!(skipped.len(), 5);
    assert!(
        skipped[4]
            .message
            .as_deref()
            .unwrap()
            .contains("skip pattern"),
        "{skipped:?}"
    );
    assert!(
        skipped[0]
            .message
            .as_deref()
            .unwrap()
            .contains("number \"+61 400 000 005\" is on the exclusion list")
    );

    // Anything else still runs.
    d.send(
        "sms_received",
        sms(
            "sms:23",
            Some("+61400000007"),
            "0400000007",
            "t22",
            "See you",
        ),
    );
    assert_eq!(d.query("from message").len(), 1);
}

#[test]
fn contacts_sync_with_people_both_ways_and_deletes_only_unlink() {
    let d = Device::open();
    let contact = |id: &str, name: &str, phones: Json, updated: &str| {
        json!({
            "contact_id": id,
            "name": name,
            "phones": phones,
            "emails": ["sam@example.com"],
            "updated": updated,
        })
    };

    // A new contact becomes a person linked to it.
    d.send(
        "contact_changed",
        contact(
            "c9",
            "Sam Lee",
            json!(["+61400000002"]),
            "2020-01-01T00:00:00Z",
        ),
    );
    let people = d.query("from person");
    assert_eq!(people.len(), 1);
    let sam = people[0]["id"].as_str().unwrap().to_owned();
    let f = d.fields(&sam);
    assert_eq!(f["title"], "Sam Lee");
    assert_eq!(f["phones"], json!(["+61400000002"]));
    assert_eq!(f["emails"], json!(["sam@example.com"]));
    assert_eq!(d.links(&sam), json!({ SLOT: "c9" }));

    // A later change on the phone wins over the person.
    d.send(
        "contact_changed",
        contact(
            "c9",
            "Samuel Lee",
            json!(["+61400000002", "+61400000022"]),
            "2999-01-01T00:00:00Z",
        ),
    );
    let f = d.fields(&sam);
    assert_eq!(f["title"], "Samuel Lee");
    assert_eq!(f["phones"], json!(["+61400000002", "+61400000022"]));
    assert_eq!(d.query("from person").len(), 1);

    // A phone change older than the person's last edit gives way: the phone takes the person's
    // fields when the device may write contacts, and nothing changes when it may not.
    d.send(
        "contact_changed",
        contact(
            "c9",
            "Old Name",
            json!(["+61400000002"]),
            "2000-01-01T00:00:00Z",
        ),
    );
    assert_eq!(d.fields(&sam)["title"], "Samuel Lee");
    assert!(d.phone.actions().is_empty());
    d.settings(json!({ "contacts_write": true }));
    d.send(
        "contact_changed",
        contact(
            "c9",
            "Old Name",
            json!(["+61400000002"]),
            "2000-01-01T00:00:00Z",
        ),
    );
    let actions = d.phone.actions();
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert!(
        actions[0].contains("\"name\":\"Samuel Lee\""),
        "{actions:?}"
    );
    assert!(actions[0].contains("\"contact_id\":\"c9\""), "{actions:?}");

    // An existing person with the contact's number is linked, not duplicated.
    let alex = d.create(
        "person",
        json!({ "title": "Alex Doe", "phones": ["+61400000001"] }),
    );
    d.send(
        "contact_changed",
        contact(
            "c1",
            "Alex Doe",
            json!(["+61400000001"]),
            "2999-01-01T00:00:00Z",
        ),
    );
    assert_eq!(d.query("from person").len(), 2);
    assert_eq!(d.links(&alex), json!({ SLOT: "c1" }));

    // Deleting the contact on the phone unlinks the person and keeps it.
    d.send("contact_deleted", json!({ "contact_id": "c9" }));
    let f = d.fields(&sam);
    assert_eq!(f["title"], "Samuel Lee");
    assert_eq!(d.links(&sam), Json::Null);
    assert_eq!(d.query("from person").len(), 2);
}

/// What the phone records as agreed after an upsert: the detail keys it carried.
fn synced_from(upsert: &Json) -> Json {
    let mut synced = upsert.clone();
    let m = synced.as_object_mut().unwrap();
    m.remove("contact_id");
    m.remove("person");
    synced
}

/// The last `upsert` the phone was asked for, as JSON.
fn last_upsert(phone: &Phone) -> Json {
    let actions = phone.actions();
    let last = actions
        .iter()
        .rev()
        .find_map(|a| a.strip_prefix("upsert "))
        .unwrap_or_else(|| panic!("no upsert in {actions:?}"));
    serde_json::from_str(last).unwrap()
}

#[test]
fn contacts_settle_field_by_field_and_person_changes_reach_the_phone() {
    let d = Device::open();
    assert_eq!(
        d.runtime.queue_person_changes().unwrap(),
        0,
        "the first call only starts"
    );
    d.settings(json!({ "contacts_write": true }));

    // A new contact becomes a person, and the contact is given the person's id and the fields
    // both sides now agree on.
    d.send(
        "contact_changed",
        json!({
            "contact_id": "c5",
            "name": "Kim Park",
            "phones": ["+61400000005"],
            "emails": [],
            "updated": "2020-01-01T00:00:00Z",
        }),
    );
    let kim = d.query("from person")[0]["id"].as_str().unwrap().to_owned();
    let up = last_upsert(&d.phone);
    assert_eq!(up["person"], kim.as_str());
    assert_eq!(up["contact_id"], "c5");
    assert_eq!(up["name"], "Kim Park");
    let synced = synced_from(&up);

    // The person is renamed in the vault, and the phone adds a number, in an edit older than
    // the rename. Each side changed a different field, so each change stands.
    Vault::patch(
        &**d.writer(),
        &kim,
        json!({ "title": "Kim Park-Lee" }),
        "",
        &Author::Session("test".into()),
    )
    .unwrap();
    d.send(
        "contact_changed",
        json!({
            "contact_id": "c5",
            "name": "Kim Park",
            "phones": ["+61400000005", "+61400000055"],
            "emails": [],
            "updated": "2020-02-01T00:00:00Z",
            "person": kim,
            "synced": synced,
        }),
    );
    let f = d.fields(&kim);
    assert_eq!(f["title"], "Kim Park-Lee");
    assert_eq!(f["phones"], json!(["+61400000005", "+61400000055"]));
    let up = last_upsert(&d.phone);
    assert_eq!(up["name"], "Kim Park-Lee");
    assert_eq!(up["phones"], json!(["+61400000005", "+61400000055"]));
    let synced = json!({
        "name": "Kim Park-Lee",
        "phones": ["+61400000005", "+61400000055"],
        "emails": [],
    });

    // Both sides changed the name: the more recent edit wins, here the phone's.
    Vault::patch(
        &**d.writer(),
        &kim,
        json!({ "title": "K. Park-Lee" }),
        "",
        &Author::Session("test".into()),
    )
    .unwrap();
    d.send(
        "contact_changed",
        json!({
            "contact_id": "c5",
            "name": "Kimberly Park-Lee",
            "phones": ["+61400000005", "+61400000055"],
            "emails": [],
            "updated": "2999-01-01T00:00:00Z",
            "person": kim,
            "synced": synced,
        }),
    );
    assert_eq!(d.fields(&kim)["title"], "Kimberly Park-Lee");

    // A contact that agrees with its person and holds what they agreed on asks nothing more.
    let synced = json!({
        "name": "Kimberly Park-Lee",
        "phones": ["+61400000005", "+61400000055"],
        "emails": [],
    });
    let contact = json!({
        "contact_id": "c5",
        "name": "Kimberly Park-Lee",
        "phones": ["+61400000005", "+61400000055"],
        "emails": [],
        "updated": "2999-01-02T00:00:00Z",
        "person": kim,
        "synced": synced,
    });
    let before = d.phone.actions().len();
    d.send("contact_changed", contact.clone());
    assert_eq!(d.phone.actions().len(), before, "{:?}", d.phone.actions());

    // A person edited after a sync round reaches the phone: the runtime queues its contact,
    // as the phone holds it, and the handler hands the contact the person's change.
    d.phone.contacts.lock().unwrap().push(json!({
        "contact_id": "c5",
        "name": "Kimberly Park-Lee",
        "phones": ["+61400000005", "+61400000055"],
        "emails": [],
        "updated": "2020-03-01T00:00:00Z",
        "person": kim,
        "synced": contact["synced"],
    }));
    d.runtime.queue_person_changes().unwrap();
    d.runtime.run_pending().unwrap();
    Vault::patch(
        &**d.writer(),
        &kim,
        json!({ "emails": ["kim@example.com"] }),
        "",
        &Author::Session("test".into()),
    )
    .unwrap();
    d.settings(json!({ "contacts_write": false }));
    assert_eq!(
        d.runtime.queue_person_changes().unwrap(),
        0,
        "not while writes are off"
    );
    d.settings(json!({ "contacts_write": true }));
    let before = d.phone.actions().len();
    assert_eq!(d.runtime.queue_person_changes().unwrap(), 1);
    d.runtime.run_pending().unwrap();
    assert_eq!(
        d.phone.actions().len(),
        before + 1,
        "{:?}",
        d.phone.actions()
    );
    let up = last_upsert(&d.phone);
    assert_eq!(up["contact_id"], "c5");
    assert_eq!(up["emails"], json!(["kim@example.com"]));
    assert_eq!(up["email_labels"], json!([""]));
    assert_eq!(up["name"], "Kimberly Park-Lee");
    assert_eq!(
        d.runtime.queue_person_changes().unwrap(),
        0,
        "each change once"
    );

    // A contact whose person was deleted in the vault stays unlinked: no person comes back.
    let version = Vault::get(&**d.writer(), &kim).unwrap()["version"]
        .as_str()
        .unwrap()
        .to_owned();
    Vault::delete(
        &**d.writer(),
        &kim,
        &version,
        &Author::Session("test".into()),
    )
    .unwrap();
    d.send("contact_changed", contact);
    assert!(d.query("from person").is_empty());
}

impl Device {
    fn asset(&self, name: &str, bytes: &[u8]) -> String {
        self.writer().put_asset(name, bytes.to_vec()).unwrap().id
    }

    fn patch(&self, id: &str, fields: Json) {
        Vault::patch(
            &**self.writer(),
            id,
            fields,
            "",
            &Author::Session("test".into()),
        )
        .unwrap();
    }
}

/// A contact as the phone reports it, with every detail.
fn full_contact(photo: &str, hash: &str) -> Json {
    json!({
        "contact_id": "c7",
        "name": "Robin Rivera",
        "phones": ["+61400000007", "+61290000007"],
        "phone_labels": ["mobile", "work"],
        "emails": ["robin@example.com"],
        "email_labels": ["home"],
        "addresses": ["1 Main St, Springfield, 2000"],
        "address_labels": ["Holiday house"],
        "organisation": "Example Co",
        "job_title": "Engineer",
        "nickname": "Rob",
        "birthday": "--04-01",
        "websites": ["robin.example", "https://example.org/robin", "not a url"],
        "contact_note": "Met at the conference",
        "photo": { "asset": photo, "hash": hash },
        "updated": "2020-01-01T00:00:00Z",
    })
}

#[test]
fn a_contact_brings_every_detail_and_a_person_change_reaches_every_detail_of_the_contact() {
    let d = Device::open();
    d.settings(json!({ "contacts_write": true }));
    let photo = d.asset("robin.jpg", b"\xff\xd8\xffrobin");
    d.send("contact_changed", full_contact(&photo, "h1"));

    let robin = d.query("from person")[0]["id"].as_str().unwrap().to_owned();
    let f = d.fields(&robin);
    assert_eq!(f["title"], "Robin Rivera");
    assert_eq!(f["phones"], json!(["+61400000007", "+61290000007"]));
    assert_eq!(f["phone_labels"], json!(["mobile", "work"]));
    assert_eq!(f["emails"], json!(["robin@example.com"]));
    assert_eq!(f["email_labels"], json!(["home"]));
    assert_eq!(f["addresses"], json!(["1 Main St, Springfield, 2000"]));
    assert_eq!(f["address_labels"], json!(["Holiday house"]));
    assert_eq!(f["organisation"], "Example Co");
    assert_eq!(f["job_title"], "Engineer");
    assert_eq!(f["nickname"], "Rob");
    assert_eq!(f["birthday"], "--04-01");
    // A website without a scheme gets https://, and one that is still not a URL is dropped.
    assert_eq!(
        f["websites"],
        json!(["https://robin.example", "https://example.org/robin"])
    );
    assert_eq!(f["contact_note"], "Met at the conference");
    assert_eq!(f["photo"], photo.as_str());

    // The contact is told the person's id and every detail they now agree on.
    let up = last_upsert(&d.phone);
    assert_eq!(up["person"], robin.as_str());
    assert_eq!(up["photo"], photo.as_str());
    assert_eq!(up["websites"], f["websites"]);
    for key in [
        "phones",
        "phone_labels",
        "emails",
        "email_labels",
        "addresses",
        "address_labels",
        "organisation",
        "job_title",
        "nickname",
        "birthday",
        "contact_note",
    ] {
        assert_eq!(up[key], f[key], "{key}");
    }
}

#[test]
fn every_detail_settles_field_by_field_and_labels_follow_their_values() {
    let d = Device::open();
    d.settings(json!({ "contacts_write": true }));
    let photo = d.asset("robin.jpg", b"\xff\xd8\xffrobin");
    d.send("contact_changed", full_contact(&photo, "h1"));
    let robin = d.query("from person")[0]["id"].as_str().unwrap().to_owned();
    let mut synced = synced_from(&last_upsert(&d.phone));
    synced["photo"] = json!({ "asset": photo, "hash": "h1" });

    // The person changes the job title and the birthday and swaps the phones, labels with them;
    // the phone, in an older edit, changes the organisation and the nickname and relabels the
    // email. Each side's change stands.
    d.patch(
        &robin,
        json!({
            "job_title": "Lead Engineer",
            "birthday": "1990-04-01",
            "phones": ["+61290000007", "+61400000007"],
            "phone_labels": ["work", "mobile"],
        }),
    );
    let mut contact = full_contact(&photo, "h1");
    contact["organisation"] = json!("Other Co");
    contact["nickname"] = json!("Robbie");
    contact["email_labels"] = json!(["work"]);
    contact["updated"] = json!("2020-02-01T00:00:00Z");
    contact["person"] = json!(robin);
    contact["synced"] = synced.clone();
    d.send("contact_changed", contact);
    let f = d.fields(&robin);
    assert_eq!(f["job_title"], "Lead Engineer");
    assert_eq!(f["birthday"], "1990-04-01");
    assert_eq!(f["organisation"], "Other Co");
    assert_eq!(f["nickname"], "Robbie");
    assert_eq!(f["emails"], json!(["robin@example.com"]));
    assert_eq!(f["email_labels"], json!(["work"]));
    assert_eq!(f["phones"], json!(["+61290000007", "+61400000007"]));
    assert_eq!(f["phone_labels"], json!(["work", "mobile"]));
    let up = last_upsert(&d.phone);
    assert_eq!(up["job_title"], "Lead Engineer");
    assert_eq!(up["birthday"], "1990-04-01");
    assert_eq!(up["organisation"], "Other Co");
    assert_eq!(up["phones"], json!(["+61290000007", "+61400000007"]));
    assert_eq!(up["phone_labels"], json!(["work", "mobile"]));
    assert_eq!(up["email_labels"], json!(["work"]));

    // Both sides change the contact note: the more recent edit wins, here the person's.
    let mut synced = synced_from(&up);
    synced["photo"] = json!({ "asset": photo, "hash": "h1" });
    d.patch(&robin, json!({ "contact_note": "Prefers email" }));
    let mut contact = full_contact(&photo, "h1");
    for key in [
        "phones",
        "phone_labels",
        "organisation",
        "job_title",
        "nickname",
        "birthday",
        "email_labels",
    ] {
        contact[key] = up[key].clone();
    }
    contact["contact_note"] = json!("Prefers calls");
    contact["updated"] = json!("2000-01-01T00:00:00Z");
    contact["person"] = json!(robin);
    contact["synced"] = synced;
    d.send("contact_changed", contact);
    assert_eq!(d.fields(&robin)["contact_note"], "Prefers email");
    assert_eq!(last_upsert(&d.phone)["contact_note"], "Prefers email");

    // A label changed on the phone alone moves with its value.
    let up = last_upsert(&d.phone);
    let mut synced = synced_from(&up);
    synced["photo"] = json!({ "asset": photo, "hash": "h1" });
    let mut contact = synced.clone();
    contact["contact_id"] = json!("c7");
    contact["phone_labels"] = json!(["main", "mobile"]);
    contact["photo"] = json!({ "asset": photo, "hash": "h1" });
    contact["updated"] = json!("2000-01-02T00:00:00Z");
    contact["person"] = json!(robin);
    contact["synced"] = synced;
    d.send("contact_changed", contact);
    assert_eq!(d.fields(&robin)["phone_labels"], json!(["main", "mobile"]));
}

#[test]
fn a_detail_the_phone_does_not_report_is_left_alone() {
    let d = Device::open();
    let photo = d.asset("robin.jpg", b"\xff\xd8\xffrobin");
    d.send("contact_changed", full_contact(&photo, "h1"));
    let robin = d.query("from person")[0]["id"].as_str().unwrap().to_owned();
    let before = d.fields(&robin);

    // A newer contact from an app that reports only the name and phones changes nothing else.
    d.send(
        "contact_changed",
        json!({
            "contact_id": "c7",
            "name": "Robin Rivera",
            "phones": ["+61400000007", "+61290000007"],
            "phone_labels": ["mobile", "work"],
            "updated": "2999-01-01T00:00:00Z",
            "person": robin,
        }),
    );
    let after = d.fields(&robin);
    for key in [
        "emails",
        "email_labels",
        "addresses",
        "address_labels",
        "organisation",
        "job_title",
        "nickname",
        "birthday",
        "websites",
        "contact_note",
        "photo",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }

    // An app from before emails had their own key reports them as `addresses`: they are emails,
    // and the contact it is sent back takes them as `addresses` too.
    d.settings(json!({ "contacts_write": true }));
    d.send(
        "contact_changed",
        json!({
            "contact_id": "c8",
            "name": "Old App",
            "phones": ["+61400000008"],
            "addresses": ["old@example.com"],
            "updated": "2020-01-01T00:00:00Z",
        }),
    );
    let old = d.query("from person where title == \"Old App\"").remove(0);
    let f = d.fields(old["id"].as_str().unwrap());
    assert_eq!(f["emails"], json!(["old@example.com"]));
    assert!(f.get("addresses").is_none(), "{f}");
    let up = last_upsert(&d.phone);
    assert_eq!(up["addresses"], json!(["old@example.com"]));
    assert_eq!(up["emails"], json!(["old@example.com"]));
}

#[test]
fn a_contact_agreed_before_the_new_details_comes_through_as_no_change() {
    let d = Device::open();
    d.settings(json!({ "contacts_write": true }));
    let photo = d.asset("robin.jpg", b"\xff\xd8\xffrobin");
    d.send("contact_changed", full_contact(&photo, "h1"));
    let robin = d.query("from person")[0]["id"].as_str().unwrap().to_owned();
    let versions = d.writer().history(&robin).unwrap().len();
    let asked = d.phone.actions().len();

    // The app rewrote its old agreed row, `addresses` read as emails, and reports the contact
    // once more: only the name, phones and emails were agreed, and nothing changed.
    let mut contact = full_contact(&photo, "h1");
    contact["websites"] = json!(["https://robin.example", "https://example.org/robin"]);
    contact["updated"] = json!("2999-01-01T00:00:00Z");
    contact["person"] = json!(robin);
    contact["synced"] = json!({
        "name": "Robin Rivera",
        "phones": ["+61400000007", "+61290000007"],
        "emails": ["robin@example.com"],
    });
    d.send("contact_changed", contact);
    assert_eq!(d.writer().history(&robin).unwrap().len(), versions);
    assert_eq!(d.phone.actions().len(), asked, "{:?}", d.phone.actions());
}

#[test]
fn the_photo_compares_by_hash() {
    let d = Device::open();
    d.settings(json!({ "contacts_write": true }));
    let photo = d.asset("robin.jpg", b"\xff\xd8\xffrobin");
    d.send("contact_changed", full_contact(&photo, "h1"));
    let robin = d.query("from person")[0]["id"].as_str().unwrap().to_owned();
    let mut synced = synced_from(&last_upsert(&d.phone));
    synced["photo"] = json!({ "asset": photo, "hash": "h1" });
    let event = |photo: &str, hash: &str, updated: &str, synced: &Json| {
        let mut c = full_contact(photo, hash);
        c["websites"] = json!(["https://robin.example", "https://example.org/robin"]);
        c["updated"] = json!(updated);
        c["person"] = json!(robin);
        c["synced"] = synced.clone();
        c
    };

    // The same photo stored again on the phone, as a new asset with the same hash, is no change:
    // the person keeps its asset and the phone is asked nothing.
    let again = d.asset("robin-again.jpg", b"\xff\xd8\xffrobin");
    let asked = d.phone.actions().len();
    d.send(
        "contact_changed",
        event(&again, "h1", "2999-01-01T00:00:00Z", &synced),
    );
    assert_eq!(d.fields(&robin)["photo"], photo.as_str());
    assert_eq!(d.phone.actions().len(), asked, "{:?}", d.phone.actions());

    // A new photo on the phone, a new hash, reaches the person.
    let new_photo = d.asset("robin-new.jpg", b"\xff\xd8\xffnew");
    d.send(
        "contact_changed",
        event(&new_photo, "h2", "2020-02-01T00:00:00Z", &synced),
    );
    assert_eq!(d.fields(&robin)["photo"], new_photo.as_str());
    let up = last_upsert(&d.phone);
    assert_eq!(up["photo"], new_photo.as_str());

    // A new photo on the person, with the phone's still the agreed one, reaches the phone.
    let mut synced = synced_from(&up);
    synced["photo"] = json!({ "asset": new_photo, "hash": "h2" });
    let person_photo = d.asset("robin-web.jpg", b"\xff\xd8\xffweb");
    d.patch(&robin, json!({ "photo": person_photo }));
    d.send(
        "contact_changed",
        event(&new_photo, "h2", "2020-03-01T00:00:00Z", &synced),
    );
    assert_eq!(d.fields(&robin)["photo"], person_photo.as_str());
    assert_eq!(last_upsert(&d.phone)["photo"], person_photo.as_str());

    // The photo removed on the phone, with the person's the agreed one, removes it.
    let mut synced = synced_from(&last_upsert(&d.phone));
    synced["photo"] = json!({ "asset": person_photo, "hash": "h3" });
    let mut c = event(&person_photo, "h3", "2999-02-01T00:00:00Z", &synced);
    c["photo"] = Json::Null;
    d.send("contact_changed", c);
    assert!(d.fields(&robin).get("photo").is_none());
}

/// A share as the Android share target queues it: a photo and a PDF the app stored as assets.
fn share(d: &Device, text: &str) -> (Json, String, String) {
    let photo = d
        .writer()
        .put_asset("cat.png", b"\x89PNG".to_vec())
        .unwrap()
        .id;
    let pdf = d
        .writer()
        .put_asset("menu.pdf", b"%PDF-1.4".to_vec())
        .unwrap()
        .id;
    let data = json!({
        "text": text,
        "subject": "",
        "url": "https://example.com/post",
        "attachments": [
            { "asset": photo, "filename": "cat.png", "mime": "image/png", "size": 4 },
            { "asset": pdf, "filename": "menu.pdf", "mime": "application/pdf", "size": 8 },
        ],
    });
    (data, photo, pdf)
}

#[test]
fn a_share_becomes_exactly_one_fleeting_note_through_device_share() {
    let d = Device::open();
    assert!(d.runtime.handles("share_received").unwrap());
    let (data, photo, pdf) = share(&d, "Read *this* later https://example.com/post");
    let id = d.runtime.enqueue("share_received", data, 1).unwrap();
    let e = d.runtime.run_event(id).unwrap().unwrap();
    assert_eq!(e.status, EventStatus::Done, "{e:?}");
    assert_eq!(e.notes.len(), 1, "{e:?}");
    assert_eq!(
        d.runtime.event(id).unwrap().unwrap().notes,
        e.notes,
        "kept in the queue"
    );

    let notes = d.query("from fleeting");
    assert_eq!(notes.len(), 1, "exactly one note: {notes:?}");
    assert_eq!(notes[0]["id"], e.notes[0].as_str());
    assert_eq!(
        notes[0]["title"],
        "Read *this* later https://example.com/post"
    );
    let note = Vault::get(&**d.writer(), &e.notes[0]).unwrap()["note"].clone();
    assert_eq!(note["fields"]["image"], json!([photo]));
    assert_eq!(note["fields"]["processed"], false);
    let body = note["blocks"][0]["blocks"].as_array().unwrap();
    assert_eq!(
        body[0]["fields"]["text"], "Read \\*this\\* later https://example.com/post",
        "shared text is escaped, not read as markup"
    );
    assert_eq!(body.last().unwrap()["kind"], "asset");
    assert_eq!(body.last().unwrap()["fields"]["asset"], pdf.as_str());

    // Running it again, or a pass over the queue, makes no second note.
    assert_eq!(d.runtime.run_event(id).unwrap().unwrap().notes, e.notes);
    assert!(d.runtime.run_pending().unwrap().is_empty());
    assert_eq!(d.query("from fleeting").len(), 1);
}

#[test]
fn a_users_share_handler_decides_what_a_share_becomes() {
    let d = Device::open();
    d.writer()
        .put_script(
            "device_share",
            r#"use vault

fn share_received(event: Value) -> Result[(), string] {
    let data = event.get("data").unwrap()
    let files = match data.get("attachments") {
        Some(Value::List(items)) => items,
        _ => [],
    }
    let assets = files.map(|a| a.get("asset").unwrap())
    vault::create("fleeting", Value::Map(#{
        "fields": Value::Map(#{
            "title": Value::String("Shared: " + data.get("url").unwrap().as_string().unwrap()),
            "image": Value::List(assets),
        }),
    }))?
    Ok(())
}
"#,
        )
        .unwrap();
    let (data, photo, pdf) = share(&d, "Read this later https://example.com/post");
    let id = d.runtime.enqueue("share_received", data, 1).unwrap();
    let e = d.runtime.run_event(id).unwrap().unwrap();
    assert_eq!(e.status, EventStatus::Done, "{e:?}");
    assert_eq!(e.notes.len(), 1);
    let notes = d.query("from fleeting");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["title"], "Shared: https://example.com/post");
    assert_eq!(d.fields(&e.notes[0])["image"], json!([photo, pdf]));
}

#[test]
fn a_device_file_without_a_share_handler_makes_nothing_of_a_share() {
    let d = Device::open();
    d.writer()
        .put_script(
            "device_calls",
            writer::default_device_script("device_calls").unwrap(),
        )
        .unwrap();
    d.writer()
        .put_device_file(
            "android",
            "handler \"call_ended\" {\n  script = \"device_calls\"\n}\n",
        )
        .unwrap();
    // The caller sees this and writes the note itself, so the share is not lost.
    assert!(!d.runtime.handles("share_received").unwrap());
    assert!(d.runtime.handles("call_ended").unwrap());
    let (data, _, _) = share(&d, "nobody takes this");
    let id = d.runtime.enqueue("share_received", data, 1).unwrap();
    let e = d.runtime.run_event(id).unwrap().unwrap();
    assert_eq!(e.status, EventStatus::Done);
    assert!(e.notes.is_empty());
    assert!(d.query("from fleeting").is_empty());
}

#[test]
fn a_pass_records_the_notes_each_event_made() {
    let d = Device::open();
    d.runtime
        .enqueue("share_received", share(&d, "one").0, 1)
        .unwrap();
    d.runtime
        .enqueue("share_received", share(&d, "two").0, 2)
        .unwrap();
    let ran = d.runtime.run_pending().unwrap();
    assert_eq!(ran.len(), 2);
    assert!(ran.iter().all(|e| e.notes.len() == 1), "{ran:?}");
    assert_ne!(ran[0].notes, ran[1].notes);
    assert_eq!(d.fields(&ran[0].notes[0])["title"], "one");
    assert_eq!(d.fields(&ran[1].notes[0])["title"], "two");
}

#[test]
fn switches_block_contact_writes_and_sending_sms() {
    let d = Device::open();
    let sam = d.create("person", json!({ "title": "Sam Lee" }));
    d.settings(json!({ "commands": ["send_sms", "upsert_contact", "delete_contact"] }));
    let command = |id: &str, name: &str, payload: Json| json!({ "id": id, "name": name, "payload": payload, "status": "pending" });
    let upsert = command(
        "cmd1",
        "upsert_contact",
        json!({ "contact_id": null, "name": "Sam Lee", "phones": ["+61400000002"], "emails": [], "person": sam }),
    );
    let send = command(
        "cmd2",
        "send_sms",
        json!({ "to": "+61400000002", "body": "Bins out" }),
    );
    let delete = command("cmd3", "delete_contact", json!({ "contact_id": "c-new" }));

    for (c, switch) in [
        (&upsert, "contacts write"),
        (&send, "SMS send"),
        (&delete, "contacts write"),
    ] {
        let report = d.runtime.run_command(c);
        assert!(!report.done, "{report:?}");
        let message = report.result.as_str().unwrap();
        assert!(
            message.contains(&format!("turn on {switch} in Settings > Devices")),
            "{message}"
        );
        assert_eq!(report.body()["status"], "failed");
    }
    assert!(d.phone.actions().is_empty());

    d.settings(json!({
        "commands": ["send_sms", "upsert_contact", "delete_contact"],
        "contacts_write": true,
        "sms_send": true,
    }));
    let report = d.runtime.run_command(&upsert);
    assert!(report.done, "{report:?}");
    assert_eq!(report.id, "cmd1");
    assert_eq!(report.result, json!({ "contact_id": "c-new" }));
    assert_eq!(d.links(&sam), json!({ SLOT: "c-new" }));
    let report = d.runtime.run_command(&send);
    assert!(report.done, "{report:?}");
    assert_eq!(
        report.body(),
        json!({ "status": "done", "result": { "device_ref": "sms:900" } })
    );
    let report = d.runtime.run_command(&delete);
    assert!(report.done, "{report:?}");
    assert_eq!(d.links(&sam), Json::Null);
    let actions = d.phone.actions();
    assert_eq!(actions.len(), 3, "{actions:?}");
    assert!(actions[0].starts_with("upsert "));
    assert_eq!(actions[1], "sms +61400000002: Bins out");
    assert_eq!(actions[2], "delete c-new");
}

#[test]
fn an_upsert_for_a_person_not_synced_here_yet_still_adds_the_contact() {
    // The home server queues upsert_contact as a person is saved there, and the phone may run it
    // before the person reaches it: the contact is added, holding the person's id, and reported
    // done, so the server links the person.
    let d = Device::open();
    d.settings(json!({ "commands": ["upsert_contact"], "contacts_write": true }));
    let report = d.runtime.run_command(&json!({
        "id": "cmd1",
        "name": "upsert_contact",
        "payload": {
            "contact_id": null,
            "name": "Carla Created",
            "phones": ["+16502530005"],
            "emails": [],
            "person": "01PERSONNOTHEREYET00000000",
        },
        "status": "pending",
    }));
    assert!(report.done, "{report:?}");
    assert_eq!(report.result, json!({ "contact_id": "c-new" }));
    let upsert = last_upsert(&d.phone);
    assert_eq!(upsert["person"], "01PERSONNOTHEREYET00000000");
    assert_eq!(upsert["name"], "Carla Created");
    assert!(d.query("from person").is_empty());
}

#[test]
fn queued_events_survive_a_restart_and_failures_are_retried() {
    let dir = TempDir::new().unwrap();
    let open = || {
        Embedded::open(Config {
            vault: dir.path().to_owned(),
            web: None,
            secret_key: None,
        })
        .unwrap()
    };
    let config = || DeviceConfig {
        id: DEVICE.to_owned(),
        platform: "android".to_owned(),
        data: Arc::new(Phone::default()),
    };
    {
        let server = open();
        let runtime = server.device(config()).unwrap();
        runtime
            .enqueue("call_ended", json!({ "device_ref": "call:5", "direction": "in", "number": "+61400000001", "number_raw": "", "contact": null, "started": "2026-10-20T12:00:00+11:00", "duration": 1 }), 1)
            .unwrap();
        runtime.set_settings(&json!({ "sms_send": true })).unwrap();
        assert!(runtime.enqueue("sms_deleted", json!({}), 1).is_err());
        assert!(runtime.enqueue("sms_received", json!([]), 1).is_err());
        // A queue made before events recorded their notes takes the column on the next open.
        let db = server.writer().vault().server_dir().join("state.db");
        rusqlite::Connection::open(db)
            .unwrap()
            .execute_batch("ALTER TABLE device_events DROP COLUMN notes")
            .unwrap();
    }
    let server = open();
    let runtime = server.device(config()).unwrap();
    assert!(runtime.settings().sms_send, "settings are kept");
    let ran = runtime.run_pending().unwrap();
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0].status, EventStatus::Done, "{ran:?}");

    // A handler that fails leaves its event queued, then failed after the last attempt.
    server
        .writer()
        .put_script(
            "device_calls",
            "fn call_ended(event: Value) -> Result[(), string] { Err(\"broken\") }\n",
        )
        .unwrap();
    let id = runtime
        .enqueue("call_ended", json!({ "device_ref": "call:6" }), 2)
        .unwrap();
    for attempt in 1..=embedded::device::MAX_ATTEMPTS {
        let ran = runtime.run_pending().unwrap();
        let e = ran.iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.attempts, attempt);
        assert!(e.message.as_deref().unwrap().contains("broken"), "{e:?}");
        let expected = if attempt < embedded::device::MAX_ATTEMPTS {
            EventStatus::Queued
        } else {
            EventStatus::Failed
        };
        assert_eq!(e.status, expected);
    }
    assert!(runtime.run_pending().unwrap().is_empty());
    assert_eq!(
        runtime.event(id).unwrap().unwrap().status,
        EventStatus::Failed
    );
}

/// Sends `method path` to the embedded server, as the app's webview does, and expects `status`.
async fn app_json(server: &Embedded, method: &str, path: &str, status: u16) -> Json {
    let req = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .body(axum::body::Body::empty())
        .unwrap();
    let res = server.handle(req).await;
    let got = res.status().as_u16();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        got,
        status,
        "{method} {path}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap_or(Json::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handler_that_fails_three_times_is_listed_and_retry_reruns_it_and_dismiss_hides_it() {
    let dir = TempDir::new().unwrap();
    let server = Embedded::open(Config {
        vault: dir.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    // No device runtime yet: nothing to list, nothing to retry.
    let none = app_json(&server, "GET", "/api/app/device-events", 200).await;
    assert_eq!(none, json!({ "device": false, "failed": 0, "events": [] }));
    app_json(&server, "POST", "/api/app/device-events/1/retry", 404).await;

    let runtime = Arc::new(
        server
            .device(DeviceConfig {
                id: DEVICE.to_owned(),
                platform: "android".to_owned(),
                data: Arc::new(Phone::default()),
            })
            .unwrap(),
    );
    server.set_device(&runtime);
    let broken = "fn call_ended(event: Value) -> Result[(), string] { Err(\"the call note could not be written\") }\n";
    server.writer().put_script("device_calls", broken).unwrap();
    let call = |device_ref: &str| {
        json!({
            "device_ref": device_ref,
            "direction": "in",
            "number": "+61400000001",
            "number_raw": "0400 000 001",
            "contact": { "id": "c7", "name": "Ada Lovelace" },
            "started": "2026-10-20T12:00:00+11:00",
            "duration": 30,
        })
    };
    let first = runtime
        .enqueue("call_ended", call("call:1"), 1_760_000_000_000)
        .unwrap();
    let second = runtime
        .enqueue("call_ended", call("call:2"), 1_760_000_100_000)
        .unwrap();
    // A skipped event is listed too, with why, but is not counted as failed.
    let code = sms(
        "sms:9",
        Some("+61400000009"),
        "",
        "t9",
        "Your verification code is 482913",
    );
    let skipped = runtime
        .enqueue("sms_received", code, 1_760_000_050_000)
        .unwrap();
    for pass in 1..=embedded::device::MAX_ATTEMPTS {
        runtime.run_pending().unwrap();
        if pass < embedded::device::MAX_ATTEMPTS {
            assert!(runtime.new_failures().unwrap().is_empty(), "pass {pass}");
        }
    }
    assert_eq!(
        runtime.event(first).unwrap().unwrap().status,
        EventStatus::Failed
    );

    // Each failure is announced once, and reported once.
    let fresh: Vec<i64> = runtime
        .new_failures()
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(fresh, vec![first, second]);
    assert!(runtime.new_failures().unwrap().is_empty());
    assert_eq!(runtime.unreported_failures().unwrap().len(), 2);
    runtime.mark_reported(first).unwrap();
    let left: Vec<i64> = runtime
        .unreported_failures()
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(left, vec![second]);

    // Newest first, with who, the handler's message and the attempts.
    let listed = app_json(&server, "GET", "/api/app/device-events", 200).await;
    assert_eq!(listed["device"], true);
    assert_eq!(listed["failed"], 2);
    let events = listed["events"].as_array().unwrap();
    let ids: Vec<i64> = events.iter().map(|e| e["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![second, skipped, first]);
    assert_eq!(events[0]["event"], "call_ended");
    assert_eq!(events[0]["status"], "failed");
    assert_eq!(events[0]["at"], 1_760_000_100_000_i64);
    assert_eq!(events[0]["who"], "Ada Lovelace");
    assert_eq!(events[0]["attempts"], 3);
    assert!(
        events[0]["message"]
            .as_str()
            .unwrap()
            .contains("the call note could not be written"),
        "{listed}"
    );
    assert_eq!(events[1]["status"], "skipped");
    assert_eq!(events[1]["message"], "the body matches the skip pattern");
    assert_eq!(events[1]["who"], "+61400000009");

    // Retry gives it three more passes and runs the first now: fixed, it is done.
    server
        .writer()
        .put_script(
            "device_calls",
            "fn call_ended(event: Value) -> Result[(), string] { Ok(()) }\n",
        )
        .unwrap();
    let retried = app_json(
        &server,
        "POST",
        &format!("/api/app/device-events/{first}/retry"),
        200,
    )
    .await;
    assert_eq!(retried["status"], "done", "{retried}");
    assert_eq!(retried["attempts"], 1);
    assert_eq!(retried["failed"], 1);
    // Only a failed event can be retried or dismissed.
    app_json(
        &server,
        "POST",
        &format!("/api/app/device-events/{first}/retry"),
        404,
    )
    .await;
    app_json(
        &server,
        "POST",
        &format!("/api/app/device-events/{skipped}/dismiss"),
        404,
    )
    .await;

    // Retried and failing again, it is announced and reported again once it fails for good.
    server.writer().put_script("device_calls", broken).unwrap();
    let again = runtime.retry(second).unwrap().unwrap();
    assert_eq!((again.status, again.attempts), (EventStatus::Queued, 1));
    runtime.run_pending().unwrap();
    runtime.run_pending().unwrap();
    assert_eq!(
        runtime.event(second).unwrap().unwrap().status,
        EventStatus::Failed
    );
    assert_eq!(runtime.new_failures().unwrap().len(), 1);
    assert_eq!(runtime.unreported_failures().unwrap().len(), 1);

    // Dismiss hides it from the list and the count.
    let dismissed = app_json(
        &server,
        "POST",
        &format!("/api/app/device-events/{second}/dismiss"),
        200,
    )
    .await;
    assert_eq!(dismissed["failed"], 0);
    let listed = app_json(&server, "GET", "/api/app/device-events", 200).await;
    assert_eq!(listed["failed"], 0);
    let ids: Vec<i64> = listed["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![skipped]);
    assert_eq!(
        runtime.event(second).unwrap().unwrap().status,
        EventStatus::Dismissed
    );
    assert!(runtime.unreported_failures().unwrap().is_empty());
    assert!(runtime.run_pending().unwrap().is_empty());
}
