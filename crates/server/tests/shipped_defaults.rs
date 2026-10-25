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

const SCRIPTS: [&str; 9] = [
    "device_calls",
    "device_contacts",
    "device_import",
    "device_messages",
    "device_share",
    "google_calendar",
    "google_event",
    "google_gmail",
    "outbox_send",
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
    assert!(!fields.iter().any(|n| n == "email"), "{fields:?}");
    for f in [
        "emails",
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
    // The upgrade step moved the single email into emails.
    assert_eq!(
        alex["note"]["fields"]["emails"],
        serde_json::json!([{ "address": "alex@example.com" }]),
        "{alex}"
    );
    assert!(alex["note"]["fields"].get("email").is_none(), "{alex}");

    // The device file and the shipped scripts, none with a compile problem.
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

/// The starter's `person` type in 0.1.5, when `addresses` held email addresses beside `email`.
const PERSON_0_1_5: &str =
    include_str!("../../writer/tests/fixtures/shipped/person_0_1_5_starter.wcl");

const SAM_ID: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

const SAM: &str = r#"title     = "Sam Sample"
email     = "sam@example.com"
addresses = ["sam@home.example", "SAM@example.com"]
phones    = ["+61400000001"]
homepage  = "https://sam.example"
created   = "2026-10-01T00:00:00Z"
updated   = "2026-10-01T00:00:00Z"
"#;

#[test]
fn a_0_1_5_vault_starts_with_every_email_in_emails() {
    let server = TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(vault.join("types/person.wcl"), PERSON_0_1_5).unwrap();
        std::fs::create_dir_all(vault.join("notes/person")).unwrap();
        std::fs::write(vault.join(format!("notes/person/{ALEX_ID}.wcl")), ALEX).unwrap();
        std::fs::write(vault.join(format!("notes/person/{SAM_ID}.wcl")), SAM).unwrap();
    });
    let fields = field_names(&server, "person");
    assert!(!fields.iter().any(|n| n == "email"), "{fields:?}");
    for f in ["emails", "addresses", "homepage", "organisation", "photo"] {
        assert!(
            fields.iter().any(|n| n == f),
            "person lacks {f}: {fields:?}"
        );
    }
    let sam = get(&server, &format!("/api/notes/{SAM_ID}"))["note"]["fields"].clone();
    assert_eq!(
        sam["emails"],
        serde_json::json!([{ "address": "sam@example.com" }, { "address": "sam@home.example" }]),
        "{sam}"
    );
    assert_eq!(
        sam["phones"],
        serde_json::json!([{ "number": "+61400000001" }])
    );
    assert_eq!(sam["homepage"], "https://sam.example");
    let alex = get(&server, &format!("/api/notes/{ALEX_ID}"))["note"]["fields"].clone();
    assert_eq!(
        alex["emails"],
        serde_json::json!([{ "address": "alex@example.com" }]),
        "{alex}"
    );
    assert_eq!(alex["homepage"], "https://example.com");
    for person in [&sam, &alex] {
        assert!(person.get("email").is_none(), "{person}");
        assert!(person.get("addresses").is_none(), "{person}");
    }
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

/// The `google_sync` type 0.1.7 shipped (google-sync D21 removes it).
const GOOGLE_SYNC_0_1_7: &str =
    include_str!("../../writer/tests/fixtures/shipped/google_sync_0_1_7.wcl");

/// The `event` type 0.1.7 shipped.
const EVENT_0_1_7: &str = include_str!("../../writer/tests/fixtures/shipped/event_0_1_7.wcl");

const SYNC_ID: &str = "01JABCDEFGHJKMNPQRSTVWXYZ0";

/// A 0.1.7 vault: its `event`, `person` and `google_sync` types, and a settings note holding
/// labels and a history id when `note`.
fn vault_0_1_7(vault: &std::path::Path, note: bool) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/event.wcl"), EVENT_0_1_7).unwrap();
    std::fs::write(vault.join("types/person.wcl"), writer::DEFAULT_PERSON_TYPE).unwrap();
    std::fs::write(vault.join("types/google_sync.wcl"), GOOGLE_SYNC_0_1_7).unwrap();
    if note {
        std::fs::create_dir_all(vault.join("notes/google_sync")).unwrap();
        std::fs::write(
            vault.join(format!("notes/google_sync/{SYNC_ID}.wcl")),
            "title = \"Google\"\ncreated = \"2026-10-01T00:00:00Z\"\nupdated = \"2026-10-01T00:00:00Z\"\n\
             gmail_labels = [\"Work\", \"INBOX\"]\naccount = \"me@example.com\"\n\
             gmail_history_id = \"12345\"\n",
        )
        .unwrap();
    }
}

fn type_names(server: &TestServer) -> Vec<String> {
    get(server, "/api/types")["types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_0_1_7_vault_moves_its_google_sync_note_into_secrets_once() {
    let mut server = TestServer::start_seeded(|vault| vault_0_1_7(vault, true));

    let settings = server.secret_json("google_settings");
    assert_eq!(
        settings["gmail_labels"],
        serde_json::json!(["Work", "INBOX"])
    );
    assert_eq!(settings["gmail_mode"], "labels");
    assert_eq!(settings["calendars"], serde_json::json!(["primary"]));
    assert_eq!(
        server.secret_json("google_gmail_state")["history_id"],
        "12345"
    );
    assert!(server.secret("google_calendar_state").is_some());
    assert_eq!(
        server.secret("google_account").as_deref(),
        Some("me@example.com")
    );
    assert!(!type_names(&server).iter().any(|t| t == "google_sync"));
    assert!(
        !server
            .vault_path()
            .join(format!("notes/google_sync/{SYNC_ID}.wcl"))
            .exists()
    );
    let trash = get(&server, "/api/trash");
    assert!(trash["notes"].as_array().unwrap().is_empty(), "{trash}");
    let status = get(&server, "/api/integrations/google");
    assert_eq!(status["account"], "me@example.com");
    assert_eq!(
        status["settings"]["gmail_labels"],
        serde_json::json!(["Work", "INBOX"])
    );

    // A second start does nothing.
    let before = generation(&server);
    let secrets: Vec<Option<String>> = ["google_settings", "google_gmail_state", "google_account"]
        .iter()
        .map(|n| server.secret(n))
        .collect();
    server.restart();
    assert_eq!(generation(&server), before);
    let after: Vec<Option<String>> = ["google_settings", "google_gmail_state", "google_account"]
        .iter()
        .map(|n| server.secret(n))
        .collect();
    assert_eq!(after, secrets);
}

#[test]
fn a_0_1_7_vault_with_the_type_and_no_note_just_loses_the_type() {
    let server = TestServer::start_seeded(|vault| vault_0_1_7(vault, false));
    assert!(!type_names(&server).iter().any(|t| t == "google_sync"));
    assert!(!server.vault_path().join("types/google_sync.wcl").exists());
    for name in [
        "google_settings",
        "google_gmail_state",
        "google_calendar_state",
        "google_account",
    ] {
        assert!(server.secret(name).is_none(), "{name}");
    }
}

/// A user's own `event` type, made before `event` was a built-in: unrelated fields, no
/// `google_ref`.
const USER_EVENT: &str = r#"label  = "Event"
plural = "Events"

field venue {
  kind  = "string"
  label = "Venue"
}

template default {
  tab "Event" {
    slot "title"
    slot "venue"
  }
}
"#;

fn type_json(server: &TestServer, name: &str) -> Value {
    get(server, "/api/types")["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no type {name}"))
        .clone()
}

#[test]
fn a_users_own_event_type_is_left_alone_can_be_deleted_and_then_becomes_the_built_in() {
    let mut server = TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(vault.join("types/event.wcl"), USER_EVENT).unwrap();
    });
    let before = read(&server, "types/event.wcl");
    assert_eq!(field_names(&server, "event"), ["venue"]);
    assert_eq!(type_json(&server, "event")["builtin"], false);
    assert_eq!(type_json(&server, "call")["builtin"], true);

    // A restart leaves it alone too.
    server.restart();
    assert_eq!(read(&server, "types/event.wcl"), before);

    let res = Client::new()
        .delete(server.url("/api/types/event"))
        .header(AUTHORIZATION, format!("Bearer {}", server.admin_token()))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{}", res.text().unwrap());
    assert!(!type_names(&server).iter().any(|t| t == "event"));

    // The next start adds the built-in in its place.
    server.restart();
    let fields = field_names(&server, "event");
    assert!(fields.iter().any(|f| f == "google_ref"), "{fields:?}");
    assert_eq!(type_json(&server, "event")["builtin"], true);
    let res = Client::new()
        .delete(server.url("/api/types/event"))
        .header(AUTHORIZATION, format!("Bearer {}", server.admin_token()))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
}
