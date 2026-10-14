//! Cron triggers, retries with backoff, run records and an injectable clock (D10).
//!
//! A [`Scheduler`] reads the task files in `tasks/` through `writer` and keeps, for each task,
//! when it next fires and whether a retry is waiting. [`Scheduler::tick`] runs everything due at
//! the clock's current time and returns when those runs end. A task runs its script as a
//! [`script::Kind::Task`] job, with the domain allowlist, secrets, directories and timeout its
//! file names. Its changes are recorded as made by `task <name>`.
//!
//! - A schedule fires once however far the clock moved past it: missed firings are not replayed,
//!   and the next firing is the first one after the current time.
//! - A failed run is retried up to `retries` times. Retry `n` waits `retry_delay * 2^(n-1)` from
//!   the moment the failed run ended. A new scheduled firing replaces a retry still waiting.
//! - A task never runs twice at once. A paused task never fires on its schedule but can still be
//!   triggered by hand.
//! - Every run, retries included, is recorded in `state.db` with its attempt number, start and
//!   end times, outcome and log.
//!
//! Time comes from a [`state::Clock`]. The server uses the system clock and calls
//! [`Scheduler::start`] to tick in the background; tests use a [`TestClock`] they move by hand and
//! call [`Scheduler::tick`] after each move (S7).

mod describe;
mod task;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use script::{Host, Job, Kind};
use state::{Clock, RunOutcome, TaskRun};
use writer::Writer;

pub use describe::describe;
pub use task::{MAX_RETRIES, TaskDef, parse_duration, preview};

/// Errors from the scheduler.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("there is no task {0:?}")]
    NoTask(String),
    /// The task file is on disk but does not check.
    #[error("task {name:?} cannot run: {message}")]
    Broken { name: String, message: String },
    #[error("task {0:?} is already running")]
    Running(String),
    #[error(transparent)]
    Writer(#[from] writer::Error),
    #[error(transparent)]
    State(#[from] state::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An RFC 3339 time, such as `2026-01-05T00:30:00Z`, in milliseconds since the Unix epoch.
pub fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// A clock that moves only when told to (S7).
#[derive(Debug)]
pub struct TestClock {
    now_ms: AtomicI64,
}

impl TestClock {
    pub fn new(now_ms: i64) -> TestClock {
        TestClock {
            now_ms: AtomicI64::new(now_ms),
        }
    }

    /// Move the clock forward by `by`, returning the new time.
    pub fn advance(&self, by: Duration) -> i64 {
        let by = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
        self.now_ms.fetch_add(by, Ordering::SeqCst) + by
    }

    /// Set the clock to `now_ms`. It may move backwards.
    pub fn set(&self, now_ms: i64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

/// What the scheduler knows about one task right now.
#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub name: String,
    /// The task, or why its file does not check.
    pub def: std::result::Result<TaskDef, String>,
    /// When the schedule next fires, or `None` when paused or never.
    pub next_run_ms: Option<i64>,
    /// When the waiting retry runs, and its attempt number.
    pub retry: Option<(u32, i64)>,
    pub running: bool,
    pub latest_run: Option<TaskRun>,
}

impl TaskStatus {
    /// Whether the latest run failed, which the UI shows (D10).
    pub fn failing(&self) -> bool {
        matches!(
            self.latest_run.as_ref().map(|r| &r.outcome),
            Some(RunOutcome::Failed(_))
        )
    }
}

struct Slot {
    def: std::result::Result<TaskDef, String>,
    next_fire: Option<i64>,
    /// The attempt number of the waiting retry and when it is due.
    retry: Option<(u32, i64)>,
    running: bool,
}

#[derive(Default)]
struct Plan {
    slots: BTreeMap<String, Slot>,
    stopped: bool,
}

/// The longest the background loop sleeps before looking at the clock again.
const MAX_SLEEP: Duration = Duration::from_secs(30);

/// Runs the vault's scheduled tasks.
pub struct Scheduler {
    writer: Arc<Writer>,
    host: Arc<Host>,
    clock: Arc<dyn Clock>,
    plan: Mutex<Plan>,
    wake: Condvar,
}

impl Scheduler {
    /// A scheduler over the task files in `writer`'s vault, running scripts on `host` and reading
    /// time from `clock`. Every task's first firing is the first one after the clock's time now.
    pub fn new(
        writer: Arc<Writer>,
        host: Arc<Host>,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<Scheduler>> {
        let s = Arc::new(Scheduler {
            writer,
            host,
            clock,
            plan: Mutex::new(Plan::default()),
            wake: Condvar::new(),
        });
        s.reload()?;
        Ok(s)
    }

    pub fn writer(&self) -> &Arc<Writer> {
        &self.writer
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    fn plan(&self) -> MutexGuard<'_, Plan> {
        self.plan.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Read the task files again. A task whose file did not change keeps its next firing and any
    /// waiting retry; a new or changed task fires next at the first time after now.
    pub fn reload(&self) -> Result<()> {
        let files = self.writer.task_files()?;
        let now = self.clock.now_ms();
        let mut plan = self.plan();
        let mut slots = BTreeMap::new();
        for (name, text) in files {
            let def = TaskDef::parse(&name, &text).map_err(|inv| inv.to_string());
            let slot = match plan.slots.remove(&name) {
                Some(old) if old.def == def => old,
                old => Slot {
                    next_fire: def.as_ref().ok().and_then(|d| d.next_after(now)),
                    def,
                    retry: None,
                    running: old.is_some_and(|o| o.running),
                },
            };
            slots.insert(name, slot);
        }
        plan.slots = slots;
        drop(plan);
        self.wake.notify_all();
        Ok(())
    }

    /// Check `source` as task `name`, write it to `tasks/<name>.wcl` and reload.
    pub fn put_task(&self, name: &str, source: &str) -> Result<String> {
        let vault = self.writer.vault().root().to_path_buf();
        let text = self.writer.put_task(name, source, |text| {
            let def = TaskDef::parse(name, text)?;
            check_paths(&def, &vault)
        })?;
        self.reload()?;
        Ok(text)
    }

    /// Delete `tasks/<name>.wcl` and forget the task. Its run records stay.
    pub fn delete_task(&self, name: &str) -> Result<()> {
        self.writer.delete_task(name)?;
        self.reload()
    }

    /// Every task's status, by name.
    pub fn statuses(&self) -> Result<Vec<TaskStatus>> {
        let names: Vec<String> = self.plan().slots.keys().cloned().collect();
        names.iter().map(|n| self.status(n)).collect()
    }

    /// Task `name`'s status.
    pub fn status(&self, name: &str) -> Result<TaskStatus> {
        let (def, next_run_ms, retry, running) = {
            let plan = self.plan();
            let slot = plan
                .slots
                .get(name)
                .ok_or_else(|| Error::NoTask(name.to_owned()))?;
            let paused = slot.def.as_ref().is_ok_and(|d| d.paused);
            (
                slot.def.clone(),
                slot.next_fire.filter(|_| !paused),
                slot.retry,
                slot.running,
            )
        };
        Ok(TaskStatus {
            name: name.to_owned(),
            def,
            next_run_ms,
            retry,
            running,
            latest_run: self.writer.state().latest_run(name)?,
        })
    }

    /// Run every firing and retry due at the clock's time now, and wait for them to end. Returns
    /// the runs, in task name order.
    pub fn tick(&self) -> Vec<TaskRun> {
        let due = self.take_due();
        thread::scope(|s| {
            let handles: Vec<_> = due
                .into_iter()
                .map(|(def, attempt)| s.spawn(move || self.run(&def, attempt)))
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok().flatten())
                .collect()
        })
    }

    /// Run task `name` now, as a first attempt, whether or not it is paused. A failure schedules
    /// retries as a scheduled run's does. Returns the run record.
    pub fn trigger(&self, name: &str) -> Result<TaskRun> {
        let def = {
            let mut plan = self.plan();
            let slot = plan
                .slots
                .get_mut(name)
                .ok_or_else(|| Error::NoTask(name.to_owned()))?;
            let def = slot.def.clone().map_err(|message| Error::Broken {
                name: name.to_owned(),
                message,
            })?;
            if slot.running {
                return Err(Error::Running(name.to_owned()));
            }
            slot.running = true;
            slot.retry = None;
            def
        };
        self.run(&def, 1)
            .ok_or_else(|| Error::Running(name.to_owned()))
            .and_then(|r| self.writer.state().run(r.id)?.ok_or(Error::NoTask(r.task)))
    }

    /// Tick on a background thread until [`Scheduler::stop`], sleeping until the next firing or
    /// retry is due. Runs start on their own threads, so a slow task does not hold up the others.
    pub fn start(self: &Arc<Self>) {
        let s = self.clone();
        thread::Builder::new()
            .name("scheduler".into())
            .spawn(move || s.background())
            .expect("start the scheduler thread");
    }

    /// End the background loop.
    pub fn stop(&self) {
        self.plan().stopped = true;
        self.wake.notify_all();
    }

    fn background(self: Arc<Self>) {
        loop {
            for (def, attempt) in self.take_due() {
                let s = self.clone();
                thread::spawn(move || s.run(&def, attempt));
            }
            let plan = self.plan();
            if plan.stopped {
                return;
            }
            let now = self.clock.now_ms();
            let next = plan
                .slots
                .values()
                .filter(|s| !s.running)
                .flat_map(|s| {
                    let paused = s.def.as_ref().is_ok_and(|d| d.paused);
                    [s.next_fire.filter(|_| !paused), s.retry.map(|(_, at)| at)]
                })
                .flatten()
                .min();
            let sleep = next.map_or(MAX_SLEEP, |at| {
                Duration::from_millis(u64::try_from(at - now).unwrap_or(0)).min(MAX_SLEEP)
            });
            let _ = self
                .wake
                .wait_timeout(plan, sleep.max(Duration::from_millis(10)));
        }
    }

    /// Mark every due task running and return what to run: a waiting retry when one is due,
    /// otherwise a scheduled firing.
    fn take_due(&self) -> Vec<(TaskDef, u32)> {
        let now = self.clock.now_ms();
        let mut plan = self.plan();
        let mut due = Vec::new();
        for slot in plan.slots.values_mut() {
            let Ok(def) = &slot.def else { continue };
            if slot.running {
                continue;
            }
            let fires = slot.next_fire.is_some_and(|at| at <= now);
            if fires {
                slot.next_fire = def.next_after(now);
            }
            let attempt = if fires && !def.paused {
                1
            } else if let Some((attempt, at)) = slot.retry
                && at <= now
            {
                attempt
            } else {
                continue;
            };
            slot.retry = None;
            slot.running = true;
            due.push((def.clone(), attempt));
        }
        due
    }

    /// Run one attempt of a task already marked running, record it, and schedule a retry when it
    /// failed and has retries left. `None` only when `state.db` could not record the run.
    fn run(&self, def: &TaskDef, attempt: u32) -> Option<TaskRun> {
        let record = self.attempt(def, attempt);
        let failed = !matches!(
            record.as_ref().map(|r| &r.outcome),
            Ok(RunOutcome::Succeeded)
        );
        if let Err(e) = &record {
            eprintln!("task {:?}: recording the run failed: {e}", def.name);
        }
        let mut plan = self.plan();
        if let Some(slot) = plan.slots.get_mut(&def.name) {
            slot.running = false;
            if failed && attempt <= def.retries && slot.def.as_ref() == Ok(def) {
                let delay = i64::try_from(def.backoff(attempt).as_millis()).unwrap_or(i64::MAX);
                slot.retry = Some((attempt + 1, self.clock.now_ms().saturating_add(delay)));
            }
        }
        drop(plan);
        self.wake.notify_all();
        record.ok()
    }

    fn attempt(&self, def: &TaskDef, attempt: u32) -> Result<TaskRun> {
        let state = self.writer.state();
        let id = state.start_run(&def.name, attempt)?;
        let outcome = match self.writer.read_script(&def.script) {
            Ok(source) => {
                let mut job = Job::new(Kind::Task, &def.script, &source, &def.function);
                job.allow_domains = def.allow_domains.clone();
                job.secrets = def.secrets.clone();
                job.read_paths = def.read_paths.clone();
                job.write_paths = def.write_paths.clone();
                job.author = Some(state::Author::Task(def.name.clone()));
                job.timezone = Some(def.timezone.name().to_owned());
                job.timeout = Some(def.timeout);
                let out = self.host.run(job);
                let log = out.log.trim_end_matches('\n');
                if !log.is_empty() {
                    state.append_run_log(id, log)?;
                }
                match out.result {
                    Ok(_) => RunOutcome::Succeeded,
                    Err(e) => RunOutcome::Failed(e.to_string()),
                }
            }
            Err(e) => RunOutcome::Failed(e.to_string()),
        };
        state.finish_run(id, &outcome)?;
        state
            .run(id)?
            .ok_or(Error::State(state::Error::UnknownRun(id)))
    }
}

/// Refuse a task whose file access reaches into the vault: scripts change notes through `vault`,
/// never by editing vault files. The `fs` module checks again at every call, after resolving
/// symbolic links, since a directory can change after the task is saved.
fn check_paths(def: &TaskDef, vault: &std::path::Path) -> std::result::Result<(), schema::Invalid> {
    let vault = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    let mut issues = Vec::new();
    for (field, paths) in [
        ("read_paths", &def.read_paths),
        ("write_paths", &def.write_paths),
    ] {
        for p in paths {
            let path = std::path::Path::new(p);
            let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if real.starts_with(&vault) {
                issues.push(schema::Issue::new(
                    field,
                    format!(
                        "{p:?} is inside the vault; scripts change notes through the vault module"
                    ),
                ));
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(schema::Invalid {
            file: format!("tasks/{}.wcl", def.name),
            issues,
        })
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        self.stop();
    }
}
