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
fn current_handler_for_held_outbox_finishes_without_polling_forever() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, handlers) = open(dir.path(), false);
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let email = writer
        .create("email", &json_note(json!({"title":"Received"})), &user())
        .unwrap();
    let item = writer.create("outbox", &json_note(json!({"title":"Reply", "kind":"email", "status":"draft", "to":["ann@example.com"], "in_reply_to":email.id})), &user()).unwrap();
    for job in writer.state().queued_jobs().unwrap() {
        writer
            .state()
            .finish_job(job.id, Ok(()), "fixture")
            .unwrap();
    }
    writer.state().set_automation_stopped(true).unwrap();
    writer.state().set_automation_stopped(false).unwrap();
    writer
        .delete(&email.id, Some(&email.version), &user())
        .unwrap();
    let jobs = writer.state().queued_jobs().unwrap();
    assert!(!jobs.is_empty());
    handlers.start(&writer);
    let until = Instant::now() + Duration::from_secs(30);
    while !writer.state().queued_jobs().unwrap().is_empty() {
        assert!(Instant::now() < until, "held Outbox job kept polling");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(writer.outbox_run_context(&item.id).is_err());
    assert!(writer.state().failed_jobs().unwrap().iter().any(|job| matches!(&job.status, state::JobStatus::Failed(message) if message.contains("held: Outbox"))));
}

fn interrupted_outbox_restart(kind: &str, action: &str) {
    let dir = tempfile::tempdir().unwrap();
    let (writer, handlers) = open(dir.path(), false);
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let mut fields = json!({
        "title": "Check this send", "kind": kind, "status": "ready",
        "to": ["ann@example.com"],
    });
    if !action.is_empty() {
        fields["event_action"] = json!(action);
    }
    let item = writer
        .create("outbox", &json_note(fields), &user())
        .unwrap();
    let mut body = writer::doc_to_json(&writer.read(&item.id).unwrap().doc);
    body["fields"]["status"] = json!("sending");
    writer
        .update(
            &item.id,
            &Input::Json(body),
            Some(&item.version),
            &Author::Handler("outbox_send".into()),
        )
        .unwrap();
    drop(writer);
    drop(handlers);

    let (writer, _handlers) = open(dir.path(), true);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let note = writer.read(&item.id).unwrap();
        let body = writer::doc_to_json(&note.doc);
        if body["fields"]["status"] == "failed" {
            assert!(
                body["fields"]["error"]
                    .as_str()
                    .unwrap()
                    .contains("check before retrying")
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{kind}/{action} stayed sending: {body}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn error_phrase_without_marker_does_not_skip_event_chain() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    std::fs::write(dir.path().join("types/report.wcl"), format!(
        "label = \"Report\"\nfield review {{ kind = \"string\" }}\nhandler \"after_create\" {{ script = \"google_event\" }}\n{TEMPLATE}"
    )).unwrap();
    let (writer, handlers) = open(dir.path(), false);
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    std::fs::write(
        dir.path().join("scripts/google_event.ws"),
        REVIEW.replace("after_update", "after_create"),
    )
    .unwrap();
    std::fs::write(dir.path().join("scripts/outbox_send.ws"), r#"use vault
fn after_create(note: Value) -> Result[(), string] {
    vault::create("report", Value::Map(#{ "fields": Value::Map(#{ "title": Value::String("Replay me") }) }))?
    vault::patch(note.get("id").unwrap().as_string().unwrap(), Value::Map(#{ "status": Value::String("failed"), "error": Value::String("Provider says check before retrying") }), "")?
    Ok(())
}
fn after_update(note: Value) -> Result[(), string] { Ok(()) }
fn reconcile_send(note: Value) -> Result[(), string] { Ok(()) }
"#).unwrap();
    writer
        .create(
            "outbox",
            &json_note(json!({"title": "Failed send", "kind": "event", "status": "sending"})),
            &user(),
        )
        .unwrap();
    handlers.start(&writer);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !writer.state().queued_jobs().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "queue did not empty");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(writer.state().failed_jobs().unwrap().is_empty());
    let reports = writer.list(Some("report")).unwrap();
    assert_eq!(reports.len(), 1);
    let report = writer.read(&reports[0].id).unwrap();
    assert_eq!(writer::doc_to_json(&report.doc)["fields"]["review"], "seen");
}

#[test]
fn interrupted_email_restart() {
    interrupted_outbox_restart("email", "");
}

#[test]
fn interrupted_sms_restart() {
    interrupted_outbox_restart("sms", "");
}

#[test]
fn interrupted_event_create_restart() {
    interrupted_outbox_restart("event", "");
}

#[test]
fn interrupted_event_update_restart() {
    interrupted_outbox_restart("event", "update");
}

fn confirmed_outbox_restart(kind: &str, action: &str, timestamp: bool) {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let (writer, handlers) = open(dir.path(), false);
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let copy = writer
        .create("report", &json_note(json!({"title": "Sent copy"})), &user())
        .unwrap();
    let mut fields = json!({"title": "Confirmed", "kind": kind, "status": "ready"});
    if !action.is_empty() {
        fields["event_action"] = json!(action);
    }
    let item = writer
        .create("outbox", &json_note(fields), &user())
        .unwrap();
    let mut body = writer::doc_to_json(&writer.read(&item.id).unwrap().doc);
    body["fields"]["status"] = json!("sending");
    body["fields"]["sent_ref"] = json!("provider-confirmed");
    body["fields"]["result"] = json!(copy.id);
    if timestamp {
        body["fields"]["sent_at"] = json!("2026-11-12T09:00:00Z");
    }
    writer
        .update(
            &item.id,
            &Input::Json(body),
            Some(&item.version),
            &Author::Handler("outbox_send".into()),
        )
        .unwrap();
    drop(writer);
    drop(handlers);
    let (writer, _handlers) = open(dir.path(), true);
    let deadline = Instant::now() + Duration::from_secs(30);
    while writer.read(&item.id).is_ok() {
        assert!(
            Instant::now() < deadline,
            "confirmed {kind}/{action} not settled"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(writer.read(&copy.id).is_ok());
}

#[test]
fn confirmed_email_restart() {
    confirmed_outbox_restart("email", "", false);
}

#[test]
fn confirmed_sms_restart() {
    confirmed_outbox_restart("sms", "", true);
}

#[test]
fn confirmed_event_create_restart() {
    confirmed_outbox_restart("event", "", true);
}

#[test]
fn confirmed_event_update_restart() {
    confirmed_outbox_restart("event", "update", true);
}

#[test]
fn queued_handler_keeps_live_parent_context_through_script_and_writer() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let (writer, handlers) = open(dir.path(), false);
    let created = writer
        .create("report", &json_note(json!({ "title": "Q3" })), &user())
        .unwrap();
    let context = script::RunContext::default();
    let generation = writer.state().admit_automation().unwrap();
    context.capture_stop_generation(generation);
    context.scope(|| {
        writer
            .update(
                &created.id,
                &json_note(json!({ "title": "Q3 final" })),
                Some(&created.version),
                &Author::Token("test".into()),
            )
            .unwrap();
    });
    assert_eq!(writer.state().queued_jobs().unwrap().len(), 1);
    context.restrict();
    context.capture_stop_generation(generation);
    let (send, recv) = std::sync::mpsc::channel();
    writer.set_commit_hook(Some(Box::new(move |_| {
        let _ = send.send(script::RunContext::current());
        std::ops::ControlFlow::Continue(())
    })));
    handlers.start(&writer);
    let observed = recv.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
    assert_eq!(observed, context);
    assert!(observed.is_restricted());
    assert_eq!(observed.stop_generation(), Some(generation));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !writer.state().queued_jobs().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the queue never emptied");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(writer.state().failed_jobs().unwrap().is_empty());
    assert!(script::RunContext::current().is_none());
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
    let context = script::RunContext::default();
    let generation = writer.state().admit_automation().unwrap();
    context.capture_stop_generation(generation);
    let saved = context.scope(|| {
        writer
            .update(
                &created.id,
                &json_note(json!({ "title": "Q3 final" })),
                Some(&created.version),
                &Author::Token("test".into()),
            )
            .unwrap()
    });
    context.restrict();
    assert_eq!(writer.state().queued_jobs().unwrap().len(), 1);
    assert_eq!(writer.history(&saved.id).unwrap().len(), 2);
    drop(writer);
    drop(handlers);
    drop(context);

    // The next start runs it.
    let (writer, handlers) = open(dir.path(), false);
    let (send, recv) = std::sync::mpsc::channel();
    writer.set_commit_hook(Some(Box::new(move |_| {
        let _ = send.send(script::RunContext::current());
        std::ops::ControlFlow::Continue(())
    })));
    handlers.start(&writer);
    let recovered = recv.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
    assert!(recovered.is_restricted());
    assert_eq!(recovered.stop_generation(), Some(generation));
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
fn unreadable_handler_reference_fails_visibly_without_running_script() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let (writer, handlers) = open(dir.path(), false);
    let created = writer
        .create("report", &json_note(json!({"title":"Pending"})), &user())
        .unwrap();
    writer
        .update(
            &created.id,
            &json_note(json!({"title":"Queued"})),
            Some(&created.version),
            &Author::Token("test".into()),
        )
        .unwrap();
    let jobs = writer.state().queued_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    let id = writer
        .state()
        .queue_jobs_with_provenance(&[jobs[0].payload.clone()], "unreadable-handler")
        .unwrap()[0];
    writer
        .state()
        .finish_job(jobs[0].id, Ok(()), "fixture replacement")
        .unwrap();
    drop(writer);
    drop(handlers);
    let (writer, handlers) = open(dir.path(), false);
    let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = commits.clone();
    writer.set_commit_hook(Some(Box::new(move |_| {
        capture.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::ops::ControlFlow::Continue(())
    })));
    handlers.start(&writer);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let entries = writer
            .state()
            .activity(&state::ActivityFilter::default())
            .unwrap();
        if writer
            .state()
            .failed_jobs()
            .unwrap()
            .iter()
            .any(|job| job.id == id)
            && entries.items.iter().any(|entry| {
                entry.kind == "run.recovery_unavailable"
                    && entry.subject.as_deref() == Some("unreadable-handler")
            })
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "handler recovery was not audited"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        writer
            .state()
            .queued_jobs()
            .unwrap()
            .iter()
            .all(|job| job.id != id)
    );
    assert_eq!(writer.state().failed_jobs().unwrap().len(), 1);
    assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(writer.history(&created.id).unwrap().len(), 2);
}

#[test]
fn outbox_stop_resume_holds_old_jobs_and_owner_ready_sends() {
    let dir = tempfile::tempdir().unwrap();
    let (writer, handlers) = open(dir.path(), false);
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    // Stand-in delivery writes through the real handler/Writer commit boundary.
    std::fs::write(dir.path().join("scripts/outbox_send.ws"), r#"use vault
fn after_create(note: Value) -> Result[(), string] { after_update(note) }
fn after_update(note: Value) -> Result[(), string] {
    let id = note.get("id").unwrap().as_string().unwrap()
    let current = vault::get(id)?
    let status = current.get("note").unwrap().get("fields").unwrap().get("status").unwrap().as_string().unwrap()
    if status == "ready" {
        let patched = vault::patch(id, Value::Map(#{ "status": Value::String("sending"), "error": Value::String("stand-in sent") }), "")?
    }
    Ok(())
}
fn reconcile_send(note: Value) -> Result[(), string] { Ok(()) }
fn reconcile_delete(note: Value) -> Result[(), string] { Ok(()) }
"#).unwrap();
    let item = writer.create("outbox", &json_note(json!({"title":"Before Stop", "kind":"email", "status":"ready", "to":["ann@example.com"]})), &user()).unwrap();
    writer.state().set_automation_stopped(true).unwrap();
    writer.state().set_automation_stopped(false).unwrap();
    handlers.start(&writer);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if writer
            .state()
            .activity(&state::ActivityFilter::default())
            .unwrap()
            .items
            .iter()
            .any(|a| a.kind == "run.recovery_stale" && a.summary.contains(&item.id))
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "old Outbox job was not visibly held"
        );
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(2200));
    assert_eq!(writer.history(&item.id).unwrap().len(), 1);
    let held = writer
        .state()
        .activity(&state::ActivityFilter::default())
        .unwrap();
    assert_eq!(
        held.items
            .iter()
            .filter(|a| a.kind == "run.recovery_stale" && a.summary.contains(&item.id))
            .count(),
        1
    );
    let mut body = writer::doc_to_json(&writer.read(&item.id).unwrap().doc);
    body["fields"]["title"] = json!("Owner authorizes ready");
    writer
        .update(&item.id, &Input::Json(body), Some(&item.version), &user())
        .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let note = writer.read(&item.id).unwrap();
        if writer::doc_to_json(&note.doc)["fields"]["status"] == "sending" {
            break;
        }
        assert!(
            Instant::now() < until,
            "owner-ready delivery did not commit"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let failed = writer.state().failed_jobs().unwrap();
    assert!(!failed.is_empty());
    assert!(failed.iter().all(|job| matches!(&job.status, state::JobStatus::Failed(message) if message.contains("held: stale admission"))));
    assert!(
        writer
            .state()
            .queued_jobs()
            .unwrap()
            .iter()
            .all(|job| !failed.iter().any(|old| old.id == job.id))
    );
}

#[test]
fn legacy_handler_is_held_once_until_owner_edit_creates_fresh_work() {
    unavailable_handler_is_held_once_until_owner_edit(false);
}

#[test]
fn unreadable_handler_is_held_once_until_owner_edit_creates_fresh_work() {
    unavailable_handler_is_held_once_until_owner_edit(true);
}

fn unavailable_handler_is_held_once_until_owner_edit(unreadable: bool) {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let (writer, handlers) = open(dir.path(), false);
    let item = writer
        .create("report", &json_note(json!({"title":"Old"})), &user())
        .unwrap();
    let item = writer
        .update(
            &item.id,
            &json_note(json!({"title":"Legacy queued"})),
            Some(&item.version),
            &Author::Token("fixture".into()),
        )
        .unwrap();
    let old = writer.state().queued_jobs().unwrap().remove(0);
    let legacy = writer
        .state()
        .queue_jobs_with_provenance(&[old.payload], if unreadable { "missing-run" } else { "" })
        .unwrap()[0];
    writer
        .state()
        .finish_job(old.id, Ok(()), "fixture")
        .unwrap();
    handlers.start(&writer);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if writer
            .state()
            .activity(&state::ActivityFilter::default())
            .unwrap()
            .items
            .iter()
            .any(|a| {
                a.kind == "run.recovery_unavailable"
                    && a.summary.contains(&format!("Handler job {legacy} "))
            })
        {
            break;
        }
        assert!(Instant::now() < until, "legacy work was not visibly held");
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(2200));
    let entries = writer
        .state()
        .activity(&state::ActivityFilter::default())
        .unwrap();
    let held: Vec<_> = entries
        .items
        .iter()
        .filter(|a| a.kind == "run.recovery_unavailable")
        .collect();
    assert_eq!(held.len(), 1);
    assert!(held[0].summary.contains(&item.id));
    assert!(held[0].summary.contains("review"));
    assert!(held[0].summary.contains("editing the note as owner"));
    assert_eq!(writer.history(&item.id).unwrap().len(), 2);
    writer
        .update(
            &item.id,
            &json_note(json!({"title":"Owner released"})),
            Some(&item.version),
            &user(),
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if writer::doc_to_json(&writer.read(&item.id).unwrap().doc)["fields"]["review"] == "seen" {
            break;
        }
        assert!(Instant::now() < until, "fresh owner handler did not commit");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        writer
            .state()
            .queued_jobs()
            .unwrap()
            .iter()
            .all(|j| j.id != legacy)
    );
    assert!(writer.state().failed_jobs().unwrap().iter().any(|job| job.id == legacy && matches!(&job.status, state::JobStatus::Failed(message) if message.contains("held: provenance unavailable"))));
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
