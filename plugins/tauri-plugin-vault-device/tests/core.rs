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
    }
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

    // Not paired yet: the share is stored and queued all the same.
    let shared = Shared {
        text: "Read this https://example.net/post".into(),
        subject: String::new(),
        files: vec![
            file(dir.path(), "photo.jpg", b"jpeg bytes"),
            file(dir.path(), "notes.pdf", b"%PDF-1.4"),
        ],
    };
    let id = core.share(&shared).unwrap();
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
    let images = on_server["note"]["fields"]["image"].as_array().unwrap();
    assert_eq!(images.len(), 2);
    for a in images {
        let a = a.as_str().unwrap();
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
        .unwrap();
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
