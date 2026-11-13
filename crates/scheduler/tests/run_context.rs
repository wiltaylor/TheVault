//! Live task provenance across admission, script workers and fake-clock retries.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use scheduler::{Scheduler, TestClock};
use script::{Ai, Host, Limits, RunContext};
use state::{Author, RunOutcome, State};
use writer::{Hooks, NoteChange, Writer};

#[derive(Debug)]
struct Observation {
    context: RunContext,
    restricted: bool,
    generation: Option<u64>,
}

struct Probe(mpsc::Sender<Observation>);

impl Hooks for Probe {
    fn before(&self, _: &mut NoteChange, _: &Author) -> Result<(), String> {
        let context = RunContext::current().expect("task context at writer callback");
        self.0
            .send(Observation {
                restricted: context.is_restricted(),
                generation: context.stop_generation(),
                context,
            })
            .unwrap();
        Err("fake failure for retry".into())
    }
}

struct Fixture {
    scheduler: Arc<Scheduler>,
    clock: Arc<TestClock>,
    seen: Mutex<mpsc::Receiver<Observation>>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        std::fs::write(
            dir.path().join("types/entry.wcl"),
            "label = \"Entry\"\ntemplate default { tab \"Main\" { slot \"title\" } }\n",
        )
        .unwrap();
        let clock = Arc::new(TestClock::new(0));
        let state =
            State::open_with_clock(&vault.server_dir().join("state.db"), None, clock.clone())
                .unwrap();
        let (send, recv) = mpsc::channel();
        let writer = Arc::new(
            Writer::open(vault, state)
                .unwrap()
                .with_hooks(Box::new(Probe(send))),
        );
        writer
            .put_script(
                "probe",
                r#"use vault
fn run() -> Result[Value, string] {
    vault::create("entry", Value::Map(#{
        "fields": Value::Map(#{ "title": Value::String("probe") }),
    }))
}
"#,
            )
            .unwrap();
        let host = Arc::new(Host::new(
            writer.clone(),
            2,
            Limits::default(),
            Ai::of(&writer),
        ));
        let scheduler = Scheduler::new(writer, host, clock.clone()).unwrap();
        scheduler
            .put_task(
                "probe",
                "schedule = \"* * * * *\"\nscript = \"probe\"\nretries = 2\nretry_delay = \"PT1S\"\n",
            )
            .unwrap();
        Self {
            scheduler,
            clock,
            seen: Mutex::new(recv),
            _dir: dir,
        }
    }

    fn observe(&self) -> Observation {
        self.seen
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
    }
}

#[test]
fn task_retries_keep_live_restriction_and_oldest_generation_across_workers() {
    let f = Fixture::new();
    let parent = RunContext::default();
    parent.capture_stop_generation(7);
    let first = f
        .scheduler
        .trigger_with_context("probe", parent.clone())
        .unwrap();
    assert!(matches!(first.outcome, RunOutcome::Failed(_)));
    let seen = f.observe();
    assert_eq!(seen.context, parent);
    assert!(!seen.restricted);
    assert_eq!(seen.generation, Some(7));

    // Restrict after admission and failure, while the retry is waiting.
    parent.restrict();
    parent.capture_stop_generation(99);
    f.scheduler.reload().unwrap();
    for (delay, attempt) in [(1, 2), (2, 3)] {
        f.clock.advance(Duration::from_secs(delay));
        let runs = f.scheduler.tick();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].attempt, attempt);
        let seen = f.observe();
        assert_eq!(seen.context, parent);
        assert!(seen.restricted);
        assert_eq!(seen.generation, Some(7));
    }
    assert!(f.scheduler.status("probe").unwrap().retry.is_none());
    assert!(RunContext::current().is_none());
}

#[test]
fn scheduled_firing_replaces_retry_with_an_independent_context() {
    let f = Fixture::new();
    let parent = RunContext::default();
    parent.restrict();
    parent.scope(|| f.scheduler.trigger("probe").unwrap());
    assert_eq!(f.observe().context, parent);
    f.clock.advance(Duration::from_secs(60));
    let runs = f.scheduler.tick();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].attempt, 1);
    let seen = f.observe();
    assert_ne!(seen.context, parent);
    assert!(!seen.restricted);
    assert_eq!(seen.generation, None);
}

#[test]
fn background_admission_carries_explicit_context_to_writer_callback() {
    let f = Fixture::new();
    let parent = RunContext::default();
    parent.restrict();
    parent.capture_stop_generation(3);
    f.scheduler
        .trigger_in_background_with_context("probe", parent.clone())
        .unwrap();
    let seen = f.observe();
    assert_eq!(seen.context, parent);
    assert!(seen.restricted);
    assert_eq!(seen.generation, Some(3));
}
