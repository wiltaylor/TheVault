//! S1: device files (`devices/<platform>.wcl`) through the API, and the commands server scripts
//! queue for a device: allowlisting, availability by kind of run, results and expiry (D5, D22),
//! and the `upsert_contact` the server queues itself for a `person` (D16, D24).

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
        Api::start_seeded(seed)
    }

    fn start_seeded(seed: fn(&Path)) -> Api {
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
    // Every vault starts with the shipped Android device file; delete it to start from none.
    let shipped = send(api.admin("GET", "/api/device-files"), 200);
    assert_eq!(shipped["files"][0]["platform"], "android", "{shipped}");
    send(api.admin("DELETE", "/api/device-files/android"), 200);
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
            "show_notification",
            "import_history"
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
    let run = send(api.admin("POST", "/api/tasks/remind/run?wait=true"), 200);
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
    let run = send(api.admin("POST", "/api/tasks/remind/run?wait=true"), 200);
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

/// The default device file and scripts, as the starter vault has them.
fn seed_defaults(vault: &Path) {
    let mut files = vec![(
        "devices/android.wcl".to_owned(),
        writer::DEFAULT_ANDROID_DEVICE_FILE,
    )];
    for (name, source) in writer::DEFAULT_DEVICE_SCRIPTS {
        files.push((format!("scripts/{name}.ws"), source));
    }
    for (path, body) in files {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

impl Api {
    fn settings(&self, device: &str, settings: Value) {
        send(
            self.admin("PATCH", &format!("/api/devices/{device}"))
                .json(&json!({ "settings": settings })),
            200,
        );
    }

    fn upserts(&self, device: &str) -> Vec<Value> {
        self.commands(&self.admin, device, "")
            .into_iter()
            .filter(|c| c["name"] == "upsert_contact")
            .collect()
    }

    fn create_person(&self, c: &Client, fields: Value) -> Value {
        send(
            self.req(c, "POST", "/api/notes?type=person")
                .json(&json!({ "note": { "fields": fields } })),
            201,
        )
    }

    fn update_note(&self, c: &Client, note: &Value) -> Value {
        let id = note["id"].as_str().unwrap();
        send(
            self.req(c, "PUT", &format!("/api/notes/{id}"))
                .json(&json!({
                    "note": note["note"],
                    "base_version": note["version"],
                })),
            200,
        )
    }

    fn note(&self, id: &str) -> Value {
        send(self.admin("GET", &format!("/api/notes/{id}")), 200)
    }
}

/// The person's `device_contacts`, `null` when it has none.
fn links(note: &Value) -> Value {
    note["note"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "dict" && b["labels"] == json!(["device_contacts"]))
        .map_or(Value::Null, |b| b["fields"].clone())
}

#[test]
fn a_person_saved_on_the_server_queues_upsert_contact_for_each_phone_without_its_contact() {
    let api = Api::start_seeded(seed_defaults);
    let (pixel, pixel_client) = api.pair("Pixel");
    let (tablet, _) = api.pair("Tablet");
    let (laptop, _) = api.pair("Laptop");
    let (quiet, _) = api.pair("Quiet");
    let on = json!({ "contacts_write": true, "commands": ["upsert_contact", "delete_contact"] });
    api.settings(&pixel, on.clone());
    api.settings(&tablet, on);
    // The contacts-write switch off, or the command not enabled, queues nothing for the device.
    api.settings(
        &laptop,
        json!({ "contacts_write": false, "commands": ["upsert_contact"] }),
    );
    api.settings(
        &quiet,
        json!({ "contacts_write": true, "commands": ["delete_contact"] }),
    );

    // A person made in the web app becomes one upsert_contact for each phone that may take it,
    // carrying every detail a contact holds: label lists one per value, the photo as its asset.
    let photo = send(
        api.admin("POST", "/api/assets?filename=carla.jpg")
            .body(b"jpeg bytes".to_vec()),
        201,
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let carla = api.create_person(
        &api.admin,
        json!({
            "title": "Carla Created",
            "phones": ["+16502530005", "+16502530015"],
            "phone_labels": ["mobile"],
            "emails": ["carla@example.com"],
            "email_labels": ["work"],
            "addresses": ["1 Main St, Springfield"],
            "address_labels": ["home"],
            "organisation": "Example Co",
            "job_title": "Engineer",
            "nickname": "Carlie",
            "birthday": "--04-01",
            "websites": ["https://carla.example"],
            "contact_note": "Met at the conference",
            "photo": photo,
        }),
    );
    let id = carla["id"].as_str().unwrap().to_owned();
    let expected = json!({
        "contact_id": null,
        "name": "Carla Created",
        "phones": ["+16502530005", "+16502530015"],
        "phone_labels": ["mobile", ""],
        "emails": ["carla@example.com"],
        "email_labels": ["work"],
        "addresses": ["1 Main St, Springfield"],
        "address_labels": ["home"],
        "organisation": "Example Co",
        "job_title": "Engineer",
        "nickname": "Carlie",
        "birthday": "--04-01",
        "websites": ["https://carla.example"],
        "contact_note": "Met at the conference",
        "photo": photo,
        "person": id,
    });
    for device in [&pixel, &tablet] {
        let queued = api.upserts(device);
        assert_eq!(queued.len(), 1, "{queued:?}");
        assert_eq!(queued[0]["payload"], expected);
        assert_eq!(queued[0]["source"], "contacts");
        assert_eq!(queued[0]["status"], "pending");
    }
    assert!(api.commands(&api.admin, &laptop, "").is_empty());
    assert!(api.commands(&api.admin, &quiet, "").is_empty());

    // A person with nothing a contact holds stays off the phones, and a change to a field no
    // contact holds queues nothing more.
    api.create_person(&api.admin, json!({ "title": "No Number" }));
    let mut edited = carla.clone();
    edited["note"]["blocks"] = json!([{ "kind": "content", "labels": ["body"], "blocks": [
        { "kind": "paragraph", "fields": { "text": "Prefers email." } }
    ] }]);
    let carla = api.update_note(&api.admin, &edited);
    assert_eq!(api.upserts(&pixel).len(), 1);
    assert_eq!(api.upserts(&tablet).len(), 1);

    // The Pixel adds the contact and reports it; the server links the person to it, as the
    // Pixel, which queues nothing back to the Pixel and, the contact's fields being unchanged,
    // nothing for the Tablet.
    let command = api.upserts(&pixel)[0]["id"].as_str().unwrap().to_owned();
    api.report(
        &pixel_client,
        &pixel,
        &command,
        json!({ "status": "delivered" }),
        200,
    );
    api.report(
        &pixel_client,
        &pixel,
        &command,
        json!({ "status": "done", "result": { "contact_id": "42" } }),
        200,
    );
    let linked = api.note(&id);
    assert_eq!(
        links(&linked),
        json!({ format!("device_{pixel}"): "42" }),
        "{linked}"
    );
    assert_ne!(linked["version"], carla["version"]);
    let history = send(api.admin("GET", &format!("/api/notes/{id}/history")), 200);
    assert_eq!(
        history["versions"][0]["author"]["kind"], "token",
        "{history}"
    );
    assert_eq!(api.upserts(&pixel).len(), 1);
    assert_eq!(api.upserts(&tablet).len(), 1);

    // Renaming the person in the web app queues an upsert_contact for the Tablet, which has no
    // contact for it yet. The Pixel's contact is linked: the Pixel settles it field by field
    // against what the two last agreed on once it syncs, so no command overwrites it.
    let mut renamed = linked.clone();
    renamed["note"]["fields"]["title"] = json!("Carla Renamed");
    api.update_note(&api.admin, &renamed);
    assert_eq!(api.upserts(&pixel).len(), 1);
    let tablet_upserts = api.upserts(&tablet);
    assert_eq!(tablet_upserts.len(), 2, "{tablet_upserts:?}");
    assert_eq!(tablet_upserts[1]["payload"]["name"], "Carla Renamed");
    assert_eq!(tablet_upserts[1]["payload"]["contact_id"], Value::Null);

    // A person the Pixel itself makes or changes, from a contact captured there, does not come
    // back to the Pixel as a command; the Tablet still hears of it.
    let from_phone = api.create_person(
        &pixel_client,
        json!({ "title": "Dana Phone", "phones": ["+16502530006"] }),
    );
    let mut changed = api.note(&id);
    changed["note"]["fields"]["phones"] = json!(["+16502530005", "+16502530007"]);
    api.update_note(&pixel_client, &changed);
    assert_eq!(api.upserts(&pixel).len(), 1);
    let tablet_upserts = api.upserts(&tablet);
    assert_eq!(tablet_upserts.len(), 4, "{tablet_upserts:?}");
    assert_eq!(tablet_upserts[2]["payload"]["person"], from_phone["id"]);
    assert_eq!(
        tablet_upserts[3]["payload"]["phones"],
        json!(["+16502530005", "+16502530007"])
    );

    // A person with only an email address is one a contact can hold.
    api.create_person(
        &api.admin,
        json!({ "title": "Email Only", "emails": ["only@example.com"] }),
    );
    assert_eq!(api.upserts(&pixel).len(), 2);
    assert_eq!(api.upserts(&tablet).len(), 5);

    // Deleting a person deletes no phone contact (D16).
    let version = api.note(&id)["version"].as_str().unwrap().to_owned();
    send(
        api.admin("DELETE", &format!("/api/notes/{id}"))
            .header("if-match", format!("\"{version}\"")),
        200,
    );
    for device in [&pixel, &tablet, &laptop, &quiet] {
        let commands = api.commands(&api.admin, device, "");
        assert!(
            commands.iter().all(|c| c["name"] != "delete_contact"),
            "{commands:?}"
        );
    }
}

#[test]
fn settings_queues_import_history_only_when_declared_and_enabled_and_sees_its_progress() {
    // No device file: nothing is declared.
    let bare = Api::start();
    send(bare.admin("DELETE", "/api/device-files/android"), 200);
    let (pixel, _) = bare.pair("Pixel");
    let path = format!("/api/devices/{pixel}/commands");
    let refused = send(
        bare.admin("POST", &path)
            .json(&json!({ "name": "import_history", "payload": { "kinds": ["sms"] } })),
        400,
    );
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("accepts no commands"),
        "{refused}"
    );

    let api = Api::start_seeded(seed_defaults);
    let (pixel, phone) = api.pair("Pixel");
    let path = format!("/api/devices/{pixel}/commands");
    let ask = json!({
        "name": "import_history",
        "payload": { "kinds": ["sms", "calls"], "since": "2026-10-01T00:00:00Z", "until": null },
    });

    // Declared by the default file, but not enabled for this phone.
    let off = send(api.admin("POST", &path).json(&ask), 400);
    assert!(
        off["message"].as_str().unwrap().contains("is not enabled"),
        "{off}"
    );
    // A command the file does not declare.
    api.settings(&pixel, json!({ "commands": ["import_history"] }));
    let undeclared = send(
        api.admin("POST", &path)
            .json(&json!({ "name": "ring", "payload": {} })),
        400,
    );
    assert!(
        undeclared["message"]
            .as_str()
            .unwrap()
            .contains("does not declare it")
    );
    // A phone's own token queues nothing but an import, and only for itself.
    api.settings(
        &pixel,
        json!({ "commands": ["import_history", "show_notification"] }),
    );
    let other = send(
        api.req(&phone, "POST", &path)
            .json(&json!({ "name": "show_notification", "payload": {} })),
        403,
    );
    assert!(other["message"].as_str().unwrap().contains("only an admin"));
    let (tablet, _) = api.pair("Tablet");
    api.settings(&tablet, json!({ "commands": ["import_history"] }));
    send(
        api.req(&phone, "POST", &format!("/api/devices/{tablet}/commands"))
            .json(&ask),
        403,
    );
    // An unknown device, and a device named rather than given by id.
    send(
        api.admin("POST", "/api/devices/nope/commands").json(&ask),
        404,
    );
    send(
        api.admin("POST", "/api/devices/Pixel/commands").json(&ask),
        404,
    );

    let queued = send(api.admin("POST", &path).json(&ask), 201);
    assert_eq!(queued["name"], "import_history");
    assert_eq!(queued["status"], "pending");
    assert_eq!(queued["source"], "settings");
    assert_eq!(queued["payload"], ask["payload"]);
    let id = queued["id"].as_str().unwrap();

    // The phone sees it pending, reports progress as it goes, then the result.
    let pending = api.commands(&phone, &pixel, "?status=pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["id"], id);
    api.report(&phone, &pixel, id, json!({ "status": "delivered" }), 200);
    let progress = json!({
        "progress": { "kinds": [{ "kind": "sms", "found": 120, "done": false }], "done": false },
    });
    let mid = api.report(
        &phone,
        &pixel,
        id,
        json!({ "status": "delivered", "result": progress }),
        200,
    );
    assert_eq!(mid["status"], "delivered");
    assert_eq!(mid["result"], progress);
    let shown = api.commands(&api.admin, &pixel, "");
    assert_eq!(shown[0]["result"]["progress"]["kinds"][0]["found"], 120);

    let result = json!({
        "kinds": [{ "kind": "sms", "found": 130, "notes": 4, "done": true }],
        "done": true,
        "message": "SMS: 130 found, 4 new notes",
    });
    let done = api.report(
        &phone,
        &pixel,
        id,
        json!({ "status": "done", "result": result }),
        200,
    );
    assert_eq!(done["status"], "done");
    assert_eq!(done["result"], result);
    // The phone asks itself, as its app's Settings > Devices does through the home server.
    let own = send(api.req(&phone, "POST", &path).json(&ask), 201);
    assert_eq!(own["name"], "import_history");
    assert_eq!(own["status"], "pending");
    // Finished: no more reports.
    api.report(
        &phone,
        &pixel,
        id,
        json!({ "status": "delivered", "result": progress }),
        409,
    );
}

fn put_script(api: &Api, name: &str, source: &str) -> Value {
    send(
        api.admin("PUT", &format!("/api/scripts/{name}"))
            .json(&json!({ "source": source })),
        200,
    )
}

fn problems(api: &Api, name: &str) -> Value {
    send(api.admin("GET", &format!("/api/scripts/{name}")), 200)["problems"].clone()
}

/// A vault whose `devices/android.wcl` is gone, however the server seeded it.
fn without_device_file(api: &Api) {
    let res = api
        .admin("GET", "/api/device-files/android")
        .send()
        .unwrap();
    if res.status().as_u16() == 200 {
        send(api.admin("DELETE", "/api/device-files/android"), 200);
    }
}

#[test]
fn device_scripts_put_before_the_device_file_naming_them_report_one_clear_problem_until_it_does() {
    let api = Api::start_seeded(|_| {});
    without_device_file(&api);

    // A script using `device` that no device file names reports exactly one clear problem, not
    // the compile errors of checking it as a task.
    // One that does not use it compiles as a task.
    let mut used = 0;
    for (name, source) in writer::DEFAULT_DEVICE_SCRIPTS {
        let expected = if source.lines().any(|l| l.trim() == "use device") {
            used += 1;
            json!([{
                "role": "device",
                "message": format!(
                    "script {name:?} uses the device module but no devices/<platform>.wcl names it"
                ),
            }])
        } else {
            json!([])
        };
        let out = put_script(&api, name, source);
        assert_eq!(out["problems"], expected, "{name}: {out}");
        assert_eq!(problems(&api, name), expected, "{name}");
    }
    assert!(used > 0, "no default device script uses the device module");

    // Putting the device file re-checks every script it names as a device script.
    let out = put_file(&api, "android", writer::DEFAULT_ANDROID_DEVICE_FILE, 200);
    let checked = out["scripts"].as_array().expect("scripts").clone();
    let names: Vec<&str> = checked
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    for (name, _) in writer::DEFAULT_DEVICE_SCRIPTS {
        assert!(names.contains(&name), "{name} not re-checked: {out}");
    }
    for s in &checked {
        assert_eq!(s["problems"], json!([]), "{s}");
    }
    for (name, _) in writer::DEFAULT_DEVICE_SCRIPTS {
        assert_eq!(problems(&api, name), json!([]), "{name}");
    }

    // A script that does not use `device` and that nothing names is still checked as a task.
    let out = put_script(&api, "broken", "fn run() -> Result[(), string] { nope }\n");
    let p = out["problems"].as_array().unwrap();
    assert_eq!(p.len(), 1, "{out}");
    assert_eq!(p[0]["role"], "task", "{out}");
}

#[test]
fn a_device_event_that_failed_for_good_becomes_one_bell_notification_from_the_device() {
    let api = Api::start();
    let (pixel, phone) = api.pair("Pixel");
    let (_, other) = api.pair("Nokia");
    let failure = json!({
        "id": 12,
        "event": "sms_received",
        "at": 1_767_573_000_000_i64,
        "who": "Ada Lovelace",
        "message": "the message note could not be written",
        "attempts": 3,
    });
    let path = format!("/api/devices/{pixel}/event-failures");
    let made = send(api.req(&phone, "POST", &path).json(&failure), 201);
    assert_eq!(made["source"], format!("device:{pixel}"));

    // The seed's own device scripts also earn a "newer version" notice, so count the device's.
    let source = format!("device:{pixel}");
    let from_device = |list: &Value| -> Vec<Value> {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["source"] == source.as_str())
            .cloned()
            .collect()
    };
    let list = send(api.admin("GET", "/api/notifications"), 200);
    let items = from_device(&list);
    assert_eq!(items.len(), 1, "{list}");
    let n = &items[0];
    assert_eq!(n["title"], "A message could not be saved");
    let body = n["body"].as_str().unwrap();
    for part in [
        "Ada Lovelace",
        "Pixel",
        "the message note could not be written",
        "3",
    ] {
        assert!(body.contains(part), "{part:?} in {body:?}");
    }
    assert_eq!(n["url"], "/settings/devices#this-device");
    assert_eq!(n["read"], Value::Null);
    assert!(list["unread"].as_u64().unwrap() >= 1, "{list}");

    // Another device may not report for it, and the event must be a device event.
    send(api.req(&other, "POST", &path).json(&failure), 403);
    let mut bad = failure.clone();
    bad["event"] = json!("sms_deleted");
    send(api.req(&phone, "POST", &path).json(&bad), 400);
    send(
        api.req(
            &phone,
            "POST",
            "/api/devices/01NODEVICE0000000000000000/event-failures",
        )
        .json(&failure),
        404,
    );
    let list = send(api.admin("GET", "/api/notifications"), 200);
    assert_eq!(from_device(&list).len(), 1, "{list}");

    // A call's failure says so.
    let mut call = failure.clone();
    call["event"] = json!("call_ended");
    call["who"] = Value::Null;
    let made = send(api.req(&phone, "POST", &path).json(&call), 201);
    assert_eq!(made["title"], "A call could not be saved");
}

#[test]
fn a_person_kept_off_the_phones_gets_no_contact_until_ticked_again_even_from_the_phone() {
    let api = Api::start_seeded(seed_defaults);
    let (pixel, pixel_client) = api.pair("Pixel");
    api.settings(
        &pixel,
        json!({ "contacts_write": true, "commands": ["upsert_contact", "delete_contact"] }),
    );

    // Unticked from the start: nothing for the phone.
    let erin = api.create_person(
        &api.admin,
        json!({ "title": "Erin Off", "phones": ["+16502530009"], "phone_contact": false }),
    );
    assert!(api.upserts(&pixel).is_empty());
    let mut renamed = erin.clone();
    renamed["note"]["fields"]["title"] = json!("Erin Still Off");
    let erin = api.update_note(&api.admin, &renamed);
    assert!(api.upserts(&pixel).is_empty());

    // Ticked again, with no detail changed: the contact goes back.
    let mut on = erin.clone();
    on["note"]["fields"]["phone_contact"] = json!(true);
    let erin = api.update_note(&api.admin, &on);
    let queued = api.upserts(&pixel);
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0]["payload"]["name"], "Erin Still Off");
    assert_eq!(queued[0]["payload"]["contact_id"], Value::Null);

    // Off again, then ticked on the phone itself: the phone gets the contact back too.
    let mut off = erin.clone();
    off["note"]["fields"]["phone_contact"] = json!(false);
    let erin = api.update_note(&api.admin, &off);
    assert_eq!(api.upserts(&pixel).len(), 1);
    let mut on = erin.clone();
    on["note"]["fields"]["phone_contact"] = json!(true);
    api.update_note(&pixel_client, &on);
    assert_eq!(api.upserts(&pixel).len(), 2);
}
