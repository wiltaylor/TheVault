//! S4 with a home server: a command a server script queues for a device is fetched by the device
//! and run there only when the device file declares it and the device's settings enable it (D22,
//! D23). The default device scripts also compile on the server, against a `device` module that
//! only a device can run.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use embedded::device::{DeviceConfig, DeviceData, DeviceRuntime};
use embedded::{Config, Embedded};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
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

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

fn send(rb: RequestBuilder, status: u16) -> Value {
    let res = rb.send().unwrap();
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
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
        Ok(json!({ "device_ref": "sms:4412" }))
    }
    fn show_notification(&self, title: &str, body: &str, _: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push(format!("notify {title}: {body}"));
        Ok(())
    }
}

struct Home {
    server: TestServer,
    admin: Client,
    phone: Client,
    device: String,
}

impl Home {
    fn get(&self, c: &Client, path: &str) -> Value {
        send(c.get(self.server.url(path)), 200)
    }

    fn remind(&self) {
        let run = send(
            self.admin.post(self.server.url("/api/tasks/remind/run")),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run}");
    }

    /// The device's own settings, as it fetches them with its token.
    fn settings(&self) -> Value {
        let list = self.get(&self.phone, "/api/devices");
        list["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == self.device.as_str())
            .unwrap()["settings"]
            .clone()
    }

    /// What the device does on a push: fetch its pending commands, take each, run it and report.
    fn deliver(&self, runtime: &DeviceRuntime) -> Vec<Value> {
        let base = format!("/api/devices/{}/commands", self.device);
        let pending = self.get(&self.phone, &format!("{base}?status=pending"));
        let mut reports = Vec::new();
        for c in pending["commands"].as_array().unwrap() {
            let result = format!("{base}/{}/result", c["id"].as_str().unwrap());
            let url = self.server.url(&result);
            send(
                self.phone
                    .post(&url)
                    .json(&json!({ "status": "delivered" })),
                200,
            );
            let report = runtime.run_command(c);
            reports.push(send(self.phone.post(&url).json(&report.body()), 200));
        }
        reports
    }
}

#[test]
fn a_device_runs_a_server_command_only_when_declared_and_enabled() {
    let server = TestServer::start_with_args(
        seed,
        &[
            OsStr::new("--test-clock"),
            OsStr::new("2026-01-05T00:30:00Z"),
        ],
    );
    let admin = client(server.admin_token());
    let paired = send(
        Client::new()
            .post(server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": "Pixel", "platform": "android" })),
        201,
    );
    let home = Home {
        phone: client(paired["token"].as_str().unwrap()),
        device: paired["device_id"].as_str().unwrap().to_owned(),
        admin,
        server,
    };

    // The default device scripts compile on the server, where they are checked as device scripts.
    for (name, _) in writer::DEFAULT_DEVICE_SCRIPTS {
        let script = home.get(&home.admin, &format!("/api/scripts/{name}"));
        assert_eq!(script["problems"], json!([]), "{name}: {script}");
    }

    send(
        home.admin
            .patch(home.server.url(&format!("/api/devices/{}", home.device)))
            .json(&json!({ "settings": {
                "commands": ["send_sms", "show_notification"],
                "sms_send": true,
            } })),
        200,
    );
    home.remind();

    // The device's own copy of its settings is older: it has show_notification off.
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
            id: home.device.clone(),
            platform: "android".into(),
            data: phone.clone(),
        })
        .unwrap();
    runtime
        .set_settings(&json!({ "commands": ["send_sms"], "sms_send": true }))
        .unwrap();

    let reports = home.deliver(&runtime);
    assert_eq!(reports.len(), 2, "{reports:?}");
    let sms = reports.iter().find(|r| r["name"] == "send_sms").unwrap();
    assert_eq!(sms["status"], "done", "{sms}");
    assert_eq!(sms["result"], json!({ "device_ref": "sms:4412" }));
    let note = reports
        .iter()
        .find(|r| r["name"] == "show_notification")
        .unwrap();
    assert_eq!(note["status"], "failed", "{note}");
    assert!(
        note["result"]
            .as_str()
            .unwrap()
            .contains("command \"show_notification\" is not enabled for this device"),
        "{note}"
    );
    assert_eq!(phone.actions(), ["sms +61400000000: Bins out"]);

    // With the server's settings, the notification runs too.
    runtime.set_settings(&home.settings()).unwrap();
    home.remind();
    let reports = home.deliver(&runtime);
    assert!(reports.iter().all(|r| r["status"] == "done"), "{reports:?}");
    assert_eq!(
        phone.actions(),
        [
            "sms +61400000000: Bins out",
            "sms +61400000000: Bins out",
            "notify Bins: Tonight"
        ]
    );

    // A local device file that no longer declares the command refuses it, enabled or not.
    let w = local.writer();
    for (name, source) in writer::DEFAULT_DEVICE_SCRIPTS {
        w.put_script(name, source).unwrap();
    }
    w.put_device_file(
        "android",
        "command \"send_sms\" {\n  script = \"device_messages\"\n}\n",
    )
    .unwrap();
    home.remind();
    let reports = home.deliver(&runtime);
    let note = reports
        .iter()
        .find(|r| r["name"] == "show_notification")
        .unwrap();
    assert_eq!(note["status"], "failed");
    assert!(
        note["result"]
            .as_str()
            .unwrap()
            .contains("devices/android.wcl does not declare it"),
        "{note}"
    );
    assert_eq!(phone.actions().len(), 4);

    // The server keeps every result.
    let failed = home.get(
        &home.admin,
        &format!("/api/devices/{}/commands?status=failed", home.device),
    );
    assert_eq!(failed["commands"].as_array().unwrap().len(), 2);
    let done = home.get(
        &home.admin,
        &format!("/api/devices/{}/commands?status=done", home.device),
    );
    assert_eq!(done["commands"].as_array().unwrap().len(), 4);
}
