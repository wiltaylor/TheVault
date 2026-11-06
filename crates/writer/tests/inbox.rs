//! The inbox's one-time catch-up (TV-30): every email, message and call sent or made before a
//! date is marked processed, in chunks, and nothing else changes.

use std::path::Path;

use serde_json::{Value, json};
use state::{Author, State};
use store::Vault;
use writer::{CHUNK, Input, Op, Upkeep, Writer, instant};

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = Writer::open(vault, state).unwrap();
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    w
}

fn author() -> Author {
    Author::Session("test".into())
}

fn generation(root: &Path) -> u64 {
    std::fs::read_to_string(root.join(".vault/generation"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0)
}

fn make(w: &Writer, ty: &str, fields: Value) -> String {
    w.create(ty, &Input::Json(json!({ "fields": fields })), &author())
        .unwrap()
        .id
}

fn processed(w: &Writer, id: &str) -> Option<bool> {
    match w.read(id).unwrap().doc.field("processed") {
        Some(schema::Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
    instant(s).unwrap()
}

#[test]
fn only_email_message_and_call_notes_before_the_date_and_not_processed_are_marked() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let old_mail = make(
        &w,
        "email",
        json!({ "title": "Old", "sent": "2026-09-01T10:00:00Z" }),
    );
    // Sent mail counts too: the rule is the date, not the direction.
    let old_sent = make(
        &w,
        "email",
        json!({ "title": "Old sent", "sent": "2026-09-02T10:00:00Z", "labels": ["system/SENT"] }),
    );
    let new_mail = make(
        &w,
        "email",
        json!({ "title": "New", "sent": "2026-11-02T10:00:00Z" }),
    );
    let undated = make(&w, "email", json!({ "title": "No date" }));
    let done_mail = make(
        &w,
        "email",
        json!({ "title": "Done", "sent": "2026-09-03T10:00:00Z", "processed": true }),
    );
    // An offset is honoured: 2026-11-01T01:00+02:00 is 2026-10-31T23:00Z, before the date.
    let sms = make(
        &w,
        "message",
        json!({ "title": "Hi", "direction": "in", "sent": "2026-11-01T01:00:00+02:00", "device_ref": "sms:1" }),
    );
    let sms_new = make(
        &w,
        "message",
        json!({ "title": "Later", "direction": "in", "sent": "2026-11-01T00:00:00Z", "device_ref": "sms:2" }),
    );
    let call = make(
        &w,
        "call",
        json!({ "title": "Missed", "direction": "missed", "started": "2026-10-01T08:00:00Z", "device_ref": "call:1" }),
    );
    let fleeting = make(&w, "fleeting", json!({ "title": "A thought" }));
    let mail_before = w.read(&old_mail).unwrap().doc;

    let before = at("2026-11-01T00:00:00Z");
    let dry = w.mark_processed_before(before, true, &author()).unwrap();
    assert!(dry.dry_run);
    assert_eq!(
        dry.matched,
        [
            ("email".to_owned(), 2),
            ("message".to_owned(), 1),
            ("call".to_owned(), 1)
        ]
    );
    assert_eq!(dry.total(), 4);
    assert_eq!((dry.changed, dry.chunks), (0, 0));
    assert_eq!(
        processed(&w, &old_mail),
        Some(false),
        "a dry run changes nothing"
    );

    let done = w.mark_processed_before(before, false, &author()).unwrap();
    assert_eq!(done.total(), 4);
    assert_eq!((done.changed, done.chunks), (4, 1));
    for id in [&old_mail, &old_sent, &sms, &call] {
        assert_eq!(processed(&w, id), Some(true), "{id}");
    }
    for id in [&new_mail, &undated, &sms_new] {
        assert_ne!(processed(&w, id), Some(true), "{id}");
    }
    assert_eq!(processed(&w, &done_mail), Some(true));
    assert_eq!(
        processed(&w, &fleeting),
        Some(false),
        "fleeting notes are never touched"
    );
    // Only `processed` changed.
    let mut mail_after = w.read(&old_mail).unwrap().doc;
    mail_after.fields.retain(|(n, _)| n != "processed");
    let mut mail_before = mail_before;
    mail_before.fields.retain(|(n, _)| n != "processed");
    assert_eq!(mail_after, mail_before);
}

#[test]
fn a_second_run_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    make(
        &w,
        "email",
        json!({ "title": "Old", "sent": "2026-09-01T10:00:00Z" }),
    );
    make(
        &w,
        "call",
        json!({ "title": "Missed", "direction": "missed", "started": "2026-10-01T08:00:00Z", "device_ref": "call:1" }),
    );
    let before = at("2026-11-01T00:00:00Z");
    assert_eq!(
        w.mark_processed_before(before, false, &author())
            .unwrap()
            .changed,
        2
    );

    let gen_before = generation(dir.path());
    let again = w.mark_processed_before(before, false, &author()).unwrap();
    assert_eq!(again.total(), 0);
    assert_eq!((again.changed, again.chunks), (0, 0));
    assert_eq!(
        generation(dir.path()),
        gen_before,
        "a second run made a commit"
    );
}

#[test]
fn many_notes_are_written_in_chunks_of_one_batch_each() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let n = 2 * CHUNK + 50;
    let ops: Vec<Op> = (0..n)
        .map(|i| Op::Create {
            type_name: "email".to_owned(),
            input: Input::Json(json!({ "fields": {
                "title": format!("Mail {i}"),
                "sent": format!("2026-0{}-{:02}T10:00:00Z", 1 + i % 9, 1 + i % 28),
            } })),
        })
        .collect();
    assert!(w.batch(ops, false, &author()).unwrap().ok);

    let gen_before = generation(dir.path());
    let done = w
        .mark_processed_before(at("2026-11-01T00:00:00Z"), false, &author())
        .unwrap();
    assert_eq!(done.changed, n);
    assert_eq!(done.chunks, 3);
    // One commit per chunk: the batches wrote nothing else.
    assert_eq!(generation(dir.path()), gen_before + 3);
    let left = writer_query_total(&w, "from email where !processed");
    assert_eq!(left, 0);
}

fn writer_query_total(w: &Writer, text: &str) -> usize {
    script_core::Vault::query(w, text, 10_000).unwrap().len()
}

#[test]
fn a_time_reads_with_an_offset_or_without_one_as_utc() {
    assert_eq!(
        instant("2026-11-01T01:00:00+02:00"),
        Some(at("2026-10-31T23:00:00Z"))
    );
    assert_eq!(
        instant("2026-11-01T01:00:00"),
        Some(at("2026-11-01T01:00:00Z"))
    );
    assert_eq!(
        instant("2026-11-01T01:00"),
        Some(at("2026-11-01T01:00:00Z"))
    );
    assert_eq!(instant("yesterday"), None);
}
