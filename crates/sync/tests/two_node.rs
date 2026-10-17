//! S3, the two-node sync harness: a device node built from `crates/embedded` and a testkit home
//! server, in one test (D19, D20, D27, D28, D29).
//!
//! The device's own edits go through `Embedded::handle`, as its webview's do. The server's go
//! through its HTTP API, as a browser's or another device's do. Sync rounds run on the test
//! thread, outside any async runtime, since the engine blocks.

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, header};
use embedded::{Config, Embedded};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderValue, SET_COOKIE};
use serde_json::{Value, json};
use state::{PrunePolicy, State};
use sync::{AssetPolicy, Engine, Error, Remote, Supported};
use tempfile::TempDir;
use testkit::{PASSWORD, TestServer};

const MEMO: &str = r#"label = "Memo"

field a {
  kind = "string"
}

field b {
  kind = "string"
}

field pic {
  kind = "asset"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "a"
    slot "b"
    slot "body"
  }
}
"#;

const TEMPLATE: &str = "\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

/// A client of the home server: the admin token, a browser session, or a device token.
struct Http {
    client: Client,
    base: String,
}

impl Http {
    fn with_header(server: &TestServer, name: reqwest::header::HeaderName, value: &str) -> Http {
        let mut headers = HeaderMap::new();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        Http {
            client: Client::builder().default_headers(headers).build().unwrap(),
            base: server.base_url(),
        }
    }

    fn admin(server: &TestServer) -> Http {
        Http::with_header(
            server,
            AUTHORIZATION,
            &format!("Bearer {}", server.admin_token()),
        )
    }

    /// A browser session, whose saves merge into its own recent version.
    fn browser(server: &TestServer) -> Http {
        let res = Client::new()
            .post(server.url("/api/login"))
            .json(&json!({ "password": PASSWORD }))
            .send()
            .unwrap();
        assert_eq!(res.status(), 200);
        let cookie = res.headers()[SET_COOKIE].to_str().unwrap();
        let pair = cookie.split(';').next().unwrap().to_owned();
        Http::with_header(server, COOKIE, &pair)
    }

    fn send(&self, rb: RequestBuilder, status: u16) -> Value {
        let res = rb.send().unwrap();
        let got = res.status().as_u16();
        let text = res.text().unwrap();
        assert_eq!(got, status, "{text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn get(&self, path: &str, status: u16) -> Value {
        self.send(self.client.get(format!("{}{path}", self.base)), status)
    }

    fn post(&self, path: &str, body: Value, status: u16) -> Value {
        self.send(
            self.client.post(format!("{}{path}", self.base)).json(&body),
            status,
        )
    }

    fn put(&self, path: &str, body: Value, status: u16) -> Value {
        self.send(
            self.client.put(format!("{}{path}", self.base)).json(&body),
            status,
        )
    }

    /// Create a memo; returns `(id, version)`.
    fn create(&self, note: Value) -> (String, String) {
        let out = self.post("/api/notes?type=memo", json!({ "note": note }), 201);
        (str(&out["id"]), str(&out["version"]))
    }

    /// Replace note `id` at its current version; returns the new version.
    fn update(&self, id: &str, note: Value) -> String {
        let base = str(&self.get(&format!("/api/notes/{id}"), 200)["version"]);
        let out = self.put(
            &format!("/api/notes/{id}"),
            json!({ "note": note, "base_version": base }),
            200,
        );
        str(&out["version"])
    }

    fn note(&self, id: &str) -> Value {
        self.get(&format!("/api/notes/{id}"), 200)
    }

    fn memos(&self) -> Vec<Value> {
        self.get("/api/notes?type=memo", 200)["notes"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn history(&self, id: &str) -> Vec<String> {
        self.get(&format!("/api/notes/{id}/history"), 200)["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| str(&v["version"]))
            .collect()
    }
}

fn str(v: &Value) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("not a string: {v}"))
        .to_owned()
}

/// A memo's JSON form.
fn memo(title: &str, a: &str, b: &str, paras: &[&str]) -> Value {
    let blocks: Vec<Value> = paras
        .iter()
        .map(|p| json!({ "kind": "paragraph", "fields": { "text": p } }))
        .collect();
    json!({
        "fields": { "title": title, "a": a, "b": b },
        "blocks": [ { "kind": "content", "labels": ["body"], "blocks": blocks } ],
    })
}

/// A note's fields and paragraphs, from its JSON form.
fn shape(note: &Value) -> (String, String, String, Vec<String>) {
    let f = &note["note"]["fields"];
    let paras = note["note"]["blocks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["kind"] == "content")
        .flat_map(|b| b["blocks"].as_array().cloned().unwrap_or_default())
        .map(|p| str(&p["fields"]["text"]))
        .collect();
    (
        str(&f["title"]),
        f["a"].as_str().unwrap_or_default().to_owned(),
        f["b"].as_str().unwrap_or_default().to_owned(),
        paras,
    )
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
        let out = Http::admin(server).post(
            "/api/devices/pair",
            json!({ "password": PASSWORD, "name": name, "platform": "linux" }),
            201,
        );
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
        Device {
            embedded,
            remote: Remote {
                url: server.base_url(),
                token: str(&out["token"]),
                device_id: str(&out["device_id"]),
                device_name: name.to_owned(),
            },
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            _vault: vault,
        }
    }

    fn engine(&self, assets: AssetPolicy) -> Engine {
        self.embedded
            .sync_engine(self.remote.clone(), assets)
            .unwrap()
    }

    fn sync(&self) -> sync::Report {
        self.engine(AssetPolicy::desktop()).sync().unwrap()
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

    fn note(&self, id: &str) -> Value {
        self.call("GET", &format!("/api/notes/{id}"), None, 200)
    }

    /// Replace note `id` at its local version, offline.
    fn update(&self, id: &str, note: Value) {
        let base = str(&self.note(id)["version"]);
        self.call(
            "PUT",
            &format!("/api/notes/{id}"),
            Some(json!({ "note": note, "base_version": base })),
            200,
        );
    }
}

fn open_state(server: &TestServer) -> State {
    State::open(&server.vault_path().join(".vault/state.db"), None).unwrap()
}

#[test]
fn offline_edits_to_different_fields_and_blocks_merge() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let (id, _) = admin.create(memo("Plan", "1", "1", &["one", "two", "three"]));
    let laptop = Device::pair(&server, "Laptop");
    laptop.sync();
    assert_eq!(
        shape(&laptop.note(&id)),
        shape(&admin.note(&id)),
        "the pull brings the note"
    );

    // Both sides change different fields and different blocks while apart.
    laptop.update(&id, memo("Plan", "2", "1", &["ONE", "two", "three"]));
    admin.update(&id, memo("Plan", "1", "3", &["one", "two", "THREE"]));
    // A note made offline keeps its id on the server.
    let made = laptop.call(
        "POST",
        "/api/notes?type=memo",
        Some(json!({ "note": memo("Offline", "x", "", &["new"]) })),
        201,
    );
    let offline_id = str(&made["id"]);

    let report = laptop.sync();
    assert_eq!(report.merged, vec![id.clone()]);
    assert!(report.conflicts.is_empty(), "{report:?}");

    let want = (
        "Plan".to_owned(),
        "2".to_owned(),
        "3".to_owned(),
        vec!["ONE".to_owned(), "two".to_owned(), "THREE".to_owned()],
    );
    assert_eq!(shape(&admin.note(&id)), want);
    assert_eq!(
        laptop.note(&id)["wcl"],
        admin.note(&id)["wcl"],
        "the device holds the server's text"
    );
    assert_eq!(shape(&admin.note(&offline_id)).0, "Offline");
    assert_eq!(admin.memos().len(), 2, "no conflict copy");
    assert!(laptop.embedded.queue().pending().unwrap().is_empty());

    // Nothing left to do: a second round pushes nothing and leaves the server as it is.
    let again = laptop.sync();
    assert!(again.pushed.is_empty(), "{again:?}");
    assert_eq!(admin.history(&id).len(), 3);
}

#[test]
fn an_overlapping_edit_makes_one_conflict_copy_and_one_notification() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let (id, _) = admin.create(memo("Plan", "1", "1", &["one"]));
    let phone = Device::pair(&server, "Pixel");
    phone.sync();

    phone.update(&id, memo("Plan", "phone", "1", &["one"]));
    admin.update(&id, memo("Plan", "server", "1", &["one"]));
    let report = phone.sync();
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    let copy_id = report.conflicts[0].copy_id.clone();

    // The server keeps its version; the device's is the copy, linked to the original.
    assert_eq!(shape(&admin.note(&id)).1, "server");
    let copy = admin.note(&copy_id);
    assert_eq!(shape(&copy).0, "Plan (conflict, Pixel)");
    assert_eq!(shape(&copy).1, "phone");
    assert_eq!(copy["note"]["fields"]["conflict_of"], id.as_str());
    assert_eq!(admin.memos().len(), 2);
    let notes = admin.get("/api/notifications", 200);
    let items = notes["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{notes}");
    assert_eq!(items[0]["note"], copy_id.as_str());

    // The device takes the server's version, and the copy arrives with the pull.
    assert_eq!(shape(&phone.note(&id)).1, "server");
    assert_eq!(shape(&phone.note(&copy_id)).0, "Plan (conflict, Pixel)");

    // The clash is settled: another round makes no second copy or notification.
    let again = phone.sync();
    assert!(again.conflicts.is_empty(), "{again:?}");
    assert_eq!(admin.memos().len(), 2);
    assert_eq!(
        admin.get("/api/notifications", 200)["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn reordered_blocks_fall_back_to_a_conflict_copy() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let (id, _) = admin.create(memo("List", "", "", &["one", "two", "three"]));
    let phone = Device::pair(&server, "Pixel");
    phone.sync();

    phone.update(&id, memo("List", "", "", &["two", "one", "three"]));
    admin.update(&id, memo("List", "", "", &["one", "two", "THREE"]));
    let report = phone.sync();
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    assert_eq!(shape(&admin.note(&id)).3, ["one", "two", "THREE"]);
    assert_eq!(
        shape(&admin.note(&report.conflicts[0].copy_id)).3,
        ["two", "one", "three"]
    );
}

#[test]
fn a_newer_server_refuses_sync_and_keeps_the_queue() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let (id, v1) = admin.create(memo("Plan", "1", "1", &[]));
    let laptop = Device::pair(&server, "Laptop");
    laptop.sync();
    laptop.update(&id, memo("Plan", "2", "1", &[]));
    let queued = laptop.embedded.queue().pending().unwrap();
    assert_eq!(queued.len(), 1);

    // An app that knows only an older protocol, or an older format, than the server's.
    for old in [
        Supported {
            protocol: vocab::SYNC_PROTOCOL - 1,
            format_version: vocab::FORMAT_VERSION,
        },
        Supported {
            protocol: vocab::SYNC_PROTOCOL,
            format_version: vocab::FORMAT_VERSION - 1,
        },
    ] {
        let err = laptop
            .engine(AssetPolicy::desktop())
            .with_supported(old)
            .sync()
            .unwrap_err();
        assert!(matches!(err, Error::ServerNewer { .. }), "{err}");
        assert!(err.to_string().contains("update the app"), "{err}");
        assert_eq!(laptop.embedded.queue().pending().unwrap(), queued);
        assert_eq!(admin.history(&id), vec![v1.clone()], "nothing was pushed");
    }

    // The updated app syncs what waited.
    laptop.sync();
    assert_eq!(shape(&admin.note(&id)).1, "2");
    assert!(laptop.embedded.queue().pending().unwrap().is_empty());
}

#[test]
fn the_base_survives_a_browser_session_merge_and_pruning() {
    let server = TestServer::start_seeded(seed);
    let browser = Http::browser(&server);
    let (id, v1) = browser.create(memo("Plan", "1", "1", &[]));
    let laptop = Device::pair(&server, "Laptop");
    laptop.sync();

    laptop.update(&id, memo("Plan", "2", "1", &[]));
    // The device tells the server its base while it still has a connection.
    laptop
        .engine(AssetPolicy::desktop())
        .report_bases()
        .unwrap();

    // Two quick browser saves would merge into v1; the base keeps it.
    browser.update(&id, memo("Plan", "1", "3", &[]));
    browser.update(&id, memo("Plan", "1", "4", &[]));
    let history = browser.history(&id);
    assert_eq!(history.first(), Some(&v1), "{history:?}");
    let keep_one = PrunePolicy {
        keep_versions: Some(1),
        max_age_ms: None,
    };
    open_state(&server).prune(&keep_one).unwrap();
    assert_eq!(browser.history(&id).first(), Some(&v1));

    let report = laptop.sync();
    assert_eq!(report.merged, vec![id.clone()]);
    assert!(report.conflicts.is_empty());
    assert_eq!(shape(&browser.note(&id)).1, "2");
    assert_eq!(shape(&browser.note(&id)).2, "4");

    // Nothing is queued now, so the base is released and pruning takes it.
    open_state(&server).prune(&keep_one).unwrap();
    assert!(!browser.history(&id).contains(&v1));
}

#[test]
fn deletions_and_trash_moves_propagate_both_ways() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let (gone_on_server, _) = admin.create(memo("Server deletes", "", "", &[]));
    let (gone_on_device, _) = admin.create(memo("Device deletes", "", "", &[]));
    let phone = Device::pair(&server, "Pixel");
    phone.sync();

    let v = str(&admin.note(&gone_on_server)["version"]);
    admin.send(
        admin
            .client
            .delete(server.url(&format!("/api/notes/{gone_on_server}")))
            .header("If-Match", format!("\"{v}\"")),
        200,
    );
    let v = str(&phone.note(&gone_on_device)["version"]);
    phone.call(
        "DELETE",
        &format!("/api/notes/{gone_on_device}?base_version={v}"),
        None,
        200,
    );
    phone.sync();

    let in_trash = |trash: &Value, id: &str| {
        trash["notes"]
            .as_array()
            .or(trash["trash"].as_array())
            .unwrap_or_else(|| panic!("{trash}"))
            .iter()
            .any(|t| t["id"] == id)
    };
    phone.call("GET", &format!("/api/notes/{gone_on_server}"), None, 404);
    assert!(in_trash(
        &phone.call("GET", "/api/trash", None, 200),
        &gone_on_server
    ));
    admin.get(&format!("/api/notes/{gone_on_device}"), 404);
    assert!(in_trash(&admin.get("/api/trash", 200), &gone_on_device));

    // Restores travel too.
    admin.post(
        &format!("/api/trash/{gone_on_server}/restore"),
        json!({}),
        200,
    );
    phone.call(
        "POST",
        &format!("/api/trash/{gone_on_device}/restore"),
        Some(json!({})),
        200,
    );
    phone.sync();
    assert_eq!(shape(&phone.note(&gone_on_server)).0, "Server deletes");
    assert_eq!(shape(&admin.note(&gone_on_device)).0, "Device deletes");
}

#[test]
fn definitions_are_not_pushed_from_the_device() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let laptop = Device::pair(&server, "Laptop");
    laptop.sync();

    // A type made on the device stays there.
    laptop.call(
        "PUT",
        "/api/types/local_only",
        Some(json!({ "wcl": format!("label = \"Local\"\n{TEMPLATE}") })),
        200,
    );
    laptop.call(
        "PUT",
        "/api/scripts/local_script",
        Some(json!({ "source": "fn run() {}\n" })),
        200,
    );
    let report = laptop.sync();
    assert!(report.pushed.is_empty(), "{report:?}");
    admin.get("/api/types/local_only", 400);
    admin.get("/api/scripts/local_script", 404);
    assert!(laptop.embedded.queue().pending().unwrap().is_empty());

    // The server's definitions come down.
    admin.send(
        admin
            .client
            .put(server.url("/api/types/extra"))
            .header("Content-Type", "text/plain")
            .body(format!("label = \"Extra\"\n{TEMPLATE}")),
        200,
    );
    laptop.sync();
    laptop.call("GET", "/api/types/extra", None, 200);
}

#[test]
fn desktop_keeps_every_asset_and_android_fetches_on_demand() {
    let server = TestServer::start_seeded(seed);
    let admin = Http::admin(&server);
    let upload = |name: &str, bytes: Vec<u8>| -> String {
        let out = admin.send(
            admin
                .client
                .post(server.url("/api/assets"))
                .query(&[("filename", name)])
                .body(bytes),
            201,
        );
        str(&out["id"])
    };
    let pinned_pic = upload("pinned.png", vec![1; 100]);
    let other_pic = upload("other.png", vec![2; 100]);
    let mut pinned = memo("Pinned", "", "", &[]);
    pinned["fields"]["pic"] = json!(pinned_pic);
    let (pinned_id, _) = admin.create(pinned);
    let mut other = memo("Other", "", "", &[]);
    other["fields"]["pic"] = json!(other_pic);
    let (other_id, _) = admin.create(other);

    // The desktop keeps everything.
    let laptop = Device::pair(&server, "Laptop");
    let report = laptop.sync();
    assert_eq!(report.assets_fetched, 2, "{report:?}");
    laptop.call("GET", &format!("/api/assets/{other_pic}"), None, 200);

    // Android fetches the assets of pinned notes, and others when a note is opened.
    let phone = Device::pair(&server, "Pixel");
    let policy = AssetPolicy::OnDemand {
        cache_bytes: 100,
        recent: 0,
    };
    phone.engine(policy).sync().unwrap();
    phone.call("PUT", &format!("/api/pins/{pinned_id}"), None, 200);
    let report = phone.engine(policy).sync().unwrap();
    assert_eq!(report.assets_fetched, 1, "{report:?}");
    phone.call("GET", &format!("/api/assets/{pinned_pic}"), None, 200);
    phone.call("GET", &format!("/api/assets/{other_pic}"), None, 404);
    phone.engine(policy).note_opened(&other_id).unwrap();
    phone.call("GET", &format!("/api/assets/{other_pic}"), None, 200);

    // Over the cache size, the least recently used asset no pinned note needs is evicted.
    let report = phone.engine(policy).sync().unwrap();
    assert_eq!(report.assets_evicted, 1, "{report:?}");
    phone.call("GET", &format!("/api/assets/{other_pic}"), None, 404);
    phone.call("GET", &format!("/api/assets/{pinned_pic}"), None, 200);
    // A note naming an evicted asset can still be edited on the device.
    let mut edited = memo("Other, edited", "", "", &[]);
    edited["fields"]["pic"] = json!(other_pic);
    phone.update(&other_id, edited);

    // An asset stored on the device reaches the server under its own id.
    let made = phone.runtime.block_on(async {
        let res = phone
            .embedded
            .handle(
                Request::builder()
                    .method("POST")
                    .uri("/api/assets?filename=phone.jpg")
                    .body(Body::from(vec![3u8; 10]))
                    .unwrap(),
            )
            .await;
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    });
    let phone_pic = str(&made["id"]);
    phone.engine(policy).sync().unwrap();
    let got = admin
        .client
        .get(server.url(&format!("/api/assets/{phone_pic}")))
        .send()
        .unwrap();
    assert_eq!(got.status(), 200);
    assert_eq!(got.bytes().unwrap().to_vec(), vec![3u8; 10]);
    assert_eq!(shape(&admin.note(&other_id)).0, "Other, edited");
}

/// A gadget type as a newer release writes it: an attribute of the type and one of a field that
/// this version does not know.
const NEWER_GADGET: &str = "label = \"Gadget\"\nshelf = \"top\"\n\nfield size {\n  kind = \"int\"\n  unit = \"cm\"\n}\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n    slot \"size\"\n  }\n}\n";

/// The device's view of the gadget type: its warnings, from `GET /api/types`.
fn gadget_warnings(device: &Device) -> Value {
    let list = device.call("GET", "/api/types", None, 200);
    list["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "gadget")
        .unwrap_or_else(|| panic!("no gadget type: {list}"))["warnings"]
        .clone()
}

#[test]
fn a_type_with_attributes_the_device_does_not_know_still_syncs_and_takes_notes() {
    let server = TestServer::start_seeded(|vault| {
        seed(vault);
        std::fs::write(
            vault.join("types/gadget.wcl"),
            format!("label = \"Gadget\"\n{TEMPLATE}"),
        )
        .unwrap();
    });
    let admin = Http::admin(&server);
    let laptop = Device::pair(&server, "Laptop");
    laptop.sync();
    assert_eq!(gadget_warnings(&laptop), json!([]));

    // The server's type gains attributes only a newer release knows.
    admin.send(
        admin
            .client
            .put(server.url("/api/types/gadget"))
            .header("Content-Type", "text/plain")
            .body(NEWER_GADGET),
        200,
    );
    laptop.sync();
    let warnings = gadget_warnings(&laptop);
    assert_eq!(
        warnings
            .as_array()
            .unwrap()
            .iter()
            .map(|w| str(&w["at"]))
            .collect::<Vec<_>>(),
        ["type.shelf", "field size.unit"],
        "the device loaded the newer type, with warnings: {warnings}"
    );

    // The device writes notes of the type, checked against what it knows, and they reach the
    // server.
    let made = laptop.call(
        "POST",
        "/api/notes?type=gadget",
        Some(json!({ "note": { "fields": { "title": "Lamp", "size": 30 } } })),
        201,
    );
    laptop.call(
        "POST",
        "/api/notes?type=gadget",
        Some(json!({ "note": { "fields": { "title": "Lamp", "size": "big" } } })),
        400,
    );
    let report = laptop.sync();
    assert_eq!(report.pushed.len(), 1, "{report:?}");
    let lamp = str(&made["id"]);
    assert_eq!(admin.note(&lamp)["note"]["fields"]["size"], 30);

    // The device opens its vault again, as the app does on its next start, and carries on.
    let Device {
        embedded,
        remote,
        runtime,
        _vault,
    } = laptop;
    drop(embedded);
    let embedded = Embedded::open(Config {
        vault: _vault.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .expect("the device opens a vault whose type has attributes it does not know");
    let laptop = Device {
        embedded,
        remote,
        runtime,
        _vault,
    };
    let made = laptop.call(
        "POST",
        "/api/notes?type=gadget",
        Some(json!({ "note": { "fields": { "title": "Fan", "size": 40 } } })),
        201,
    );
    let report = laptop.sync();
    assert_eq!(report.pushed.len(), 1, "{report:?}");
    assert_eq!(
        admin.note(&str(&made["id"]))["note"]["fields"]["title"],
        "Fan"
    );
}
