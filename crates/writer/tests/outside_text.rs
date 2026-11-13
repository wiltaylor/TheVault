use std::sync::{Arc, Mutex, Weak};

use script_core::RunContext;
use state::Author;
use writer::{Event, Hooks, Input, NoteChange, Writer};

fn owner() -> Author {
    Author::Session("owner".into())
}

fn input(title: &str) -> Input {
    Input::Json(serde_json::json!({"fields":{"title":title}}))
}

fn setup() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    std::fs::write(
        dir.path().join("types/memo.wcl"),
        "label = \"Memo\"\ntemplate default { tab Main { slot title } }\n",
    )
    .unwrap();
    let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
    (dir, Writer::open(vault, state).unwrap())
}

#[test]
fn restricted_delete_is_refused_for_every_author_but_proposals_and_fresh_owner_work() {
    let (_dir, w) = setup();
    let note = w.create("memo", &input("Keep"), &owner()).unwrap();
    let context = RunContext::default();
    context.restrict();
    for by in [
        owner(),
        Author::Agent("reader".into()),
        Author::Task("task".into()),
        Author::Handler("handler".into()),
    ] {
        let result = context.scope(|| w.delete(&note.id, Some(&note.version), &by));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains(script_core::OUTSIDE_TEXT_REFUSAL)
        );
        assert_eq!(w.read(&note.id).unwrap().version, note.version);
    }
    let report = context
        .scope(|| {
            w.batch(
                vec![writer::Op::Delete {
                    id: note.id.clone(),
                    base: Some(note.version.clone()),
                }],
                true,
                &owner(),
            )
        })
        .unwrap();
    assert!(report.error.is_none());
    assert!(report.items[0].is_ok());
    assert!(w.read(&note.id).is_ok());
    w.delete(&note.id, Some(&note.version), &owner()).unwrap();
    assert!(context.is_restricted());
    let audit = w
        .state()
        .activity(&state::ActivityFilter::default())
        .unwrap();
    assert!(
        audit
            .items
            .iter()
            .any(|a| a.kind == "run.outside_text_refused"
                && a.summary == script_core::OUTSIDE_TEXT_REFUSAL)
    );
}

struct LateChange {
    mode: &'static str,
    target: String,
    version: String,
    writer: Arc<Mutex<Weak<Writer>>>,
}

impl Hooks for LateChange {
    fn before(&self, change: &mut NoteChange, _: &Author) -> Result<(), String> {
        if change.id != self.target {
            if self.mode == "before" {
                RunContext::current().unwrap().restrict();
                change.event = Event::Delete;
                change.doc = None;
            } else if self.mode == "staged" {
                RunContext::current().unwrap().restrict();
                self.writer
                    .lock()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .stage_delete(
                        &self.target,
                        Some(&self.version),
                        &Author::Handler("child".into()),
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn side_effects(&self, _: &[NoteChange]) -> writer::Result<Vec<NoteChange>> {
        if self.mode == "expanded" {
            RunContext::current().unwrap().restrict();
            return Ok(vec![NoteChange {
                id: self.target.clone(),
                type_name: "memo".into(),
                event: Event::Delete,
                doc: None,
            }]);
        }
        Ok(vec![])
    }
}

#[test]
fn late_exposure_refuses_handler_rewrites_staged_and_expanded_deletes_atomically() {
    for mode in ["before", "staged", "expanded"] {
        let (_dir, w) = setup();
        let root = w.create("memo", &input("Root"), &owner()).unwrap();
        let target = w.create("memo", &input("Target"), &owner()).unwrap();
        let weak = Arc::new(Mutex::new(Weak::new()));
        let w = Arc::new(w.with_hooks(Box::new(LateChange {
            mode,
            target: target.id.clone(),
            version: target.version.clone(),
            writer: weak.clone(),
        })));
        *weak.lock().unwrap() = Arc::downgrade(&w);
        let context = RunContext::default();
        let result =
            context.scope(|| w.update(&root.id, &input("Changed"), Some(&root.version), &owner()));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains(script_core::OUTSIDE_TEXT_REFUSAL),
            "{mode}"
        );
        assert_eq!(w.read(&root.id).unwrap().version, root.version);
        assert_eq!(w.read(&target.id).unwrap().version, target.version);
    }
}

#[test]
fn action_matrix_keeps_local_processed_rules_but_classifies_google_and_calendar_actions() {
    let doc = |fields| {
        Input::Json(serde_json::json!({"fields":fields}))
            .parse("fixture.wcl")
            .unwrap()
    };
    let mail = doc(serde_json::json!({"gmail_ref":"fixture", "processed":false}));
    let done = doc(serde_json::json!({"gmail_ref":"fixture", "processed":true}));
    for (by, sensitive) in [
        (Author::Agent("reader".into()), false),
        (Author::AgentMail("reader".into()), true),
        (owner(), true),
    ] {
        assert_eq!(
            writer::outside_text_sensitive("email", Event::Update, Some(&mail), Some(&done), &by),
            sensitive
        );
    }
    let event = doc(serde_json::json!({"title":"Before", "google_ref":"fixture"}));
    for fields in [
        serde_json::json!({"title":"After", "google_ref":"fixture"}),
        serde_json::json!({"attendees_raw":["foreign@example.test"]}),
        serde_json::json!({"my_response":"accepted"}),
    ] {
        let next = doc(fields);
        assert!(writer::outside_text_sensitive(
            "event",
            Event::Update,
            Some(&event),
            Some(&next),
            &Author::Handler("google_event".into())
        ));
    }
}

#[test]
fn gmail_and_calendar_mutations_require_fresh_authority_after_exposure() {
    let (_dir, w) = setup();
    w.put_type("email", "label = \"Email\"\nfield gmail_ref { kind = \"string\" }\nfield processed { kind = \"bool\" }\ntemplate default { tab Main { slot title } }\n").unwrap();
    w.put_type("event", "label = \"Event\"\nfield google_ref { kind = \"string\" }\nfield my_response { kind = \"string\" }\nfield attendees_raw { kind = \"list\" items = \"string\" }\ntemplate default { tab Main { slot title } }\n").unwrap();
    for (kind, mut initial, edits, by) in [
        (
            "email",
            serde_json::json!({"gmail_ref":"fixture", "processed":false}),
            serde_json::json!({"processed":true}),
            Author::AgentMail("reader".into()),
        ),
        (
            "event",
            serde_json::json!({"google_ref":"fixture"}),
            serde_json::json!({"title":"Moved"}),
            Author::Handler("google_event".into()),
        ),
        (
            "event",
            serde_json::json!({}),
            serde_json::json!({"my_response":"accepted"}),
            Author::Handler("google_event".into()),
        ),
        (
            "event",
            serde_json::json!({}),
            serde_json::json!({"attendees_raw":["other@example.test"]}),
            Author::Handler("google_event".into()),
        ),
    ] {
        initial["title"] = serde_json::json!("Fixture");
        let note = w
            .create(
                kind,
                &Input::Json(serde_json::json!({"fields":initial})),
                &owner(),
            )
            .unwrap();
        let mut replacement = writer::doc_to_json(&w.read(&note.id).unwrap().doc);
        for (key, value) in edits.as_object().unwrap() {
            replacement["fields"][key] = value.clone();
        }
        let input = Input::Json(replacement);
        let context = RunContext::default();
        context.restrict();
        let error = context
            .scope(|| w.update(&note.id, &input, Some(&note.version), &by))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(script_core::OUTSIDE_TEXT_REFUSAL),
            "{error}"
        );
        assert_eq!(w.read(&note.id).unwrap().version, note.version);
        w.update(&note.id, &input, Some(&note.version), &owner())
            .unwrap();
        assert!(context.is_restricted());
    }
}
