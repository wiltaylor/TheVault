//! Handlers against a writer in-process: the durable `after_*` queue across a restart, and notes a
//! `before_*` handler changes joining the write in progress (D9, D30).

use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use script::{Handlers, Limits};
use serde_json::json;
use state::{Author, State};
use writer::{Input, Writer};

const TEMPLATE: &str = r#"
template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// A report is marked as reviewed by an `after_update` handler.
const REPORT: &str = r#"label = "Report"

field review {
  kind = "string"
}

handler "after_update" {
  script = "review"
}
"#;

const REVIEW: &str = r#"use vault

fn after_update(note: Value) -> Result[Value, string] {
    let id = note.get("id").unwrap().as_string().unwrap()
    let version = note.get("version").unwrap().as_string().unwrap()
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    if fields.contains_key("review") {
        Ok(Value::Null)
    } else {
        fields["review"] = Value::String("seen")
        vault::update(id, Value::Map(#{ "fields": Value::Map(fields) }), version)
    }
}
"#;

/// Saving a task logs it in a new `entry` note; a task titled "veto" is then refused by a second
/// handler.
const TASK: &str = r#"label = "Task"

handler "before_update" {
  script   = "journal"
  function = "log_it"
}

handler "before_update" {
  script   = "journal"
  function = "veto"
}
"#;

const ENTRY: &str = "label = \"Entry\"\n";

const JOURNAL: &str = r#"use vault

fn title_of(note: Value) -> string {
    note.get("note").unwrap().get("fields").unwrap().get("title").unwrap().as_string().unwrap()
}

fn log_it(note: Value) -> Result[Value, string] {
    let title = title_of(note)
    vault::create("entry", Value::Map(#{
        "fields": Value::Map(#{ "title": Value::String("saved " + title) }),
    }))?
    Ok(Value::Null)
}

fn veto(note: Value) -> Result[Value, string] {
    if title_of(note) == "veto" { Err("vetoed") } else { Ok(Value::Null) }
}
"#;

fn seed(dir: &Path) {
    for (path, body) in [
        ("types/report.wcl", format!("{REPORT}{TEMPLATE}")),
        ("types/task.wcl", format!("{TASK}{TEMPLATE}")),
        ("types/entry.wcl", format!("{ENTRY}{TEMPLATE}")),
        ("scripts/review.ws", REVIEW.to_owned()),
        ("scripts/journal.ws", JOURNAL.to_owned()),
    ] {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

/// Open the vault at `dir` with handlers, running the `after_*` queue when `start` is set.
fn open(dir: &Path, start: bool) -> (Arc<Writer>, Handlers) {
    let vault = store::Vault::init(dir).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    // A wall-clock limit a loaded test machine cannot reach: these tests are not about the limit.
    let handlers = Handlers::new(Limits {
        timeout: Duration::from_secs(600),
        ..Limits::default()
    });
    let writer = Arc::new(
        Writer::open(vault, state)
            .unwrap()
            .with_hooks(handlers.hooks()),
    );
    if start {
        handlers.start(&writer);
    } else {
        handlers.bind(&writer);
    }
    (writer, handlers)
}

fn user() -> Author {
    Author::Session("test".into())
}

fn json_note(fields: serde_json::Value) -> Input {
    Input::Json(json!({ "fields": fields }))
}

#[test]
fn after_jobs_queued_when_the_server_stops_run_when_it_starts_again() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());

    // A writer whose queue never starts: the job stays in `state.db`.
    let (writer, handlers) = open(dir.path(), false);
    let created = writer
        .create("report", &json_note(json!({ "title": "Q3" })), &user())
        .unwrap();
    // A token save: two quick saves by one browser session would merge into one version.
    let saved = writer
        .update(
            &created.id,
            &json_note(json!({ "title": "Q3 final" })),
            Some(&created.version),
            &Author::Token("test".into()),
        )
        .unwrap();
    assert_eq!(writer.state().queued_jobs().unwrap().len(), 1);
    assert_eq!(writer.history(&saved.id).unwrap().len(), 2);
    drop(writer);
    drop(handlers);

    // The next start runs it.
    let (writer, _handlers) = open(dir.path(), true);
    let deadline = Instant::now() + Duration::from_secs(300);
    while writer.history(&saved.id).unwrap().len() < 3 {
        assert!(Instant::now() < deadline, "the queued job never ran");
        thread::sleep(Duration::from_millis(50));
    }
    let history = writer.history(&saved.id).unwrap();
    assert_eq!(history[2].author, Author::Handler("review".into()));
    // Wait for the job set off by the handler's own update to finish too.
    while !writer.state().queued_jobs().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the queue never emptied");
        thread::sleep(Duration::from_millis(50));
    }
    assert!(writer.state().failed_jobs().unwrap().is_empty());
}

#[test]
fn a_note_a_before_handler_creates_commits_with_the_write_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let (writer, _handlers) = open(dir.path(), true);
    let task = writer
        .create("task", &json_note(json!({ "title": "Plan" })), &user())
        .unwrap();

    let saved = writer
        .update(
            &task.id,
            &json_note(json!({ "title": "Plan v2" })),
            Some(&task.version),
            &user(),
        )
        .unwrap();
    assert_eq!(saved.also.len(), 1, "{saved:?}");
    let entry = writer.read(&saved.also[0].id).unwrap();
    assert_eq!(entry.type_name, "entry");
    assert_eq!(
        writer::doc_to_json(&entry.doc)["fields"]["title"],
        "saved Plan v2"
    );
    assert_eq!(
        writer.history(&entry.id).unwrap()[0].author,
        Author::Handler("journal".into())
    );

    // The first handler creates an entry, the second rejects the write: no entry is written.
    let err = writer
        .update(
            &task.id,
            &json_note(json!({ "title": "veto" })),
            Some(&saved.version),
            &user(),
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "vetoed");
    assert_eq!(writer.list(Some("entry")).unwrap().len(), 1);
    let entries = std::fs::read_dir(dir.path().join("notes/entry"))
        .unwrap()
        .count();
    assert_eq!(entries, 1);
}
