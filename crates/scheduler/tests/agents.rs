//! Agent triggers (agents G9 to G12) with a stand-in runner and a clock the tests move: schedule
//! triggers, note events with their filters and debounce, no loops from agent writes, the run
//! limits, the run records and the failure notification; calls of agents committed in notes,
//! each answered once by its call id and never by rewriting the note.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use scheduler::{AgentRunner, Scheduler, TestClock, TriggerHooks, TurnReport};
use schema::Agent;
use script::{Ai, Host, Job, Kind, Limits};
use serde_json::json;
use state::{AgentRunOutcome, Author, Clock, State};
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
    /// The notes each thread was opened about, in order.
    notes: Mutex<Vec<Vec<String>>>,
    /// While set, every turn waits.
    hold: AtomicBool,
}

type OnOpen = Box<dyn FnOnce() + Send>;

impl AgentRunner for Fake {
    fn open_thread(
        &self,
        agent: &Agent,
        origin: &str,
        title: &str,
        notes: &[String],
    ) -> Result<String, String> {
        let hook = self.on_open.lock().unwrap().take();
        if let Some(f) = hook {
            f();
        }
        self.notes.lock().unwrap().push(notes.to_vec());
        let mut t = self.threads.lock().unwrap();
        t.push((agent.name.clone(), origin.to_owned(), title.to_owned()));
        Ok(format!("thread{}", t.len()))
    }

    fn run_turn(&self, agent: &Agent, thread: &str, prompt: &str, _scope: Option<state::Scope>) -> TurnReport {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(*self.delay.lock().unwrap());
        while self.hold.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
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
            log: vec!["ai: account=Anthropic kind=anthropic model=claude-test".to_owned()],
        }
    }
}

impl Fake {
    fn turns(&self) -> Vec<(String, String, String)> {
        self.turns.lock().unwrap().clone()
    }

    /// Wait, at most ten seconds, until `n` turns have ended.
    fn wait_turns(&self, n: usize) {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while self.turns.lock().unwrap().len() < n {
            assert!(
                std::time::Instant::now() < until,
                "{} turns",
                self.turns().len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
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
        runs[0].log.contains("account=Anthropic kind=anthropic"),
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

/// A series an agent saves makes its occurrences in the same write, written as the server's. The
/// write is the agent's, so no trigger fires, also not for the occurrences: an agent cannot start
/// itself again through them (TV-25).
#[test]
fn an_agent_s_series_and_its_occurrences_fire_no_triggers() {
    let r = Rig::new();
    r.writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    r.agent("any", ANY);
    let series = json!({ "fields": {
        "title": "Standup",
        "rule": ["RRULE:FREQ=WEEKLY;BYDAY=MO"],
        "starts": { "start": "2026-01-05", "end": "2026-01-05" },
    } });
    let saved = r
        .writer
        .create(
            writer::EVENT_SERIES_TYPE,
            &Input::Json(series),
            &Author::Agent("any".into()),
        )
        .unwrap();
    assert!(
        !saved.also.is_empty(),
        "the occurrences come in the same write"
    );
    r.advance(31);
    assert!(r.fake.turns().is_empty(), "{:?}", r.fake.turns());
    // A user's note still fires.
    r.create("memo", "Roof", "open", &user());
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

/// A clock that, once armed, holds the first thread to read it until the test lets it go.
struct Gate {
    inner: TestClock,
    armed: AtomicBool,
    entered: Mutex<mpsc::Sender<()>>,
    go: Mutex<mpsc::Receiver<()>>,
}

impl Clock for Gate {
    fn now_ms(&self) -> i64 {
        if self.armed.swap(false, Ordering::SeqCst) {
            let _ = self.entered.lock().unwrap().send(());
            let _ = self.go.lock().unwrap().recv();
        }
        self.inner.now_ms()
    }
}

/// The scheduler's owner drops it while `write` is held inside the writer's hooks, the scheduler
/// they upgraded in hand, and a run on the host waits for the writer's locks. `prepare` runs on
/// the writer first, unheld, and what it answers is given to `write`. Both the held write and the
/// run must finish, and the host must drop (TV-50).
fn drop_the_scheduler_during_a_held_write(
    prepare: impl FnOnce(&Writer) -> String,
    write: impl FnOnce(&Writer, &str) -> bool + Send + 'static,
) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::write(dir.path().join("types/memo.wcl"), MEMO).unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    let clock = Arc::new(TestClock::new(scheduler::parse_time(START).unwrap()));
    let state =
        State::open_with_clock(&vault.server_dir().join("state.db"), None, clock.clone()).unwrap();
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
    let host_left = Arc::downgrade(&host);
    let (entered_tx, entered) = mpsc::channel();
    let (go, go_rx) = mpsc::channel();
    let gate = Arc::new(Gate {
        inner: TestClock::new(scheduler::parse_time(START).unwrap()),
        armed: AtomicBool::new(false),
        entered: Mutex::new(entered_tx),
        go: Mutex::new(go_rx),
    });
    let scheduler = Scheduler::new(writer.clone(), host, gate.clone()).unwrap();
    binding.bind(&scheduler);
    scheduler.set_agent_runner(Arc::new(Fake::default()));
    let prepared = prepare(&writer);

    // A write that the scheduler's clock holds inside the hooks, while it holds the writer's
    // locks and the scheduler it upgraded to tell of the change.
    gate.armed.store(true, Ordering::SeqCst);
    let w = writer.clone();
    let (saved_tx, saved) = mpsc::channel();
    thread::spawn(move || {
        let _ = saved_tx.send(write(&w, &prepared));
    });
    entered
        .recv_timeout(Duration::from_secs(30))
        .expect("the write reached the hooks");

    // A run on the host that writes, so its worker waits for the writer's locks. Its caller gives
    // up at the timeout, which leaves the scheduler the only holder of the host. The timeout is
    // long enough for the run to reach vault::create under load: a host call that starts past it
    // faults without writing.
    let src = "use vault\nfn run() -> Result[(), string] {\n    vault::create(\"memo\", Value::Map(#{ \"fields\": Value::Map(#{ \"title\": Value::String(\"Late\") }) }))?\n    Ok(())\n}\n";
    let mut job = Job::new(Kind::Task, "late", src, "run");
    job.timeout = Some(Duration::from_secs(1));
    let h = host_left.upgrade().unwrap();
    let out = h.run(job);
    assert!(
        matches!(out.result, Err(script::Error::Timeout { .. })),
        "{:?}",
        out.result
    );
    drop(h);

    // The owner goes: the write's hooks now hold the last reference to the scheduler.
    drop(scheduler);
    go.send(()).unwrap();

    assert_eq!(
        saved.recv_timeout(Duration::from_secs(30)),
        Ok(true),
        "the held write finished"
    );
    let gone = (0..300).any(|_| {
        thread::sleep(Duration::from_millis(100));
        host_left.upgrade().is_none()
    });
    assert!(
        gone,
        "the host dropped once its worker finished the late write"
    );
    let titles: Vec<String> = writer
        .list(Some("memo"))
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert!(titles.contains(&"Late".to_owned()), "{titles:?}");
}

#[test]
fn the_scheduler_dropped_during_a_write_with_a_run_waiting_to_write_lets_both_finish() {
    // A create is held in the after hook, which tells the scheduler of the change.
    drop_the_scheduler_during_a_held_write(
        |_| String::new(),
        |w, _| {
            let doc = json!({ "fields": { "title": "Held", "status": "open" } });
            w.create("memo", &Input::Json(doc), &user()).is_ok()
        },
    );
}

#[test]
fn the_scheduler_dropped_during_a_delete_with_a_run_waiting_to_write_lets_both_finish() {
    // A delete is held in the before hook, which tells the scheduler of the delete to come.
    drop_the_scheduler_during_a_held_write(
        |w| {
            let doc = json!({ "fields": { "title": "Doomed", "status": "open" } });
            w.create("memo", &Input::Json(doc), &user()).unwrap().id
        },
        |w, id| {
            let n = w.read(id).unwrap();
            w.delete(id, Some(&n.version), &user()).is_ok()
        },
    );
}

const PLAIN: &str = r#"label = "Clerk"
"#;

#[test]
fn notes_sent_to_an_agent_run_once_on_a_dispatch_thread_about_them() {
    let r = Rig::new();
    r.agent("clerk", PLAIN);
    let notes = vec!["01JB8Z4K2MQ7V3X9T5R6W8Y2ZB".to_owned()];
    let thread = r
        .scheduler
        .dispatch("clerk", &notes, "Tidy: Roof", "Tidy them.", state::Scope::Admin)
        .unwrap();
    r.fake.wait_turns(1);
    assert_eq!(
        r.fake.threads.lock().unwrap().clone(),
        vec![(
            "clerk".to_owned(),
            "dispatch".to_owned(),
            "Tidy: Roof".to_owned()
        )]
    );
    assert_eq!(r.fake.notes.lock().unwrap().clone(), vec![notes]);
    assert_eq!(
        r.fake.turns(),
        vec![("clerk".to_owned(), thread, "Tidy them.".to_owned())]
    );
    // Recorded under the dispatch trigger, once it ends.
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let runs = r
            .writer
            .state()
            .agent_runs("clerk", "dispatch", 20)
            .unwrap();
        if runs.first().is_some_and(|run| run.ended_ms.is_some()) {
            break;
        }
        assert!(std::time::Instant::now() < until, "{runs:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    // An agent the scheduler does not run is refused, with no thread opened.
    assert_eq!(
        r.scheduler.dispatch("nobody", &[], "t", "p", state::Scope::Admin),
        Err(scheduler::DispatchRefused::NoAgent)
    );
    assert_eq!(r.fake.threads.lock().unwrap().len(), 1);
}

#[test]
fn a_dispatch_past_the_queue_or_the_hourly_limit_is_refused_and_opens_no_thread() {
    let r = Rig::new();
    r.agent("clerk", PLAIN);
    r.fake.hold.store(true, Ordering::SeqCst);
    let room = scheduler::MAX_RUNNING + scheduler::MAX_QUEUED;
    for i in 0..room {
        r.scheduler
            .dispatch("clerk", &[], &format!("Run {i}"), "Go.", state::Scope::Admin)
            .unwrap();
    }
    assert_eq!(
        r.scheduler.dispatch("clerk", &[], "One more", "Go.", state::Scope::Admin),
        Err(scheduler::DispatchRefused::QueueFull)
    );
    assert_eq!(r.fake.threads.lock().unwrap().len(), room);
    r.fake.hold.store(false, Ordering::SeqCst);
    r.fake.wait_turns(room);
    // Nothing queued was dropped; now the hour's runs run out.
    for i in room..scheduler::MAX_PER_HOUR {
        r.scheduler
            .dispatch("clerk", &[], &format!("Run {i}"), "Go.", state::Scope::Admin)
            .unwrap();
    }
    assert_eq!(
        r.scheduler.dispatch("clerk", &[], "Over", "Go.", state::Scope::Admin),
        Err(scheduler::DispatchRefused::OverLimit)
    );
    r.fake.wait_turns(scheduler::MAX_PER_HOUR);
    assert_eq!(
        r.fake.threads.lock().unwrap().len(),
        scheduler::MAX_PER_HOUR
    );
}

#[test]
fn a_full_queue_keeps_its_dispatch_runs_when_a_trigger_run_comes() {
    let r = Rig::new();
    r.agent("any", ANY);
    r.fake.hold.store(true, Ordering::SeqCst);
    let room = scheduler::MAX_RUNNING + scheduler::MAX_QUEUED;
    let mut sent = Vec::new();
    for i in 0..room {
        sent.push(
            r.scheduler
                .dispatch("any", &[], &format!("Run {i}"), "Go.", state::Scope::Admin)
                .unwrap(),
        );
    }
    // A trigger run finds the queue full of runs with threads: it is dropped, and no thread opens.
    r.create("memo", "Roof", "open", &user());
    r.advance(31);
    assert_eq!(r.fake.threads.lock().unwrap().len(), room);
    r.fake.hold.store(false, Ordering::SeqCst);
    r.fake.wait_turns(room);
    let mut ran: Vec<String> = r.fake.turns().into_iter().map(|t| t.1).collect();
    ran.sort();
    sent.sort();
    assert_eq!(ran, sent, "every dispatch run ran");
}

#[test]
fn a_dispatch_takes_the_place_of_a_trigger_run_waiting_in_a_full_queue() {
    let r = Rig::new();
    r.agent("any", ANY);
    r.fake.hold.store(true, Ordering::SeqCst);
    let room = scheduler::MAX_RUNNING + scheduler::MAX_QUEUED;
    for i in 0..room {
        r.create("memo", &format!("Memo {i}"), "open", &user());
    }
    // A tick waits for the runs it starts, and these are held, so the tick runs on its own
    // thread. Its runs are all admitted before any starts, so once two are running the queue is
    // full, with no thread in it.
    let sent = std::thread::scope(|s| {
        s.spawn(|| r.advance(31));
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while r.fake.running.load(Ordering::SeqCst) < scheduler::MAX_RUNNING {
            assert!(std::time::Instant::now() < until, "no two runs started");
            std::thread::sleep(Duration::from_millis(5));
        }
        let sent = r
            .scheduler
            .dispatch("any", &[], "Sent", "Go.", state::Scope::Admin)
            .expect("a trigger run gives way to a dispatch");
        r.fake.hold.store(false, Ordering::SeqCst);
        sent
    });
    r.fake.wait_turns(room);
    // One trigger run was dropped, so the turns are the room's count, the dispatch among them.
    assert_eq!(r.fake.turns().len(), room);
    assert!(r.fake.turns().iter().any(|t| t.1 == sent), "{sent} ran");
}

#[test]
fn a_call_refused_by_a_full_queue_is_not_recorded_and_the_next_save_asks_again() {
    let r = Rig::new();
    r.agent("clerk", CLERK);
    r.fake.hold.store(true, Ordering::SeqCst);
    let room = scheduler::MAX_RUNNING + scheduler::MAX_QUEUED;
    for i in 0..room {
        r.scheduler
            .dispatch("clerk", &[], &format!("Run {i}"), "Go.", state::Scope::Admin)
            .unwrap();
    }
    let text = format!(
        "Gutters: {} today.",
        call("@Clerk check the gutters", "clerk", CALL)
    );
    let id = r.note("memo", "Roof", &[&text], &user());
    r.advance(1);
    // No thread opened for it, and nothing recorded: the call is not answered.
    assert_eq!(r.fake.threads.lock().unwrap().len(), room);
    assert!(r.writer.state().agent_call(CALL).unwrap().is_none());

    r.fake.hold.store(false, Ordering::SeqCst);
    r.fake.wait_turns(room);
    r.save(&id, "Roof", &[&text, "More."], &user());
    r.advance(1);
    let recorded = r.writer.state().agent_call(CALL).unwrap().unwrap();
    assert!(recorded.thread.is_some(), "{recorded:?}");
    r.fake.wait_turns(room + 1);
    assert!(r.fake.turns()[room].2.starts_with("check the gutters\n"));
}
