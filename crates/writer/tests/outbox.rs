//! The built-in `outbox` type and who may move its items between statuses (outbox X1, X2, X14):
//! the user any way, an agent only as a draft (or ready with `send_without_review`) and only its
//! own drafts, and the send handlers and syncs only along ready, sending or failed, removing an
//! item once it was sent.

use std::path::Path;

use serde_json::{Value, json};
use state::{Author, SecretKey, State};
use store::Vault;
use writer::{Error, Input, OUTBOX_TYPE, Upkeep, Writer, doc_to_json};

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(
        &vault.server_dir().join("state.db"),
        Some(SecretKey::generate()),
    )
    .unwrap();
    let w = Writer::open(vault, state).unwrap();
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    w.put_agent("drafter", "label = \"Drafter\"\n", |_| Vec::new())
        .unwrap();
    w.put_agent(
        "careless",
        "label = \"Careless\"\n\npermissions {\n  confirm {\n    notes = false\n  }\n}\n",
        |_| Vec::new(),
    )
    .unwrap();
    w.put_agent(
        "sender",
        "label = \"Sender\"\n\npermissions {\n  outbox {\n    send_without_review = true\n  }\n}\n",
        |_| Vec::new(),
    )
    .unwrap();
    w
}

fn user() -> Author {
    Author::Session("test".into())
}

fn agent(name: &str) -> Author {
    Author::Agent(name.into())
}

fn handler() -> Author {
    Author::Handler("outbox_send".into())
}

fn task() -> Author {
    Author::Task("google_gmail".into())
}

fn email(extra: Value) -> Value {
    let mut fields = json!({
        "title": "Lunch",
        "kind": "email",
        "to": ["ann@example.com"],
        "subject": "Lunch on Friday?",
    });
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    json!({ "fields": fields })
}

fn create(w: &Writer, by: &Author, note: Value) -> Result<String, Error> {
    w.create(OUTBOX_TYPE, &Input::Json(note), by).map(|s| s.id)
}

fn fields(w: &Writer, id: &str) -> Value {
    doc_to_json(&w.read(id).unwrap().doc)["fields"].clone()
}

/// Set `changes` on item `id` as `by`, against its current version. A null removes a field.
fn edit(w: &Writer, id: &str, by: &Author, changes: Value) -> Result<(), Error> {
    let note = w.read(id).unwrap();
    let mut body = doc_to_json(&note.doc);
    for (k, v) in changes.as_object().unwrap() {
        if v.is_null() {
            body["fields"].as_object_mut().unwrap().remove(k);
        } else {
            body["fields"][k] = v.clone();
        }
    }
    w.update(id, &Input::Json(body), Some(&note.version), by)
        .map(|_| ())
}

fn status(w: &Writer, id: &str) -> String {
    fields(w, id)["status"].as_str().unwrap().to_owned()
}

/// The message of the issue refusing a write, which names the rule.
#[track_caller]
fn refused(result: Result<impl std::fmt::Debug, Error>) -> String {
    match result {
        Err(Error::Invalid(inv)) => {
            assert_eq!(inv.issues.len(), 1, "{inv:?}");
            assert_eq!(inv.issues[0].at, "status", "{inv:?}");
            inv.issues[0].message.clone()
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn every_vault_has_the_outbox_type() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    assert!(w.is_built_in(OUTBOX_TYPE));
    let schema = w.schema();
    let ty = schema.type_def(OUTBOX_TYPE).unwrap();
    assert_eq!(ty.label, "Outbox item");
    for f in [
        "kind",
        "status",
        "drafted_by",
        "error",
        "sent_at",
        "sent_ref",
        "result",
        "to",
        "cc",
        "bcc",
        "subject",
        "in_reply_to",
        "attachments",
        "body",
        "phone",
        "device",
        "event",
        "when",
        "location",
        "attendees_raw",
        "calendar",
        "description",
    ] {
        assert!(ty.field(f).is_some(), "{f}");
    }
    assert!(ty.field("sent_ref").unwrap().hidden);
    let handlers: Vec<(&str, &str)> = ty
        .handlers
        .iter()
        .map(|h| (h.event.as_str(), h.script.as_str()))
        .collect();
    assert_eq!(
        handlers,
        [
            ("after_create", "outbox_send"),
            ("after_update", "outbox_send"),
            ("after_update", "google_gmail"),
        ]
    );
}

#[test]
fn the_outbox_has_four_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let schema = w.schema();
    let ty = schema.type_def(OUTBOX_TYPE).unwrap();
    match &ty.field("status").unwrap().kind {
        schema::FieldKind::Select { options, .. } => {
            assert_eq!(options, &["draft", "ready", "sending", "failed"]);
        }
        other => panic!("status is {other:?}"),
    }
    // The old statuses are not values the type takes.
    for gone in ["review", "approved", "sent", "rejected"] {
        assert!(
            matches!(
                create(&w, &user(), email(json!({ "status": gone }))),
                Err(Error::Invalid(_))
            ),
            "{gone}"
        );
    }
}

#[test]
fn the_user_makes_any_change() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &user(), email(json!({}))).unwrap();
    let f = fields(&w, &id);
    assert_eq!(f["status"], "draft");
    assert_eq!(f["drafted_by"], "user");
    // Ready, then everything else the user may do.
    for next in ["ready", "failed", "ready", "draft", "sending", "draft"] {
        edit(&w, &id, &user(), json!({ "status": next })).unwrap();
        assert_eq!(status(&w, &id), next);
    }
    // An API token is the user too.
    let token = Author::Token("phone".into());
    edit(
        &w,
        &id,
        &token,
        json!({ "status": "ready", "subject": "Lunch?" }),
    )
    .unwrap();
    // The user may create an item ready to send.
    let ready = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    assert_eq!(status(&w, &ready), "ready");
}

#[test]
fn an_agent_creates_drafts() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &agent("drafter"), email(json!({}))).unwrap();
    let f = fields(&w, &id);
    assert_eq!(f["status"], "draft");
    assert_eq!(f["drafted_by"], "drafter");
    let named = create(&w, &agent("drafter"), email(json!({ "status": "draft" }))).unwrap();
    assert_eq!(status(&w, &named), "draft");

    let m = refused(create(
        &w,
        &agent("drafter"),
        email(json!({ "status": "ready" })),
    ));
    assert!(m.contains("may not send without review"), "{m}");
    for other in ["sending", "failed"] {
        let m = refused(create(
            &w,
            &agent("drafter"),
            email(json!({ "status": other })),
        ));
        assert!(m.contains("creates Outbox items as draft"), "{other}: {m}");
    }
    let m = refused(create(
        &w,
        &agent("drafter"),
        email(json!({ "drafted_by": "user" })),
    ));
    assert!(m.contains("drafted by drafter"), "{m}");
    let m = refused(create(
        &w,
        &agent("drafter"),
        email(json!({ "sent_ref": "m1" })),
    ));
    assert!(m.contains("may not write sent_ref"), "{m}");
}

#[test]
fn only_an_agent_with_send_without_review_creates_ready_items() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &agent("sender"), email(json!({ "status": "ready" }))).unwrap();
    assert_eq!(status(&w, &id), "ready");
    // Still a draft when it says nothing.
    let id = create(&w, &agent("sender"), email(json!({}))).unwrap();
    assert_eq!(status(&w, &id), "draft");
    // Turning off the note confirmations does not loosen the rule (ai D22).
    let m = refused(create(
        &w,
        &agent("careless"),
        email(json!({ "status": "ready" })),
    ));
    assert!(m.contains("may not send without review"), "{m}");
    // Nor does an agent with no file.
    refused(create(
        &w,
        &agent("ghost"),
        email(json!({ "status": "ready" })),
    ));
}

#[test]
fn an_agent_edits_only_its_own_drafts_and_never_makes_them_ready() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &agent("drafter"), email(json!({}))).unwrap();
    edit(
        &w,
        &id,
        &agent("drafter"),
        json!({ "subject": "Lunch, Friday?" }),
    )
    .unwrap();
    edit(&w, &id, &agent("drafter"), json!({ "status": "draft" })).unwrap();

    for next in ["ready", "sending", "failed"] {
        let m = refused(edit(&w, &id, &agent("drafter"), json!({ "status": next })));
        assert!(m.contains("may not set an Outbox item to"), "{next}: {m}");
    }
    let m = refused(edit(
        &w,
        &id,
        &agent("drafter"),
        json!({ "drafted_by": "sender" }),
    ));
    assert!(m.contains("may not change drafted_by"), "{m}");
    let m = refused(edit(&w, &id, &agent("drafter"), json!({ "result": id })));
    assert!(m.contains("may not change result"), "{m}");

    // Another agent's item, and the user's.
    let m = refused(edit(
        &w,
        &id,
        &agent("sender"),
        json!({ "subject": "Mine now" }),
    ));
    assert!(m.contains("only the Outbox items it drafted"), "{m}");
    let mine = create(&w, &user(), email(json!({}))).unwrap();
    let m = refused(edit(
        &w,
        &mine,
        &agent("drafter"),
        json!({ "subject": "x" }),
    ));
    assert!(m.contains("only the Outbox items it drafted"), "{m}");

    // Once the user marks it ready, the agent cannot touch it, even with the bypass.
    let own = create(&w, &agent("sender"), email(json!({}))).unwrap();
    edit(&w, &own, &user(), json!({ "status": "ready" })).unwrap();
    let m = refused(edit(&w, &own, &agent("sender"), json!({ "subject": "x" })));
    assert!(m.contains("only while it is draft"), "{m}");
    let m = refused(edit(
        &w,
        &own,
        &agent("sender"),
        json!({ "status": "draft" }),
    ));
    assert!(m.contains("this one is ready"), "{m}");
    // And the bypass covers creating only: never making an existing draft ready.
    let draft = create(&w, &agent("sender"), email(json!({}))).unwrap();
    let m = refused(edit(
        &w,
        &draft,
        &agent("sender"),
        json!({ "status": "ready" }),
    ));
    assert!(m.contains("only the user marks"), "{m}");
    // Nor archiving one that is ready.
    let note = w.read(&own).unwrap();
    refused(w.set_archived(&own, true, Some(&note.version), &agent("sender")));
}

#[test]
fn the_send_handlers_and_syncs_move_items_along_the_send() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    edit(
        &w,
        &id,
        &handler(),
        json!({ "status": "sending", "error": null }),
    )
    .unwrap();
    // The send is recorded on the item, which stays sending until its copy is linked.
    edit(
        &w,
        &id,
        &handler(),
        json!({ "sent_ref": "m1", "sent_at": "2026-10-23T10:00:00Z" }),
    )
    .unwrap();
    // The Gmail task links the copy without changing the status.
    let copy = create(&w, &user(), email(json!({}))).unwrap();
    edit(&w, &id, &task(), json!({ "result": copy })).unwrap();
    let f = fields(&w, &id);
    assert_eq!(f["status"], "sending");
    assert_eq!(f["sent_ref"], "m1");
    assert_eq!(f["result"], copy.as_str());

    let failing = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    edit(&w, &failing, &handler(), json!({ "status": "sending" })).unwrap();
    edit(
        &w,
        &failing,
        &handler(),
        json!({ "status": "failed", "error": "Gmail answered 500" }),
    )
    .unwrap();
    // Retry is the user's.
    let m = refused(edit(&w, &failing, &handler(), json!({ "status": "ready" })));
    assert!(m.contains("not from failed to ready"), "{m}");
    edit(&w, &failing, &user(), json!({ "status": "ready" })).unwrap();
}

#[test]
fn the_send_handlers_and_syncs_change_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let draft = create(&w, &agent("drafter"), email(json!({}))).unwrap();
    for by in [handler(), task()] {
        let m = refused(edit(&w, &draft, &by, json!({ "status": "ready" })));
        assert!(m.contains("only from ready to sending"), "{m}");
        let m = refused(edit(&w, &draft, &by, json!({ "status": "sending" })));
        assert!(m.contains("not from draft to sending"), "{m}");
    }
    let ready = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    refused(edit(&w, &ready, &handler(), json!({ "status": "failed" })));
    let m = refused(edit(
        &w,
        &ready,
        &handler(),
        json!({ "status": "sending", "subject": "Changed" }),
    ));
    assert!(m.contains("not subject"), "{m}");
    let m = refused(create(&w, &handler(), email(json!({ "status": "ready" }))));
    assert!(m.contains("may not create Outbox items"), "{m}");
    refused(create(&w, &task(), email(json!({}))));
}

/// Item `id` moved to `sending` by the send handler, with its send recorded at `sent_at`.
fn sent(w: &Writer, id: &str, sent_at: &str) {
    edit(w, id, &handler(), json!({ "status": "sending" })).unwrap();
    edit(
        w,
        id,
        &handler(),
        json!({ "sent_ref": "m1", "sent_at": sent_at }),
    )
    .unwrap();
}

#[test]
fn a_sent_item_is_removed_for_good() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    // Only an item that was sent is removed: not a draft, a ready or a failed one.
    let m = refused(w.remove_sent_outbox(&id, None, &handler()));
    assert!(m.contains("only an Outbox item that was sent"), "{m}");
    sent(&w, &id, "2026-10-23T10:00:00Z");
    let copy = create(&w, &user(), email(json!({}))).unwrap();
    edit(&w, &id, &task(), json!({ "result": copy })).unwrap();
    w.remove_sent_outbox(&id, None, &handler()).unwrap();
    // Gone from the vault, the trash and the history: its copy is the regular note.
    assert!(matches!(w.read(&id), Err(Error::NotFound(_))));
    assert!(w.trash().unwrap().iter().all(|t| t.note_id != id));
    assert!(w.state().current_version(&id).unwrap().is_none());
    assert!(w.read(&copy).is_ok());
    assert!(matches!(
        w.remove_sent_outbox(&id, None, &handler()),
        Err(Error::NotFound(_))
    ));
    // A note of another type is never removed this way.
    refused(w.remove_sent_outbox(&copy, None, &handler()));
}

#[test]
fn the_sweep_removes_sent_items_after_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let sent_at = "2026-10-23T10:00:00Z";
    let at = 1_792_749_600_000; // 2026-10-23T10:00:00Z
    let waiting = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    sent(&w, &waiting, sent_at);
    let linked = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    sent(&w, &linked, sent_at);
    let copy = create(&w, &user(), email(json!({}))).unwrap();
    edit(&w, &linked, &task(), json!({ "result": copy })).unwrap();
    // Still waiting for the phone: no send recorded, so it stays however long it waits.
    let unsent = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    edit(
        &w,
        &unsent,
        &handler(),
        json!({ "status": "sending", "sent_ref": "cmd1" }),
    )
    .unwrap();
    let failed = create(&w, &user(), email(json!({ "status": "ready" }))).unwrap();
    edit(&w, &failed, &handler(), json!({ "status": "sending" })).unwrap();
    edit(
        &w,
        &failed,
        &handler(),
        json!({ "status": "failed", "error": "no" }),
    )
    .unwrap();

    // An hour on, only the item whose copy is linked goes.
    assert_eq!(w.sweep_outbox(at + 60 * 60 * 1000), 1);
    assert!(w.read(&linked).is_err());
    assert!(w.read(&waiting).is_ok());
    // A day on, the one whose copy never came goes too.
    assert_eq!(w.sweep_outbox(at + writer::outbox::SENT_KEEP_MS), 1);
    assert!(w.read(&waiting).is_err());
    assert!(w.trash().unwrap().is_empty());
    assert_eq!(status(&w, &unsent), "sending");
    assert_eq!(status(&w, &failed), "failed");
}
