//! S1: on every start the server brings the vault's shipped defaults up to date: the person fields
//! the phone needs, the default device file and scripts, and newer shipped scripts (D15, D16,
//! D24).

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::Value;
use testkit::TestServer;

/// The `person` type `vault init --starter` wrote before people had phones.
const OLDEST_PERSON: &str =
    include_str!("../../writer/tests/fixtures/shipped/person_oldest_starter.wcl");

/// `scripts/device_calls.ws` as the first release with device scripts shipped it.
const OLD_DEVICE_CALLS: &str =
    include_str!("../../writer/tests/fixtures/shipped/device_calls_39d9091.ws");

const ALEX_ID: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";

const ALEX: &str = r#"title        = "Alex Example"
email        = "alex@example.com"
organisation = "Example Co"
homepage     = "https://example.com"
created      = "2026-10-01T00:00:00Z"
updated      = "2026-10-01T00:00:00Z"
"#;

const SCRIPTS: [&str; 5] = [
    "device_calls",
    "device_contacts",
    "device_import",
    "device_messages",
    "device_share",
];

fn get(server: &TestServer, path: &str) -> Value {
    let res: Response = Client::new()
        .get(server.url(path))
        .header(AUTHORIZATION, format!("Bearer {}", server.admin_token()))
        .send()
        .unwrap();
    let status = res.status();
    let text = res.text().unwrap();
    assert_eq!(status, StatusCode::OK, "{path}: {text}");
    serde_json::from_str(&text).unwrap()
}

fn field_names(server: &TestServer, type_name: &str) -> Vec<String> {
    let types = get(server, "/api/types");
    let ty = types["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == type_name)
        .unwrap_or_else(|| panic!("no type {type_name}"))
        .clone();
    ty["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_owned())
        .collect()
}

fn notification_titles(server: &TestServer) -> Vec<String> {
    get(server, "/api/notifications")["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap().to_owned())
        .collect()
}

fn generation(server: &TestServer) -> String {
    std::fs::read_to_string(server.vault_path().join(".vault/generation")).unwrap()
}

fn read(server: &TestServer, path: &str) -> String {
    std::fs::read_to_string(server.vault_path().join(path)).unwrap()
}

/// A vault as the oldest starter left it: a `person` type with only email, organisation,
/// homepage and body, one person, and no `devices/` or `scripts/`.
fn oldest_starter(vault: &std::path::Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/person.wcl"), OLDEST_PERSON).unwrap();
    std::fs::create_dir_all(vault.join("notes/person")).unwrap();
    std::fs::write(vault.join(format!("notes/person/{ALEX_ID}.wcl")), ALEX).unwrap();
}

#[test]
fn the_oldest_starter_vault_is_brought_up_to_date_once() {
    let mut server = TestServer::start_seeded(oldest_starter);
    let note = format!("notes/person/{ALEX_ID}.wcl");
    let note_text = read(&server, &note);

    let fields = field_names(&server, "person");
    for f in [
        "email",
        "phones",
        "addresses",
        "device_contacts",
        "organisation",
        "homepage",
        "body",
    ] {
        assert!(
            fields.iter().any(|n| n == f),
            "person lacks {f}: {fields:?}"
        );
    }
    let alex = get(&server, &format!("/api/notes/{ALEX_ID}"));
    assert_eq!(
        alex["note"]["fields"]["organisation"], "Example Co",
        "{alex}"
    );
    assert_eq!(
        alex["note"]["fields"]["homepage"], "https://example.com",
        "{alex}"
    );

    // The device file and the five scripts, none with a compile problem.
    assert_eq!(
        read(&server, "devices/android.wcl"),
        writer::DEFAULT_ANDROID_DEVICE_FILE
    );
    let listed = get(&server, "/api/scripts");
    assert_eq!(listed["scripts"], serde_json::json!(SCRIPTS));
    for name in SCRIPTS {
        let script = get(&server, &format!("/api/scripts/{name}"));
        assert_eq!(
            script["problems"],
            serde_json::json!([]),
            "{name}: {script}"
        );
    }
    assert!(notification_titles(&server).is_empty());

    // A second start changes nothing.
    let gen_before = generation(&server);
    let feed_before = get(&server, "/api/sync/changes")["latest"].clone();
    let history_before = get(&server, &format!("/api/notes/{ALEX_ID}/history"));
    server.restart();
    assert_eq!(
        generation(&server),
        gen_before,
        "the second start committed"
    );
    assert_eq!(get(&server, "/api/sync/changes")["latest"], feed_before);
    assert_eq!(
        get(&server, &format!("/api/notes/{ALEX_ID}/history")),
        history_before
    );
    assert_eq!(read(&server, &note), note_text);
    assert!(notification_titles(&server).is_empty());
}

#[test]
fn an_edited_script_is_kept_and_named_once_and_an_old_one_is_replaced() {
    let edited = "// my contacts\n";
    let mut server = TestServer::start_seeded(|vault| {
        oldest_starter(vault);
        std::fs::create_dir_all(vault.join("scripts")).unwrap();
        std::fs::write(vault.join("scripts/device_calls.ws"), OLD_DEVICE_CALLS).unwrap();
        std::fs::write(vault.join("scripts/device_contacts.ws"), edited).unwrap();
    });
    assert_eq!(
        read(&server, "scripts/device_calls.ws"),
        writer::default_device_script("device_calls").unwrap()
    );
    assert_eq!(read(&server, "scripts/device_contacts.ws"), edited);
    let titles = notification_titles(&server);
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert!(
        titles[0].contains("scripts/device_contacts.ws"),
        "{titles:?}"
    );

    server.restart();
    assert_eq!(read(&server, "scripts/device_contacts.ws"), edited);
    assert_eq!(notification_titles(&server).len(), 1);
}

#[test]
fn a_deleted_device_script_comes_back_on_the_next_start() {
    let mut server = TestServer::start();
    std::fs::remove_file(server.vault_path().join("devices/android.wcl")).unwrap();
    std::fs::remove_file(server.vault_path().join("scripts/device_share.ws")).unwrap();
    server.restart();
    assert_eq!(
        read(&server, "scripts/device_share.ws"),
        writer::default_device_script("device_share").unwrap()
    );
    assert_eq!(
        read(&server, "devices/android.wcl"),
        writer::DEFAULT_ANDROID_DEVICE_FILE
    );
}
