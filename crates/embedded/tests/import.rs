//! S4: importing the phone's history again, against a fake phone (D7, D22). The import makes a
//! note for each message and call that has none, leaves alone each that has one, whatever the
//! event queue remembers having run, and reads only the kinds and the dates asked for.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use embedded::device::{
    DeviceConfig, DeviceData, DeviceRuntime, EventStatus, ImportReport, ImportRequest,
};
use embedded::{Config, Embedded};
use script::Vault;
use serde_json::{Value as Json, json};
use state::Author;
use tempfile::TempDir;
use writer::Writer;

const DEVICE: &str = "01PHONEDEVICE0000000000000";

/// 2026-10-01T00:00:00Z, and a day.
const OCT_1: i64 = 1_790_812_800_000;
const DAY: i64 = 24 * 60 * 60 * 1000;

/// One item a provider holds: its key, its time, and the event capture makes of it.
#[derive(Clone)]
struct Item {
    key: i64,
    at: i64,
    event: &'static str,
    data: Json,
}

/// A phone whose providers hold [`Item`]s, which answers `history` page by page.
#[derive(Default)]
struct Phone {
    items: Mutex<HashMap<&'static str, Vec<Item>>>,
    denied: Mutex<Vec<&'static str>>,
    /// Every `history` request, in order.
    asked: Mutex<Vec<Json>>,
    /// Every contact upsert, in order.
    upserts: Mutex<Vec<Json>>,
}

impl Phone {
    fn hold(&self, kind: &'static str, item: Item) {
        self.items
            .lock()
            .unwrap()
            .entry(kind)
            .or_default()
            .push(item);
    }

    fn deny(&self, kind: &'static str) {
        self.denied.lock().unwrap().push(kind);
    }

    fn asked_kinds(&self) -> Vec<String> {
        let mut kinds: Vec<String> = self
            .asked
            .lock()
            .unwrap()
            .iter()
            .map(|a| a["kind"].as_str().unwrap().to_owned())
            .collect();
        kinds.dedup();
        kinds
    }
}

impl DeviceData for Phone {
    fn sms_list(&self, _: &Json) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }
    fn sms_get(&self, device_ref: &str) -> Result<Json, String> {
        Err(format!("no SMS {device_ref}"))
    }
    fn calls_list(&self, _: &Json) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }
    fn contacts_list(&self, _: &Json) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }
    fn contacts_get(&self, contact_id: &str) -> Result<Json, String> {
        Err(format!("no contact {contact_id}"))
    }
    fn contacts_upsert(&self, contact: &Json) -> Result<Json, String> {
        self.upserts.lock().unwrap().push(contact.clone());
        Ok(json!({ "contact_id": contact["contact_id"] }))
    }
    fn contacts_delete(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn sms_send(&self, _: &str, _: &str) -> Result<Json, String> {
        Err("not here".into())
    }
    fn show_notification(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn history(&self, request: &Json) -> Result<Json, String> {
        self.asked.lock().unwrap().push(request.clone());
        let kind = request["kind"].as_str().unwrap_or_default();
        if self.denied.lock().unwrap().contains(&kind) {
            let permission = match kind {
                "calls" => "android.permission.READ_CALL_LOG",
                "contacts" => "android.permission.READ_CONTACTS",
                _ => "android.permission.READ_SMS",
            };
            return Ok(json!({ "events": [], "next": null, "missing_permission": permission }));
        }
        let after = request["after"].as_i64().unwrap_or(i64::MIN);
        let limit = request["limit"].as_u64().unwrap_or(100) as usize;
        let since = request["since"].as_i64();
        let until = request["until"].as_i64();
        let mut items: Vec<Item> = self
            .items
            .lock()
            .unwrap()
            .get(kind)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|i| i.key > after)
            .filter(|i| since.is_none_or(|s| i.at >= s) && until.is_none_or(|u| i.at <= u))
            .collect();
        items.sort_by_key(|i| i.key);
        items.truncate(limit);
        let next = if items.len() == limit {
            json!(items.last().unwrap().key)
        } else {
            Json::Null
        };
        let events: Vec<Json> = items
            .iter()
            .map(|i| json!({ "event": i.event, "data": i.data, "at": i.at }))
            .collect();
        Ok(json!({ "events": events, "next": next, "missing_permission": null }))
    }
}

fn sms(key: i64, at: i64, body: &str) -> Item {
    Item {
        key,
        at,
        event: "sms_received",
        data: json!({
            "device_ref": format!("sms:{key}"),
            "thread_ref": "7",
            "number": "+61400000001",
            "number_raw": "0400 000 001",
            "contact": null,
            "body": body,
            "sent": "2026-10-02T09:30:00+10:00",
        }),
    }
}

fn call(key: i64, at: i64) -> Item {
    Item {
        key,
        at,
        event: "call_ended",
        data: json!({
            "device_ref": format!("call:{key}"),
            "direction": "in",
            "number": "+61400000001",
            "number_raw": "0400 000 001",
            "contact": null,
            "started": "2026-10-02T12:00:00+10:00",
            "duration": 30,
        }),
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

    fn query(&self, q: &str) -> Vec<Json> {
        Vault::query(&**self.writer(), q, 10_000).unwrap()
    }

    /// The notes of `type_name` with `device_ref`, archived ones too.
    fn notes_of(&self, type_name: &str, device_ref: &str) -> Vec<Json> {
        self.query(&format!(
            "from {type_name} include archived where device_ref == \"{device_ref}\""
        ))
    }

    /// Capture `item` the way live capture does: queue its event and run the queue.
    fn capture(&self, item: &Item) {
        self.runtime
            .enqueue(item.event, item.data.clone(), item.at)
            .unwrap();
        for e in self.runtime.run_pending().unwrap() {
            assert_eq!(e.status, EventStatus::Done, "{e:?}");
        }
    }

    fn import(&self, request: Json) -> ImportReport {
        let req = ImportRequest::from_json(&request).unwrap();
        self.runtime.import_history(&req, &mut |_| {}).unwrap()
    }

    fn version(&self, id: &str) -> String {
        Vault::get(&**self.writer(), id).unwrap()["version"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn delete(&self, id: &str) {
        let version = self.version(id);
        self.writer()
            .delete(id, Some(&version), &Author::Session("test".into()))
            .unwrap();
    }
}

#[test]
fn an_import_makes_notes_only_for_items_with_none_and_only_within_the_dates() {
    let d = Device::open();
    d.phone.hold("sms", sms(1, OCT_1 + DAY, "One"));
    d.phone.hold("sms", sms(2, OCT_1 + 2 * DAY, "Two"));
    d.phone.hold("sms", sms(3, OCT_1 + 3 * DAY, "Three"));
    // Before the range.
    d.phone.hold("sms", sms(4, OCT_1 - DAY, "Old"));
    d.phone.hold("calls", call(1, OCT_1 + DAY));
    d.phone.hold("calls", call(2, OCT_1 + 2 * DAY));
    // After the range.
    d.phone.hold("calls", call(3, OCT_1 + 40 * DAY));
    // Live capture already filed SMS 1 and call 1.
    d.capture(&sms(1, OCT_1 + DAY, "One"));
    d.capture(&call(1, OCT_1 + DAY));
    assert_eq!(d.query("from message").len(), 1);

    let report = d.import(json!({
        "kinds": ["sms", "calls"],
        "since": "2026-10-01T00:00:00Z",
        "until": "2026-10-31T00:00:00Z",
    }));
    assert!(report.done);
    let sms_report = report.kind("sms").unwrap();
    assert_eq!(
        (sms_report.found, sms_report.existing, sms_report.imported),
        (3, 1, 2)
    );
    assert!(sms_report.notes >= 2, "{sms_report:?}");
    let calls = report.kind("calls").unwrap();
    assert_eq!((calls.found, calls.existing, calls.imported), (2, 1, 1));
    assert!(report.kind("mms").is_none());

    for r in ["sms:1", "sms:2", "sms:3"] {
        assert_eq!(d.notes_of("message", r).len(), 1, "{r}");
    }
    assert!(d.notes_of("message", "sms:4").is_empty());
    assert_eq!(d.notes_of("call", "call:1").len(), 1);
    assert_eq!(d.notes_of("call", "call:2").len(), 1);
    assert!(d.notes_of("call", "call:3").is_empty());
    assert_eq!(d.phone.asked_kinds(), ["sms", "calls"]);

    // Importing the same again finds everything filed.
    let again = d.import(json!({ "kinds": ["sms", "calls"] }));
    let s = again.kind("sms").unwrap();
    assert_eq!((s.found, s.existing, s.imported, s.notes), (4, 3, 1, 1));
    assert_eq!(d.notes_of("message", "sms:4").len(), 1);
    assert_eq!(d.query("from message").len(), 4);
    assert_eq!(d.query("from call").len(), 3);
}

#[test]
fn a_deleted_note_comes_back_exactly_once_though_its_event_ran_before() {
    let d = Device::open();
    let item = sms(10, OCT_1, "Bins out tonight");
    d.phone.hold("sms", item.clone());
    d.capture(&item);
    let first = d.notes_of("message", "sms:10");
    assert_eq!(first.len(), 1);
    // The queue remembers that sms:10 ran.
    assert!(
        d.runtime
            .events()
            .unwrap()
            .iter()
            .any(|e| e.data["device_ref"] == "sms:10" && e.status == EventStatus::Done)
    );

    d.delete(first[0]["id"].as_str().unwrap());
    assert!(d.notes_of("message", "sms:10").is_empty());

    let report = d.import(json!({ "kinds": ["sms"] }));
    let s = report.kind("sms").unwrap();
    assert_eq!((s.found, s.existing, s.imported), (1, 0, 1));
    let back = d.notes_of("message", "sms:10");
    assert_eq!(back.len(), 1);
    assert_ne!(back[0]["id"], first[0]["id"]);

    let twice = d.import(json!({ "kinds": ["sms"] }));
    assert_eq!(twice.kind("sms").unwrap().existing, 1);
    assert_eq!(d.notes_of("message", "sms:10").len(), 1);
}

#[test]
fn an_archived_note_counts_as_there() {
    let d = Device::open();
    let item = call(5, OCT_1);
    d.phone.hold("calls", item.clone());
    d.capture(&item);
    let id = d.notes_of("call", "call:5")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let version = d.version(&id);
    d.writer()
        .set_archived(&id, true, Some(&version), &Author::Session("test".into()))
        .unwrap();
    assert!(d.query("from call").is_empty());

    let report = d.import(json!({ "kinds": ["calls"] }));
    assert_eq!(report.kind("calls").unwrap().existing, 1);
    assert_eq!(d.notes_of("call", "call:5").len(), 1);
}

#[test]
fn only_the_kinds_asked_for_are_read() {
    let d = Device::open();
    d.phone.hold("sms", sms(1, OCT_1, "Hi"));
    d.phone.hold("calls", call(1, OCT_1));
    let report = d.import(json!({ "kinds": ["calls"] }));
    assert_eq!(report.kinds.len(), 1);
    assert_eq!(d.phone.asked_kinds(), ["calls"]);
    assert!(d.query("from message").is_empty());
    assert_eq!(d.query("from call").len(), 1);
}

#[test]
fn long_histories_arrive_page_by_page_with_progress_after_each() {
    let d = Device::open();
    for key in 1..=205 {
        d.phone.hold("calls", call(key, OCT_1 + key));
    }
    let mut heard: Vec<(u64, bool)> = Vec::new();
    let req = ImportRequest::from_json(&json!({ "kinds": ["calls"] })).unwrap();
    let report = d
        .runtime
        .import_history(&req, &mut |r| {
            heard.push((r.kinds[0].found, r.done));
        })
        .unwrap();
    assert_eq!(report.kind("calls").unwrap().imported, 205);
    assert_eq!(d.query("from call").len(), 205);
    let pages: Vec<Json> = d.phone.asked.lock().unwrap().clone();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["after"], Json::Null);
    assert_eq!(pages[1]["after"], 200);
    // Before anything, after each page, after the kind, and at the end.
    assert_eq!(heard.first(), Some(&(0, false)));
    assert!(heard.contains(&(200, false)));
    assert_eq!(heard.last(), Some(&(205, true)));
}

#[test]
fn a_missing_permission_is_reported_and_the_other_kinds_still_import() {
    let d = Device::open();
    d.phone.hold("sms", sms(1, OCT_1, "Hi"));
    d.phone.deny("calls");
    let report = d.import(json!({ "kinds": ["sms", "calls"] }));
    let calls = report.kind("calls").unwrap();
    assert_eq!(
        calls.missing_permission.as_deref(),
        Some("android.permission.READ_CALL_LOG")
    );
    assert!(calls.done);
    assert_eq!(report.kind("sms").unwrap().imported, 1);
    assert!(!report.nothing_read());
    assert!(
        report
            .summary()
            .contains("calls: not read, Android has not given TheVault")
    );
}

#[test]
fn excluded_numbers_are_skipped_not_filed() {
    let d = Device::open();
    d.phone.hold("sms", sms(1, OCT_1, "Hi"));
    d.runtime
        .set_settings(&json!({ "exclusions": [{ "kind": "number", "value": "+61400000001" }] }))
        .unwrap();
    let report = d.import(json!({ "kinds": ["sms"] }));
    let s = report.kind("sms").unwrap();
    assert_eq!((s.found, s.skipped, s.imported), (1, 1, 0));
    assert!(d.query("from message").is_empty());
}

#[test]
fn the_import_history_command_runs_only_when_enabled_and_reports_the_import() {
    let d = Device::open();
    d.phone.hold("sms", sms(1, OCT_1, "Hi"));
    let command = json!({
        "id": "01COMMAND00000000000000000",
        "name": "import_history",
        "payload": { "kinds": ["sms"], "since": "2026-09-01T00:00:00Z" },
    });
    let refused = d.runtime.run_command(&command);
    assert!(!refused.done);
    assert!(
        refused.result.as_str().unwrap().contains("is not enabled"),
        "{refused:?}"
    );
    assert!(d.phone.asked.lock().unwrap().is_empty());

    d.runtime
        .set_settings(&json!({ "commands": ["import_history"] }))
        .unwrap();
    let mut progress = Vec::new();
    let ran = d
        .runtime
        .run_command_with(&command, &mut |p| progress.push(p.clone()));
    assert!(ran.done, "{ran:?}");
    assert_eq!(ran.result["kinds"][0]["kind"], "sms");
    assert_eq!(ran.result["kinds"][0]["imported"], 1);
    assert_eq!(ran.result["done"], true);
    assert_eq!(ran.result["since"], 1_788_220_800_000_i64);
    assert!(
        ran.result["message"]
            .as_str()
            .unwrap()
            .starts_with("SMS: 1 found")
    );
    assert_eq!(progress.first().unwrap()["done"], false);
    assert_eq!(progress.last().unwrap()["done"], true);
    assert_eq!(d.notes_of("message", "sms:1").len(), 1);

    // A request the script cannot run fails with the reason.
    let rcs = d.runtime.run_command(&json!({
        "id": "01COMMAND00000000000000001",
        "name": "import_history",
        "payload": { "kinds": ["rcs"] },
    }));
    assert!(!rcs.done);
    assert!(
        rcs.result
            .as_str()
            .unwrap()
            .starts_with("RCS messages cannot be imported")
    );

    // Nothing readable at all fails, saying why.
    d.phone.deny("sms");
    let denied = d.runtime.run_command(&command);
    assert!(!denied.done);
    assert!(
        denied.result["message"]
            .as_str()
            .unwrap()
            .contains("android.permission.READ_SMS"),
        "{denied:?}"
    );
}

/// A contact as the phone's reader reports one whose link row belongs to another vault: as never
/// linked, `person` and `synced` null (ticket 143).
fn contact_from_another_vault(key: i64, at: i64, name: &str, phone: &str) -> Item {
    Item {
        key,
        at,
        event: "contact_changed",
        data: json!({
            "contact_id": format!("c{key}"),
            "name": name,
            "phones": [phone],
            "phone_labels": ["mobile"],
            "emails": [],
            "email_labels": [],
            "updated": "2026-10-02T09:30:00Z",
            "person": null,
            "synced": null,
        }),
    }
}

#[test]
fn contacts_linked_to_another_vault_import_as_fresh_people_and_queue_only_creates() {
    let d = Device::open();
    d.runtime
        .set_settings(&json!({ "contacts_write": true }))
        .unwrap();
    // The ids the contacts' link rows named in the vault the phone synced with before.
    let old_ids = ["01M54NTRRY755C5BKDRZPEVDEC", "01M54NTRRY755C5BKDRZPEVDED"];
    d.phone.hold(
        "contacts",
        contact_from_another_vault(1, OCT_1 + DAY, "Ada Lovelace", "+61400000011"),
    );
    d.phone.hold(
        "contacts",
        contact_from_another_vault(2, OCT_1 + 2 * DAY, "Grace Hopper", "+61400000012"),
    );

    let report = d.import(json!({ "kinds": ["contacts"] }));
    assert!(report.done, "{report:?}");

    let people = d.query("from person");
    assert_eq!(people.len(), 2, "{people:?}");
    let ids: Vec<String> = people
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect();
    for id in &ids {
        assert!(!old_ids.contains(&id.as_str()), "a fresh id: {id}");
    }
    // Every queued change is a create: nothing is based on a version this vault never had.
    let queued = d.server.queue().pending().unwrap();
    assert_eq!(queued.len(), 2, "{queued:?}");
    for p in &queued {
        assert_eq!(p.base_version, None, "{p:?}");
        assert!(ids.contains(&p.note_id), "{p:?}");
    }
    // Each contact's link row is written again for the new person.
    let upserts = d.phone.upserts.lock().unwrap().clone();
    let mut linked: Vec<(String, String)> = upserts
        .iter()
        .map(|u| {
            (
                u["contact_id"].as_str().unwrap().to_owned(),
                u["person"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    linked.sort();
    linked.dedup();
    assert_eq!(linked.len(), 2, "{upserts:?}");
    for (_, person) in &linked {
        assert!(ids.contains(person), "{linked:?}");
    }
}
