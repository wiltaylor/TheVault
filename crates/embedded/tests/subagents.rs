//! S3: a sub agent's sessions follow their parent thread whichever side changes it (subagents U5).
//!
//! A device node built from `crates/embedded`, paired with a testkit home server. Archiving,
//! unarchiving or deleting a parent thread through the app's note routes does the same to its
//! sessions in the same local write, and the home server holds the same after the sync. A
//! parent change that reaches the home server without its sessions, as an older app pushes it,
//! takes them along there, and a session already where it should be is left as it is.

use axum::body::Body;
use axum::http::{Request, header};
use embedded::{Config, Embedded};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use sync::{AssetPolicy, Remote};
use tempfile::TempDir;
use testkit::{PASSWORD, TestServer};

/// A caller of the home server with one token: the admin's or a device's.
struct Admin {
    client: Client,
    base: String,
    token: String,
}

impl Admin {
    fn of(server: &TestServer) -> Admin {
        Admin {
            client: Client::new(),
            base: server.base_url(),
            token: server.admin_token().to_owned(),
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>, status: u16) -> Value {
        let mut rb = self
            .client
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            rb = rb.json(&body);
        }
        let res = rb.send().unwrap();
        let got = res.status().as_u16();
        let text = res.text().unwrap();
        assert_eq!(got, status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn status(&self, path: &str) -> u16 {
        self.client
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .unwrap()
            .status()
            .as_u16()
    }
}

/// A device: an embedded server on its own local vault, paired with the home server.
struct Device {
    embedded: Embedded,
    remote: Remote,
    runtime: tokio::runtime::Runtime,
    _vault: TempDir,
}

impl Device {
    fn pair(server: &TestServer, name: &str) -> Device {
        let out: Value = Client::new()
            .post(server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": name, "platform": "linux" }))
            .send()
            .unwrap()
            .json()
            .unwrap();
        let vault = tempfile::Builder::new()
            .prefix("thevault-device-")
            .tempdir()
            .unwrap();
        let embedded = Embedded::open(Config {
            vault: vault.path().to_owned(),
            web: None,
            secret_key: None,
        })
        .unwrap();
        let remote = Remote {
            url: server.base_url(),
            token: out["token"].as_str().unwrap().to_owned(),
            device_id: out["device_id"].as_str().unwrap().to_owned(),
            device_name: name.to_owned(),
        };
        embedded.set_remote(Some(remote.clone()));
        Device {
            embedded,
            remote,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            _vault: vault,
        }
    }

    /// The home server as this device's own token calls it, as its sync pushes.
    fn token(&self) -> Admin {
        Admin {
            client: Client::new(),
            base: self.remote.url.clone(),
            token: self.remote.token.clone(),
        }
    }

    /// One request to the device's own server, as its webview makes it.
    fn call(&self, method: &str, path: &str, body: Option<Value>, status: u16) -> Value {
        let mut req = Request::builder().method(method).uri(path);
        let body = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let req = req.body(body).unwrap();
        let (got, bytes) = self.runtime.block_on(async {
            let res = self.embedded.handle(req).await;
            let got = res.status().as_u16();
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            (got, bytes)
        });
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(got, status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn status(&self, path: &str) -> u16 {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();
        self.runtime
            .block_on(self.embedded.handle(req))
            .status()
            .as_u16()
    }

    /// One sync round, on a thread of its own: the engine blocks.
    fn sync(&self) -> sync::Report {
        let engine = self
            .embedded
            .sync_engine(self.remote.clone(), AssetPolicy::desktop())
            .unwrap();
        std::thread::scope(|s| s.spawn(|| engine.sync().unwrap()).join().unwrap())
    }
}

/// A parent thread of agent `lead` and its two sessions, made on the home server as a turn there
/// makes them: `(parent, [session of toolu_0, session of toolu_1])`.
fn family() -> (TestServer, String, [String; 2]) {
    let parent = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA".to_owned();
    let subs = [
        "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB".to_owned(),
        "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC".to_owned(),
    ];
    let listed: Vec<Value> = subs
        .iter()
        .zip(["toolu_0", "toolu_1"])
        .enumerate()
        .map(|(i, (id, call))| {
            json!({
                "call": call, "thread": id, "agent": "lead", "task": "Look around",
                "n": i + 1, "status": "done", "started_ms": 0
            })
        })
        .collect();
    let server = TestServer::start_seeded(|vault| {
        let vault = store::Vault::init(vault).unwrap();
        let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
        let writer = writer::Writer::open(vault, state).unwrap();
        writer
            .upgrade_shipped_defaults(writer::Upkeep::Home)
            .unwrap();
        for (i, id) in std::iter::once(&parent).chain(subs.iter()).enumerate() {
            let (origin, transcript) = if i == 0 {
                (
                    "user".to_owned(),
                    json!({"messages": [], "subagents": listed}),
                )
            } else {
                (
                    format!("subagent:{parent}"),
                    json!({"messages": [], "parent": {
                        "thread": parent, "turn": "t1", "call": format!("toolu_{}", i - 1)
                    }}),
                )
            };
            writer.agent_thread_runtime().create_with_id(id, &writer::Input::Json(json!({
                "fields": {"title": "Check", "agent": "lead", "origin": origin, "transcript": transcript.to_string()}
            })), &state::Author::Agent("lead".into())).unwrap();
        }
    });
    (server, parent, subs)
}

fn version(note: &Value) -> String {
    note["version"].as_str().unwrap().to_owned()
}

fn archived(note: &Value) -> bool {
    note["note"]["fields"]["archived"] == true
}

#[test]
fn writer_refuses_offline_transcript_content_edits() {
    let (server, parent, _) = family();
    let admin = Admin::of(&server);
    let device = Device::pair(&server, "Phone");
    device.sync();
    let writer = device.embedded.writer();
    let note = writer.read(&parent).unwrap();
    let mut doc = writer::doc_to_json(&note.doc);
    doc["fields"]["transcript"] =
        json!("{\"messages\": [], \"proposals\": [{\"id\": \"forged\"}]}");
    let rejected = writer
        .update(
            &parent,
            &writer::Input::Json(doc),
            Some(&note.version),
            &state::Author::Session("local".into()),
        );
    assert!(matches!(rejected, Err(writer::Error::ServerOwnedThread)));
    let engine = device
        .embedded
        .sync_engine(device.remote.clone(), AssetPolicy::desktop())
        .unwrap();
    let result = std::thread::scope(|s| s.spawn(|| engine.sync()).join().unwrap());
    let report = result.unwrap();
    assert!(report.held.is_empty());
    assert!(device.embedded.queue().pending().unwrap().is_empty());
    let home = admin.call("GET", &format!("/api/notes/{parent}"), None, 200);
    assert!(
        !home["note"]["fields"]["transcript"]
            .as_str()
            .unwrap()
            .contains("forged")
    );
}

/// The author of note `id`'s current version on the home server.
fn author(admin: &Admin, id: &str) -> Value {
    let history = admin.call("GET", &format!("/api/notes/{id}/history"), None, 200);
    history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["current"] == true)
        .unwrap()["author"]
        .clone()
}

#[test]
fn offline_archive_unarchive_and_delete_sync_thread_metadata_without_content_writes() {
    let (server, parent, subs) = family();
    let admin = Admin::of(&server);
    let device = Device::pair(&server, "Laptop");
    device.sync();

    // Offline archive queues only a metadata change; sessions follow locally.
    device.embedded.set_remote(None);
    let note = device.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let path = format!(
        "/api/notes/{parent}/archive?base_version={}",
        version(&note)
    );
    device.call("POST", &path, Some(json!({})), 200);
    for s in &subs {
        let local = device.call("GET", &format!("/api/notes/{s}"), None, 200);
        assert!(archived(&local), "{local}");
    }
    let queued: Vec<String> = device
        .embedded
        .queue()
        .pending()
        .unwrap()
        .into_iter()
        .map(|p| p.note_id)
        .collect();
    for s in &subs {
        assert!(queued.contains(s), "{queued:?}");
    }
    device.embedded.set_remote(Some(device.remote.clone()));
    let report = device.sync();
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert!(device.embedded.queue().pending().unwrap().is_empty());
    for id in subs.iter().chain([&parent]) {
        let home = admin.call("GET", &format!("/api/notes/{id}"), None, 200);
        assert!(archived(&home), "{home}");
    }

    // Offline unarchive follows the same metadata-only path.
    device.embedded.set_remote(None);
    let note = device.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let path = format!(
        "/api/notes/{parent}/unarchive?base_version={}",
        version(&note)
    );
    device.call("POST", &path, Some(json!({})), 200);
    for s in &subs {
        let local = device.call("GET", &format!("/api/notes/{s}"), None, 200);
        assert!(!archived(&local), "{local}");
    }
    device.embedded.set_remote(Some(device.remote.clone()));
    let report = device.sync();
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    for id in subs.iter().chain([&parent]) {
        let home = admin.call("GET", &format!("/api/notes/{id}"), None, 200);
        assert!(!archived(&home), "{home}");
    }

    // Deleted in the app: the sessions go to the trash with it, here and on the server.
    let note = device.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let path = format!("/api/notes/{parent}?base_version={}", version(&note));
    device.call("DELETE", &path, None, 200);
    for s in &subs {
        assert_eq!(device.status(&format!("/api/notes/{s}")), 404);
    }
    device.embedded.set_remote(Some(device.remote.clone()));
    let report = device.sync();
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert!(device.embedded.queue().pending().unwrap().is_empty());
    let trash = admin.call("GET", "/api/trash", None, 200).to_string();
    for id in subs.iter().chain([&parent]) {
        assert_eq!(admin.status(&format!("/api/notes/{id}")), 404);
        assert!(trash.contains(id.as_str()), "{trash}");
    }
}

#[test]
fn a_parent_change_pushed_without_its_sessions_takes_them_along_on_the_server() {
    let (server, parent, subs) = family();
    let admin = Admin::of(&server);
    let device = Device::pair(&server, "Phone");
    device.sync();
    // An older app pushes only the parent, with the device's token.
    let old_app = device.token();

    // The first session was archived on its own already: it is left at its version.
    let first = admin.call("GET", &format!("/api/notes/{}", subs[0]), None, 200);
    let path = format!(
        "/api/notes/{}/archive?base_version={}",
        subs[0],
        version(&first)
    );
    let first = admin.call("POST", &path, Some(json!({})), 200);
    let note = admin.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let path = format!(
        "/api/notes/{parent}/archive?base_version={}",
        version(&note)
    );
    old_app.call("POST", &path, Some(json!({})), 200);
    let now = admin.call("GET", &format!("/api/notes/{}", subs[0]), None, 200);
    assert_eq!(
        version(&now),
        version(&first),
        "an archived session stays as it is"
    );
    let second = admin.call("GET", &format!("/api/notes/{}", subs[1]), None, 200);
    assert!(archived(&second), "{second}");
    assert_eq!(author(&admin, &subs[1]), author(&admin, &parent));
    // The same archive again changes nothing.
    let note = admin.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let path = format!(
        "/api/notes/{parent}/archive?base_version={}",
        version(&note)
    );
    let again = old_app.call("POST", &path, Some(json!({})), 200);
    assert_eq!(version(&again), version(&note));
    assert_eq!(
        version(&admin.call("GET", &format!("/api/notes/{}", subs[1]), None, 200)),
        version(&second)
    );
    device.sync();
    for s in &subs {
        let local = device.call("GET", &format!("/api/notes/{s}"), None, 200);
        assert!(archived(&local), "{local}");
    }

    // A truncate past the second call, pushed as a plain write, deletes that call's session.
    let note = admin.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let mut transcript: Value =
        serde_json::from_str(note["note"]["fields"]["transcript"].as_str().unwrap()).unwrap();
    transcript["subagents"].as_array_mut().unwrap().truncate(1);
    old_app.call(
        "PUT",
        &format!("/api/notes/{parent}"),
        Some(
            json!({ "base_version": version(&note), "note": { "fields": {
            "title": "Check", "agent": "lead", "origin": "user",
            "transcript": transcript.to_string(),
        } } }),
        ),
        403,
    );
    assert_eq!(admin.status(&format!("/api/notes/{}", subs[1])), 200);
    assert_eq!(admin.status(&format!("/api/notes/{}", subs[0])), 200);

    // A deletion of the parent alone takes the remaining session to the trash.
    let note = admin.call("GET", &format!("/api/notes/{parent}"), None, 200);
    let path = format!("/api/notes/{parent}?base_version={}", version(&note));
    old_app.call("DELETE", &path, None, 200);
    assert_eq!(admin.status(&format!("/api/notes/{}", subs[0])), 404);
    device.embedded.set_remote(Some(device.remote.clone()));
    let report = device.sync();
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    for id in subs.iter().chain([&parent]) {
        assert_eq!(device.status(&format!("/api/notes/{id}")), 404);
    }
}
