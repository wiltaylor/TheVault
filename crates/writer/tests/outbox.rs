//! The built-in `outbox` type and who may move its items between statuses (outbox X1, X2): the
//! user any way, an agent only into Review (or approved with `send_without_review`) and only its
//! own drafts, and the send handlers and syncs only along approved, sending, sent or failed.

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
fn the_user_makes_any_change() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &user(), email(json!({}))).unwrap();
    let f = fields(&w, &id);
    assert_eq!(f["status"], "draft");
    assert_eq!(f["drafted_by"], "user");
    // Send, then everything else the user may do.
    for next in [
        "approved", "failed", "approved", "review", "rejected", "draft", "sent",
    ] {
        edit(&w, &id, &user(), json!({ "status": next })).unwrap();
        assert_eq!(status(&w, &id), next);
    }
    // An API token is the user too.
    let token = Author::Token("phone".into());
    edit(
        &w,
        &id,
        &token,
        json!({ "status": "approved", "subject": "Lunch?" }),
    )
    .unwrap();
    // The user may create an item ready to send.
    let ready = create(&w, &user(), email(json!({ "status": "approved" }))).unwrap();
    assert_eq!(status(&w, &ready), "approved");
}

#[test]
fn an_agent_creates_items_for_review() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &agent("drafter"), email(json!({}))).unwrap();
    let f = fields(&w, &id);
    assert_eq!(f["status"], "review");
    assert_eq!(f["drafted_by"], "drafter");
    let named = create(&w, &agent("drafter"), email(json!({ "status": "review" }))).unwrap();
    assert_eq!(status(&w, &named), "review");

    let m = refused(create(
        &w,
        &agent("drafter"),
        email(json!({ "status": "approved" })),
    ));
    assert!(m.contains("may not send without review"), "{m}");
    for other in ["draft", "sending", "sent", "failed", "rejected"] {
        let m = refused(create(
            &w,
            &agent("drafter"),
            email(json!({ "status": other })),
        ));
        assert!(m.contains("creates Outbox items as review"), "{other}: {m}");
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
fn only_an_agent_with_send_without_review_creates_approved_items() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &agent("sender"), email(json!({ "status": "approved" }))).unwrap();
    assert_eq!(status(&w, &id), "approved");
    // Still review when it says nothing.
    let id = create(&w, &agent("sender"), email(json!({}))).unwrap();
    assert_eq!(status(&w, &id), "review");
    // Turning off the note confirmations does not loosen the rule (ai D22).
    let m = refused(create(
        &w,
        &agent("careless"),
        email(json!({ "status": "approved" })),
    ));
    assert!(m.contains("may not send without review"), "{m}");
    // Nor does an agent with no file.
    refused(create(
        &w,
        &agent("ghost"),
        email(json!({ "status": "approved" })),
    ));
}

#[test]
fn an_agent_edits_only_its_own_drafts_and_never_approves() {
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
    edit(&w, &id, &agent("drafter"), json!({ "status": "review" })).unwrap();

    for next in ["approved", "rejected", "sending", "sent", "failed"] {
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

    // Once the user approves, the agent cannot touch it, even with the bypass.
    let own = create(&w, &agent("sender"), email(json!({}))).unwrap();
    edit(&w, &own, &user(), json!({ "status": "approved" })).unwrap();
    let m = refused(edit(&w, &own, &agent("sender"), json!({ "subject": "x" })));
    assert!(m.contains("only while it is draft or review"), "{m}");
    let m = refused(edit(
        &w,
        &own,
        &agent("sender"),
        json!({ "status": "review" }),
    ));
    assert!(m.contains("this one is approved"), "{m}");
    // And the bypass covers creating only: never approving an existing item.
    let review = create(&w, &agent("sender"), email(json!({}))).unwrap();
    refused(edit(
        &w,
        &review,
        &agent("sender"),
        json!({ "status": "approved" }),
    ));
    // Nor archiving one that is sent.
    edit(&w, &own, &user(), json!({ "status": "sent" })).unwrap();
    let note = w.read(&own).unwrap();
    refused(w.set_archived(&own, true, Some(&note.version), &agent("sender")));
}

#[test]
fn the_send_handlers_and_syncs_move_items_along_the_send() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let id = create(&w, &user(), email(json!({ "status": "approved" }))).unwrap();
    edit(
        &w,
        &id,
        &handler(),
        json!({ "status": "sending", "error": null }),
    )
    .unwrap();
    edit(
        &w,
        &id,
        &handler(),
        json!({ "status": "sent", "sent_ref": "m1", "sent_at": "2026-10-23T10:00:00Z" }),
    )
    .unwrap();
    // The Gmail task links the copy without changing the status.
    let copy = create(&w, &user(), email(json!({}))).unwrap();
    edit(&w, &id, &task(), json!({ "result": copy })).unwrap();
    let f = fields(&w, &id);
    assert_eq!(f["status"], "sent");
    assert_eq!(f["sent_ref"], "m1");
    assert_eq!(f["result"], copy.as_str());

    let failing = create(&w, &user(), email(json!({ "status": "approved" }))).unwrap();
    edit(&w, &failing, &handler(), json!({ "status": "sending" })).unwrap();
    edit(
        &w,
        &failing,
        &handler(),
        json!({ "status": "failed", "error": "Gmail answered 500" }),
    )
    .unwrap();
    // Retry is the user's.
    let m = refused(edit(
        &w,
        &failing,
        &handler(),
        json!({ "status": "approved" }),
    ));
    assert!(m.contains("not from failed to approved"), "{m}");
    edit(&w, &failing, &user(), json!({ "status": "approved" })).unwrap();
}

#[test]
fn the_send_handlers_and_syncs_change_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let review = create(&w, &agent("drafter"), email(json!({}))).unwrap();
    for by in [handler(), task()] {
        let m = refused(edit(&w, &review, &by, json!({ "status": "approved" })));
        assert!(m.contains("only from approved to sending"), "{m}");
        refused(edit(&w, &review, &by, json!({ "status": "rejected" })));
    }
    let draft = create(&w, &user(), email(json!({}))).unwrap();
    let m = refused(edit(&w, &draft, &handler(), json!({ "status": "sending" })));
    assert!(m.contains("not from draft to sending"), "{m}");
    let approved = create(&w, &user(), email(json!({ "status": "approved" }))).unwrap();
    refused(edit(&w, &approved, &handler(), json!({ "status": "sent" })));
    let m = refused(edit(
        &w,
        &approved,
        &handler(),
        json!({ "status": "sending", "subject": "Changed" }),
    ));
    assert!(m.contains("not subject"), "{m}");
    let m = refused(create(
        &w,
        &handler(),
        email(json!({ "status": "approved" })),
    ));
    assert!(m.contains("may not create Outbox items"), "{m}");
    refused(create(&w, &task(), email(json!({}))));
}
