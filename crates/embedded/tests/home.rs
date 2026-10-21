//! S3: the routes an app's embedded server sends on to its home server (D2, D18, D20).
//!
//! A device node built from `crates/embedded`, paired with a testkit home server. Settings >
//! Devices, the notifications and their event stream, task runs and the full history come from
//! the home server's `state.db`, not the device's; the notes stay local and work offline.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, header};
use embedded::{Config, Embedded};
use futures_util::StreamExt;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use sync::{AssetPolicy, Remote};
use tempfile::TempDir;
use testkit::{PASSWORD, TestServer};

const MEMO: &str = r#"label = "Memo"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn seed(vault: &std::path::Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

/// The home server, called with its admin token.
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
}

/// A device: an embedded server on its own local vault, paired with the home server when asked.
struct Device {
    embedded: Embedded,
    remote: Remote,
    runtime: tokio::runtime::Runtime,
    _vault: TempDir,
}

impl Device {
    /// A device paired with `server` under `name`, its embedded server told so.
    fn pair(server: &TestServer, name: &str) -> Device {
        Device::pair_as(server, name, "linux")
    }

    /// [`Device::pair`] as a device of `platform`.
    fn pair_as(server: &TestServer, name: &str, platform: &str) -> Device {
        let out: Value = Client::new()
            .post(server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": name, "platform": platform }))
            .send()
            .unwrap()
            .json()
            .unwrap();
        let vault = tempfile::Builder::new()
            .prefix("thevault-device-")
            .tempdir()
            .unwrap();
        seed(vault.path());
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

    /// One sync round, on a thread of its own: the engine blocks.
    fn sync(&self) -> sync::Report {
        let engine = self
            .embedded
            .sync_engine(self.remote.clone(), AssetPolicy::desktop())
            .unwrap();
        std::thread::scope(|s| s.spawn(|| engine.sync().unwrap()).join().unwrap())
    }

    /// Point the embedded server at an address nothing listens on: the home server is offline.
    fn go_offline(&self) {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = closed.local_addr().unwrap();
        drop(closed);
        self.embedded.set_remote(Some(Remote {
            url: format!("http://{addr}"),
            ..self.remote.clone()
        }));
    }
}

fn ids(list: &Value, key: &str) -> Vec<String> {
    list[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {list}"))
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn settings_devices_lists_the_home_servers_devices_and_says_which_is_this_one() {
    let server = TestServer::start_seeded(seed);
    let admin = Admin::of(&server);
    let laptop = Device::pair(&server, "Laptop");
    let _phone = Device::pair(&server, "Pixel");

    let home = admin.call("GET", "/api/devices", None, 200);
    let here = laptop.call("GET", "/api/devices", None, 200);
    assert_eq!(ids(&here, "devices"), ids(&home, "devices"), "{here}");
    assert_eq!(ids(&here, "devices").len(), 2, "{here}");
    // Each device call moves its own `last_seen`; the rest is the server's list as it is.
    let without_seen = |v: &Value| {
        let mut v = v.clone();
        for d in v["devices"].as_array_mut().unwrap() {
            d.as_object_mut().unwrap().remove("last_seen");
        }
        v
    };
    assert_eq!(without_seen(&here), without_seen(&home));

    let app = laptop.call("GET", "/api/app", None, 200);
    assert_eq!(app["app"], true, "{app}");
    assert_eq!(app["paired"], true, "{app}");
    assert_eq!(app["device_id"], json!(laptop.remote.device_id), "{app}");
    assert_eq!(app["device_name"], "Laptop", "{app}");

    // Renaming or revoking needs admin, which a device token never has (D2).
    let path = format!("/api/devices/{}", laptop.remote.device_id);
    let refused = laptop.call("PATCH", &path, Some(json!({ "name": "Mine" })), 403);
    assert_eq!(refused["error"], "needs_web_app", "{refused}");
    assert_eq!(refused["message"], embedded::NEEDS_WEB_APP_MESSAGE);
    let refused = laptop.call("GET", "/api/tokens", None, 403);
    assert_eq!(refused["error"], "needs_web_app", "{refused}");
    let refused = laptop.call("GET", "/api/secrets", None, 403);
    assert_eq!(refused["error"], "needs_web_app", "{refused}");
    assert_eq!(
        without_seen(&admin.call("GET", "/api/devices", None, 200)),
        without_seen(&home)
    );

    // Revoked on the server, the device is no longer paired there.
    admin.call("DELETE", &path, None, 204);
    let gone = laptop.call("GET", "/api/devices", None, 503);
    assert_eq!(gone["error"], "not_paired", "{gone}");
}

#[test]
fn the_bell_reads_and_changes_the_home_servers_notifications() {
    let server = TestServer::start_seeded(seed);
    let admin = Admin::of(&server);
    let device = Device::pair(&server, "Laptop");

    let first = admin.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "One" })),
        201,
    );
    admin.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "Two" })),
        201,
    );
    let here = device.call("GET", "/api/notifications", None, 200);
    assert_eq!(here["unread"], 2, "{here}");
    assert_eq!(here, admin.call("GET", "/api/notifications", None, 200));

    // Read here, it is read on the server.
    let id = first["id"].as_str().unwrap();
    let read = device.call("POST", &format!("/api/notifications/{id}/read"), None, 200);
    assert_eq!(read["unread"], 1, "{read}");
    assert_eq!(
        admin.call("GET", "/api/notifications", None, 200)["unread"],
        1
    );

    // Dismissed and cleared here, they go on the server.
    device.call("DELETE", &format!("/api/notifications/{id}"), None, 200);
    device.call("POST", "/api/notifications/read-all", None, 200);
    device.call("DELETE", "/api/notifications", None, 200);
    let home = admin.call("GET", "/api/notifications", None, 200);
    assert_eq!(home["items"], json!([]), "{home}");

    // A device script's notification is made in the local state.db: the next round moves it.
    device
        .embedded
        .writer()
        .notify(&state::NewNotification {
            title: "From a device script".to_owned(),
            body: None,
            note_id: None,
            url: None,
            source: "script:capture".to_owned(),
        })
        .unwrap();
    let report = device.sync();
    assert_eq!(report.notifications_pushed, 1, "{report:?}");
    let home = admin.call("GET", "/api/notifications", None, 200);
    assert_eq!(home["items"][0]["title"], "From a device script", "{home}");
    let here = device.call("GET", "/api/notifications", None, 200);
    assert_eq!(here, home);
    let local = device
        .embedded
        .writer()
        .state()
        .notifications(false, 10, None);
    assert!(local.unwrap().items.is_empty(), "moved, not copied");
}

#[test]
fn the_event_stream_carries_the_servers_notifications_and_the_devices_note_changes() {
    let server = TestServer::start_seeded(seed);
    let admin = Admin::of(&server);
    let device = Device::pair(&server, "Laptop");

    let res = device.runtime.block_on(
        device
            .embedded
            .handle(Request::get("/api/events").body(Body::empty()).unwrap()),
    );
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut body = res.into_body().into_data_stream();
    let mut seen = String::new();
    let mut read_until = |want: &dyn Fn(&str) -> bool, seen: &mut String| {
        device.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(20), async {
                while !want(seen) {
                    let chunk = body.next().await.expect("the stream ended").unwrap();
                    seen.push_str(&String::from_utf8_lossy(&chunk));
                }
            })
            .await
            .unwrap_or_else(|_| panic!("timed out; the stream so far:\n{seen}"));
        });
    };
    read_until(&|s| s.contains(": open"), &mut seen);

    // A notification on the server streams through.
    let n = admin.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "Live" })),
        201,
    );
    let id = n["id"].as_str().unwrap().to_owned();
    read_until(&|s| s.contains("event: notification.created"), &mut seen);
    assert!(seen.contains(&id), "{seen}");

    // A note written on the device streams with the device's own version; the server's
    // note.changed would name a version the device does not hold.
    let note = device.call(
        "POST",
        "/api/notes?type=memo",
        Some(json!({ "note": { "fields": { "title": "Local" } } })),
        201,
    );
    let version = note["version"].as_str().unwrap().to_owned();
    read_until(&|s| s.contains(&version), &mut seen);
    assert!(seen.contains("event: note.changed"), "{seen}");
    assert!(!seen.contains("\nid:"), "ids are left out: {seen}");
}

#[test]
fn offline_or_unpaired_the_server_routes_say_so_and_the_notes_still_work() {
    let server = TestServer::start_seeded(seed);
    let device = Device::pair(&server, "Laptop");
    device.go_offline();

    for path in ["/api/devices", "/api/notifications", "/api/clients"] {
        let out = device.call("GET", path, None, 503);
        assert_eq!(out["error"], "offline", "{path}: {out}");
        assert_eq!(out["message"], embedded::OFFLINE_MESSAGE, "{path}: {out}");
    }
    let out = device.call("GET", "/api/events", None, 503);
    assert_eq!(out["error"], "offline", "{out}");

    // The vault's files are served from the local copy.
    let note = device.call(
        "POST",
        "/api/notes?type=memo",
        Some(json!({ "note": { "fields": { "title": "Written offline" } } })),
        201,
    );
    let id = note["id"].as_str().unwrap();
    let read = device.call("GET", &format!("/api/notes/{id}"), None, 200);
    assert_eq!(read["note"]["fields"]["title"], "Written offline");
    let found = device.call("GET", "/api/search?q=offline", None, 200);
    assert_eq!(found["total"], 1, "{found}");
    device.call("GET", "/api/types", None, 200);
    device.call("GET", "/api/pins", None, 200);
    device.call("GET", "/api/trash", None, 200);
    // The scheduled tasks list falls back to the local task files.
    device.call("GET", "/api/tasks", None, 200);
    // The history is the device's own, and says so.
    let history = device.call("GET", &format!("/api/notes/{id}/history"), None, 200);
    assert_eq!(history["source"], "device", "{history}");
    assert_eq!(
        history["versions"].as_array().unwrap().len(),
        1,
        "{history}"
    );

    device.embedded.set_remote(None);
    let out = device.call("GET", "/api/devices", None, 503);
    assert_eq!(out["error"], "not_paired", "{out}");
    assert_eq!(out["message"], embedded::NOT_PAIRED_MESSAGE, "{out}");
    let out = device.call("POST", "/api/tasks/nightly/run", None, 503);
    assert_eq!(out["error"], "not_paired", "{out}");
    let app = device.call("GET", "/api/app", None, 200);
    assert_eq!(app["paired"], false, "{app}");
    device.call("GET", &format!("/api/notes/{id}"), None, 200);
}

#[test]
fn the_history_is_the_servers_and_a_server_version_restores_as_a_local_write() {
    let server = TestServer::start_seeded(seed);
    let admin = Admin::of(&server);
    let device = Device::pair(&server, "Laptop");

    // Two versions on the server; the device pulls only the latest.
    let created = admin.call(
        "POST",
        "/api/notes?type=memo",
        Some(json!({ "note": { "fields": { "title": "First" } } })),
        201,
    );
    let id = created["id"].as_str().unwrap().to_owned();
    let first = created["version"].as_str().unwrap().to_owned();
    admin.call(
        "PUT",
        &format!("/api/notes/{id}"),
        Some(json!({ "note": { "fields": { "title": "Second" } }, "base_version": first })),
        200,
    );
    device.sync();

    let path = format!("/api/notes/{id}/history");
    let history = device.call("GET", &path, None, 200);
    assert_eq!(history["source"], "server", "{history}");
    let versions: Vec<&str> = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap())
        .collect();
    assert!(versions.contains(&first.as_str()), "{history}");
    let local = device.embedded.writer().history(&id).unwrap();
    assert_eq!(local.len(), 1, "the device recorded only what it pulled");

    // A server version the device never held previews from the server.
    let old = device.call("GET", &format!("{path}/{first}"), None, 200);
    assert_eq!(old["note"]["fields"]["title"], "First", "{old}");

    // Restoring it writes its content here, over the local version, and it syncs.
    let current = device.call("GET", &format!("/api/notes/{id}"), None, 200);
    let restored = device.call(
        "POST",
        &format!("/api/notes/{id}/restore"),
        Some(json!({ "version": first, "base_version": current["version"] })),
        200,
    );
    assert_eq!(restored["note"]["fields"]["title"], "First", "{restored}");
    assert_eq!(device.embedded.queue().pending().unwrap().len(), 1);
    device.sync();
    let home = admin.call("GET", &format!("/api/notes/{id}"), None, 200);
    assert_eq!(home["note"]["fields"]["title"], "First", "{home}");

    // A stale base is still a conflict.
    let stale = device.call(
        "POST",
        &format!("/api/notes/{id}/restore"),
        Some(json!({ "version": first, "base_version": current["version"] })),
        409,
    );
    assert_eq!(stale["error"], "conflict", "{stale}");

    // A note the server does not have yet shows the device's history.
    let note = device.call(
        "POST",
        "/api/notes?type=memo",
        Some(json!({ "note": { "fields": { "title": "Not pushed" } } })),
        201,
    );
    let mine = note["id"].as_str().unwrap();
    let history = device.call("GET", &format!("/api/notes/{mine}/history"), None, 200);
    assert_eq!(history["source"], "device", "{history}");

    // Task runs are the server's; running a task now needs admin there.
    let runs = device.call("GET", "/api/tasks/nothing/runs", None, 200);
    assert_eq!(
        runs,
        admin.call("GET", "/api/tasks/nothing/runs", None, 200)
    );
    let refused = device.call("POST", "/api/tasks/nothing/run", None, 403);
    assert_eq!(refused["error"], "needs_web_app", "{refused}");
}

/// The default device file and scripts, as the starter vault holds them, beside the test type.
fn seed_devices(vault: &std::path::Path) {
    seed(vault);
    std::fs::create_dir_all(vault.join("devices")).unwrap();
    std::fs::create_dir_all(vault.join("scripts")).unwrap();
    std::fs::write(
        vault.join("devices/android.wcl"),
        writer::DEFAULT_ANDROID_DEVICE_FILE,
    )
    .unwrap();
    for (name, source) in writer::DEFAULT_DEVICE_SCRIPTS {
        std::fs::write(vault.join(format!("scripts/{name}.ws")), source).unwrap();
    }
}

#[test]
fn a_phone_asks_itself_to_import_through_the_home_server() {
    let server = TestServer::start_seeded(seed_devices);
    let admin = Admin::of(&server);
    let phone = Device::pair_as(&server, "Pixel", "android");
    let tablet = Device::pair_as(&server, "Tablet", "android");
    let own = format!("/api/devices/{}/commands", phone.remote.device_id);
    let ask = json!({ "name": "import_history", "payload": { "kinds": ["sms"] } });

    // Not enabled yet: the home server's refusal comes back as it is.
    let off = phone.call("POST", &own, Some(ask.clone()), 400);
    assert!(
        off["message"].as_str().unwrap().contains("is not enabled"),
        "{off}"
    );

    for d in [&phone, &tablet] {
        admin.call(
            "PATCH",
            &format!("/api/devices/{}", d.remote.device_id),
            Some(json!({ "settings": { "commands": ["import_history", "show_notification"] } })),
            200,
        );
    }
    let queued = phone.call("POST", &own, Some(ask.clone()), 201);
    assert_eq!(queued["name"], "import_history", "{queued}");
    let on_server = admin.call("GET", &own, None, 200);
    assert_eq!(
        ids(&on_server, "commands"),
        [queued["id"].as_str().unwrap()]
    );
    assert_eq!(on_server["commands"][0]["payload"], ask["payload"]);

    // Another device, or another command, needs the web app.
    let other = format!("/api/devices/{}/commands", tablet.remote.device_id);
    phone.call("POST", &other, Some(ask), 403);
    phone.call(
        "POST",
        &own,
        Some(json!({ "name": "show_notification", "payload": {} })),
        403,
    );
    assert!(ids(&admin.call("GET", &other, None, 200), "commands").is_empty());
}

#[test]
fn the_app_says_its_build_and_asks_the_home_server_for_its_version() {
    let server = TestServer::start_seeded(seed);
    let device = Device::pair(&server, "Laptop");

    // Before the app says which build it is, the build fields are null.
    let app = device.call("GET", "/api/app", None, 200);
    assert_eq!(app["version"], Value::Null, "{app}");
    device.embedded.set_app(embedded::AppBuild {
        version: "9.8.7".to_owned(),
        platform: "linux-appimage".to_owned(),
        build: "release".to_owned(),
    });
    let app = device.call("GET", "/api/app", None, 200);
    assert_eq!(app["version"], "9.8.7", "{app}");
    assert_eq!(app["platform"], "linux-appimage", "{app}");
    assert_eq!(app["build"], "release", "{app}");
    assert_eq!(app["device_name"], "Laptop", "{app}");

    // `/api/version` is the home server's, not the embedded server's own.
    let version = device.call("GET", "/api/version", None, 200);
    assert_eq!(
        version,
        json!({ "server_version": env!("CARGO_PKG_VERSION") })
    );

    device.go_offline();
    let out = device.call("GET", "/api/version", None, 503);
    assert_eq!(out["error"], "offline", "{out}");
    device.embedded.set_remote(None);
    let out = device.call("GET", "/api/version", None, 503);
    assert_eq!(out["error"], "not_paired", "{out}");
    // The app build stays.
    let app = device.call("GET", "/api/app", None, 200);
    assert_eq!(app["version"], "9.8.7", "{app}");
}

/// An app's sync loop as a test drives it: it says what it is told to, and counts the rounds it
/// was asked for.
#[derive(Clone, Default)]
struct FakeLoop {
    report: std::sync::Arc<std::sync::Mutex<Option<embedded::SyncReport>>>,
    asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeLoop {
    fn set(&self, phase: embedded::SyncPhase, message: Option<&str>, last_ok_ms: Option<u64>) {
        *self.report.lock().unwrap() = Some(embedded::SyncReport {
            phase,
            message: message.map(str::to_owned),
            last_ok_ms,
            last_attempt_ms: last_ok_ms,
            server: Some("http://home.test".to_owned()),
            warning: None,
        });
    }
}

impl embedded::SyncSource for FakeLoop {
    fn report(&self) -> embedded::SyncReport {
        self.report
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(embedded::SyncReport::unpaired)
    }

    fn sync_now(&self) {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn the_sync_status_says_synced_syncing_with_writes_waiting_or_error() {
    use embedded::SyncPhase;
    let server = TestServer::start_seeded(seed);
    let device = Device::pair(&server, "Laptop");

    // With no sync loop handed over, the app is not syncing at all, and cannot be asked to.
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "error", "{out}");
    assert_eq!(out["phase"], "unpaired", "{out}");
    let out = device.call("POST", "/api/app/sync", None, 503);
    assert_eq!(out["error"], "not_paired", "{out}");

    let fake = FakeLoop::default();
    device.embedded.set_sync_source(Box::new(fake.clone()));

    // Before the first round finishes: yellow.
    fake.set(SyncPhase::Starting, None, None);
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "syncing", "{out}");

    // Idle after a round, nothing queued: green, with the time and the server.
    fake.set(SyncPhase::Synced, None, Some(1_700_000_000_000));
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(
        out,
        json!({
            "state": "synced",
            "phase": "synced",
            "queued": 0,
            "held": 0,
            "refused_notes": [],
            "assets_waiting": 0,
            "stuck_assets": [],
            "last_ok_ms": 1_700_000_000_000u64,
            "last_attempt_ms": 1_700_000_000_000u64,
            "last_error": null,
            "server": "http://home.test",
            "warning": null,
            // No round has run on this device: no progress yet (ticket 144).
            "progress": null,
        })
    );

    // A local write waits for the next round: yellow, counted from the queue.
    device.call(
        "POST",
        "/api/notes?type=memo",
        Some(json!({ "note": { "fields": { "title": "Waiting" }, "blocks": [] } })),
        201,
    );
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "syncing", "{out}");
    assert_eq!(out["queued"], 1, "{out}");

    // A round runs: yellow.
    fake.set(SyncPhase::Syncing, None, Some(1_700_000_000_000));
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "syncing", "{out}");

    // The round pushed the write: green again.
    device.sync();
    fake.set(SyncPhase::Synced, None, Some(1_700_000_100_000));
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "synced", "{out}");
    assert_eq!(out["queued"], 0, "{out}");

    // The server cannot be reached, or wants a newer app: red, with the reason.
    fake.set(
        SyncPhase::Offline,
        Some("cannot reach the server: refused"),
        Some(1_700_000_100_000),
    );
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "error", "{out}");
    assert_eq!(out["phase"], "offline", "{out}");
    assert_eq!(
        out["last_error"], "cannot reach the server: refused",
        "{out}"
    );
    assert_eq!(out["last_ok_ms"], 1_700_000_100_000u64, "{out}");
    fake.set(SyncPhase::UpdateRequired, Some("Update the app"), None);
    let out = device.call("GET", "/api/app/sync", None, 200);
    assert_eq!(out["state"], "error", "{out}");
    assert_eq!(out["phase"], "update_required", "{out}");

    // Sync now asks the loop for a round.
    let out = device.call("POST", "/api/app/sync", None, 202);
    assert_eq!(out["phase"], "update_required", "{out}");
    assert_eq!(fake.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
}
