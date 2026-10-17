//! Keeping a vault's shipped defaults up to date on start: the built-in types and the fields
//! they gained, the default device file and scripts, shipped file upgrades, and the one-time
//! upgrade steps (D15, D16, D24).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use state::{Author, State};
use store::Vault;
use writer::{SHIPPED_FILES, UpgradeStep, Upgraded, Upkeep, Writer, content_hash};

/// The `person` type `vault init --starter` wrote before people had phones.
const OLDEST_PERSON: &str = include_str!("fixtures/shipped/person_oldest_starter.wcl");

/// The starter's `person` type as it is now.
const STARTER_PERSON: &str = include_str!("../../cli/starter/types/person.wcl");

/// `scripts/device_calls.ws` as the first release with device scripts shipped it.
const OLD_DEVICE_CALLS: &str = include_str!("fixtures/shipped/device_calls_39d9091.ws");

/// `devices/android.wcl` as the first release with device files shipped it.
const OLD_ANDROID: &str = include_str!("fixtures/shipped/android_39d9091.wcl");

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

    let fields: Vec<(&str, &str)> = done
        .fields_added
        .iter()
        .map(|(t, f)| (t.as_str(), f.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            ("person", "phones"),
            ("person", "addresses"),
            ("person", "device_contacts")
        ]
    );
    // Laid out as the starter now writes it: organisation and homepage kept, the new fields
    // after email, the new slots after email's, the card and the Work tab untouched.
    assert_eq!(
        w.read_type("person").unwrap(),
        store::canonical(STARTER_PERSON, "types/person.wcl").unwrap()
    );
    let mut added = done.types_added.clone();
    added.sort();
    assert_eq!(
        added,
        ["call", "conversation", "dailynote", "fleeting", "message"]
    );
    let ty = w.schema();
    let person = ty.type_def("person").unwrap();
    for f in [
        "email",
        "organisation",
        "homepage",
        "body",
        "phones",
        "addresses",
        "device_contacts",
    ] {
        assert!(person.field(f).is_some(), "person lacks {f}");
    }

    // Every note unchanged.
    assert_eq!(read(root, &note_path).unwrap(), note_before);
    assert_eq!(w.history(&alex).unwrap().len(), versions_before);

    // The device file and the five device scripts.
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
            "device_share"
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
    assert_eq!(fields, ["addresses", "device_contacts"]);
    let schema = w.schema();
    let phones = schema.type_def("person").unwrap().field("phones").unwrap();
    assert_eq!(phones.label, "Phone");
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
