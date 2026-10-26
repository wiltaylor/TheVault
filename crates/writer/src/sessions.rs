//! Step 6, built in: a sub agent's sessions follow their parent thread (subagents U5).
//!
//! A session is an `agent_thread` note whose `transcript` names the thread that started it in
//! `parent.thread`; that thread's `transcript` lists the sessions it started in `subagents`, each
//! by its `call` and `thread`. Whatever makes the change to a parent, an API route, a script, an
//! AI proposal, an app's local write or a device's push arriving on the home server, the write
//! takes the sessions along in the same commit:
//!
//! - deleting the parent moves its sessions to the trash;
//! - archiving or unarchiving it does the same to them;
//! - a write that drops calls from its `subagents`, as a truncate does, deletes those calls'
//!   sessions.
//!
//! Only a session that still names the parent follows it: a fork copies the parent's list, but
//! the sessions stay the parent's. A session already as the change asks is left as it is, so the
//! same change made twice, or made on a device and arriving again on the home server, writes
//! nothing more.

use schema::{Doc, Value};

use crate::{
    AGENT_THREAD_TYPE, Error, Event, Inner, Input, NoteChange, Result, Writer, created_of, note,
    note_path, time,
};

/// The submitted changes with the sessions of every parent thread they change added: a delete
/// for each session that goes, an update for each that is archived or unarchived. A change the
/// write already makes to a session is turned into what its parent asks.
pub(crate) fn expand(
    writer: &Writer,
    inner: &Inner,
    mut changes: Vec<NoteChange>,
) -> Result<Vec<NoteChange>> {
    let mut i = 0;
    while i < changes.len() {
        let parent = changes[i].id.clone();
        for (sub, archived) in follow(writer, &changes[i])? {
            add(writer, inner, &mut changes, &parent, &sub, archived)?;
        }
        i += 1;
    }
    Ok(changes)
}

/// The sessions change `c` takes along, each with `None` to delete it or `Some(archived)`.
fn follow(writer: &Writer, c: &NoteChange) -> Result<Vec<(String, Option<bool>)>> {
    if c.type_name != AGENT_THREAD_TYPE || c.event == Event::Create {
        return Ok(Vec::new());
    }
    let Some(disk) = load(writer, &c.id, &c.type_name)? else {
        return Ok(Vec::new());
    };
    let before = sessions(&disk);
    let Some(doc) = &c.doc else {
        return Ok(before.into_iter().map(|(_, s)| (s, None)).collect());
    };
    let archived = note::is_archived(doc);
    let flipped = archived != note::is_archived(&disk);
    if before.is_empty() && !flipped {
        return Ok(Vec::new());
    }
    let after = sessions(doc);
    let mut out: Vec<(String, Option<bool>)> = before
        .into_iter()
        .filter(|(call, _)| !after.iter().any(|(c, _)| c == call))
        .map(|(_, s)| (s, None))
        .collect();
    if flipped {
        out.extend(after.into_iter().map(|(_, s)| (s, Some(archived))));
    }
    Ok(out)
}

/// Add to `changes` what session `sub` of thread `parent` does with it.
fn add(
    writer: &Writer,
    inner: &Inner,
    changes: &mut Vec<NoteChange>,
    parent: &str,
    sub: &str,
    archived: Option<bool>,
) -> Result<()> {
    if sub == parent {
        return Ok(());
    }
    let Some(type_name) = writer
        .locate(sub)?
        .filter(|t| t.as_str() == AGENT_THREAD_TYPE)
    else {
        return Ok(());
    };
    let Some(disk) = load(writer, sub, &type_name)? else {
        return Ok(());
    };
    if parent_of(&disk).as_deref() != Some(parent) {
        return Ok(());
    }
    let at = changes.iter().position(|c| c.id == sub);
    let doc = match at {
        Some(i) => match &changes[i].doc {
            Some(doc) => doc.clone(),
            // The write deletes it already.
            None => return Ok(()),
        },
        None => disk,
    };
    let change = match archived {
        None => NoteChange {
            id: sub.to_owned(),
            type_name,
            event: Event::Delete,
            doc: None,
        },
        Some(a) if note::is_archived(&doc) == a => return Ok(()),
        Some(a) => {
            let ty = inner
                .schema
                .type_def(&type_name)
                .ok_or_else(|| Error::UnknownType(type_name.clone()))?;
            let now = time::utc(writer.clock.now_ms());
            let created = created_of(&doc).unwrap_or_else(|| now.clone());
            let mut doc = doc;
            note::set_archived(&mut doc, a);
            NoteChange {
                id: sub.to_owned(),
                type_name,
                event: Event::Update,
                doc: Some(note::arrange(ty, doc, &created, &now)),
            }
        }
    };
    match at {
        Some(i) => changes[i] = change,
        None => changes.push(change),
    }
    Ok(())
}

/// The note on disk, parsed, or `None` when there is no such file.
fn load(writer: &Writer, id: &str, type_name: &str) -> Result<Option<Doc>> {
    let path = note_path(type_name, id);
    if !writer.vault.root().join(&path).is_file() {
        return Ok(None);
    }
    let text = writer.vault.read_text(&path)?;
    Ok(Some(Input::Wcl(text).parse(&path)?))
}

/// The thread's transcript, when it says something about sessions: most threads start none, and
/// their transcripts are not parsed.
fn transcript(doc: &Doc, key: &str) -> Option<serde_json::Value> {
    match doc.field("transcript") {
        Some(Value::Str(text)) if text.contains(key) => serde_json::from_str(text).ok(),
        _ => None,
    }
}

/// The sessions a thread lists, as `(call, thread)`.
fn sessions(doc: &Doc) -> Vec<(String, String)> {
    let Some(t) = transcript(doc, "\"subagents\"") else {
        return Vec::new();
    };
    t["subagents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some((
                s["call"].as_str()?.to_owned(),
                s["thread"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// The thread a session names as its parent.
fn parent_of(doc: &Doc) -> Option<String> {
    transcript(doc, "\"parent\"")?["parent"]["thread"]
        .as_str()
        .map(str::to_owned)
}
