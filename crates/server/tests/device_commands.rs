//! S1: device files (`devices/<platform>.wcl`) through the API, and the commands server scripts
//! queue for a device: allowlisting, availability by kind of run, results and expiry (D5, D22).

use std::ffi::OsStr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::{PASSWORD, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

/// A device file in canonical form, so it must come back byte for byte.
const ANDROID: &str = r#"handler "sms_received" {
  script = "device_messages"
}

handler "sms_received" {
  script   = "device_extra"
  function = "on_sms"
}

handler "call_ended" {
  script = "device_messages"
}

command "send_sms" {
  script = "device_messages"
}

command "delete_contact" {
  script = "device_messages"
}
"#;

const DEVICE_SCRIPT: &str = "fn sms_received(event: Value) -> Result[(), string] { Ok(()) }\n";

/// A scheduled script that queues one command and checks that the refusals are refused.
const REMIND: &str = r#"use vault

fn run() -> Result[(), string] {
    let sms = Value::Map(#{ "to": Value::String("+61400000000"), "body": Value::String("Bins out") })
    let queued = vault::device_command("Pixel", "send_sms", sms)?
    vault::log("queued " + queued.get("status").unwrap().as_string().unwrap())
    match vault::device_command("Pixel", "delete_contact", Value::Map(#{})) {
        Ok(_) => return Err("a disabled command was queued"),
        Err(e) => vault::log("disabled: " + e),
    }
    match vault::device_command("Pixel", "show_notification", Value::Map(#{})) {
        Ok(_) => return Err("an undeclared command was queued"),
        Err(e) => vault::log("undeclared: " + e),
    }
    match vault::device_command("Nokia", "send_sms", Value::Map(#{})) {
        Ok(_) => return Err("a command for no device was queued"),
        Err(e) => vault::log("no device: " + e),
    }
    Ok(())
}
"#;

const MEMO: &str = r#"label = "Memo"

handler "before_create" {
  script = "memo_before"
}

handler "after_create" {
  script = "memo_after"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// A `before_*` handler cannot queue a command; it rejects the save if it could.
const MEMO_BEFORE: &str = r#"use vault

fn before_create(note: Value) -> Result[Value, string] {
    match vault::device_command("Pixel", "send_sms", Value::Map(#{})) {
        Ok(_) => Err("vault::device_command ran inside the write"),
        Err(_) => Ok(Value::Null),
    }
}
"#;

const MEMO_AFTER: &str = r#"use vault

fn after_create(note: Value) -> Result[(), string] {
    let sms = Value::Map(#{ "to": Value::String("+61400000001"), "body": Value::String("Memo filed") })
    vault::device_command("Pixel", "send_sms", sms)?
    Ok(())
}
"#;

fn seed(vault: &Path) {
    for (path, body) in [
        ("types/memo.wcl", MEMO),
        ("scripts/memo_before.ws", MEMO_BEFORE),
        ("scripts/memo_after.ws", MEMO_AFTER),
        ("scripts/remind.ws", REMIND),
        ("scripts/device_messages.ws", DEVICE_SCRIPT),
        ("scripts/device_extra.ws", DEVICE_SCRIPT),
        (
            "tasks/remind.wcl",
            "schedule = \"0 7 * * *\"\nscript   = \"remind\"\npaused   = true\n",
        ),
    ] {
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

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn send(rb: RequestBuilder, status: u16) -> Value {
    expect(rb.send().unwrap(), status)
}

struct Api {
    server: TestServer,
    admin: Client,
}

impl Api {
    fn start() -> Api {
        let server =
            TestServer::start_with_args(seed, &[OsStr::new("--test-clock"), OsStr::new(START)]);
        let admin = client(server.admin_token());
        Api { server, admin }
    }

    fn req(&self, c: &Client, method: &str, path: &str) -> RequestBuilder {
        c.request(method.parse().unwrap(), self.server.url(path))
    }

    fn admin(&self, method: &str, path: &str) -> RequestBuilder {
        self.req(&self.admin, method, path)
    }

    /// Pair a device and return its id and a client holding its token.
    fn pair(&self, name: &str) -> (String, Client) {
        let out = send(
            Client::new()
                .post(self.server.url("/api/devices/pair"))
                .json(&json!({ "password": PASSWORD, "name": name, "platform": "android" })),
            201,
        );
        (
            out["device_id"].as_str().unwrap().to_owned(),
            client(out["token"].as_str().unwrap()),
        )
    }

    fn commands(&self, c: &Client, device: &str, query: &str) -> Vec<Value> {
        let out = send(
            self.req(c, "GET", &format!("/api/devices/{device}/commands{query}")),
            200,
        );
        out["commands"].as_array().unwrap().clone()
    }

    fn report(&self, c: &Client, device: &str, id: &str, body: Value, status: u16) -> Value {
        send(
            self.req(
                c,
                "POST",
                &format!("/api/devices/{device}/commands/{id}/result"),
            )
            .json(&body),
            status,
        )
    }
}

fn put_file(api: &Api, platform: &str, wcl: &str, status: u16) -> Value {
    send(
        api.admin("PUT", &format!("/api/device-files/{platform}"))
            .header("content-type", "text/plain")
            .body(wcl.to_owned()),
        status,
    )
}

#[test]
fn device_files_round_trip_and_reject_unknown_events_and_commands() {
    let api = Api::start();
    let empty = send(api.admin("GET", "/api/device-files"), 200);
    assert_eq!(empty["files"], json!([]));
    assert_eq!(empty["platforms"], json!(["linux", "android"]));
    assert_eq!(empty["events"].as_array().unwrap().len(), 8);
    assert_eq!(
        empty["commands"],
        json!([
            "send_sms",
            "upsert_contact",
            "delete_contact",
            "show_notification"
        ])
    );
    send(api.admin("GET", "/api/device-files/android"), 404);

    let saved = put_file(&api, "android", ANDROID, 200);
    assert_eq!(saved["wcl"], ANDROID);
    assert_eq!(
        saved["handlers"][1],
        json!({ "event": "sms_received", "script": "device_extra", "function": "on_sms" })
    );
    assert_eq!(
        saved["commands"][0],
        json!({ "name": "send_sms", "script": "device_messages", "function": "send_sms" })
    );
    let text = api
        .admin("GET", "/api/device-files/android")
        .header("accept", "text/plain")
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert_eq!(text, ANDROID);
    let on_disk =
        std::fs::read_to_string(api.server.vault_path().join("devices/android.wcl")).unwrap();
    assert_eq!(on_disk, ANDROID);
    let read = send(api.admin("GET", "/api/device-files/android"), 200);
    assert_eq!(read["wcl"], ANDROID);
    let list = send(api.admin("GET", "/api/device-files"), 200);
    assert_eq!(list["files"][0]["platform"], "android");

    // Non-canonical input is stored canonically.
    let loose = put_file(
        &api,
        "linux",
        "handler \"share_received\" { script = \"device_extra\" }",
        200,
    );
    assert_eq!(
        loose["wcl"],
        "handler \"share_received\" {\n  script = \"device_extra\"\n}\n"
    );

    for (wcl, expected) in [
        (
            "handler \"sms_deleted\" {\n  script = \"device_messages\"\n}\n",
            "the label must be one event",
        ),
        (
            "command \"ring_phone\" {\n  script = \"device_messages\"\n}\n",
            "the label must be one command",
        ),
        (
            "handler \"sms_sent\" {\n  script = \"nowhere\"\n}\n",
            "there is no script \\\"nowhere\\\"",
        ),
        ("label = \"Phone\"\n", "is not an item of this block"),
        ("handler \"sms_sent\" {\n  script = 1 + 2\n}\n", ""),
    ] {
        let err = put_file(&api, "android", wcl, 400);
        assert!(err.to_string().contains(expected), "{wcl}\n{err}");
    }
    put_file(&api, "windows", "", 400);
    // The refused writes left the file alone.
    let read = send(api.admin("GET", "/api/device-files/android"), 200);
    assert_eq!(read["wcl"], ANDROID);

    // A script a device file names cannot be deleted, and a device file is admin-only to change.
    let in_use = send(api.admin("DELETE", "/api/scripts/device_extra"), 409);
    assert!(
        in_use
            .to_string()
            .contains("devices/android.wcl (handler \\\"sms_received\\\")"),
        "{in_use}"
    );
    let (_, phone) = api.pair("Pixel");
    send(
        api.req(&phone, "PUT", "/api/device-files/android")
            .body(ANDROID.to_owned()),
        403,
    );
    send(api.req(&phone, "GET", "/api/device-files/android"), 200);

    let gone = send(api.admin("DELETE", "/api/device-files/linux"), 200);
    assert_eq!(gone["deleted"], true);
    send(api.admin("DELETE", "/api/device-files/linux"), 404);
}

fn wait_for_commands(api: &Api, device: &str, n: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let list = api.commands(&api.admin, device, "");
        if list.len() >= n {
            return list;
        }
        assert!(
            Instant::now() < deadline,
            "only {} commands: {list:?}",
            list.len()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn scripts_queue_only_declared_and_enabled_commands_and_devices_report_on_them() {
    let api = Api::start();
    put_file(&api, "android", ANDROID, 200);
    let (device, phone) = api.pair("Pixel");
    let (other, other_phone) = api.pair("Tablet");

    // Nothing is enabled yet, so even a declared command is refused.
    let run = send(api.admin("POST", "/api/tasks/remind/run"), 200);
    assert_eq!(run["outcome"], "failed", "{run}");
    assert!(
        run["message"]
            .as_str()
            .unwrap()
            .contains("command \"send_sms\" is not enabled for device \"Pixel\""),
        "{run}"
    );
    assert!(api.commands(&api.admin, &device, "").is_empty());

    send(
        api.admin("PATCH", &format!("/api/devices/{device}"))
            .json(&json!({ "settings": { "commands": ["send_sms", "show_notification"] } })),
        200,
    );
    let run = send(api.admin("POST", "/api/tasks/remind/run"), 200);
    assert_eq!(run["outcome"], "succeeded", "{run}");
    let log = run["log"].as_str().unwrap();
    assert!(log.contains("queued pending"), "{log}");
    assert!(
        log.contains("disabled: command \"delete_contact\" is not enabled"),
        "{log}"
    );
    assert!(
        log.contains("undeclared: device \"Pixel\" does not accept command \"show_notification\": devices/android.wcl does not declare it"),
        "{log}"
    );
    assert!(
        log.contains("no device: there is no device \"Nokia\""),
        "{log}"
    );

    let pending = api.commands(&phone, &device, "?status=pending");
    assert_eq!(pending.len(), 1, "{pending:?}");
    let sms = &pending[0];
    assert_eq!(sms["name"], "send_sms");
    assert_eq!(sms["device"], device.as_str());
    assert_eq!(sms["source"], "task:remind");
    assert_eq!(
        sms["payload"],
        json!({ "to": "+61400000000", "body": "Bins out" })
    );
    assert_eq!(sms["status"], "pending");
    assert_eq!(sms["result"], Value::Null);
    assert_eq!(
        sms["expires"].as_i64().unwrap() - sms["created"].as_i64().unwrap(),
        24 * 60 * 60 * 1000
    );
    let sms_id = sms["id"].as_str().unwrap();

    // A before_* handler cannot queue one; the after_* handler of the same save can.
    let memo = send(
        api.admin("POST", "/api/notes?type=memo")
            .json(&json!({ "note": { "fields": { "title": "Filed" } } })),
        201,
    );
    assert_eq!(memo["type"], "memo");
    let all = wait_for_commands(&api, &device, 2);
    assert_eq!(all[1]["source"], "handler:memo.after_create");
    assert_eq!(all[1]["payload"]["body"], "Memo filed");

    // Only the device itself or an admin sees or reports on its commands.
    send(
        api.req(
            &other_phone,
            "GET",
            &format!("/api/devices/{device}/commands"),
        ),
        403,
    );
    api.report(
        &other_phone,
        &device,
        sms_id,
        json!({ "status": "done" }),
        403,
    );
    assert!(api.commands(&other_phone, &other, "").is_empty());
    send(
        api.admin(
            "GET",
            &format!("/api/devices/{device}/commands?status=sent"),
        ),
        400,
    );
    api.report(&phone, &device, sms_id, json!({ "status": "pending" }), 400);
    api.report(
        &phone,
        &device,
        "01NOSUCHCOMMAND",
        json!({ "status": "done" }),
        404,
    );

    // The device takes it, then reports it done with a result, which is stored.
    let delivered = api.report(
        &phone,
        &device,
        sms_id,
        json!({ "status": "delivered" }),
        200,
    );
    assert_eq!(delivered["status"], "delivered");
    let done = api.report(
        &phone,
        &device,
        sms_id,
        json!({ "status": "done", "result": { "device_ref": "sms:4412" } }),
        200,
    );
    assert_eq!(done["status"], "done");
    assert_eq!(done["result"], json!({ "device_ref": "sms:4412" }));
    let again = api.report(&phone, &device, sms_id, json!({ "status": "failed" }), 409);
    assert_eq!(again["error"], "command_closed", "{again}");
    let stored = api.commands(&api.admin, &device, "?status=done");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["result"], json!({ "device_ref": "sms:4412" }));
    let pending = api.commands(&phone, &device, "?status=pending");
    assert_eq!(pending.len(), 1);
    let memo_sms = pending[0]["id"].as_str().unwrap().to_owned();

    // A command the device never finishes expires after a day and can no longer be reported on.
    send(
        api.admin("POST", "/api/test/clock")
            .json(&json!({ "advance": "PT25H" })),
        200,
    );
    assert!(api.commands(&phone, &device, "?status=pending").is_empty());
    let expired = api.commands(&phone, &device, "?status=expired");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0]["id"], memo_sms.as_str());
    let late = api.report(
        &phone,
        &device,
        &memo_sms,
        json!({ "status": "done", "result": {} }),
        409,
    );
    assert!(late.to_string().contains("is expired"), "{late}");

    // Revoking the device removes its commands with it.
    send(api.admin("DELETE", &format!("/api/devices/{device}")), 204);
    send(
        api.admin("GET", &format!("/api/devices/{device}/commands")),
        404,
    );
}
