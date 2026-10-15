//! The Android core against a real home server (`crates/testkit`): pairing by password and by
//! code, a share made offline that reaches the server, the foreground sync loop, and assets
//! fetched on demand (D2, D19, D20).
//!
//! The core is one per process, so the tests take turns: each opens its own vault folder, which
//! replaces the previous test's core.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde_json::{Value, json};
use tauri_plugin_vault_device::core::{self, Core, Link};
use tauri_plugin_vault_device::pair::{self, Paired, Secret};
use tauri_plugin_vault_device::share::{Shared, SharedFile};
use tempfile::TempDir;
use testkit::{PASSWORD, TestServer};

static TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

struct Admin {
    client: Client,
    server: TestServer,
}

impl Admin {
    fn start() -> Admin {
        Admin {
            client: Client::new(),
            server: TestServer::start(),
        }
    }

    fn send(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let rb = self
            .client
            .request(method.parse().unwrap(), self.server.url(path))
            .bearer_auth(self.server.admin_token());
        let rb = match body {
            Some(b) => rb.json(&b),
            None => rb,
        };
        let res = rb.send().unwrap();
        let status = res.status();
        let body: Value = res.json().unwrap_or(Value::Null);
        assert!(status.is_success(), "{method} {path}: {status} {body}");
        body
    }

    fn upload(&self, filename: &str, bytes: &[u8]) -> String {
        let res = self
            .client
            .post(self.server.url("/api/assets"))
            .query(&[("filename", filename)])
            .bearer_auth(self.server.admin_token())
            .body(bytes.to_vec())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.status());
        let body: Value = res.json().unwrap();
        body["id"]
            .as_str()
            .or(body["asset"]["id"].as_str())
            .unwrap()
            .to_owned()
    }

    fn pair(&self, name: &str) -> Paired {
        pair::pair(
            &self.server.base_url(),
            &Secret::Password(PASSWORD.to_owned()),
            name,
        )
        .unwrap()
    }
}

fn link(p: &Paired, cache_mb: u64) -> Link {
    Link {
        server_url: p.server_url.clone(),
        token: p.token.clone(),
        device_id: p.device_id.clone(),
        device_name: p.device_name.clone(),
        cache_mb,
    }
}

fn open(dir: &TempDir) -> Arc<Core> {
    core::open(&core::vault_dir(dir.path())).unwrap()
}

fn file(dir: &Path, name: &str, bytes: &[u8]) -> SharedFile {
    let path = dir.join(format!("copy-{name}"));
    std::fs::write(&path, bytes).unwrap();
    SharedFile {
        path,
        filename: name.to_owned(),
        mime: String::new(),
    }
}

/// The asset kinds of the note's body blocks, in order.
fn body_kinds(note: &Value) -> Vec<String> {
    note["blocks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn pairs_by_password_and_by_code_and_refuses_bad_secrets() {
    let _turn = turn();
    let admin = Admin::start();
    let url = format!("{}/", admin.server.base_url());

    let by_password = pair::pair(&url, &Secret::Password(PASSWORD.into()), "Pixel").unwrap();
    assert_eq!(by_password.server_url, admin.server.base_url());
    assert_eq!(by_password.device_name, "Pixel");
    assert!(!by_password.token.is_empty());

    let wrong = pair::pair(&url, &Secret::Password("nope".into()), "Pixel").unwrap_err();
    assert!(wrong.to_string().contains("wrong password"), "{wrong}");

    let issued = admin.send("POST", "/api/devices/pair-codes", Some(json!({})));
    let code = issued["code"].as_str().unwrap();
    let by_code = pair::pair(&url, &Secret::Code(code.into()), "Tablet").unwrap();
    assert_ne!(by_code.device_id, by_password.device_id);
    let again = pair::pair(&url, &Secret::Code(code.into()), "Tablet").unwrap_err();
    assert!(again.to_string().contains("pairing code"), "{again}");

    let devices = admin.send("GET", "/api/devices", None);
    let names: Vec<&str> = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Pixel") && names.contains(&"Tablet"),
        "{devices}"
    );
    assert!(devices["devices"][0]["platform"] == "android", "{devices}");

    let unreachable = pair::pair(
        "http://127.0.0.1:9",
        &Secret::Password(PASSWORD.into()),
        "Pixel",
    )
    .unwrap_err();
    assert!(
        unreachable.to_string().contains("cannot reach"),
        "{unreachable}"
    );
}

#[test]
fn a_share_made_offline_is_queued_then_reaches_the_server() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = open(&dir);
    // As the app opens it before pairing.
    core.device("unpaired", || Arc::new(NoPhone)).unwrap();

    // Not paired yet: the share is stored and queued all the same.
    let shared = Shared {
        text: "Read this https://example.net/post".into(),
        subject: String::new(),
        files: vec![
            file(dir.path(), "photo.jpg", b"jpeg bytes"),
            file(dir.path(), "notes.pdf", b"%PDF-1.4"),
        ],
    };
    let outcome = core.share(&shared).unwrap();
    // The note comes from the default `share_received` handler, run through the device queue.
    assert_eq!(outcome.status, "done", "{outcome:?}");
    let event = outcome.event.expect("the share ran as a device event");
    let e = core.runtime().unwrap().event(event).unwrap().unwrap();
    assert_eq!(e.event, "share_received");
    assert_eq!(e.data["url"], "https://example.net/post");
    let attached: Vec<&str> = e.data["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["filename"].as_str().unwrap())
        .collect();
    assert_eq!(attached, ["photo.jpg", "notes.pdf"]);
    assert_eq!(e.data["attachments"][0]["mime"], "image/jpeg");
    assert_eq!(outcome.notes.len(), 1, "exactly one note: {outcome:?}");
    let id = outcome.notes[0].clone();
    let queued = core.queued().unwrap();
    let queued = queued.as_array().unwrap();
    assert_eq!(queued.len(), 1, "{queued:?}");
    let note = &queued[0];
    assert_eq!(note["id"], id.as_str());
    assert_eq!(note["type"], "fleeting");
    assert_eq!(note["fields"]["processed"], false);
    assert_eq!(
        note["fields"]["title"],
        "Read this https://example.net/post"
    );
    let files: Vec<&str> = note["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["filename"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["photo.jpg", "notes.pdf"]);
    assert_eq!(body_kinds(note), ["paragraph", "asset"]);
    assert!(
        !shared.files[0].path.exists(),
        "the copy is removed once stored"
    );

    // Paired: one round pushes the assets and the note.
    let admin = Admin::start();
    core.set_link(Some(link(&admin.pair("Pixel"), 64)));
    let report = core.sync_now().unwrap();
    assert_eq!(report.pushed, vec![id.clone()]);
    assert_eq!(core.status().unwrap()["pending"], 0);
    let on_server = admin.send("GET", &format!("/api/notes/{id}"), None);
    assert_eq!(on_server["type"], "fleeting", "{on_server}");
    let note = &on_server["note"];
    let image = note["fields"]["image"][0].as_str().unwrap();
    let pdf = note["blocks"][0]["blocks"][1]["fields"]["asset"]
        .as_str()
        .unwrap();
    for a in [image, pdf] {
        let res = admin
            .client
            .get(admin.server.url(&format!("/api/assets/{a}")))
            .bearer_auth(admin.server.admin_token())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "asset {a}: {}", res.status());
    }
}

#[test]
fn an_empty_share_is_refused() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = open(&dir);
    let err = core.share(&Shared::default()).unwrap_err();
    assert!(err.to_string().contains("nothing was shared"), "{err}");
    assert_eq!(core.queued().unwrap(), json!([]));
}

#[test]
fn a_write_kicks_the_foreground_loop() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = open(&dir);
    let admin = Admin::start();
    core.set_link(Some(link(&admin.pair("Pixel"), 64)));
    let id = core
        .share(&Shared {
            text: "kick".into(),
            ..Shared::default()
        })
        .unwrap()
        .notes
        .remove(0);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = core.status().unwrap();
        if status["pending"] == 0 && status["last_sync_ms"].is_i64() {
            assert_eq!(status["last_error"], Value::Null, "{status}");
            break;
        }
        assert!(Instant::now() < deadline, "no round ran: {status}");
        std::thread::sleep(Duration::from_millis(100));
    }
    admin.send("GET", &format!("/api/notes/{id}"), None);
}

/// Ticket 82: with the app in front, a note changed on the server reaches the local vault within
/// seconds, through the live event stream and a pull round, with no local write and no timer.
#[test]
fn in_front_a_server_change_is_pulled_at_once() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = open(&dir);
    let admin = Admin::start();
    core.set_link(Some(link(&admin.pair("Pixel"), 64)));
    core.set_foreground(true);
    let wait = |what: &str, ok: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !ok() {
            assert!(
                Instant::now() < deadline,
                "{what}: {}",
                core.status().unwrap()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // Opening the stream asks for a round.
    wait("the stream opens and a round runs", &|| {
        let s = core.status().unwrap();
        s["live"] == true && s["last_sync_ms"].is_i64()
    });
    let first = core.status().unwrap()["last_sync_ms"].clone();

    let note = admin.send(
        "POST",
        "/api/notes?type=fleeting",
        Some(json!({ "note": { "fields": { "title": "From the web" }, "blocks": [] } })),
    );
    let id = note["id"].as_str().unwrap().to_owned();
    let writer = core.server().writer();
    wait("the change is pulled by a round of its own", &|| {
        writer.read(&id).is_ok() && core.status().unwrap()["last_sync_ms"] != first
    });

    // Out of the front, the stream closes within a heartbeat.
    core.set_foreground(false);
    wait("the stream closes", &|| {
        core.status().unwrap()["live"] == false
    });
}

/// Ticket 78: what the background capture path does once a receiver has queued an SMS and a
/// call. `RunEventsWorker` runs the handlers, and the prompt `SyncWorker` round it schedules runs
/// `sync_now`, which pushes the notes the handlers wrote and empties the queue.
#[test]
fn background_captures_reach_the_server_in_one_round() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = open(&dir);
    let admin = Admin::start();
    let paired = admin.pair("Pixel");
    core.set_link(Some(link(&paired, 64)));
    let runtime = core
        .device(&paired.device_id, || Arc::new(NoPhone))
        .unwrap();

    runtime
        .enqueue(
            "sms_received",
            json!({
                "device_ref": "sms:1",
                "thread_ref": "7",
                "number": "+61400000001",
                "number_raw": "0400 000 001",
                "contact": null,
                "body": "Captured in the background",
                "sent": "2026-10-15T09:30:00+11:00",
            }),
            1_760_000_000_000,
        )
        .unwrap();
    runtime
        .enqueue(
            "call_ended",
            json!({
                "device_ref": "call:1",
                "number": "+61400000001",
                "number_raw": "0400 000 001",
                "contact": null,
                "direction": "in",
                "duration": 30,
                "started": "2026-10-15T09:31:00+11:00",
            }),
            1_760_000_000_000,
        )
        .unwrap();
    for e in runtime.run_pending().unwrap() {
        assert_eq!(e.status.as_str(), "done", "{e:?}");
    }
    let queued = core.queued().unwrap();
    let types: Vec<&str> = queued
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"message"), "{types:?}");
    assert!(types.contains(&"call"), "{types:?}");
    let waiting = core.status().unwrap()["pending"].as_u64().unwrap();
    assert_eq!(waiting as usize, types.len());

    let report = core.sync_now().unwrap();
    assert_eq!(report.pushed.len(), types.len(), "{report:?}");
    assert_eq!(core.status().unwrap()["pending"], 0);
    for note in queued.as_array().unwrap() {
        let id = note["id"].as_str().unwrap();
        let on_server = admin.send("GET", &format!("/api/notes/{id}"), None);
        assert_eq!(on_server["type"], note["type"], "{on_server}");
    }
}

#[test]
fn unpaired_sync_is_refused_and_assets_come_on_demand() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = open(&dir);
    let err = core.sync_now().unwrap_err();
    assert!(err.to_string().contains("not paired"), "{err}");

    let admin = Admin::start();
    let asset = admin.upload("big.bin", &vec![7u8; 3 * 1024 * 1024]);
    // A 1 MiB cache: the asset no note opened recently names is not kept.
    core.set_link(Some(link(&admin.pair("Pixel"), 1)));
    let report = core.sync_now().unwrap();
    assert_eq!(report.assets_fetched, 0);
    let writer = core.server().writer();
    assert!(writer.asset_info(&asset).is_err(), "not fetched ahead");

    assert!(core.fetch_asset(&asset).unwrap());
    assert_eq!(writer.asset_info(&asset).unwrap().size, 3 * 1024 * 1024);
    assert!(!core.fetch_asset("01J00000000000000000000000").unwrap());

    // Over the cache size, the next round evicts it again.
    let report = core.sync_now().unwrap();
    assert_eq!(report.assets_evicted, 1);
    assert!(writer.asset_info(&asset).is_err());
}

/// A phone with nothing on it; the share handler never asks it anything.
struct NoPhone;

impl embedded::device::DeviceData for NoPhone {
    fn sms_list(&self, _: &Value) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }
    fn sms_get(&self, r: &str) -> Result<Value, String> {
        Err(format!("no SMS {r}"))
    }
    fn calls_list(&self, _: &Value) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }
    fn contacts_list(&self, _: &Value) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }
    fn contacts_get(&self, c: &str) -> Result<Value, String> {
        Err(format!("no contact {c}"))
    }
    fn contacts_upsert(&self, _: &Value) -> Result<Value, String> {
        Err("no contacts".into())
    }
    fn contacts_delete(&self, _: &str) -> Result<(), String> {
        Err("no contacts".into())
    }
    fn sms_send(&self, _: &str, _: &str) -> Result<Value, String> {
        Err("no SMS".into())
    }
    fn show_notification(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A vault folder whose `scripts/device_share.ws` titles each share's note "From my handler".
fn vault_with_share_handler(dir: &TempDir) -> std::path::PathBuf {
    let vault = core::vault_dir(dir.path());
    std::fs::create_dir_all(vault.join("scripts")).unwrap();
    std::fs::write(
        vault.join("scripts/device_share.ws"),
        r#"use vault

fn share_received(event: Value) -> Result[(), string] {
    vault::create("fleeting", Value::Map(#{ "fields": Value::Map(#{ "title": Value::String("From my handler") }) }))?
    Ok(())
}
"#,
    )
    .unwrap();
    vault
}

#[test]
fn a_share_runs_the_users_share_handler() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let core = core::open(&vault_with_share_handler(&dir)).unwrap();
    core.device("01PHONE", || Arc::new(NoPhone)).unwrap();
    let outcome = core
        .share(&Shared {
            text: "hello".into(),
            ..Shared::default()
        })
        .unwrap();
    assert!(outcome.event.is_some(), "{outcome:?}");
    assert_eq!(outcome.notes.len(), 1, "{outcome:?}");
    let queued = core.queued().unwrap();
    assert_eq!(queued.as_array().unwrap().len(), 1, "one note: {queued}");
    assert_eq!(queued[0]["id"], outcome.notes[0].as_str());
    assert_eq!(queued[0]["fields"]["title"], "From my handler", "{queued}");
}

#[test]
fn a_handler_that_fails_keeps_the_share_queued_and_makes_no_note() {
    let _turn = turn();
    let dir = TempDir::new().unwrap();
    let vault = core::vault_dir(dir.path());
    std::fs::create_dir_all(vault.join("scripts")).unwrap();
    std::fs::write(
        vault.join("scripts/device_share.ws"),
        "fn share_received(event: Value) -> Result[(), string] { Err(\"offline service\") }\n",
    )
    .unwrap();
    let core = core::open(&vault).unwrap();
    core.device("01PHONE", || Arc::new(NoPhone)).unwrap();
    let outcome = core
        .share(&Shared {
            text: "keep me".into(),
            ..Shared::default()
        })
        .unwrap();
    assert_eq!(outcome.status, "queued", "{outcome:?}");
    assert!(outcome.message.unwrap().contains("offline service"));
    assert!(outcome.notes.is_empty());
    assert_eq!(
        core.queued().unwrap(),
        json!([]),
        "no fallback note as well"
    );
    let e = core
        .runtime()
        .unwrap()
        .event(outcome.event.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        e.data["text"], "keep me",
        "the share waits for the next pass"
    );
}

#[test]
fn with_no_share_handler_the_share_falls_back_to_a_fleeting_note() {
    let _turn = turn();
    // No device runtime open: the note is written directly.
    let dir = TempDir::new().unwrap();
    let core = core::open(&vault_with_share_handler(&dir)).unwrap();
    let outcome = core
        .share(&Shared {
            text: "no runtime *yet*".into(),
            files: vec![file(dir.path(), "scan.pdf", b"%PDF-1.4")],
            ..Shared::default()
        })
        .unwrap();
    assert_eq!(outcome.event, None);
    assert_eq!(outcome.notes.len(), 1);
    let queued = core.queued().unwrap();
    assert_eq!(queued.as_array().unwrap().len(), 1, "{queued}");
    assert_eq!(queued[0]["fields"]["title"], "no runtime *yet*");
    assert_eq!(queued[0]["fields"]["processed"], false);
    assert_eq!(queued[0]["assets"][0]["filename"], "scan.pdf");
    assert_eq!(body_kinds(&queued[0]), ["paragraph", "asset"]);

    // A device file that declares no `share_received` handler: the same fallback, once.
    let dir = TempDir::new().unwrap();
    let core = core::open(&vault_with_share_handler(&dir)).unwrap();
    let writer = core.server().writer();
    writer
        .put_script(
            "device_calls",
            writer::default_device_script("device_calls").unwrap(),
        )
        .unwrap();
    writer
        .put_device_file(
            "android",
            "handler \"call_ended\" {\n  script = \"device_calls\"\n}\n",
        )
        .unwrap();
    let runtime = core.device("01PHONE", || Arc::new(NoPhone)).unwrap();
    let outcome = core
        .share(&Shared {
            text: "nobody handles this".into(),
            ..Shared::default()
        })
        .unwrap();
    assert_eq!(outcome.event, None, "{outcome:?}");
    let queued = core.queued().unwrap();
    assert_eq!(queued.as_array().unwrap().len(), 1, "{queued}");
    assert_eq!(queued[0]["id"], outcome.notes[0].as_str());
    assert_eq!(queued[0]["fields"]["title"], "nobody handles this");
    assert!(runtime.events().unwrap().is_empty(), "nothing queued");
}
