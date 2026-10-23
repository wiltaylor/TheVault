//! Open taxonomies (`allow_new = true`): a note saved with a term the taxonomy lacks adds the
//! term in the same commit, and a closed taxonomy still refuses it.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::Path;

use state::{Author, State};
use store::{CommitStep, Vault};
use writer::{Error, Input, Op, TypeChange, Writer};

const PLACES: &str = r#"label     = "Places"
allow_new = true

term "Plant" {
  colour = "green"

  term "Electrical"
}

term "Office"
"#;

const STATUS: &str = r#"label = "Status"

term "Todo"

term "Done"
"#;

const SITE: &str = r#"label = "Site"

field places {
  kind     = "list"
  items    = "taxonomy"
  taxonomy = "places"
}

field areas {
  kind     = "taxonomy"
  taxonomy = "places"
  multiple = true
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

field labels {
  kind  = "list"
  items = "string"
}

field count {
  kind = "int"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn author() -> Author {
    Author::Session("test".into())
}

fn wcl(text: &str) -> Input {
    Input::Wcl(text.to_owned())
}

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn setup(root: &Path) -> Writer {
    for dir in ["types", "taxonomies"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("types/site.wcl"), SITE).unwrap();
    std::fs::write(root.join("taxonomies/places.wcl"), PLACES).unwrap();
    std::fs::write(root.join("taxonomies/status.wcl"), STATUS).unwrap();
    open(root)
}

fn places(root: &Path) -> String {
    std::fs::read_to_string(root.join("taxonomies/places.wcl")).unwrap()
}

fn notes(root: &Path) -> usize {
    std::fs::read_dir(root.join("notes/site")).map_or(0, |d| d.count())
}

fn paths(w: &Writer) -> Vec<String> {
    w.schema().taxonomy("places").unwrap().paths()
}

#[test]
fn a_list_of_terms_adds_a_new_top_level_term_and_one_under_an_existing_parent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    let saved = w
        .create(
            "site",
            &wcl("title = \"A\"\nplaces = [\"Office\", \"Depot\", \"Plant/Solar\"]\n"),
            &author(),
        )
        .unwrap();
    assert!(
        saved
            .wcl
            .contains("places  = [\"Office\", \"Depot\", \"Plant/Solar\"]"),
        "{}",
        saved.wcl
    );
    assert_eq!(
        paths(&w),
        [
            "Plant",
            "Plant/Electrical",
            "Plant/Solar",
            "Office",
            "Depot"
        ]
    );
    // The file is valid, canonical WCL that keeps what it held.
    let text = places(root);
    assert_eq!(
        store::canonical(&text, "taxonomies/places.wcl").unwrap(),
        text
    );
    assert!(text.contains("allow_new = true"), "{text}");
    assert!(text.contains("colour = \"green\""), "{text}");

    // Reopening the vault reads the grown taxonomy from its file.
    drop(w);
    let w = open(root);
    assert!(
        w.schema()
            .taxonomy("places")
            .unwrap()
            .contains("Plant/Solar")
    );
}

#[test]
fn a_multiple_taxonomy_field_adds_terms_too_and_a_name_in_another_case_is_the_existing_term() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    let saved = w
        .create(
            "site",
            &wcl("title = \"A\"\nareas = [\"plant/electrical/Wiring\", \"OFFICE\"]\n"),
            &author(),
        )
        .unwrap();
    assert!(
        saved
            .wcl
            .contains("areas   = [\"Plant/Electrical/Wiring\", \"Office\"]"),
        "{}",
        saved.wcl
    );
    assert_eq!(
        paths(&w),
        [
            "Plant",
            "Plant/Electrical",
            "Plant/Electrical/Wiring",
            "Office"
        ]
    );
    // A term and a term under it, both new, in one write.
    w.create(
        "site",
        &wcl("title = \"B\"\nplaces = [\"Yard/Shed\", \"Yard\"]\n"),
        &author(),
    )
    .unwrap();
    assert!(paths(&w).contains(&"Yard/Shed".to_owned()));
}

#[test]
fn a_new_term_under_a_missing_parent_is_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    let before = places(root);
    let err = w
        .create(
            "site",
            &wcl("title = \"A\"\nplaces = [\"Depot\", \"Nowhere/Solar\"]\n"),
            &author(),
        )
        .unwrap_err();
    let Error::Invalid(inv) = &err else {
        panic!("{err}")
    };
    assert_eq!(inv.issues[0].at, "places");
    assert!(
        inv.issues[0].message.contains("no term \"Nowhere\""),
        "{err}"
    );
    assert_eq!(places(root), before);
    assert_eq!(notes(root), 0);
    assert!(!paths(&w).contains(&"Depot".to_owned()));
}

#[test]
fn a_closed_taxonomy_still_refuses_a_term_it_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    let before = std::fs::read_to_string(root.join("taxonomies/status.wcl")).unwrap();
    let err = w
        .create(
            "site",
            &wcl("title = \"A\"\nstatus = \"Blocked\"\n"),
            &author(),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("\"Blocked\" is not a term of taxonomy \"status\""),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("taxonomies/status.wcl")).unwrap(),
        before
    );
    assert_eq!(notes(root), 0);
}

#[test]
fn no_term_is_added_when_the_note_fails_or_its_commit_does() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    let before = places(root);

    // The note is invalid for another reason.
    let err = w
        .create(
            "site",
            &wcl("title = \"A\"\nplaces = [\"Depot\"]\ncount = \"many\"\n"),
            &author(),
        )
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    assert_eq!(places(root), before);
    assert!(!paths(&w).contains(&"Depot".to_owned()));

    // The commit stops part way: neither the taxonomy nor the note changes.
    w.set_commit_hook(Some(Box::new(|step| {
        if step == CommitStep::Staged(1) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })));
    let err = w
        .create(
            "site",
            &wcl("title = \"A\"\nplaces = [\"Depot\"]\n"),
            &author(),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::Store(store::Error::Interrupted)),
        "{err}"
    );
    assert_eq!(places(root), before);
    assert_eq!(notes(root), 0);
    assert!(!paths(&w).contains(&"Depot".to_owned()));
    w.set_commit_hook(None);

    // A dry run checks the new term and writes nothing.
    let op = Op::Create {
        type_name: "site".into(),
        input: wcl("title = \"A\"\nplaces = [\"Depot\"]\n"),
    };
    let report = w.batch(vec![op], true, &author()).unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!(places(root), before);
    assert_eq!(notes(root), 0);
}

#[test]
fn a_type_change_to_a_list_of_terms_adds_the_values_notes_hold() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    w.create(
        "site",
        &wcl("title = \"A\"\nlabels = [\"Office\", \"Depot\", \"depot\"]\n"),
        &author(),
    )
    .unwrap();
    let new_type = SITE.replace(
        "field labels {\n  kind  = \"list\"\n  items = \"string\"\n}",
        "field labels {\n  kind     = \"list\"\n  items    = \"taxonomy\"\n  taxonomy = \"places\"\n}",
    );
    let change = TypeChange {
        wcl: new_type,
        renames: BTreeMap::new(),
        migration: None,
    };
    let plan = w.plan_type_change("site", &change).unwrap();
    assert!(plan.changes.is_empty(), "{plan:?}");
    // Planning writes nothing.
    assert!(!places(root).contains("Depot"));
    w.apply_type_change("site", &change, &plan.token, &author())
        .unwrap();
    // A planned change keeps notes as they are, so both cases become terms.
    assert_eq!(
        paths(&w),
        ["Plant", "Plant/Electrical", "Office", "Depot", "depot"]
    );
    assert!(places(root).contains("term \"depot\""));
}

#[test]
fn checking_a_taxonomy_change_or_delete_reports_what_it_would_do_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = setup(root);
    let saved = w
        .create(
            "site",
            &wcl("title = \"A\"\nstatus = \"Todo\"\n"),
            &author(),
        )
        .unwrap();
    let before = std::fs::read_to_string(root.join("taxonomies/status.wcl")).unwrap();
    let renamed = "label = \"Status\"\n\nterm \"Open\"\n\nterm \"Done\"\n";
    let renames = BTreeMap::from([("Todo".to_owned(), "Open".to_owned())]);

    let check = w.check_taxonomy("status", renamed, &renames).unwrap();
    assert!(check.exists);
    assert_eq!(check.notes, std::slice::from_ref(&saved.id));
    assert!(check.wcl.contains("term \"Open\""), "{}", check.wcl);
    // Nothing moved: the file and the note are as they were.
    assert_eq!(
        std::fs::read_to_string(root.join("taxonomies/status.wcl")).unwrap(),
        before
    );
    assert!(w.read(&saved.id).unwrap().wcl.contains("\"Todo\""));

    // Dropping a held term without a rename is refused, as the write refuses it.
    let err = w
        .check_taxonomy("status", renamed, &BTreeMap::new())
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::InUse {
                code: "term_in_use",
                ..
            }
        ),
        "{err:?}"
    );
    // Invalid WCL is refused.
    assert!(
        w.check_taxonomy("status", "term {", &BTreeMap::new())
            .is_err()
    );

    // A new name checks as a create, and is not written.
    let new = w
        .check_taxonomy(
            "size",
            "label = \"Size\"\n\nterm \"Big\"\n",
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(!new.exists);
    assert!(!root.join("taxonomies/size.wcl").exists());

    // A taxonomy a type uses cannot be deleted; the check says so and deletes nothing.
    let err = w.check_delete_taxonomy("status").unwrap_err();
    assert!(
        matches!(
            err,
            Error::InUse {
                code: "taxonomy_in_use",
                ..
            }
        ),
        "{err:?}"
    );
    assert!(root.join("taxonomies/status.wcl").exists());
    assert!(w.check_delete_taxonomy("nope").is_err());
}
