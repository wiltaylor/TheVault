//! A sub agent's sessions follow their parent thread in the write that changes it
//! (subagents U5): deleting the parent deletes them, archiving or unarchiving it does the same
//! to them, and dropping a call from its `subagents` deletes that call's session. Only sessions
//! that name the parent follow it, and one already as asked is left as it is.

use std::path::Path;

use serde_json::{Value, json};
use state::{Author, State};
use store::Vault;
use writer::{AGENT_THREAD_TYPE, Event, Input, Saved, Upkeep, Writer};

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

fn doc(origin: &str, transcript: &Value) -> Input {
    Input::Json(json!({ "fields": {
        "title": "Check", "agent": "lead", "origin": origin,
        "transcript": transcript.to_string(),
    } }))
}

fn thread(w: &Writer, origin: &str, transcript: Value) -> Saved {
    w.create(AGENT_THREAD_TYPE, &doc(origin, &transcript), &author())
        .unwrap()
}

fn listing(subs: &[(&str, &str)]) -> Value {
    let list: Vec<Value> = subs
        .iter()
        .map(|(call, thread)| {
            json!({ "call": call, "thread": thread, "agent": "lead", "task": "Look",
                    "n": 1, "status": "done", "started_ms": 0 })
        })
        .collect();
    json!({ "messages": [], "subagents": list })
}

/// A parent thread with sessions for `toolu_0` and `toolu_1`: `(parent, [sessions])`.
fn family(w: &Writer) -> (Saved, [String; 2]) {
    let parent = thread(w, "user", json!({ "messages": [] }));
    let sub = |call: &str| {
        let t =
            json!({ "messages": [], "parent": { "thread": parent.id, "turn": "t", "call": call } });
        thread(w, &format!("subagent:{}", parent.id), t).id
    };
    let subs = [sub("toolu_0"), sub("toolu_1")];
    let parent = w
        .update(
            &parent.id,
            &doc(
                "user",
                &listing(&[("toolu_0", &subs[0]), ("toolu_1", &subs[1])]),
            ),
            Some(&parent.version),
            &author(),
        )
        .unwrap();
    (parent, subs)
}

fn archived(w: &Writer, id: &str) -> bool {
    writer::is_archived(&w.read(id).unwrap().doc)
}

fn also(saved: &Saved) -> Vec<(String, Event)> {
    saved.also.iter().map(|s| (s.id.clone(), s.event)).collect()
}

#[test]
fn deleting_a_parent_deletes_its_sessions_in_the_same_write() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let (parent, subs) = family(&w);
    // A fork lists the same sessions, which stay the parent's.
    let fork = thread(
        &w,
        "user",
        listing(&[("toolu_0", &subs[0]), ("toolu_1", &subs[1])]),
    );
    w.delete(&fork.id, Some(&fork.version), &author()).unwrap();
    for s in &subs {
        assert!(w.read(s).is_ok(), "a fork's delete leaves session {s}");
    }

    let saved = w
        .delete(&parent.id, Some(&parent.version), &author())
        .unwrap();
    let also = also(&saved);
    assert_eq!(also.len(), 2, "{also:?}");
    for s in &subs {
        assert!(also.contains(&(s.clone(), Event::Delete)), "{also:?}");
    }
    let trash: Vec<String> = w.trash().unwrap().into_iter().map(|t| t.note_id).collect();
    for s in &subs {
        assert!(w.read(s).is_err());
        assert!(trash.contains(s), "{trash:?}");
    }
}

#[test]
fn archiving_a_parent_archives_its_sessions_and_leaves_one_already_archived() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let (parent, subs) = family(&w);
    let first = w.read(&subs[0]).unwrap();
    let first = w
        .set_archived(&subs[0], true, Some(&first.version), &author())
        .unwrap();

    let versions = w.history(&parent.id).unwrap().len();
    let saved = w
        .set_archived(&parent.id, true, Some(&parent.version), &author())
        .unwrap();
    assert_eq!(also(&saved), vec![(subs[1].clone(), Event::Update)]);
    assert!(archived(&w, &subs[1]));
    assert_eq!(w.read(&subs[0]).unwrap().version, first.version);
    assert_eq!(w.history(&parent.id).unwrap().len(), versions + 1);

    let saved = w
        .set_archived(&parent.id, false, Some(&saved.version), &author())
        .unwrap();
    assert_eq!(saved.also.len(), 2);
    // Taking it out of the archive is a version of its own too, though the same session archived
    // it a moment before and its file aligns `archived` with the longer field names, as
    // `archived   = true`. A device keeps the version before as its sync base.
    assert_eq!(w.history(&parent.id).unwrap().len(), versions + 2);
    for s in &subs {
        assert!(!archived(&w, s));
    }
    // Already out of the archive: nothing more is written.
    let again = w
        .set_archived(&parent.id, false, Some(&saved.version), &author())
        .unwrap();
    assert_eq!(again.version, saved.version);
    assert!(again.also.is_empty());
}

#[test]
fn dropping_a_call_from_the_parent_deletes_that_calls_session() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let (parent, subs) = family(&w);
    let saved = w
        .update(
            &parent.id,
            &doc("user", &listing(&[("toolu_0", &subs[0])])),
            Some(&parent.version),
            &author(),
        )
        .unwrap();
    assert_eq!(also(&saved), vec![(subs[1].clone(), Event::Delete)]);
    assert!(w.read(&subs[0]).is_ok());
    assert!(w.read(&subs[1]).is_err());
    // A write that keeps the list leaves the sessions alone.
    let saved = w
        .update(
            &parent.id,
            &doc("user", &listing(&[("toolu_0", &subs[0])])),
            Some(&saved.version),
            &author(),
        )
        .unwrap();
    assert!(saved.also.is_empty());
    assert!(w.read(&subs[0]).is_ok());
}
