//! Agent triggers (agents G9 to G12) with a stand-in runner and a clock the tests move: schedule
//! triggers, note events with their filters and debounce, no loops from agent writes, the run
//! limits, the run records and the failure notification; calls of agents committed in notes,
//! each answered once by its call id and never by rewriting the note.

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

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
    slot "body"
  }
}
"#;

const LOG: &str = r#"label = "Log"

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
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

const CALL: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const CALL2: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";

impl Rig {
    /// A note of `type_name` whose body is one paragraph per text.
    fn note(&self, type_name: &str, title: &str, paragraphs: &[&str], by: &Author) -> String {
        self.writer
            .create(type_name, &Input::Json(body_json(title, paragraphs)), by)
            .unwrap()
            .id
    }

    fn save(&self, id: &str, title: &str, paragraphs: &[&str], by: &Author) {
        let n = self.writer.read(id).unwrap();
        self.writer
            .update(
                id,
                &Input::Json(body_json(title, paragraphs)),
                Some(&n.version),
                by,
            )
            .unwrap();
    }
}

fn body_json(title: &str, paragraphs: &[&str]) -> serde_json::Value {
    let blocks: Vec<_> = paragraphs
        .iter()
        .map(|p| json!({ "kind": "paragraph", "fields": { "text": p } }))
        .collect();
    json!({
        "fields": { "title": title },
        "blocks": [{ "kind": "content", "labels": ["body"], "blocks": blocks }],
    })
}

/// A call of `agent` with link text `text` and call id `call`.
fn call(text: &str, agent: &str, call: &str) -> String {
    format!("[{text}](agent:{agent}?call={call})")
}

#[test]
fn a_committed_call_is_answered_at_once_and_once_without_touching_the_note() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    let text = format!(
        "Gutters: {} today.",
        call("@Clerk check the gutters", "clerk", CALL)
    );
    let id = r.note("memo", "Roof", &[&text], &user());
    let version = r.writer.read(&id).unwrap().version;
    r.advance(1);
    let threads = r.fake.threads.lock().unwrap().clone();
    assert_eq!(
        threads,
        vec![(
            "clerk".to_owned(),
            "mention".to_owned(),
            "Roof: check the gutters".to_owned()
        )]
    );
    assert_eq!(
        r.fake.turns(),
        vec![(
            "clerk".to_owned(),
            "thread1".to_owned(),
            format!(
                "check the gutters\nContext: Gutters: @Clerk check the gutters today.\nCalled from note {id} (type memo, title \"Roof\")."
            )
        )]
    );
    // The note is as the user saved it.
    assert_eq!(r.writer.read(&id).unwrap().version, version);
    let recorded = r.writer.state().agent_call(CALL).unwrap().unwrap();
    assert_eq!(
        (recorded.agent.as_str(), recorded.note.as_str()),
        ("clerk", id.as_str())
    );
    assert_eq!(recorded.thread.as_deref(), Some("thread1"));
    let runs = r.writer.state().agent_runs("clerk", "asked", 20).unwrap();
    assert_eq!(runs.len(), 1);

    // Saving again, editing the call's text or moving it calls no one again.
    r.save(&id, "Roof", &[&text, "More."], &user());
    r.advance(1);
    let edited = call("@Clerk check the gutters and the roof", "clerk", CALL);
    r.save(&id, "Roof", &["Moved.", &edited], &user());
    r.advance(1);
    // The same call in another note calls no one either.
    r.note("memo", "Copy", &[&edited], &user());
    r.advance(1);
    assert_eq!(r.fake.turns().len(), 1);
    // A new call does.
    let again = call("@clerk and the shed", "clerk", CALL2);
    r.save(&id, "Roof", &[&edited, &again], &user());
    r.advance(1);
    assert_eq!(r.fake.turns().len(), 2);
    assert!(r.fake.turns()[1].2.starts_with("and the shed\n"));
}

#[test]
fn plain_text_and_an_agent_s_own_calls_call_no_one() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    r.note(
        "memo",
        "Roof",
        &[
            "@clerk look",
            "`[@Clerk x](agent:clerk?call=01JB8Z4K2MQ7V3X9T5R6W8Y2ZB)`",
        ],
        &user(),
    );
    r.advance(10);
    assert!(r.fake.turns().is_empty());
    assert_eq!(r.writer.state().agent_call(CALL).unwrap(), None);
    let by = Author::Agent("other".into());
    r.note("memo", "Roof", &[&call("@Clerk look", "clerk", CALL)], &by);
    r.advance(10);
    assert!(r.fake.turns().is_empty());
    assert_eq!(r.writer.state().agent_call(CALL).unwrap(), None);
}

#[test]
fn a_call_the_agent_may_not_answer_is_recorded_with_no_thread() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    // Clerk answers only memos; nobody is called "ghost".
    let id = r.note(
        "log",
        "Day",
        &[
            &call("@Clerk tidy this", "clerk", CALL),
            &call("@ghost boo", "ghost", CALL2),
        ],
        &user(),
    );
    r.advance(1);
    assert!(r.fake.turns().is_empty());
    for c in [CALL, CALL2] {
        let got = r.writer.state().agent_call(c).unwrap().unwrap();
        assert_eq!((got.note.as_str(), got.thread), (id.as_str(), None));
    }
    // It is never tried again.
    r.save(
        &id,
        "Day",
        &[&call("@Clerk tidy this", "clerk", CALL)],
        &user(),
    );
    r.advance(1);
    assert!(r.fake.threads.lock().unwrap().is_empty());
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
