//! The phone's side of device commands against a real home server (D22, D23): it takes its
//! settings from the server, fetches what server scripts queued for it, runs only what its device
//! file declares and its settings enable, and reports every result back.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use embedded::device::{DeviceConfig, DeviceData};
use embedded::{Config, Embedded};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use tauri_plugin_vault_device::commands;
use tauri_plugin_vault_device::core::Link;
use tauri_plugin_vault_device::pair::{self, Secret};
use tempfile::TempDir;
use testkit::{PASSWORD, TestServer};

/// A scheduled script that asks the phone for a text and a notification.
const REMIND: &str = r#"use vault

fn run() -> Result[(), string] {
    let sms = Value::Map(#{ "to": Value::String("+61400000000"), "body": Value::String("Bins out") })
    vault::device_command("Pixel", "send_sms", sms)?
    let note = Value::Map(#{ "title": Value::String("Bins"), "body": Value::String("Tonight"), "note": Value::String("") })
    vault::device_command("Pixel", "show_notification", note)?
    Ok(())
}
"#;

fn seed(vault: &Path) {
    let mut files: Vec<(String, String)> = writer::DEFAULT_DEVICE_SCRIPTS
        .iter()
        .map(|(name, source)| (format!("scripts/{name}.ws"), (*source).to_owned()))
        .collect();
    files.push((
        "devices/android.wcl".into(),
        writer::DEFAULT_ANDROID_DEVICE_FILE.into(),
    ));
    files.push(("scripts/remind.ws".into(), REMIND.into()));
    files.push((
        "tasks/remind.wcl".into(),
        "schedule = \"0 7 * * *\"\nscript   = \"remind\"\npaused   = true\n".into(),
    ));
    for (path, body) in files {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

fn admin(server: &TestServer, method: &str, path: &str, body: Option<Value>) -> Value {
    let rb = Client::new()
        .request(method.parse().unwrap(), server.url(path))
        .bearer_auth(server.admin_token());
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

/// A phone that records what it is asked to do.
#[derive(Default)]
struct Phone(Mutex<Vec<String>>);

impl Phone {
    fn actions(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl DeviceData for Phone {
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
    fn contacts_get(&self, id: &str) -> Result<Value, String> {
        Err(format!("no contact {id}"))
    }
    fn contacts_upsert(&self, _: &Value) -> Result<Value, String> {
        Err("not in this test".into())
    }
    fn contacts_delete(&self, _: &str) -> Result<(), String> {
        Err("not in this test".into())
    }
    fn sms_send(&self, to: &str, body: &str) -> Result<Value, String> {
        self.0.lock().unwrap().push(format!("sms {to}: {body}"));
        Ok(json!({ "device_ref": null }))
    }
    fn show_notification(&self, title: &str, body: &str, _: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push(format!("notify {title}: {body}"));
        Ok(())
    }
}

fn by_name<'a>(ran: &'a Value, name: &str) -> &'a Value {
    ran.as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {ran}"))
}

#[test]
fn commands_run_only_when_declared_and_switched_on_and_every_result_is_reported() {
    let server = TestServer::start_with_args(
        seed,
        &[
            OsStr::new("--test-clock"),
            OsStr::new("2026-01-05T00:30:00Z"),
        ],
    );
    let p = pair::pair(
        &server.base_url(),
        &Secret::Password(PASSWORD.to_owned()),
        "Pixel",
    )
    .unwrap();
    let link = Link {
        server_url: p.server_url,
        token: p.token,
        device_id: p.device_id,
        device_name: p.device_name,
        cache_mb: 64,
    };
    let settings = |s: Value| {
        admin(
            &server,
            "PATCH",
            &format!("/api/devices/{}", link.device_id),
            Some(json!({ "settings": s })),
        );
    };
    let remind = || {
        let run = admin(&server, "POST", "/api/tasks/remind/run", None);
        assert_eq!(run["outcome"], "succeeded", "{run}");
    };

    let dir = TempDir::new().unwrap();
    let local = Embedded::open(Config {
        vault: dir.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    let phone = Arc::new(Phone::default());
    let runtime = local
        .device(DeviceConfig {
            id: link.device_id.clone(),
            platform: "android".into(),
            data: phone.clone(),
        })
        .unwrap();

    // Both commands enabled, but the SMS-send switch off: the text is refused on the phone and
    // the notification shows. The phone's own settings had nothing enabled; it takes the
    // server's before it runs anything.
    settings(json!({ "commands": ["send_sms", "show_notification"], "sms_send": false }));
    remind();
    let ran = commands::run(&link, &runtime).unwrap();
    assert_eq!(ran.as_array().unwrap().len(), 2, "{ran}");
    let sms = by_name(&ran, "send_sms");
    assert_eq!(sms["status"], "failed", "{sms}");
    assert!(
        sms["result"]
            .as_str()
            .unwrap()
            .contains("turn on SMS send in Settings > Devices"),
        "{sms}"
    );
    assert_eq!(by_name(&ran, "show_notification")["status"], "done");
    assert_eq!(phone.actions(), ["notify Bins: Tonight"]);
    assert_eq!(
        runtime.settings().commands,
        ["send_sms", "show_notification"]
    );

    // Nothing is pending any more, so nothing runs twice.
    assert_eq!(commands::run(&link, &runtime).unwrap(), json!([]));

    // With the switch on, the text goes.
    settings(json!({ "commands": ["send_sms", "show_notification"], "sms_send": true }));
    remind();
    let ran = commands::run(&link, &runtime).unwrap();
    assert_eq!(by_name(&ran, "send_sms")["status"], "done", "{ran}");
    assert_eq!(
        phone.actions(),
        [
            "notify Bins: Tonight",
            "sms +61400000000: Bins out",
            "notify Bins: Tonight"
        ]
    );

    // A command the phone's device file does not declare is refused, enabled or not.
    let w = local.writer();
    for (name, source) in writer::DEFAULT_DEVICE_SCRIPTS {
        w.put_script(name, source).unwrap();
    }
    w.put_device_file(
        "android",
        "command \"show_notification\" {\n  script = \"device_messages\"\n}\n",
    )
    .unwrap();
    remind();
    let ran = commands::run(&link, &runtime).unwrap();
    let sms = by_name(&ran, "send_sms");
    assert_eq!(sms["status"], "failed");
    assert!(
        sms["result"]
            .as_str()
            .unwrap()
            .contains("devices/android.wcl does not declare it"),
        "{sms}"
    );
    assert_eq!(phone.actions().len(), 4);

    // The server holds every result the phone reported.
    let list = |status: &str| {
        admin(
            &server,
            "GET",
            &format!("/api/devices/{}/commands?status={status}", link.device_id),
            None,
        )["commands"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(list("failed"), 2);
    assert_eq!(list("done"), 4);
    assert_eq!(list("pending"), 0);
    assert_eq!(list("delivered"), 0);
}

/// A phone whose call log holds two calls, which it hands an import of history.
struct CallLog;

impl DeviceData for CallLog {
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
    fn contacts_get(&self, id: &str) -> Result<Value, String> {
        Err(format!("no contact {id}"))
    }
    fn contacts_upsert(&self, _: &Value) -> Result<Value, String> {
        Err("not in this test".into())
    }
    fn contacts_delete(&self, _: &str) -> Result<(), String> {
        Err("not in this test".into())
    }
    fn sms_send(&self, _: &str, _: &str) -> Result<Value, String> {
        Err("not in this test".into())
    }
    fn show_notification(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("not in this test".into())
    }
    fn history(&self, request: &Value) -> Result<Value, String> {
        if request["kind"] != "calls" {
            return Ok(json!({ "events": [], "next": null, "missing_permission": null }));
        }
        let call = |key: i64| {
            json!({
                "event": "call_ended",
                "at": 1_767_225_600_000_i64 + key,
                "data": {
                    "device_ref": format!("call:{key}"),
                    "direction": "in",
                    "number": "+61400000001",
                    "number_raw": "0400 000 001",
                    "contact": null,
                    "started": "2026-01-01T11:00:00+11:00",
                    "duration": 30,
                },
            })
        };
        Ok(json!({ "events": [call(1), call(2)], "next": null, "missing_permission": null }))
    }
}

#[test]
fn import_history_from_settings_runs_on_the_phone_and_reports_its_result() {
    let server = TestServer::start_seeded(seed);
    let p = pair::pair(
        &server.base_url(),
        &Secret::Password(PASSWORD.to_owned()),
        "Pixel",
    )
    .unwrap();
    let link = Link {
        server_url: p.server_url,
        token: p.token,
        device_id: p.device_id,
        device_name: p.device_name,
        cache_mb: 64,
    };
    let dir = TempDir::new().unwrap();
    let local = Embedded::open(Config {
        vault: dir.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    let runtime = local
        .device(DeviceConfig {
            id: link.device_id.clone(),
            platform: "android".into(),
            data: Arc::new(CallLog),
        })
        .unwrap();
    admin(
        &server,
        "PATCH",
        &format!("/api/devices/{}", link.device_id),
        Some(json!({ "settings": { "commands": ["import_history"] } })),
    );
    let queued = admin(
        &server,
        "POST",
        &format!("/api/devices/{}/commands", link.device_id),
        Some(json!({ "name": "import_history", "payload": { "kinds": ["calls"] } })),
    );

    let ran = commands::run(&link, &runtime).unwrap();
    let import = by_name(&ran, "import_history");
    assert_eq!(import["status"], "done", "{import}");
    assert_eq!(import["result"]["kinds"][0]["imported"], 2);

    let id = queued["id"].as_str().unwrap();
    let on_server = admin(
        &server,
        "GET",
        &format!("/api/devices/{}/commands?status=done", link.device_id),
        None,
    )["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .unwrap();
    assert_eq!(on_server["result"]["kinds"][0]["kind"], "calls");
    assert_eq!(on_server["result"]["done"], true);
    assert_eq!(
        on_server["result"]["message"],
        "calls: 2 found, 2 new notes"
    );
    assert_eq!(local.writer().list(Some("call")).unwrap().len(), 2);
}
