//! Keeping a vault's shipped defaults up to date on start: the built-in types and the fields
//! they gained, the default device file and scripts, shipped file upgrades, and the one-time
//! upgrade steps (D15, D16, D24).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use state::{Author, State};
use store::Vault;
use writer::{
    SHIPPED_FILES, ShippedFile, UpgradeStep, Upgraded, Upkeep, Writer, content_hash, shipped_hash,
};

/// The `person` type `vault init --starter` wrote before people had phones.
const OLDEST_PERSON: &str = include_str!("fixtures/shipped/person_oldest_starter.wcl");

/// The starter's `person` type as it is now.
const STARTER_PERSON: &str = include_str!("../../cli/starter/types/person.wcl");

/// The starter's `person` type in 0.1.5, when `addresses` held email addresses beside `email`.
const PERSON_0_1_5: &str = include_str!("fixtures/shipped/person_0_1_5_starter.wcl");

/// The starter's `person` type in 0.1.6, before values and their labels were one chip.
const PERSON_0_1_6: &str = include_str!("fixtures/shipped/person_0_1_6_starter.wcl");

/// The built-in default `person` type in 0.1.6.
const DEFAULT_PERSON_0_1_6: &str = include_str!("fixtures/shipped/person_0_1_6_default.wcl");

/// `scripts/device_calls.ws` as the first release with device scripts shipped it.
const OLD_DEVICE_CALLS: &str = include_str!("fixtures/shipped/device_calls_39d9091.ws");

/// `devices/android.wcl` as the first release with device files shipped it.
const OLD_ANDROID: &str = include_str!("fixtures/shipped/android_39d9091.wcl");

/// The `event` type as main shipped it before the Google Calendar write-back, with no handlers.
const EVENT_BEFORE_HANDLERS: &str = include_str!("fixtures/shipped/event_4718609.wcl");

const ALEX: &str = r#"title        = "Alex Example"
email        = "alex@example.com"
organisation = "Example Co"
homepage     = "https://example.com"
"#;

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn generation(root: &Path) -> String {
    std::fs::read_to_string(root.join(".vault/generation")).unwrap_or_default()
}

fn read(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path)).ok()
}

fn notification_titles(w: &Writer) -> Vec<String> {
    w.state()
        .notifications(false, 100, None)
        .unwrap()
        .items
        .into_iter()
        .map(|n| n.title)
        .collect()
}

/// The fields of note `id`, in the JSON form of the notes API.
fn fields_of(w: &Writer, id: &str) -> serde_json::Value {
    let note = w.read(id).unwrap();
    writer::doc_to_json(&note.doc)["fields"].clone()
}

fn shipped(path: &str) -> String {
    let file = SHIPPED_FILES.iter().find(|f| f.path == path).unwrap();
    if path.ends_with(".wcl") {
        store::canonical(file.source, path).unwrap()
    } else {
        file.source.to_owned()
    }
}

/// A vault as the oldest starter left it: a `person` type with only email, organisation,
/// homepage and body, one person note, and no `devices/` or `scripts/` files. Returns the
/// person's id.
fn oldest_starter(root: &Path) -> String {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/person.wcl"), OLDEST_PERSON).unwrap();
    let w = open(root);
    w.create("person", &writer::Input::Wcl(ALEX.into()), &author())
        .unwrap()
        .id
}

#[test]
fn the_oldest_starter_vault_gains_the_person_fields_and_the_device_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let alex = oldest_starter(root);
    let note_path = format!("notes/person/{alex}.wcl");
    let note_before = read(root, &note_path).unwrap();
    let w = open(root);
    let versions_before = w.history(&alex).unwrap().len();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    // The step moved email into emails; the seeding added the rest.
    assert_eq!(
        done.steps,
        [
            "0001_person_emails",
            "0002_person_labels",
            "0003_event_google_handler",
            "0004_remove_google_sync",
            "0005_list_page_defaults",
            "0006_assistant_settings",
            "0007_person_correspondence",
            "0008_outbox_template",
            "0009_outbox_statuses"
        ]
    );
    let fields: Vec<&str> = done
        .fields_added
        .iter()
        .map(|(t, f)| {
            assert_eq!(t, "person");
            f.as_str()
        })
        .collect();
    // Step 0002 added email_labels with the labels of emails.
    assert_eq!(
        fields,
        [
            "phones",
            "phone_labels",
            "addresses",
            "address_labels",
            "job_title",
            "nickname",
            "birthday",
            "websites",
            "contact_note",
            "phone_contact",
            "photo",
            "device_contacts",
        ]
    );
    // Laid out as the starter now writes it: organisation and homepage kept, the new fields
    // in the shipped order, the Work tab untouched, and the card's photo before its title, where
    // the shipped card puts it.
    assert_eq!(
        w.read_type("person").unwrap(),
        store::canonical(STARTER_PERSON, "types/person.wcl").unwrap()
    );
    let mut added = done.types_added.clone();
    added.sort();
    assert_eq!(
        added,
        [
            "agent_memory",
            "agent_thread",
            "call",
            "conversation",
            "dailynote",
            "email",
            "email_thread",
            "event",
            "fleeting",
            "message",
            "outbox"
        ]
    );
    assert_eq!(done.taxonomies_added, ["gmail_label"]);
    let ty = w.schema();
    let person = ty.type_def("person").unwrap();
    assert!(person.field("email").is_none());
    for f in [
        "emails",
        "organisation",
        "homepage",
        "body",
        "phones",
        "addresses",
        "device_contacts",
    ] {
        assert!(person.field(f).is_some(), "person lacks {f}");
    }

    // The one note the step edits: its email is now its first entry in emails, one version more.
    let note_after = read(root, &note_path).unwrap();
    assert_ne!(note_after, note_before);
    let fields = fields_of(&w, &alex);
    assert_eq!(fields["emails"], json!(["alex@example.com"]));
    assert!(fields.get("email").is_none(), "{fields}");
    assert_eq!(fields["homepage"], "https://example.com");
    assert_eq!(w.history(&alex).unwrap().len(), versions_before + 1);

    // The device file, the device scripts, and the Google sync tasks and scripts.
    let paths: Vec<&str> = SHIPPED_FILES.iter().map(|f| f.path).collect();
    assert_eq!(done.files_added, paths);
    for path in paths {
        assert_eq!(read(root, path).unwrap(), shipped(path), "{path}");
    }
    assert_eq!(
        w.scripts().unwrap(),
        [
            "device_calls",
            "device_contacts",
            "device_import",
            "device_messages",
            "device_share",
            "google_calendar",
            "google_event",
            "google_gmail",
            "outbox_send"
        ]
    );
    assert!(notification_titles(&w).is_empty());

    // A person note can now hold phones.
    let edited = format!("{ALEX}phones = [\"+61400000000\"]\n");
    let base = w.current_version(&alex).unwrap();
    w.update(
        &alex,
        &writer::Input::Wcl(edited),
        base.as_deref(),
        &author(),
    )
    .unwrap();
}

#[test]
fn a_second_start_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let alex = oldest_starter(root);
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    let gen_before = generation(root);
    let w = open(root);
    let feed_before = w.state().changes_since(0, 10_000).unwrap().latest;
    let versions_before = w.history(&alex).unwrap().len();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert_eq!(done, Upgraded::default());
    assert_eq!(generation(root), gen_before, "a second start made a commit");
    assert_eq!(
        w.state().changes_since(0, 10_000).unwrap().latest,
        feed_before
    );
    assert_eq!(w.history(&alex).unwrap().len(), versions_before);
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn a_new_vault_gets_every_built_in_type_and_the_device_files_in_one_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    let before: u64 = generation(root).trim().parse().unwrap_or(0);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let after: u64 = generation(root).trim().parse().unwrap();
    assert_eq!(after, before + 1);
    assert!(done.types_added.iter().any(|t| t == "person"));
    assert_eq!(done.files_added.len(), SHIPPED_FILES.len());
    assert!(done.fields_added.is_empty());
}

#[test]
fn an_edited_script_is_kept_with_one_notification_and_an_unedited_old_one_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oldest_starter(root);
    // A vault from the first release with device scripts: every file as it shipped then, with
    // device_contacts edited by the user and no record of what was written.
    let edited = "// my contacts\nfn changed(event) {\n}\n";
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::create_dir_all(root.join("devices")).unwrap();
    for file in SHIPPED_FILES {
        std::fs::write(root.join(file.path), shipped(file.path)).unwrap();
    }
    std::fs::write(root.join("scripts/device_calls.ws"), OLD_DEVICE_CALLS).unwrap();
    std::fs::write(root.join("devices/android.wcl"), OLD_ANDROID).unwrap();
    std::fs::write(root.join("scripts/device_contacts.ws"), edited).unwrap();
    assert_eq!(
        store::canonical(OLD_ANDROID, "devices/android.wcl").unwrap(),
        OLD_ANDROID,
        "the old device file is canonical, so its listed hash matches the vault's text"
    );
    assert!(
        SHIPPED_FILES
            .iter()
            .any(|f| f.hashes.contains(&content_hash(OLD_DEVICE_CALLS).as_str()))
    );

    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert_eq!(
        done.files_upgraded,
        ["scripts/device_calls.ws", "devices/android.wcl"]
    );
    assert_eq!(done.files_kept, ["scripts/device_contacts.ws"]);
    assert_eq!(
        read(root, "scripts/device_calls.ws").unwrap(),
        shipped("scripts/device_calls.ws")
    );
    assert_eq!(
        read(root, "devices/android.wcl").unwrap(),
        shipped("devices/android.wcl")
    );
    assert_eq!(read(root, "scripts/device_contacts.ws").unwrap(), edited);
    let titles = notification_titles(&w);
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert!(
        titles[0].contains("scripts/device_contacts.ws"),
        "{titles:?}"
    );
    drop(w);

    // The next start neither replaces it nor tells the user again.
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(done, Upgraded::default());
    assert_eq!(read(root, "scripts/device_contacts.ws").unwrap(), edited);
    assert_eq!(notification_titles(&w).len(), 1);
}

#[test]
fn a_script_the_server_wrote_and_the_user_then_edited_earns_no_notice_until_a_newer_one_ships() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    let edited = "// mine\n";
    std::fs::write(root.join("scripts/device_share.ws"), edited).unwrap();
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(done, Upgraded::default());
    assert_eq!(read(root, "scripts/device_share.ws").unwrap(), edited);
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn a_deleted_device_script_is_added_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        w.delete_device_file("android").unwrap();
        w.delete_script("device_import").unwrap();
    }
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(
        done.files_added,
        ["scripts/device_import.ws", "devices/android.wcl"]
    );
    assert_eq!(
        read(root, "scripts/device_import.ws").unwrap(),
        shipped("scripts/device_import.ws")
    );
}

#[test]
fn a_device_vault_tells_no_one_about_an_edited_script() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/device_contacts.ws"), "// mine\n").unwrap();
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Device).unwrap();
    assert!(done.files_kept.is_empty());
    assert_eq!(
        read(root, "scripts/device_contacts.ws").unwrap(),
        "// mine\n"
    );
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn a_field_the_user_changed_is_left_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    // `phones` as a single string and no default template slot for it: kept as it is.
    let person = OLDEST_PERSON.replace(
        "field organisation {",
        "field phones {\n  kind  = \"string\"\n  label = \"Phone\"\n}\n\nfield organisation {",
    );
    std::fs::write(root.join("types/person.wcl"), &person).unwrap();
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let fields: Vec<&str> = done.fields_added.iter().map(|(_, f)| f.as_str()).collect();
    assert_eq!(
        fields,
        [
            "phone_labels",
            "addresses",
            "address_labels",
            "job_title",
            "nickname",
            "birthday",
            "websites",
            "contact_note",
            "phone_contact",
            "photo",
            "device_contacts"
        ]
    );
    let schema = w.schema();
    let phones = schema.type_def("person").unwrap().field("phones").unwrap();
    assert_eq!(phones.label, "Phone");
}

#[test]
fn a_0_1_5_vault_moves_every_email_into_emails_and_keeps_the_users_fields() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/person.wcl"), PERSON_0_1_5).unwrap();
    let w = open(root);
    let make = |fields: serde_json::Value| {
        w.create(
            "person",
            &writer::Input::Json(json!({ "fields": fields })),
            &author(),
        )
        .unwrap()
        .id
    };
    // Both, with the single email also among the addresses in another case.
    let sam = make(json!({
        "title": "Sam",
        "email": "Sam@Example.com",
        "addresses": ["sam@home.example", "sam@example.com"],
        "phones": ["+61400000001"],
        "homepage": "https://sam.example",
    }));
    let only_email = make(json!({ "title": "Kim", "email": "kim@example.com" }));
    let only_addresses = make(json!({
        "title": "Lee",
        "addresses": ["lee@example.com", "lee@work.example"],
    }));
    let empty_email = make(json!({ "title": "Ash", "email": "" }));
    let neither = make(json!({ "title": "Bo" }));
    drop(w);

    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(
        done.steps,
        [
            "0001_person_emails",
            "0002_person_labels",
            "0003_event_google_handler",
            "0004_remove_google_sync",
            "0005_list_page_defaults",
            "0006_assistant_settings",
            "0007_person_correspondence",
            "0008_outbox_template",
            "0009_outbox_statuses"
        ]
    );
    // Seeding added postal addresses after the step had moved the email addresses out of it.
    assert!(
        done.fields_added
            .contains(&("person".to_owned(), "addresses".to_owned())),
        "{:?}",
        done.fields_added
    );

    let schema = w.schema();
    let person = schema.type_def("person").unwrap();
    assert!(person.field("email").is_none());
    for f in [
        "emails",
        "email_labels",
        "phones",
        "phone_labels",
        "addresses",
        "address_labels",
        "organisation",
        "job_title",
        "nickname",
        "birthday",
        "websites",
        "contact_note",
        "phone_contact",
        "photo",
        "homepage",
        "device_contacts",
        "body",
    ] {
        assert!(person.field(f).is_some(), "person lacks {f}");
    }
    let wcl = w.read_type("person").unwrap();
    assert!(!wcl.contains("slot \"email\""), "{wcl}");
    assert_eq!(wcl.matches("slot \"emails\"").count(), 1, "{wcl}");

    let f = fields_of(&w, &sam);
    assert_eq!(
        f["emails"],
        json!(["Sam@Example.com", "sam@home.example"]),
        "{f}"
    );
    assert_eq!(f["phones"], json!(["+61400000001"]));
    assert_eq!(f["homepage"], "https://sam.example");
    let f = fields_of(&w, &only_email);
    assert_eq!(f["emails"], json!(["kim@example.com"]));
    let f = fields_of(&w, &only_addresses);
    assert_eq!(f["emails"], json!(["lee@example.com", "lee@work.example"]));
    for id in [&sam, &only_email, &only_addresses, &empty_email, &neither] {
        let f = fields_of(&w, id);
        assert!(f.get("email").is_none(), "{f}");
        assert!(
            f.get("addresses")
                .is_none_or(|a| a.as_array().is_some_and(Vec::is_empty)),
            "{f}"
        );
    }
    assert!(w.flagged().unwrap().is_empty());
    drop(w);

    // The next start runs the step no more and changes nothing.
    let gen_before = generation(root);
    let w = open(root);
    assert_eq!(
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap(),
        Upgraded::default()
    );
    assert_eq!(generation(root), gen_before);
}

#[test]
fn a_device_vault_runs_no_upgrade_step() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/person.wcl"), PERSON_0_1_5).unwrap();
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Device).unwrap();
    assert!(done.steps.is_empty());
    // The person type waits for the home server's step: nothing is added beside the old fields.
    assert!(!done.fields_added.iter().any(|(t, _)| t == "person"));
    assert_eq!(w.read_type("person").unwrap(), PERSON_0_1_5);
}

static RUNS: AtomicUsize = AtomicUsize::new(0);

fn count_run(_: &Writer) -> writer::Result<()> {
    RUNS.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

static FAILS: AtomicUsize = AtomicUsize::new(0);

fn fail_once(_: &Writer) -> writer::Result<()> {
    if FAILS.fetch_add(1, Ordering::SeqCst) == 0 {
        Err(writer::Error::BadRequest("not yet".into()))
    } else {
        Ok(())
    }
}

#[test]
fn an_upgrade_step_runs_once_and_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let steps = [UpgradeStep {
        id: "0001_count",
        run: count_run,
    }];
    {
        let w = open(root);
        assert_eq!(w.run_upgrade_steps(&steps).unwrap(), ["0001_count"]);
        assert_eq!(w.state().applied_upgrades().unwrap(), ["0001_count"]);
    }
    let w = open(root);
    assert!(w.run_upgrade_steps(&steps).unwrap().is_empty());
    assert_eq!(RUNS.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_upgrade_step_is_not_recorded_and_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let steps = [UpgradeStep {
        id: "0001_fail_once",
        run: fail_once,
    }];
    assert!(w.run_upgrade_steps(&steps).is_err());
    assert!(w.state().applied_upgrades().unwrap().is_empty());
    assert_eq!(w.run_upgrade_steps(&steps).unwrap(), ["0001_fail_once"]);
}

/// A vault holding `person_type` with one labelled person whose photo is `photo_file`, uploaded.
/// Returns the person's id.
fn labelled_person(root: &Path, person_type: &str, photo_file: &str) -> String {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/person.wcl"), person_type).unwrap();
    let w = open(root);
    let photo = w.put_asset(photo_file, vec![1, 2, 3]).unwrap();
    w.create(
        "person",
        &writer::Input::Json(json!({ "fields": {
            "title": "Sam",
            "phones": ["+61400000020", "+61400000021", "+61400000020"],
            "phone_labels": ["mobile", "home", "work"],
            "emails": ["sam@example.com"],
            "email_labels": ["home"],
            "photo": photo.id,
        }})),
        &author(),
    )
    .unwrap()
    .id
}

#[test]
fn a_0_1_6_vault_gains_labels_and_an_image_photo_and_no_note_changes() {
    for (name, person_type) in [("starter", PERSON_0_1_6), ("default", DEFAULT_PERSON_0_1_6)] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sam = labelled_person(root, person_type, "face.JPG");
        let note_path = format!("notes/person/{sam}.wcl");
        let note_before = read(root, &note_path).unwrap();
        let w = open(root);
        let versions_before = w.history(&sam).unwrap().len();
        let fields_before = fields_of(&w, &sam);

        let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        assert_eq!(
            done.steps,
            [
                "0001_person_emails",
                "0002_person_labels",
                "0003_event_google_handler",
                "0004_remove_google_sync",
                "0005_list_page_defaults",
                "0006_assistant_settings",
                "0007_person_correspondence",
                "0008_outbox_template",
                "0009_outbox_statuses"
            ],
            "{name}"
        );
        // phone_contact came after 0.1.6: the one field it gains.
        assert_eq!(
            done.fields_added,
            [("person".to_owned(), "phone_contact".to_owned())],
            "{name}"
        );

        let schema = w.schema();
        let person = schema.type_def("person").unwrap();
        for (v, l) in [
            ("emails", "email_labels"),
            ("phones", "phone_labels"),
            ("addresses", "address_labels"),
        ] {
            assert_eq!(
                person.field(v).unwrap().labels.as_deref(),
                Some(l),
                "{name}"
            );
            assert!(person.field(l).is_some(), "{name}: {l} kept");
        }
        assert_eq!(person.field("photo").unwrap().accept, schema::Accept::Image);
        let wcl = w.read_type("person").unwrap();
        for l in ["email_labels", "phone_labels", "address_labels"] {
            assert!(!wcl.contains(&format!("slot \"{l}\"")), "{name}: {wcl}");
        }
        // Everything else the type held is as it was: the starter keeps its homepage and Work
        // tab, the slots keep their order.
        let expect = store::canonical(person_type, "types/person.wcl")
            .unwrap()
            .replace("    slot \"email_labels\"\n", "")
            .replace("    slot \"phone_labels\"\n", "")
            .replace("    slot \"address_labels\"\n", "");
        let strip = |t: &str| {
            t.lines()
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|l| !l.starts_with("labels = ") && !l.starts_with("accept = "))
                .collect::<Vec<_>>()
        };
        // Step 0007 adds the Correspondence tab at the end of the default template.
        assert!(wcl.contains("tab \"Correspondence\""), "{name}: {wcl}");
        // The seeding adds phone_contact, its field after contact_note and its slot after the
        // contact note's; with both taken out, the type is the 0.1.6 one.
        assert!(wcl.contains("slot \"phone_contact\""), "{name}: {wcl}");
        let without_phone = |t: &str| {
            let start = t
                .find("field phone_contact {")
                .expect("the field was added");
            let end = start + t[start..].find("\n}\n").unwrap() + 3;
            let t = format!("{}{}", &t[..start], t[end..].trim_start_matches('\n'));
            t.replace("    slot \"phone_contact\"\n", "")
        };
        assert_eq!(
            strip(&without_phone(&without_correspondence(&wcl))),
            strip(&expect),
            "{name}"
        );

        // No note changed.
        assert_eq!(read(root, &note_path).unwrap(), note_before, "{name}");
        assert_eq!(w.history(&sam).unwrap().len(), versions_before, "{name}");
        assert_eq!(fields_of(&w, &sam), fields_before, "{name}");
        assert!(w.flagged().unwrap().is_empty());
        drop(w);

        // A second start does nothing.
        let gen_before = generation(root);
        let w = open(root);
        assert_eq!(
            w.upgrade_shipped_defaults(Upkeep::Home).unwrap(),
            Upgraded::default(),
            "{name}"
        );
        assert_eq!(generation(root), gen_before, "{name}");
    }
}

#[test]
fn a_photo_that_is_not_an_image_keeps_the_photo_field_open_to_any_asset() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let sam = labelled_person(root, PERSON_0_1_6, "cv.pdf");
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.steps.iter().any(|s| s == "0002_person_labels"));
    let schema = w.schema();
    let person = schema.type_def("person").unwrap();
    assert_eq!(person.field("photo").unwrap().accept, schema::Accept::Any);
    assert_eq!(
        person.field("phones").unwrap().labels.as_deref(),
        Some("phone_labels")
    );
    assert!(fields_of(&w, &sam)["photo"].is_string());
}

/// The Google sync's built-in types, which a vault from before them gains (google-sync D4, D6,
/// D15, D17).
const GOOGLE_TYPES: [&str; 3] = ["email", "email_thread", "event"];

/// Copy the vault fixture `from` into `to`, keeping its folders.
fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A vault as the 0.1.6 home server left it: its built-in types, its `person`, its device file
/// and scripts, step `0001_person_emails` recorded, and one person note. Returns the person's id.
fn vault_0_1_6(root: &Path) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shipped/vault_0_1_6");
    copy_tree(&fixture, root);
    let w = open(root);
    w.state().record_upgrade("0001_person_emails").unwrap();
    w.create(
        "person",
        &writer::Input::Json(json!({ "fields": {
            "title": "Alex", "emails": ["alex@example.com"],
        }})),
        &author(),
    )
    .unwrap()
    .id
}

#[test]
fn a_0_1_6_vault_gains_the_google_types_and_the_gmail_label_taxonomy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let alex = vault_0_1_6(root);
    let w = open(root);
    let versions_before = w.history(&alex).unwrap().len();
    let before: u64 = generation(root).trim().parse().unwrap();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert_eq!(done.taxonomies_added, ["gmail_label"]);
    let mut added = done.types_added.clone();
    added.sort();
    assert_eq!(added[..2], ["agent_memory", "agent_thread"]);
    assert_eq!(added[2..5], GOOGLE_TYPES);
    assert_eq!(added[5..], ["outbox"]);
    // Steps 0002 to 0005 are the ones 0.1.6 had not run; 0003 and 0004 find no event type yet,
    // and 0005 gives the message type 0.1.6 had its `list_page` block, in a commit of its own.
    // The seeding is then one commit, with no field added to a new type and no note edited.
    assert_eq!(
        done.steps,
        [
            "0002_person_labels",
            "0003_event_google_handler",
            "0004_remove_google_sync",
            "0005_list_page_defaults",
            "0006_assistant_settings",
            "0007_person_correspondence",
            "0008_outbox_template",
            "0009_outbox_statuses"
        ]
    );
    // One more for step 0007's Correspondence tab on the person type, and one for each of the
    // three types whose `multiple` fields become lists.
    assert_eq!(
        generation(root).trim().parse::<u64>().unwrap(),
        before + 4 + 3
    );
    // The message type 0.1.6 had gains only `processed`.
    assert!(
        done.fields_added
            .iter()
            .all(|(t, f)| t == "person" || (t == "message" && f == "processed")),
        "{:?}",
        done.fields_added
    );
    assert_eq!(w.history(&alex).unwrap().len(), versions_before);

    let sources = [
        ("email", writer::DEFAULT_EMAIL_TYPE),
        ("email_thread", writer::DEFAULT_EMAIL_THREAD_TYPE),
        ("event", writer::DEFAULT_EVENT_TYPE),
    ];
    for (name, source) in sources {
        let path = format!("types/{name}.wcl");
        assert_eq!(
            read(root, &path).unwrap(),
            store::canonical(source, &path).unwrap(),
            "{name}"
        );
        assert!(writer::is_built_in_type(name), "{name}");
    }
    assert_eq!(
        read(root, "taxonomies/gmail_label.wcl").unwrap(),
        store::canonical(
            writer::DEFAULT_GMAIL_LABEL_TAXONOMY,
            "taxonomies/gmail_label.wcl"
        )
        .unwrap()
    );
    let schema = w.schema();
    let labels = schema.taxonomy("gmail_label").unwrap();
    assert!(labels.allow_new);
    for name in GOOGLE_TYPES {
        assert!(schema.type_def(name).is_some(), "{name}");
    }

    // An email can now carry a Gmail system label and a new one of its own.
    let email = w
        .create(
            "email",
            &writer::Input::Json(json!({ "fields": {
                "title": "Hello", "from": alex, "labels": ["system/INBOX", "Receipts"],
            }})),
            &author(),
        )
        .unwrap()
        .id;
    assert_eq!(
        fields_of(&w, &email)["labels"],
        json!(["system/INBOX", "Receipts"])
    );
}

#[test]
fn a_0_1_6_vault_changes_nothing_on_its_second_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let alex = vault_0_1_6(root);
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    let gen_before = generation(root);
    let taxonomy_before = read(root, "taxonomies/gmail_label.wcl").unwrap();
    let w = open(root);
    let feed_before = w.state().changes_since(0, 10_000).unwrap().latest;
    let versions_before = w.history(&alex).unwrap().len();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert_eq!(done, Upgraded::default());
    assert_eq!(generation(root), gen_before, "a second start made a commit");
    assert_eq!(
        w.state().changes_since(0, 10_000).unwrap().latest,
        feed_before
    );
    assert_eq!(w.history(&alex).unwrap().len(), versions_before);
    assert_eq!(
        read(root, "taxonomies/gmail_label.wcl").unwrap(),
        taxonomy_before
    );
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn a_gmail_label_taxonomy_the_sync_grew_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    vault_0_1_6(root);
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        // The Gmail sync adds the labels it meets to the open taxonomy.
        w.create(
            "email",
            &writer::Input::Json(json!({ "fields": { "title": "Hi", "labels": ["Work"] } })),
            &author(),
        )
        .unwrap();
    }
    let grown = read(root, "taxonomies/gmail_label.wcl").unwrap();
    assert!(grown.contains("term \"Work\""), "{grown}");
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.taxonomies_added.is_empty());
    assert_eq!(read(root, "taxonomies/gmail_label.wcl").unwrap(), grown);
}

#[test]
fn an_event_type_from_before_the_write_back_gains_its_handlers_and_no_note_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/event.wcl"), EVENT_BEFORE_HANDLERS).unwrap();
    std::fs::write(root.join("types/person.wcl"), writer::DEFAULT_PERSON_TYPE).unwrap();
    let w = open(root);
    assert!(w.schema().type_def("event").unwrap().handlers.is_empty());
    let id = w
        .create(
            "event",
            &writer::Input::Json(json!({ "fields": {
                "title": "Standup", "when": { "start": "2026-03-02", "end": "2026-03-02" },
            }})),
            &author(),
        )
        .unwrap()
        .id;
    let note_path = format!("notes/event/{id}.wcl");
    let note_before = read(root, &note_path).unwrap();
    let versions_before = w.history(&id).unwrap().len();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert!(
        done.steps.iter().any(|s| s == "0003_event_google_handler"),
        "{:?}",
        done.steps
    );
    // The type is now the shipped one: the handlers were all it lacked.
    assert_eq!(
        read(root, "types/event.wcl").unwrap(),
        store::canonical(writer::DEFAULT_EVENT_TYPE, "types/event.wcl").unwrap()
    );
    let schema = w.schema();
    let handlers = &schema.type_def("event").unwrap().handlers;
    let events: Vec<&str> = handlers.iter().map(|h| h.event.as_str()).collect();
    assert_eq!(events, ["after_create", "after_update", "after_delete"]);
    for h in handlers {
        assert_eq!(h.script, "google_event");
        assert_eq!(
            h.allow_domains,
            ["oauth2.googleapis.com", "www.googleapis.com"]
        );
        assert_eq!(
            h.secrets,
            [
                "google_client_id",
                "google_client_secret",
                "google_refresh_token",
                "google_settings"
            ]
        );
    }
    assert_eq!(
        read(root, "scripts/google_event.ws").unwrap(),
        shipped("scripts/google_event.ws")
    );
    assert_eq!(read(root, &note_path).unwrap(), note_before);
    assert_eq!(w.history(&id).unwrap().len(), versions_before);

    // The step is recorded; a second start changes nothing.
    let type_after = read(root, "types/event.wcl").unwrap();
    let before = generation(root);
    let again = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(again.steps.is_empty(), "{:?}", again.steps);
    assert_eq!(generation(root), before);
    assert_eq!(read(root, "types/event.wcl").unwrap(), type_after);
}

const GMAIL_TASK: &str = "tasks/google_gmail.wcl";

/// Set the `paused` field of the shipped Gmail task as the scheduler's pause switch does, with
/// the rest of the file as it is on disk.
fn set_gmail_paused(w: &Writer, root: &Path, paused: bool) {
    let text = read(root, GMAIL_TASK).unwrap();
    let edited: String = text
        .lines()
        .map(|l| {
            if l.starts_with("paused") {
                format!("paused = {paused}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    assert_ne!(edited, text);
    w.put_task("google_gmail", &edited, |_| Ok(())).unwrap();
}

/// The shipped files with a newer release's Gmail task, whose timeout changed, in place of the
/// current one. Its hash list ends with the newer version's, after every shipped one.
fn with_newer_gmail_task() -> (Vec<ShippedFile>, String) {
    let current = shipped(GMAIL_TASK);
    let newer = current.replace("timeout       = \"PT5M\"", "timeout       = \"PT10M\"");
    assert_ne!(newer, current);
    let newer: &'static str = Box::leak(newer.into_boxed_str());
    let files = SHIPPED_FILES
        .iter()
        .map(|f| {
            if f.path != GMAIL_TASK {
                return *f;
            }
            let mut hashes: Vec<&'static str> = f.hashes.to_vec();
            hashes.push(Box::leak(shipped_hash(GMAIL_TASK, newer).into_boxed_str()));
            ShippedFile {
                path: f.path,
                source: newer,
                hashes: Box::leak(hashes.into_boxed_slice()),
            }
        })
        .collect();
    (files, newer.to_owned())
}

#[test]
fn pausing_a_shipped_task_alone_earns_no_notice_and_is_kept_on_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        set_gmail_paused(&w, root, false);
    }
    let unpaused = read(root, GMAIL_TASK).unwrap();
    let gen_before = generation(root);

    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert_eq!(done, Upgraded::default());
    assert_eq!(generation(root), gen_before, "the start made a commit");
    assert_eq!(read(root, GMAIL_TASK).unwrap(), unpaused);
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn an_unpaused_shipped_task_is_upgraded_to_a_newer_version_and_stays_unpaused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        set_gmail_paused(&w, root, false);
    }
    let (files, newer) = with_newer_gmail_task();

    let w = open(root);
    let done = w.seed_shipped_defaults_with(Upkeep::Home, &files).unwrap();

    assert_eq!(done.files_upgraded, [GMAIL_TASK]);
    assert!(done.files_kept.is_empty());
    let text = read(root, GMAIL_TASK).unwrap();
    assert!(text.contains("timeout       = \"PT10M\""), "{text}");
    assert!(text.contains("paused        = false"), "{text}");
    assert!(!text.contains("paused        = true"), "{text}");
    assert_eq!(
        text,
        newer.replace("paused        = true", "paused        = false")
    );
    assert!(notification_titles(&w).is_empty());
    drop(w);

    // The next start finds the newer version current and changes nothing.
    let w = open(root);
    let gen_before = generation(root);
    let done = w.seed_shipped_defaults_with(Upkeep::Home, &files).unwrap();
    assert_eq!(done, Upgraded::default());
    assert_eq!(generation(root), gen_before);
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn an_unpaused_shipped_task_with_no_record_of_what_was_written_is_upgraded_and_stays_unpaused() {
    // A vault holding the shipped task, unpaused by a connect, with no record of what the
    // server wrote: the hash list alone says it is unedited.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(
        root.join("scripts/google_gmail.ws"),
        shipped("scripts/google_gmail.ws"),
    )
    .unwrap();
    std::fs::write(
        root.join(GMAIL_TASK),
        shipped(GMAIL_TASK).replace("paused        = true", "paused = false"),
    )
    .unwrap();
    let (files, newer) = with_newer_gmail_task();

    let w = open(root);
    let done = w.seed_shipped_defaults_with(Upkeep::Home, &files).unwrap();

    assert_eq!(done.files_upgraded, [GMAIL_TASK]);
    assert_eq!(
        read(root, GMAIL_TASK).unwrap(),
        newer.replace("paused        = true", "paused        = false")
    );
    assert!(notification_titles(&w).is_empty());
}

#[test]
fn a_schedule_edit_to_a_shipped_task_is_kept_and_notified_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        set_gmail_paused(&w, root, false);
        let text = read(root, GMAIL_TASK).unwrap();
        let edited = text.replace("*/10 * * * *", "*/30 * * * *");
        assert_ne!(edited, text);
        w.put_task("google_gmail", &edited, |_| Ok(())).unwrap();
    }
    let edited = read(root, GMAIL_TASK).unwrap();
    let (files, _) = with_newer_gmail_task();

    let w = open(root);
    let done = w.seed_shipped_defaults_with(Upkeep::Home, &files).unwrap();

    assert!(done.files_upgraded.is_empty());
    assert_eq!(done.files_kept, [GMAIL_TASK]);
    assert_eq!(read(root, GMAIL_TASK).unwrap(), edited);
    let titles = notification_titles(&w);
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert!(titles[0].contains(GMAIL_TASK), "{titles:?}");
    drop(w);

    let w = open(root);
    let done = w.seed_shipped_defaults_with(Upkeep::Home, &files).unwrap();
    assert_eq!(done, Upgraded::default());
    assert_eq!(notification_titles(&w).len(), 1);
}

/// The `google_sync` type 0.1.7 shipped, which step `0004_remove_google_sync` removes.
const GOOGLE_SYNC_0_1_7: &str = include_str!("fixtures/shipped/google_sync_0_1_7.wcl");

/// The `event` type 0.1.7 shipped, whose write-back handlers read no `google_settings`.
const EVENT_0_1_7: &str = include_str!("fixtures/shipped/event_0_1_7.wcl");

/// A 0.1.7 settings note: labels, a history id, calendar tokens, the account and the endpoints.
const SYNC_NOTE_0_1_7: &str = r#"title               = "Google"
created             = "2026-10-01T00:00:00Z"
updated             = "2026-10-18T09:05:00Z"
gmail_mode          = "labels"
gmail_labels        = ["Work", "INBOX"]
gmail_window_days   = 45
calendars           = ["primary", "team@example.com"]
default_calendar    = "primary"
account             = "me@example.com"
gmail_history_id    = "12345"
calendar_window_end = "2027-10-18"
last_gmail_run      = "2026-10-18T09:00:00Z"
last_gmail_error    = "invalid_grant: Token has been expired or revoked."
last_calendar_run   = "2026-10-18T09:05:00Z"

dict calendar_sync_tokens {
  c_primary                = "tok-primary"
  c_team_40example_2ecom   = "tok-team"
}

dict endpoints {
  token = "http://127.0.0.1:9/token"
}
"#;

/// The steps 0.1.7 had run.
const STEPS_0_1_7: [&str; 3] = [
    "0001_person_emails",
    "0002_person_labels",
    "0003_event_google_handler",
];

fn open_with(root: &Path, key: Option<state::SecretKey>) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), key).unwrap();
    Writer::open(vault, state).unwrap()
}

/// A vault as 0.1.7 left it: its `event` and `google_sync` types, the steps it had run, and a
/// settings note when `note`. Returns the note's id.
fn vault_0_1_7(root: &Path, key: Option<state::SecretKey>, note: bool) -> Option<String> {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/event.wcl"), EVENT_0_1_7).unwrap();
    std::fs::write(root.join("types/person.wcl"), writer::DEFAULT_PERSON_TYPE).unwrap();
    std::fs::write(root.join("types/google_sync.wcl"), GOOGLE_SYNC_0_1_7).unwrap();
    let w = open_with(root, key);
    for step in STEPS_0_1_7 {
        w.state().record_upgrade(step).unwrap();
    }
    drop(w);
    note.then(|| {
        // As the 0.1.7 server wrote it: only the server writes the computed fields.
        let id = "01JABCDEFGHJKMNPQRSTVWXYZ0";
        std::fs::create_dir_all(root.join("notes/google_sync")).unwrap();
        std::fs::write(
            root.join(format!("notes/google_sync/{id}.wcl")),
            SYNC_NOTE_0_1_7,
        )
        .unwrap();
        id.to_owned()
    })
}

fn secret_json(w: &Writer, name: &str) -> serde_json::Value {
    serde_json::from_str(&w.state().get_secret(name).unwrap().unwrap()).unwrap()
}

#[test]
fn a_0_1_7_vault_moves_its_google_sync_note_into_secrets_and_loses_the_type() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let key = state::SecretKey::generate();
    let id = vault_0_1_7(root, Some(key.clone()), true).unwrap();
    let w = open_with(root, Some(key));
    // A key a secret holds already is kept.
    w.state()
        .set_secret("google_calendar_state", r#"{"last_error":"kept"}"#)
        .unwrap();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(
        done.steps,
        [
            "0004_remove_google_sync",
            "0005_list_page_defaults",
            "0006_assistant_settings",
            "0007_person_correspondence",
            "0008_outbox_template",
            "0009_outbox_statuses"
        ]
    );

    assert_eq!(
        secret_json(&w, "google_settings"),
        json!({
            "gmail_mode": "labels",
            "gmail_labels": ["Work", "INBOX"],
            "gmail_window_days": 45,
            "gmail_backfill_to": null,
            "calendars": ["primary", "team@example.com"],
            "default_calendar": "primary",
            "endpoints": { "token": "http://127.0.0.1:9/token" },
        })
    );
    assert_eq!(
        secret_json(&w, "google_gmail_state"),
        json!({
            "history_id": "12345",
            "backfill_page": null,
            "last_run": "2026-10-18T09:00:00Z",
            "last_error": "invalid_grant: Token has been expired or revoked.",
            "notified_invalid_grant": true,
        })
    );
    assert_eq!(
        secret_json(&w, "google_calendar_state"),
        json!({
            "sync_tokens": { "primary": "tok-primary", "team@example.com": "tok-team" },
            "window_end": "2027-10-18",
            "last_run": "2026-10-18T09:05:00Z",
            "last_error": "kept",
            "notified_invalid_grant": false,
        })
    );
    assert_eq!(
        w.state().get_secret("google_account").unwrap().as_deref(),
        Some("me@example.com")
    );

    // No note, type or trash entry is left of it.
    assert!(w.schema().type_def("google_sync").is_none());
    assert!(read(root, "types/google_sync.wcl").is_none());
    assert!(read(root, &format!("notes/google_sync/{id}.wcl")).is_none());
    assert!(w.trash().unwrap().iter().all(|t| t.note_id != id));
    // The write-back handlers read the settings now.
    for h in &w.schema().type_def("event").unwrap().handlers {
        assert!(h.secrets.iter().any(|s| s == "google_settings"), "{h:?}");
    }

    // A second start does nothing.
    let before = generation(root);
    let secrets: Vec<Option<String>> = [
        "google_settings",
        "google_gmail_state",
        "google_calendar_state",
        "google_account",
    ]
    .iter()
    .map(|n| w.state().get_secret(n).unwrap())
    .collect();
    let again = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(again.steps.is_empty(), "{:?}", again.steps);
    assert_eq!(again, Upgraded::default());
    assert_eq!(generation(root), before);
    let after: Vec<Option<String>> = [
        "google_settings",
        "google_gmail_state",
        "google_calendar_state",
        "google_account",
    ]
    .iter()
    .map(|n| w.state().get_secret(n).unwrap())
    .collect();
    assert_eq!(after, secrets);
}

#[test]
fn a_0_1_7_vault_with_the_type_and_no_note_just_loses_the_type_even_without_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    vault_0_1_7(root, None, false);
    let w = open_with(root, None);

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(
        done.steps,
        [
            "0004_remove_google_sync",
            "0005_list_page_defaults",
            "0006_assistant_settings",
            "0007_person_correspondence",
            "0008_outbox_template",
            "0009_outbox_statuses"
        ]
    );
    assert!(w.schema().type_def("google_sync").is_none());
    assert!(read(root, "types/google_sync.wcl").is_none());
    assert!(w.state().list_secrets().unwrap().is_empty());
    assert!(
        w.state()
            .applied_upgrades()
            .unwrap()
            .iter()
            .any(|s| s == "0004_remove_google_sync")
    );
}

#[test]
fn without_the_secrets_key_a_settings_note_is_kept_and_the_step_runs_again_on_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let id = vault_0_1_7(root, None, true).unwrap();
    let note_path = format!("notes/google_sync/{id}.wcl");
    let note = read(root, &note_path).unwrap();
    {
        let w = open_with(root, None);
        let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        assert_eq!(
            done.steps,
            [
                "0005_list_page_defaults",
                "0006_assistant_settings",
                "0007_person_correspondence",
                "0008_outbox_template",
                "0009_outbox_statuses"
            ]
        );
        assert!(w.schema().type_def("google_sync").is_some());
        assert_eq!(read(root, &note_path).unwrap(), note);
        assert!(
            !w.state()
                .applied_upgrades()
                .unwrap()
                .iter()
                .any(|s| s == "0004_remove_google_sync")
        );
    }

    // The next start has the key, and finishes the step.
    let key = state::SecretKey::generate();
    let w = open_with(root, Some(key));
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(done.steps, ["0004_remove_google_sync"]);
    assert!(w.schema().type_def("google_sync").is_none());
    assert!(read(root, &note_path).is_none());
    assert_eq!(secret_json(&w, "google_gmail_state")["history_id"], "12345");
}

/// The `email` type as 0.1.13-alpha.4 shipped it, before `processed`.
const EMAIL_0_1_13: &str = include_str!("fixtures/shipped/email_0_1_13.wcl");

/// The `message` type as 0.1.13-alpha.4 shipped it, before `processed`.
const MESSAGE_0_1_13: &str = include_str!("fixtures/shipped/message_0_1_13.wcl");

/// The ids of the notes `text` matches, sorted.
fn matched(w: &Writer, text: &str) -> Vec<String> {
    let mut ids: Vec<String> = script_core::Vault::query(w, text, 1000)
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

#[test]
fn a_0_1_13_vault_gains_processed_on_email_and_message_and_no_note_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    for (name, old) in [("email", EMAIL_0_1_13), ("message", MESSAGE_0_1_13)] {
        let path = format!("types/{name}.wcl");
        std::fs::write(root.join(&path), store::canonical(old, &path).unwrap()).unwrap();
    }
    let (email, message) = {
        let w = open(root);
        assert!(
            w.schema()
                .type_def("email")
                .unwrap()
                .field("processed")
                .is_none()
        );
        let email = w
            .create(
                "email",
                &writer::Input::Json(json!({ "fields": { "title": "Hello" } })),
                &author(),
            )
            .unwrap()
            .id;
        let message = w
            .create(
                "message",
                &writer::Input::Json(json!({ "fields": { "title": "Hi", "device_ref": "sms:1" } })),
                &author(),
            )
            .unwrap()
            .id;
        (email, message)
    };
    let email_path = format!("notes/email/{email}.wcl");
    let message_path = format!("notes/message/{message}.wcl");
    let email_before = read(root, &email_path).unwrap();
    let message_before = read(root, &message_path).unwrap();
    assert!(!email_before.contains("processed"), "{email_before}");

    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    let mut added = done.fields_added.clone();
    added.sort();
    assert_eq!(
        added,
        [
            ("email".to_owned(), "processed".to_owned()),
            ("message".to_owned(), "processed".to_owned())
        ]
    );
    // The type is the shipped one, its card included: `processed` sits where the shipped card
    // puts it. The `list_page` block comes with step `0005_list_page_defaults`, which this vault
    // ran on its first start, before the old type files were written over the new ones.
    for name in ["email", "message"] {
        let path = format!("types/{name}.wcl");
        let source = if name == "email" {
            writer::DEFAULT_EMAIL_TYPE
        } else {
            writer::DEFAULT_MESSAGE_TYPE
        };
        assert_eq!(
            read(root, &path).unwrap(),
            without_list_page(&store::canonical(source, &path).unwrap()),
            "{name}"
        );
    }
    for name in ["email", "message"] {
        let schema = w.schema();
        let field = schema.type_def(name).unwrap().field("processed").unwrap();
        assert_eq!(field.label, "Processed", "{name}");
    }
    assert_eq!(read(root, &email_path).unwrap(), email_before);
    assert_eq!(read(root, &message_path).unwrap(), message_before);
    drop(w);

    // A second start does nothing.
    let gen_before = generation(root);
    let w = open(root);
    assert_eq!(
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap(),
        Upgraded::default()
    );
    assert_eq!(generation(root), gen_before, "a second start made a commit");

    // `!processed` matches the notes with no value as well as the false ones.
    let make = |ty: &str, fields: serde_json::Value| {
        w.create(
            ty,
            &writer::Input::Json(json!({ "fields": fields })),
            &author(),
        )
        .unwrap()
        .id
    };
    let email_false = make("email", json!({ "title": "Two", "processed": false }));
    let email_true = make("email", json!({ "title": "Three", "processed": true }));
    let message_false = make("message", json!({ "title": "Yo", "processed": false }));
    let message_true = make("message", json!({ "title": "Done", "processed": true }));
    let mut want = vec![email.clone(), email_false];
    want.sort();
    assert_eq!(matched(&w, "from email where !processed"), want);
    assert_eq!(matched(&w, "from email where processed"), [email_true]);
    let mut want = vec![message.clone(), message_false];
    want.sort();
    assert_eq!(matched(&w, "from message where !processed"), want);
    assert_eq!(matched(&w, "from message where processed"), [message_true]);
}

#[test]
fn a_vault_card_template_gains_no_slot_when_the_vault_type_has_no_card() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let path = "types/email.wcl";
    let old = store::canonical(EMAIL_0_1_13, path).unwrap();
    let card = old
        .find("\ntemplate card {")
        .expect("the 0.1.13 email has a card");
    std::fs::write(root.join(path), format!("{}\n", old[..card].trim_end())).unwrap();

    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert_eq!(
        done.fields_added,
        [("email".to_owned(), "processed".to_owned())]
    );
    let schema = w.schema();
    let email = schema.type_def("email").unwrap();
    assert!(email.field("processed").is_some());
    assert!(
        !read(root, path).unwrap().contains("template card"),
        "the seeding made a card the user removed"
    );
}

/// A user's own `event` type, made before `event` was a built-in: unrelated fields, no
/// `google_ref`.
const USER_EVENT: &str = r#"label  = "Event"
plural = "Events"

field venue {
  kind  = "string"
  label = "Venue"
}

field starts {
  kind  = "date"
  label = "Starts"
}

template default {
  tab "Event" {
    slot "title"
    slot "venue"
    slot "starts"
  }
}
"#;

/// A user's own type with one unrelated field, to save under a built-in's name.
const USER_TYPE: &str = r#"label = "Mine"

field mood {
  kind  = "string"
  label = "Mood"
}

template default {
  tab "Main" {
    slot "title"
    slot "mood"
  }
}
"#;

#[test]
fn a_users_own_type_holding_a_built_in_name_is_left_alone_and_can_be_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/event.wcl"), USER_EVENT).unwrap();
    for name in ["email", "email_thread", "message"] {
        let path = format!("types/{name}.wcl");
        std::fs::write(root.join(&path), USER_TYPE).unwrap();
    }
    let w = open(root);
    let party = w
        .create(
            "event",
            &writer::Input::Json(json!({ "fields": { "title": "Party", "venue": "Home" } })),
            &author(),
        )
        .unwrap()
        .id;
    let texts: Vec<(String, String)> = ["event", "email", "email_thread", "message"]
        .iter()
        .map(|n| {
            let path = format!("types/{n}.wcl");
            (path.clone(), read(root, &path).unwrap())
        })
        .collect();
    let note_path = format!("notes/event/{party}.wcl");
    let note_before = read(root, &note_path).unwrap();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    let mut skipped = done.types_skipped.clone();
    skipped.sort();
    assert_eq!(skipped, ["email", "email_thread", "event", "message"]);
    assert!(
        done.fields_added.iter().all(|(t, _)| t == "person"),
        "{:?}",
        done.fields_added
    );
    for (path, before) in &texts {
        assert_eq!(&read(root, path).unwrap(), before, "{path} changed");
    }
    assert!(w.schema().type_def("event").unwrap().handlers.is_empty());
    assert_eq!(read(root, &note_path).unwrap(), note_before);
    for name in ["event", "email", "email_thread", "message"] {
        assert!(!w.is_built_in(name), "{name}");
        assert!(
            w.state()
                .shipped_file(&format!("types/{name}.wcl"))
                .unwrap()
                .is_none(),
            "{name} recorded as seeded"
        );
    }
    // The built-ins the vault lacked are the server's.
    for name in ["call", "conversation", "dailynote", "fleeting"] {
        assert!(w.is_built_in(name), "{name}");
    }
    assert!(matches!(
        w.delete_type("call"),
        Err(writer::Error::BuiltInType(_))
    ));

    // A second start commits nothing, and still skips them.
    let before = generation(root);
    let again = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(generation(root), before, "a second start made a commit");
    assert_eq!(again.types_skipped.len(), 4);
    assert!(again.fields_added.is_empty());

    // The user's types can be deleted once empty.
    w.delete(&party, Some(&w.read(&party).unwrap().version), &author())
        .unwrap();
    w.state().purge_from_trash(&party).unwrap();
    w.delete_type("event").unwrap();
    assert!(w.schema().type_def("event").is_none());

    // The next start adds the built-ins in their place, as the server's.
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let mut added = done.types_added.clone();
    added.sort();
    assert_eq!(added, ["event"]);
    assert_eq!(
        read(root, "types/event.wcl").unwrap(),
        store::canonical(writer::DEFAULT_EVENT_TYPE, "types/event.wcl").unwrap()
    );
    assert!(w.is_built_in("event"));
    assert!(w.state().shipped_file("types/event.wcl").unwrap().is_some());
    let mut skipped = done.types_skipped.clone();
    skipped.sort();
    assert_eq!(skipped, ["email", "email_thread", "message"]);
}

#[test]
fn a_type_the_server_seeded_stays_the_built_in_without_its_key_field() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
        for name in [
            "event",
            "email",
            "email_thread",
            "message",
            "call",
            "conversation",
        ] {
            assert!(
                w.state()
                    .shipped_file(&format!("types/{name}.wcl"))
                    .unwrap()
                    .is_some(),
                "{name} not recorded as seeded"
            );
        }
    }
    // Without google_ref the type no longer matches the built-in's identity, but the server
    // wrote it, so it is still the built-in and gains the field back.
    let path = "types/event.wcl";
    let text = read(root, path).unwrap();
    let start = text.find("field google_ref {").unwrap();
    let end = start + text[start..].find("}\n").unwrap() + 2;
    std::fs::write(
        root.join(path),
        format!("{}{}", &text[..start], &text[end..]),
    )
    .unwrap();

    let w = open(root);
    assert!(w.is_built_in("event"));
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.types_skipped.is_empty(), "{:?}", done.types_skipped);
    assert_eq!(
        done.fields_added,
        [("event".to_owned(), "google_ref".to_owned())]
    );
    assert!(matches!(
        w.delete_type("event"),
        Err(writer::Error::BuiltInType(_))
    ));
}

#[test]
fn an_unrecorded_type_with_the_built_ins_key_field_is_the_built_in_and_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // As 0.1.7 left it: the server wrote event before it recorded the types it seeded.
    vault_0_1_7(root, None, false);
    let w = open_with(root, None);
    assert!(w.state().shipped_file("types/event.wcl").unwrap().is_none());
    assert!(w.is_built_in("event"));

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert!(done.types_skipped.is_empty(), "{:?}", done.types_skipped);
    assert!(w.state().shipped_file("types/event.wcl").unwrap().is_some());
    assert!(matches!(
        w.delete_type("event"),
        Err(writer::Error::BuiltInType(_))
    ));
}

#[test]
fn an_event_type_of_the_users_gains_no_google_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/event.wcl"), USER_EVENT).unwrap();
    let w = open(root);
    let before = read(root, "types/event.wcl").unwrap();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert!(done.steps.iter().any(|s| s == "0003_event_google_handler"));
    assert_eq!(read(root, "types/event.wcl").unwrap(), before);
    assert!(w.schema().type_def("event").unwrap().handlers.is_empty());
}

#[test]
fn a_google_sync_type_of_the_users_is_not_removed_by_step_0004() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/google_sync.wcl"), USER_TYPE).unwrap();
    let w = open(root);
    let id = w
        .create(
            "google_sync",
            &writer::Input::Json(json!({ "fields": { "title": "Mine", "mood": "fine" } })),
            &author(),
        )
        .unwrap()
        .id;
    let note_path = format!("notes/google_sync/{id}.wcl");
    let note_before = read(root, &note_path).unwrap();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();

    assert!(done.steps.iter().any(|s| s == "0004_remove_google_sync"));
    assert!(w.schema().type_def("google_sync").is_some());
    assert_eq!(read(root, &note_path).unwrap(), note_before);
    assert!(w.state().list_secrets().unwrap().is_empty());
}

/// Type file `text`, canonical, without its `list_page` block, which the shipped files put last.
fn without_list_page(text: &str) -> String {
    match text.find("\nlist_page {") {
        Some(at) => format!("{}\n", text[..at].trim_end()),
        None => text.to_owned(),
    }
}

/// The `list_page` block of canonical type file `text`, as the file writes it.
fn list_page_block(text: &str) -> String {
    let at = text.find("\nlist_page {").expect("a list_page block");
    text[at..].trim().to_owned()
}

/// Run step `0005_list_page_defaults` again under a new id, as a vault that has not run it yet
/// would on its next start.
fn run_list_page_step(w: &Writer) -> Vec<String> {
    let step = writer::UPGRADE_STEPS
        .iter()
        .find(|s| s.id == "0005_list_page_defaults")
        .unwrap();
    w.run_upgrade_steps(&[UpgradeStep {
        id: "test_0005_list_page_defaults",
        run: step.run,
    }])
    .unwrap()
}

#[test]
fn a_new_vaults_email_and_message_types_open_in_details_by_sent_date_fifty_a_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let schema = w.schema();
    for name in ["email", "message"] {
        let lp = schema.type_def(name).unwrap().list_page.clone().unwrap();
        assert_eq!(lp.view.as_deref(), Some("details"), "{name}");
        assert_eq!(lp.sort, Some(("sent".to_owned(), true)), "{name}");
        assert_eq!(lp.page_size, Some(50), "{name}");
    }
    // The other built-in types keep the defaults: no block.
    assert!(schema.type_def("person").unwrap().list_page.is_none());
}

#[test]
fn an_edited_email_or_message_type_gains_the_list_page_block_and_keeps_the_edit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    // As the user's vault has them: types from before `list_page`, each with a slot of the
    // user's added to its card, after the card's last slot.
    let mut edited = Vec::new();
    for (name, last, slot) in [
        ("email", "    slot \"snippet\"\n", "    slot \"labels\"\n"),
        (
            "message",
            "    slot \"processed\"\n",
            "    slot \"channel\"\n",
        ),
    ] {
        let path = format!("types/{name}.wcl");
        let old = without_list_page(&read(root, &path).unwrap());
        let at = old.rfind(last).unwrap() + last.len();
        let text = format!("{}{slot}{}", &old[..at], &old[at..]);
        std::fs::write(root.join(&path), &text).unwrap();
        edited.push((name, path, text));
    }
    let w = open(root);
    let id = w
        .create(
            "email",
            &writer::Input::Json(json!({ "fields": { "title": "Hello" } })),
            &author(),
        )
        .unwrap()
        .id;
    let note_path = format!("notes/email/{id}.wcl");
    let note_before = read(root, &note_path).unwrap();
    assert!(w.schema().type_def("email").unwrap().list_page.is_none());

    assert_eq!(run_list_page_step(&w), ["test_0005_list_page_defaults"]);

    for (name, path, text) in &edited {
        let source = if *name == "email" {
            writer::DEFAULT_EMAIL_TYPE
        } else {
            writer::DEFAULT_MESSAGE_TYPE
        };
        let shipped = store::canonical(source, path).unwrap();
        assert_eq!(
            read(root, path).unwrap(),
            format!("{text}\n{}\n", list_page_block(&shipped)),
            "{name}"
        );
        let schema = w.schema();
        let ty = schema.type_def(name).unwrap();
        let lp = ty.list_page.clone().unwrap();
        assert_eq!(lp.view.as_deref(), Some("details"), "{name}");
        assert_eq!(lp.sort, Some(("sent".to_owned(), true)), "{name}");
        assert_eq!(lp.page_size, Some(50), "{name}");
        let card = ty.template("card").unwrap();
        assert_eq!(
            card.tabs[0].items.len(),
            schema::TypeDef::parse(name, &shipped)
                .unwrap()
                .template("card")
                .unwrap()
                .tabs[0]
                .items
                .len()
                + 1,
            "{name}: the user's slot is kept"
        );
    }
    assert_eq!(read(root, &note_path).unwrap(), note_before);

    // The next start's seeding changes nothing more in either file.
    let before: Vec<String> = edited
        .iter()
        .map(|(_, p, _)| read(root, p).unwrap())
        .collect();
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let after: Vec<String> = edited
        .iter()
        .map(|(_, p, _)| read(root, p).unwrap())
        .collect();
    assert_eq!(after, before);
}

#[test]
fn a_type_that_already_has_a_list_page_block_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let path = "types/email.wcl";
    let mine = format!(
        "{}\nlist_page {{\n  view = \"list\"\n}}\n",
        without_list_page(&read(root, path).unwrap())
    );
    std::fs::write(root.join(path), &mine).unwrap();
    let w = open(root);
    let before = generation(root);

    run_list_page_step(&w);

    assert_eq!(read(root, path).unwrap(), mine);
    let lp = w
        .schema()
        .type_def("email")
        .unwrap()
        .list_page
        .clone()
        .unwrap();
    assert_eq!(lp.view.as_deref(), Some("list"));
    assert_eq!(lp.sort, None);
    // The message type already had the shipped block, so the step made no commit at all.
    assert_eq!(generation(root), before);
}

#[test]
fn an_event_type_from_before_the_outbox_gains_send_updates_and_no_note_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let path = "types/event.wcl";
    let current = writer::DEFAULT_EVENT_TYPE;
    let start = current
        .find("field send_updates {")
        .expect("the shipped event type has send_updates");
    let end = start + current[start..].find("}\n").unwrap() + 2;
    let old = format!("{}{}", &current[..start], current[end..].trim_start());
    std::fs::write(root.join(path), store::canonical(&old, path).unwrap()).unwrap();
    let event = {
        let w = open(root);
        assert!(
            w.schema()
                .type_def("event")
                .unwrap()
                .field("send_updates")
                .is_none()
        );
        w.create(
            "event",
            &writer::Input::Json(json!({ "fields": {
                "title": "Standup",
                "when": { "start": "2026-03-05T10:00:00Z", "end": "2026-03-05T10:15:00Z" },
            } })),
            &author(),
        )
        .unwrap()
        .id
    };
    let note_path = format!("notes/event/{event}.wcl");
    let before = read(root, &note_path).unwrap();

    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(
        done.fields_added,
        [("event".to_owned(), "send_updates".to_owned())]
    );
    assert_eq!(
        read(root, path).unwrap(),
        store::canonical(current, path).unwrap()
    );
    let schema = w.schema();
    let field = schema
        .type_def("event")
        .unwrap()
        .field("send_updates")
        .unwrap();
    assert!(field.hidden);
    assert_eq!(field.kind, schema::FieldKind::String);
    assert_eq!(read(root, &note_path).unwrap(), before);
}

/// `text` without its Correspondence tab (step 0007).
fn without_correspondence(text: &str) -> String {
    let Some(start) = text.find("  tab \"Correspondence\" {") else {
        return text.to_owned();
    };
    let end = start + text[start..].find("\n  }\n").unwrap() + "\n  }\n".len();
    // The canonical writer sets the tab off from the one before by a blank line.
    let before = text[..start]
        .strip_suffix("\n\n")
        .map_or(&text[..start], |b| &text[..b.len() + 1]);
    format!("{}{}", before, &text[end..])
}

/// The shipped `person` type as it was before its Correspondence tab, with a slot of the user's
/// moved to the end of the default template's tab.
fn person_before_correspondence() -> String {
    let shipped = writer::DEFAULT_PERSON_TYPE;
    assert!(
        shipped.contains("  tab \"Correspondence\" {"),
        "the shipped type has the tab"
    );
    let without = without_correspondence(shipped);
    without.replacen("    slot \"birthday\"\n", "", 1).replacen(
        "    slot \"body\"\n",
        "    slot \"body\"\n    slot \"birthday\"\n",
        1,
    )
}

#[test]
fn a_vault_whose_person_type_predates_the_correspondence_tab_gains_it_and_keeps_its_edits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(
        root.join("types/person.wcl"),
        person_before_correspondence(),
    )
    .unwrap();
    let w = open(root);
    let alex = w
        .create("person", &writer::Input::Wcl(ALEX.into()), &author())
        .unwrap();
    let note_before = read(root, &format!("notes/person/{}.wcl", alex.id)).unwrap();

    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(
        done.steps.iter().any(|s| s == "0007_person_correspondence"),
        "{:?}",
        done.steps
    );
    let after = read(root, "types/person.wcl").unwrap();
    assert!(after.contains("tab \"Correspondence\""), "{after}");
    assert!(
        after.contains("from conversation where has(participants, this) sort updated desc"),
        "{after}"
    );
    assert!(
        after.contains(
            "from email where from == this || has(to, this) || has(cc, this) sort sent desc"
        ),
        "{after}"
    );
    // The user's edit stays: birthday after body in the Person tab.
    let person_tab = &after
        [after.find("tab \"Person\"").unwrap()..after.find("tab \"Correspondence\"").unwrap()];
    assert!(
        person_tab.find("slot \"body\"").unwrap() < person_tab.find("slot \"birthday\"").unwrap(),
        "{person_tab}"
    );
    // No note changed.
    assert_eq!(
        read(root, &format!("notes/person/{}.wcl", alex.id)).unwrap(),
        note_before
    );
    drop(w);

    // The next start adds nothing: the step ran once, and the tab is there.
    let w = open(root);
    let again = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(
        !again
            .steps
            .iter()
            .any(|s| s == "0007_person_correspondence"),
        "{:?}",
        again.steps
    );
    assert_eq!(
        read(root, "types/person.wcl")
            .unwrap()
            .matches("tab \"Correspondence\"")
            .count(),
        1
    );
}

/// The `outbox` type as ticket 180 shipped it, before step 0008.
const OUTBOX_5E81292: &str = include_str!("../defaults/old/outbox_5e81292.wcl");

/// Run step `0008_outbox_template` again under a new id, as a vault that has not run it yet would
/// on its next start.
fn run_outbox_step(w: &Writer) -> Vec<String> {
    run_step_again(w, "0008_outbox_template")
}

/// Run upgrade step `id` again under the id `test_<id>`.
fn run_step_again(w: &Writer, id: &'static str) -> Vec<String> {
    let step = writer::UPGRADE_STEPS.iter().find(|s| s.id == id).unwrap();
    let test_id: &'static str = Box::leak(format!("test_{id}").into_boxed_str());
    w.run_upgrade_steps(&[UpgradeStep {
        id: test_id,
        run: step.run,
    }])
    .unwrap()
}

#[test]
fn the_outbox_template_ticket_180_shipped_is_replaced_and_no_note_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = "types/outbox.wcl";
    let current = store::canonical(writer::DEFAULT_OUTBOX_TYPE, path).unwrap();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    // A new vault has the new template already.
    assert_eq!(read(root, path).unwrap(), current);
    assert!(!current.contains("tab \"Message\""), "{current}");
    assert!(!current.contains("\"dropdown\""), "{current}");

    // As ticket 180 left the vault, with one draft in it.
    std::fs::write(
        root.join(path),
        store::canonical(OUTBOX_5E81292, path).unwrap(),
    )
    .unwrap();
    let w = open(root);
    let item = w
        .create(
            "outbox",
            &writer::Input::Json(json!({ "fields": {
                "title": "Lunch", "kind": "email", "status": "draft", "to": ["ada@example.com"],
            } })),
            &author(),
        )
        .unwrap()
        .id;
    let note_path = format!("notes/outbox/{item}.wcl");
    let before = read(root, &note_path).unwrap();

    assert_eq!(run_outbox_step(&w), ["test_0008_outbox_template"]);
    // Step 0009 then brings the statuses and the card up to date too.
    assert_eq!(
        run_step_again(&w, "0009_outbox_statuses"),
        ["test_0009_outbox_statuses"]
    );
    // The start after the steps brings the old `multiple` attachments to the list form.
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(read(root, path).unwrap(), current);
    assert_eq!(read(root, &note_path).unwrap(), before);
    let schema = w.schema();
    let tabs = &schema
        .type_def("outbox")
        .unwrap()
        .template("default")
        .unwrap()
        .tabs;
    assert_eq!(tabs.len(), 1, "one form for every kind");
    assert!(tabs[0].items.iter().any(|i| matches!(
        i,
        schema::TabItem::Slot { field, when, .. }
            if field == "body" && when.as_deref() == Some("kind != \"event\"")
    )));
}

#[test]
fn an_outbox_template_the_user_changed_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = "types/outbox.wcl";
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let edited = OUTBOX_5E81292.replacen("tab \"Status\" {", "tab \"Progress\" {", 1);
    assert_ne!(edited, OUTBOX_5E81292);
    let edited = store::canonical(&edited, path).unwrap();
    std::fs::write(root.join(path), &edited).unwrap();
    let w = open(root);
    assert_eq!(run_outbox_step(&w), ["test_0008_outbox_template"]);
    assert_eq!(read(root, path).unwrap(), edited);
}

/// The `outbox` type as ticket 194 shipped it, before step 0009.
const OUTBOX_3A489B0: &str = include_str!("../defaults/old/outbox_3a489b0.wcl");

/// A new Outbox item with `status`, made by the user.
fn outbox_item(w: &Writer, status: &str, extra: serde_json::Value) -> String {
    let mut fields = json!({
        "title": format!("Item {status}"), "kind": "email", "status": status,
        "to": ["ada@example.com"],
    });
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    w.create(
        "outbox",
        &writer::Input::Json(json!({ "fields": fields })),
        &author(),
    )
    .unwrap()
    .id
}

fn outbox_status(w: &Writer, id: &str) -> String {
    writer::doc_to_json(&w.read(id).unwrap().doc)["fields"]["status"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn the_outbox_statuses_become_draft_ready_sending_and_failed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = "types/outbox.wcl";
    let current = store::canonical(writer::DEFAULT_OUTBOX_TYPE, path).unwrap();
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    // A new vault has the four statuses already, and no template shows who drafted an item.
    assert_eq!(read(root, path).unwrap(), current);
    assert!(current.contains("options = [\"draft\", \"ready\", \"sending\", \"failed\"]"));
    assert!(!current.contains("slot \"drafted_by\""), "{current}");

    // As ticket 194 left the vault, with an item in every status.
    std::fs::write(
        root.join(path),
        store::canonical(OUTBOX_3A489B0, path).unwrap(),
    )
    .unwrap();
    let w = open(root);
    let draft = outbox_item(&w, "draft", json!({}));
    let review = outbox_item(&w, "review", json!({ "drafted_by": "assistant" }));
    let approved = outbox_item(&w, "approved", json!({}));
    let sending = outbox_item(&w, "sending", json!({ "sent_ref": "cmd1" }));
    let failed = outbox_item(&w, "failed", json!({ "error": "Gmail answered 500" }));
    let copy = w
        .create(
            "email",
            &writer::Input::Json(json!({ "fields": { "title": "Lunch" } })),
            &author(),
        )
        .unwrap()
        .id;
    let sent = outbox_item(
        &w,
        "sent",
        json!({ "sent_ref": "m1", "sent_at": "2026-10-23T10:00:00Z", "result": copy }),
    );
    let rejected = outbox_item(&w, "rejected", json!({}));

    assert_eq!(
        run_step_again(&w, "0009_outbox_statuses"),
        ["test_0009_outbox_statuses"]
    );
    // The start after the steps brings the old `multiple` attachments to the list form.
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(read(root, path).unwrap(), current);
    assert_eq!(outbox_status(&w, &draft), "draft");
    assert_eq!(outbox_status(&w, &review), "draft");
    assert_eq!(outbox_status(&w, &approved), "ready");
    assert_eq!(outbox_status(&w, &sending), "sending");
    assert_eq!(outbox_status(&w, &failed), "failed");
    // A sent item is gone for good, its copy kept; a rejected one waits in the trash.
    assert!(w.read(&sent).is_err());
    assert!(w.read(&copy).is_ok());
    let trash: Vec<String> = w.trash().unwrap().into_iter().map(|t| t.note_id).collect();
    assert_eq!(trash, std::slice::from_ref(&rejected));
    assert!(w.read(&rejected).is_err());

    // Run again, it changes nothing.
    let before = read(root, path).unwrap();
    run_step_again(&w, "0009_outbox_statuses");
    assert_eq!(read(root, path).unwrap(), before);
    assert_eq!(outbox_status(&w, &approved), "ready");
}

#[test]
fn outbox_statuses_the_user_changed_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = "types/outbox.wcl";
    open(root).upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let edited = OUTBOX_3A489B0.replacen("\"rejected\"]", "\"rejected\", \"later\"]", 1);
    assert_ne!(edited, OUTBOX_3A489B0);
    let edited = store::canonical(&edited, path).unwrap();
    std::fs::write(root.join(path), &edited).unwrap();
    let w = open(root);
    let review = outbox_item(&w, "review", json!({}));
    assert_eq!(
        run_step_again(&w, "0009_outbox_statuses"),
        ["test_0009_outbox_statuses"]
    );
    assert_eq!(read(root, path).unwrap(), edited);
    assert_eq!(outbox_status(&w, &review), "review");
}
