//! Agent triggers (agents G9 to G12) with a stand-in runner and a clock the tests move: schedule
//! triggers, note events with their filters and debounce, no loops from agent writes, the run
//! limits, the run records and the failure notification; calls of agents written in notes, with
//! their idle wait and a conflicting write.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scheduler::{AgentRunner, Scheduler, TestClock, TriggerHooks, TurnReport};
use schema::Agent;
use script::{Ai, Host, Limits};
use serde_json::json;
use state::{AgentRunOutcome, Author, State};
use writer::{Input, NoHooks, Writer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MEMO: &str = r#"label = "Memo"

field status {
  kind = "string"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
  }
}
"#;

const LOG: &str = r#"label = "Log"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// What the stand-in runner was asked to do.
#[derive(Default)]
struct Fake {
    threads: Mutex<Vec<(String, String, String)>>,
    turns: Mutex<Vec<(String, String, String)>>,
    fail: Mutex<Option<String>>,
    /// How long each turn takes.
    delay: Mutex<Duration>,
    running: AtomicUsize,
    most: AtomicUsize,
    /// Run once, the next time a thread is opened.
    on_open: Mutex<Option<OnOpen>>,
}

type OnOpen = Box<dyn FnOnce() + Send>;

impl AgentRunner for Fake {
    fn open_thread(&self, agent: &Agent, origin: &str, title: &str) -> Result<String, String> {
        let hook = self.on_open.lock().unwrap().take();
        if let Some(f) = hook {
            f();
        }
        let mut t = self.threads.lock().unwrap();
        t.push((agent.name.clone(), origin.to_owned(), title.to_owned()));
        Ok(format!("thread{}", t.len()))
    }

    fn run_turn(&self, agent: &Agent, thread: &str, prompt: &str) -> TurnReport {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(*self.delay.lock().unwrap());
        self.turns
            .lock()
            .unwrap()
            .push((agent.name.clone(), thread.to_owned(), prompt.to_owned()));
        self.running.fetch_sub(1, Ordering::SeqCst);
        TurnReport {
            outcome: match self.fail.lock().unwrap().clone() {
                Some(m) => Err(m),
                None => Ok(()),
            },
            log: vec!["ai: provider=anthropic model=claude-test".to_owned()],
        }
    }
}

impl Fake {
    fn turns(&self) -> Vec<(String, String, String)> {
        self.turns.lock().unwrap().clone()
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    writer: Arc<Writer>,
    scheduler: Arc<Scheduler>,
    fake: Arc<Fake>,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("types")).unwrap();
        std::fs::write(dir.path().join("types/memo.wcl"), MEMO).unwrap();
        std::fs::write(dir.path().join("types/log.wcl"), LOG).unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        let clock = Arc::new(TestClock::new(scheduler::parse_time(START).unwrap()));
        let state =
            State::open_with_clock(&vault.server_dir().join("state.db"), None, clock.clone())
                .unwrap();
        let (hooks, binding) = TriggerHooks::wrap(Box::new(NoHooks));
        let writer = Arc::new(
            Writer::open(vault, state)
                .unwrap()
                .with_clock(clock.clone())
                .with_hooks(hooks),
        );
        let host = Arc::new(Host::new(
            writer.clone(),
            1,
            Limits::default(),
            Ai::unavailable(),
        ));
        let scheduler = Scheduler::new(writer.clone(), host, clock.clone()).unwrap();
        binding.bind(&scheduler);
        let fake = Arc::new(Fake::default());
        scheduler.set_agent_runner(fake.clone());
        Rig {
            _dir: dir,
            clock,
            writer,
            scheduler,
            fake,
        }
    }

    fn agent(&self, name: &str, wcl: &str) {
        self.writer.put_agent(name, wcl, |_| Vec::new()).unwrap();
        self.scheduler.reload_agents();
    }

    fn advance(&self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
        self.scheduler.tick();
    }

    fn create(&self, type_name: &str, title: &str, status: &str, by: &Author) -> String {
        let doc = json!({ "fields": { "title": title, "status": status } });
        let doc = if type_name == "memo" {
            doc
        } else {
            json!({ "fields": { "title": title } })
        };
        self.writer
            .create(type_name, &Input::Json(doc), by)
            .unwrap()
            .id
    }

    fn delete(&self, id: &str, by: &Author) {
        let n = self.writer.read(id).unwrap();
        self.writer.delete(id, Some(&n.version), by).unwrap();
    }

    fn set_status(&self, id: &str, status: &str, by: &Author) {
        let n = self.writer.read(id).unwrap();
        let mut doc = writer::doc_to_json(&n.doc);
        doc["fields"]["status"] = json!(status);
        self.writer
            .update(id, &Input::Json(doc), Some(&n.version), by)
            .unwrap();
    }
}

fn user() -> Author {
    Author::Session("owner".into())
}

const NIGHTLY: &str = r#"label = "Night owl"

trigger nightly {
  schedule = "0 2 * * *"
  prompt   = "Summarise the day."
}
"#;

#[test]
fn a_schedule_trigger_starts_a_thread_with_its_prompt() {
    let r = Rig::new();
    r.agent("owl", NIGHTLY);
    r.advance(60);
    assert!(r.fake.turns().is_empty(), "nothing is due at 00:31");
    let next = r.scheduler.agent_next_run("owl", "nightly").unwrap();
    assert_eq!(next, scheduler::parse_time("2026-01-05T02:00:00Z").unwrap());

    r.clock.set(next);
    r.scheduler.tick();
    let threads = r.fake.threads.lock().unwrap().clone();
    assert_eq!(
        threads,
        vec![(
            "owl".to_owned(),
            "trigger:nightly".to_owned(),
            "nightly 2026-01-05 02:00".to_owned()
        )]
    );
    assert_eq!(
        r.fake.turns(),
        vec![(
            "owl".to_owned(),
            "thread1".to_owned(),
            "Summarise the day.".to_owned()
        )]
    );
    // Recorded with its thread and trail line; the schedule moves on a day.
    let runs = r.writer.state().agent_runs("owl", "nightly", 20).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].thread.as_deref(), Some("thread1"));
    assert_eq!(runs[0].outcome, AgentRunOutcome::Succeeded);
    assert!(
        runs[0].log.contains("provider=anthropic"),
        "{}",
        runs[0].log
    );
    assert_eq!(
        r.scheduler.agent_next_run("owl", "nightly"),
        scheduler::parse_time("2026-01-06T02:00:00Z")
    );
    // Once fired, it does not fire again until tomorrow.
    r.advance(60);
    assert_eq!(r.fake.turns().len(), 1);
}

#[test]
fn a_disabled_agent_runs_no_trigger() {
    let r = Rig::new();
    r.agent("owl", &format!("enabled = false\n{NIGHTLY}"));
    r.clock
        .set(scheduler::parse_time("2026-01-05T02:00:00Z").unwrap());
    r.scheduler.tick();
    assert!(r.fake.turns().is_empty());
    assert_eq!(r.scheduler.agent_next_run("owl", "nightly"), None);
}

const WATCHER: &str = r#"label = "Watcher"

trigger opened {
  on     = ["create", "update"]
  type   = "memo"
  where  = "status == \"open\""
  prompt = "Look at this memo."
}

trigger gone {
  on     = ["delete"]
  type   = "memo"
  where  = "status == \"open\""
  prompt = "A memo went."
}
"#;

#[test]
fn note_events_start_runs_with_the_event_line_after_the_debounce() {
    let r = Rig::new();
    r.agent("watcher", WATCHER);
    let id = r.create("memo", "Roof", "open", &user());
    // Another type and a note the condition leaves out start nothing.
    r.create("log", "Roof log", "", &user());
    let closed = r.create("memo", "Gutters", "closed", &user());
    r.advance(10);
    assert!(
        r.fake.turns().is_empty(),
        "the run waits for the note to be quiet"
    );
    r.advance(25);
    let turns = r.fake.turns();
    assert_eq!(turns.len(), 1, "{turns:?}");
    assert_eq!(
        turns[0].2,
        format!("Look at this memo.\nEvent: create of note {id} (type memo, title \"Roof\")")
    );
    let threads = r.fake.threads.lock().unwrap().clone();
    assert_eq!(threads[0].1, "trigger:opened");
    assert!(threads[0].2.starts_with("opened "), "{}", threads[0].2);

    // An update that makes the closed memo match fires; one that does not, does not.
    r.set_status(&closed, "open", &user());
    r.set_status(&id, "closed", &user());
    r.advance(31);
    let turns = r.fake.turns();
    assert_eq!(turns.len(), 2, "{turns:?}");
    assert!(
        turns[1].2.ends_with(&format!(
            "Event: update of note {closed} (type memo, title \"Gutters\")"
        )),
        "{}",
        turns[1].2
    );

    // A delete is checked against the note before it went.
    r.delete(&closed, &user());
    r.delete(&id, &user());
    r.advance(31);
    let turns = r.fake.turns();
    assert_eq!(turns.len(), 3, "{turns:?}");
    assert_eq!(
        turns[2].2,
        format!("A memo went.\nEvent: delete of note {closed} (type memo, title \"Gutters\")")
    );
}

#[test]
fn changes_to_one_note_within_the_debounce_coalesce_into_one_run() {
    let r = Rig::new();
    r.agent("watcher", WATCHER);
    let id = r.create("memo", "Roof", "open", &user());
    for _ in 0..3 {
        r.advance(20);
        r.set_status(&id, "open", &user());
    }
    assert!(r.fake.turns().is_empty());
    r.advance(31);
    let turns = r.fake.turns();
    assert_eq!(turns.len(), 1, "{turns:?}");
    assert!(
        turns[0].2.contains("Event: update of note"),
        "{}",
        turns[0].2
    );
}

#[test]
fn an_agent_s_own_writes_never_fire_triggers() {
    let r = Rig::new();
    r.agent("watcher", WATCHER);
    let by = Author::Agent("watcher".into());
    let id = r.create("memo", "Roof", "open", &by);
    r.set_status(&id, "open", &Author::Agent("other".into()));
    r.delete(&id, &by);
    r.advance(31);
    assert!(r.fake.turns().is_empty());
    // A script or task does.
    r.create("memo", "Tiles", "open", &Author::Task("nightly".into()));
    r.advance(31);
    assert_eq!(r.fake.turns().len(), 1);
}

const ANY: &str = r#"label = "Any"

trigger every {
  on     = ["create"]
  prompt = "A note."
}
"#;

#[test]
fn an_agent_runs_at_most_two_at_once_and_queues_the_rest() {
    let r = Rig::new();
    r.agent("any", ANY);
    *r.fake.delay.lock().unwrap() = Duration::from_millis(60);
    for i in 0..5 {
        r.create("memo", &format!("Memo {i}"), "open", &user());
    }
    r.advance(31);
    assert_eq!(r.fake.turns().len(), 5);
    assert_eq!(r.fake.most.load(Ordering::SeqCst), 2);
}

#[test]
fn an_agent_starts_at_most_sixty_runs_an_hour_and_says_so_once() {
    let r = Rig::new();
    r.agent("any", ANY);
    // A few at a time, so none is dropped from the queue.
    for i in 0..62 {
        r.create("log", &format!("Log {i}"), "", &user());
        r.advance(31);
    }
    assert_eq!(r.fake.turns().len(), 60);
    let notes = r.writer.state().notifications(false, 50, None).unwrap();
    let limits: Vec<_> = notes
        .items
        .iter()
        .filter(|n| n.title.contains("60 trigger runs in an hour"))
        .collect();
    assert_eq!(limits.len(), 1, "{:?}", notes.items);
    assert_eq!(limits[0].url.as_deref(), Some("/agents/any/triggers"));
    // An hour on, runs start again.
    r.clock.advance(Duration::from_secs(3600));
    r.create("log", "Later", "", &user());
    r.advance(31);
    assert_eq!(r.fake.turns().len(), 61);
}

#[test]
fn a_failed_run_is_recorded_and_notified_once_linking_to_its_thread() {
    let r = Rig::new();
    r.agent("owl", NIGHTLY);
    *r.fake.fail.lock().unwrap() = Some("Anthropic is unavailable".into());
    r.clock
        .set(scheduler::parse_time("2026-01-05T02:00:00Z").unwrap());
    r.scheduler.tick();
    let runs = r.writer.state().agent_runs("owl", "nightly", 20).unwrap();
    assert_eq!(
        runs[0].outcome,
        AgentRunOutcome::Failed("Anthropic is unavailable".into())
    );
    let notes = r.writer.state().notifications(false, 50, None).unwrap();
    assert_eq!(notes.items.len(), 1);
    let n = &notes.items[0];
    assert_eq!(n.title, "Night owl\u{2019}s nightly run failed");
    assert_eq!(n.body.as_deref(), Some("Anthropic is unavailable"));
    assert_eq!(n.url.as_deref(), Some("/agents/owl/chat?thread=thread1"));
}

const CLERK: &str = r#"label = "Clerk"

trigger asked {
  mention = true
  type    = "memo"
}
"#;

fn status_of(r: &Rig, id: &str) -> String {
    let n = r.writer.read(id).unwrap();
    n.doc
        .field("status")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned()
}

#[test]
fn a_call_is_answered_once_the_note_is_idle_for_five_seconds() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    let id = r.create("memo", "Roof", "@clerk check the gutters", &user());
    r.advance(3);
    // A later save restarts the wait.
    r.set_status(&id, "@clerk check the gutters too", &user());
    r.advance(3);
    assert!(r.fake.turns().is_empty(), "the note was saved 3 s ago");
    r.advance(3);
    let threads = r.fake.threads.lock().unwrap().clone();
    assert_eq!(
        threads,
        vec![(
            "clerk".to_owned(),
            "mention".to_owned(),
            "Roof: you check the gutters too".to_owned()
        )]
    );
    let turns = r.fake.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].2,
        format!(
            "you check the gutters too\nCalled from note {id} (type memo, title \"Roof\"); the call was replaced with a link to this thread."
        )
    );
    // A string field holds no markup, so the call becomes plain text.
    assert_eq!(
        status_of(&r, &id),
        "Clerk (asked: /agents/clerk/chat?thread=thread1) check the gutters too"
    );
    let runs = r.writer.state().agent_runs("clerk", "asked", 20).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].thread.as_deref(), Some("thread1"));
    // A later save has no call left to answer.
    r.set_status(&id, &format!("{} (done)", status_of(&r, &id)), &user());
    r.advance(10);
    assert_eq!(r.fake.turns().len(), 1);
}

#[test]
fn a_write_meeting_a_newer_version_leaves_the_call_for_the_next_save() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    let id = r.create("memo", "Roof", "@clerk look", &user());
    // While the thread is opened, the user saves the note again.
    let w = r.writer.clone();
    let note = id.clone();
    *r.fake.on_open.lock().unwrap() = Some(Box::new(move || {
        let n = w.read(&note).unwrap();
        let mut doc = writer::doc_to_json(&n.doc);
        doc["fields"]["status"] = json!("@clerk look again");
        w.update(&note, &Input::Json(doc), Some(&n.version), &user())
            .unwrap();
    }));
    r.advance(6);
    assert!(r.fake.turns().is_empty(), "no run for a call left in place");
    assert_eq!(status_of(&r, &id), "@clerk look again");
    // That save is the next one: its call is answered.
    r.advance(6);
    assert_eq!(r.fake.turns().len(), 1);
    assert!(r.fake.turns()[0].2.starts_with("you look again\n"));
    assert_eq!(
        status_of(&r, &id),
        "Clerk (asked: /agents/clerk/chat?thread=thread2) look again"
    );
}

#[test]
fn a_call_written_by_an_agent_calls_no_one() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    let id = r.create(
        "memo",
        "Roof",
        "@clerk look",
        &Author::Agent("other".into()),
    );
    r.advance(10);
    assert!(r.fake.turns().is_empty());
    assert_eq!(status_of(&r, &id), "@clerk look");
}

#[test]
fn a_successful_run_sends_no_notification() {
    let r = Rig::new();
    r.agent("owl", NIGHTLY);
    r.clock
        .set(scheduler::parse_time("2026-01-05T02:00:00Z").unwrap());
    r.scheduler.tick();
    assert_eq!(r.fake.turns().len(), 1);
    let notes = r.writer.state().notifications(false, 50, None).unwrap();
    assert!(notes.items.is_empty());
}
