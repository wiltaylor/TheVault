//! Agent triggers (agents G9 to G12): schedules and note events that start unattended agent runs.
//!
//! The scheduler reads the agent files beside the task files and keeps, for each schedule
//! trigger of an enabled agent, when it next fires, read as a task's `schedule` is. A note event
//! trigger hears of note changes from [`TriggerHooks`], which wraps the writer's hooks on the
//! server (never on the embedded app servers, which install no runner):
//!
//! - A change authored by an agent never fires a trigger, so an agent's own writes never loop
//!   back into it (agents G11). Neither does a change to an `agent_thread` or `agent_memory`
//!   note, unless the trigger names that type.
//! - A change matches a trigger by `on` and `type`, then by `where`, a query condition read
//!   against the note after the change, or before it for a delete.
//! - Changes to one note within [`DEBOUNCE_MS`] coalesce into one run with the last change, which
//!   starts once the note has been quiet that long.
//!
//! A run asks the [`AgentRunner`] for a new thread (`origin = "trigger:<name>"`) and runs one turn
//! on it whose first user message is the trigger's prompt, with an `Event:` line for a note event.
//! Each agent runs at most [`MAX_RUNNING`] runs at once; the rest queue. Past [`MAX_QUEUED`] the
//! newest queued trigger run, one with no thread, is dropped. A run with a thread (a call from a
//! note, or notes sent to the agent) is never dropped from the queue: a new one that finds no room
//! is refused, and a new trigger run is dropped instead. At most [`MAX_PER_HOUR`] runs start per
//! agent per hour; further ones are skipped, with one notification an hour. Every run is recorded
//! in `agent_runs` with its trail lines, and a failed run leaves one notification linking to its
//! thread (agents G12).
//!
//! A mention trigger (agents G16), an agent's own or the implicit one, answers calls committed in
//! a note (agents G19): links `[@<Label> <request>](agent:<name>?call=<id>)`, each whole in the
//! note's save. Every note a change not authored by an agent creates or updates is read at once,
//! and each call id not seen before is recorded in `agent_calls` and answered once: the runner
//! opens a thread (`origin = "mention"`) and the run starts, its first message the trigger's
//! prompt, the request, the text around the call and a `Called from note` line. A call to an agent
//! that may not be called from the note, or is disabled or missing, or is over its hourly limit,
//! is recorded with no thread and never answered. A call that finds the agent's queue full is not
//! recorded at all, so the next save of its note asks again. The note is never rewritten for a call.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, OnceLock, Weak};
use std::thread;

use query::{BinOp, Expr, From, Lit, PageRequest, Query};
use schema::{Agent, Doc, Schedule, Schema, Trigger, TriggerWhen};
use state::{AgentRunOutcome, Author, NewNotification};
use writer::{Event, Hooks, Migration, NoteChange, Saved, ScriptRole};

use crate::{Scheduler, clip, mentions};

/// How long a note must be quiet before the run its changes started goes ahead.
pub const DEBOUNCE_MS: i64 = 30_000;

/// The origin of a thread a call from a note opened.
pub const MENTION_ORIGIN: &str = "mention";

/// The origin of a thread that notes sent to an agent opened (TV-77 D1). Like a call from a
/// note, a person asked for its run, so its changes behind an "ask before" switch wait in the
/// thread as proposals (TV-77 D2).
pub const DISPATCH_ORIGIN: &str = "dispatch";

/// The trigger name a run of notes sent to an agent is recorded under in `agent_runs` (TV-77 D1).
pub const DISPATCH_TRIGGER: &str = "dispatch";

/// How much of the request a called agent's thread title holds, in characters.
const MENTION_TITLE_CHARS: usize = 40;

/// The most runs one agent has going at once.
pub const MAX_RUNNING: usize = 2;

/// The most runs one agent has waiting for a free slot.
pub const MAX_QUEUED: usize = 50;

/// The most runs one agent starts in an hour.
pub const MAX_PER_HOUR: usize = 60;

const HOUR_MS: i64 = 60 * 60 * 1000;

/// Whether `type_name` is a type the agents keep for themselves, `agent_thread` or
/// `agent_memory`, whose changes set off only a trigger that names the type (agents G7, G21).
fn is_agent_type(type_name: &str) -> bool {
    type_name == writer::AGENT_THREAD_TYPE || type_name == writer::AGENT_MEMORY_TYPE
}

/// What a turn of an unattended run reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    /// `Err` with what went wrong: a provider error, a refusal, or a turn that never ended.
    pub outcome: Result<(), String>,
    /// The turn's trail lines (ai D12).
    pub log: Vec<String>,
}

/// Runs agent turns with no one present, for the triggers. The server's API installs one; with
/// none, no trigger fires.
pub trait AgentRunner: Send + Sync {
    /// Create a thread of `agent` with `origin` and `title` about `notes` (the note that called
    /// it, or the notes sent to it; TV-77 D4), and return its note id.
    fn open_thread(
        &self,
        agent: &Agent,
        origin: &str,
        title: &str,
        notes: &[String],
    ) -> Result<String, String>;

    /// Run one turn of `agent` on thread `thread` whose user message is `prompt`.
    fn run_turn(&self, agent: &Agent, thread: &str, prompt: &str) -> TurnReport;
}

/// One run to start: an agent's trigger, the first message and the thread's title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentJob {
    pub agent: String,
    pub trigger: String,
    pub prompt: String,
    pub title: String,
    /// The thread opened for the run already, for a call from a note or notes sent to the agent;
    /// `None` opens one.
    pub thread: Option<String>,
    /// The origin of the thread opened already: [`MENTION_ORIGIN`] or [`DISPATCH_ORIGIN`].
    pub origin: Option<&'static str>,
}

/// Why notes sent to an agent were not run (TV-77 D1). Nothing was queued and no thread stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchRefused {
    /// This scheduler runs no agents: only the home server does.
    NoRunner,
    /// The agent is disabled, or has no file that parses.
    NoAgent,
    /// The agent started [`MAX_PER_HOUR`] runs in the last hour.
    OverLimit,
    /// [`MAX_QUEUED`] runs of the agent wait already, all with a thread, so none can be dropped
    /// to make room.
    QueueFull,
    /// The thread could not be opened, with why.
    Thread(String),
}

/// A note change as the hooks heard it, before it is matched against the triggers.
struct Heard {
    op: &'static str,
    id: String,
    type_name: String,
    at: i64,
    /// For a delete: what was read before it.
    deleting: Option<Deleting>,
}

/// A note about to be deleted: its title and the triggers whose `where` it matched.
#[derive(Debug, Clone, Default)]
struct Deleting {
    title: String,
    matched: Vec<(String, String)>,
}

/// A run a note change asked for, waiting for the note to be quiet.
struct Pending {
    op: &'static str,
    type_name: String,
    due: i64,
    /// For a delete, the title it had.
    title: Option<String>,
}

/// A pending run that is due, to check against the note now.
struct DueEvent {
    agent: Agent,
    trigger: Trigger,
    note: String,
    op: &'static str,
    type_name: String,
    title: Option<String>,
}

/// One agent's runs.
#[derive(Default)]
struct Lane {
    running: usize,
    queue: VecDeque<AgentJob>,
    /// When each run of the last hour started.
    started: VecDeque<i64>,
    /// When the hourly limit was last reported.
    noticed: Option<i64>,
}

/// What the agent triggers know right now, kept in the scheduler's plan.
#[derive(Default)]
pub(crate) struct AgentPlan {
    /// The enabled agents whose files parse, by name.
    agents: BTreeMap<String, Agent>,
    /// When each schedule trigger next fires, by agent and trigger, with the schedule it was
    /// worked out for.
    next: BTreeMap<(String, String), (Schedule, Option<i64>)>,
    heard: Vec<Heard>,
    deleting: HashMap<String, Deleting>,
    pending: BTreeMap<(String, String, String), Pending>,
    /// The notes to read for calls of agents, each with its type.
    mentions: BTreeMap<String, String>,
    lanes: BTreeMap<String, Lane>,
}

/// What became of a run asked for.
enum Admitted {
    Start(AgentJob),
    Queued,
    /// Over the hourly limit; `true` when the user is to be told.
    Skipped(bool),
    /// The queue is full and the run has no thread: it is dropped.
    Dropped,
    /// The queue is full of runs with threads and this run has one: it is refused, and the
    /// thread it came with is to be deleted.
    Refused,
}

impl AgentPlan {
    /// Take the agents as read now. A schedule that did not change keeps its next firing; a new
    /// or changed one fires next at the first time after `now`.
    fn set_agents(&mut self, agents: Vec<Agent>, now: i64) {
        let mut next = BTreeMap::new();
        for a in &agents {
            for t in &a.triggers {
                let TriggerWhen::Schedule(s) = &t.when else {
                    continue;
                };
                let key = (a.name.clone(), t.name.clone());
                let entry = match self.next.remove(&key) {
                    Some((old, at)) if old == *s => (old, at),
                    _ => (s.clone(), s.next_after(now)),
                };
                next.insert(key, entry);
            }
        }
        self.next = next;
        self.agents = agents.into_iter().map(|a| (a.name.clone(), a)).collect();
    }

    /// The note event triggers that `op` on a note of `type_name` passes by `on` and `type`.
    fn event_triggers(&self, op: &str, type_name: &str) -> Vec<(&Agent, &Trigger)> {
        let mut out = Vec::new();
        for a in self.agents.values() {
            for t in &a.triggers {
                if let TriggerWhen::NoteEvent(e) = &t.when
                    && e.matches(op, type_name)
                    && (!is_agent_type(type_name) || !e.types.is_empty())
                {
                    out.push((a, t));
                }
            }
        }
        out
    }

    /// The schedule firings due at `now`, each moved on to its next time, and the note changes
    /// heard so far matched and set waiting, and those whose wait is over.
    fn take_due(&mut self, now: i64) -> (Vec<AgentJob>, Vec<DueEvent>) {
        let mut jobs = Vec::new();
        for ((agent, trigger), (schedule, at)) in &mut self.next {
            if !at.is_some_and(|t| t <= now) {
                continue;
            }
            *at = schedule.next_after(now);
            let Some(t) = self.agents.get(agent).and_then(|a| a.trigger(trigger)) else {
                continue;
            };
            jobs.push(AgentJob {
                agent: agent.clone(),
                trigger: trigger.clone(),
                prompt: t.prompt.clone(),
                title: run_title(trigger, now, Some(&schedule.timezone)),
                thread: None,
                origin: None,
            });
        }
        for h in std::mem::take(&mut self.heard) {
            let matched: Vec<(String, String)> = self
                .event_triggers(h.op, &h.type_name)
                .into_iter()
                .filter(|(a, t)| {
                    let has_where =
                        matches!(&t.when, TriggerWhen::NoteEvent(e) if e.condition.is_some());
                    // A delete was checked against the note before it went.
                    !(h.op == "delete" && has_where)
                        || h.deleting.as_ref().is_some_and(|d| {
                            d.matched.iter().any(|(x, y)| *x == a.name && *y == t.name)
                        })
                })
                .map(|(a, t)| (a.name.clone(), t.name.clone()))
                .collect();
            for (agent, trigger) in matched {
                self.pending.insert(
                    (agent, trigger, h.id.clone()),
                    Pending {
                        op: h.op,
                        type_name: h.type_name.clone(),
                        due: h.at.saturating_add(DEBOUNCE_MS),
                        title: h.deleting.as_ref().map(|d| d.title.clone()),
                    },
                );
            }
        }
        let due: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.due <= now)
            .map(|(k, _)| k.clone())
            .collect();
        let mut events = Vec::new();
        for key in due {
            let Some(p) = self.pending.remove(&key) else {
                continue;
            };
            let (agent, trigger, note) = key;
            let Some(a) = self.agents.get(&agent) else {
                continue;
            };
            let Some(t) = a.trigger(&trigger) else {
                continue;
            };
            events.push(DueEvent {
                agent: a.clone(),
                trigger: t.clone(),
                note,
                op: p.op,
                type_name: p.type_name,
                title: p.title,
            });
        }
        (jobs, events)
    }

    /// The soonest time something is due: a schedule firing, a waiting note change, or now when
    /// changes were heard and not yet matched.
    pub(crate) fn next_due(&self, now: i64) -> Option<i64> {
        if !self.heard.is_empty() || !self.mentions.is_empty() {
            return Some(now);
        }
        self.next
            .values()
            .filter_map(|(_, at)| *at)
            .chain(self.pending.values().map(|p| p.due))
            .min()
    }

    /// The notes to read for calls, taken off the list.
    fn take_mentions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.mentions).into_keys().collect()
    }

    /// The agents a note of `type_name` may call, each with the mention trigger it answers by:
    /// every enabled agent, through its own mention trigger or the implicit one when it has none
    /// (agents G16). A trigger with no `type` is not called from an `agent_thread` or
    /// `agent_memory` note.
    fn callees(&self, type_name: &str) -> Vec<(Agent, Trigger)> {
        if schema::AGENT_CALL_EXCLUDED_TYPES.contains(&type_name) {
            return Vec::new();
        }
        self.agents
            .values()
            .filter_map(|a| {
                let t = a.mention_trigger(type_name)?;
                let named = matches!(&t.when, TriggerWhen::Mention(m) if !m.types.is_empty());
                (!is_agent_type(type_name) || named).then(|| (a.clone(), t))
            })
            .collect()
    }

    /// Whether a run asked for by a person may be admitted for `agent` now: under the hourly
    /// limit, and with room in its queue when its slots are taken (TV-77 D1). A full queue has
    /// room while one of its runs has no thread: [`AgentPlan::admit`] drops that run for it.
    fn room(&mut self, agent: &str, now: i64) -> Result<(), DispatchRefused> {
        if self.over_limit(agent, now) {
            return Err(DispatchRefused::OverLimit);
        }
        let lane = self.lanes.entry(agent.to_owned()).or_default();
        let full = lane.running >= MAX_RUNNING && lane.queue.len() >= MAX_QUEUED;
        if full && lane.queue.iter().all(|j| j.thread.is_some()) {
            return Err(DispatchRefused::QueueFull);
        }
        Ok(())
    }

    /// Whether `agent` has started its [`MAX_PER_HOUR`] runs in the last hour.
    fn over_limit(&mut self, agent: &str, now: i64) -> bool {
        let lane = self.lanes.entry(agent.to_owned()).or_default();
        while lane.started.front().is_some_and(|t| *t <= now - HOUR_MS) {
            lane.started.pop_front();
        }
        lane.started.len() >= MAX_PER_HOUR
    }

    /// Start `job` now, queue it behind its agent's running runs, or skip it over the hourly
    /// limit. With the queue full, a run with no thread is dropped, and a run with a thread makes
    /// room by dropping the newest run with no thread, or is refused when there is none.
    fn admit(&mut self, job: AgentJob, now: i64) -> Admitted {
        let lane = self.lanes.entry(job.agent.clone()).or_default();
        while lane.started.front().is_some_and(|t| *t <= now - HOUR_MS) {
            lane.started.pop_front();
        }
        if lane.started.len() >= MAX_PER_HOUR {
            eprintln!(
                "agent {:?}: trigger {:?} skipped: {MAX_PER_HOUR} runs started in the last hour",
                job.agent, job.trigger
            );
            let tell = lane.noticed.is_none_or(|t| t <= now - HOUR_MS);
            if tell {
                lane.noticed = Some(now);
            }
            return Admitted::Skipped(tell);
        }
        if lane.running < MAX_RUNNING {
            lane.started.push_back(now);
            lane.running += 1;
            return Admitted::Start(job);
        }
        if lane.queue.len() >= MAX_QUEUED {
            // A run with a thread is someone's request: only a run with none makes room.
            if job.thread.is_none() {
                eprintln!(
                    "agent {:?}: {MAX_QUEUED} runs are waiting, so trigger {:?} was dropped",
                    job.agent, job.trigger
                );
                return Admitted::Dropped;
            }
            let Some(i) = lane.queue.iter().rposition(|j| j.thread.is_none()) else {
                return Admitted::Refused;
            };
            if let Some(dropped) = lane.queue.remove(i) {
                eprintln!(
                    "agent {:?}: {MAX_QUEUED} runs are waiting, so the newest run with no thread, of trigger {:?}, was dropped",
                    dropped.agent, dropped.trigger
                );
            }
        }
        lane.started.push_back(now);
        lane.queue.push_back(job);
        Admitted::Queued
    }

    /// The next waiting run of `agent`, or `None`, freeing its slot, when none waits.
    fn next_queued(&mut self, agent: &str) -> Option<AgentJob> {
        let lane = self.lanes.get_mut(agent)?;
        match lane.queue.pop_front() {
            Some(j) => Some(j),
            None => {
                lane.running = lane.running.saturating_sub(1);
                None
            }
        }
    }
}

/// The title of a run's thread: the trigger and the local date and time.
fn run_title(trigger: &str, now: i64, timezone: Option<&str>) -> String {
    use chrono::TimeZone;
    let utc = chrono::Utc
        .timestamp_millis_opt(now)
        .single()
        .unwrap_or_default();
    let when = match timezone.and_then(|z| z.parse::<chrono_tz::Tz>().ok()) {
        Some(tz) => utc.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string(),
        None => utc
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
    };
    format!("{trigger} {when}")
}

impl Scheduler {
    /// Run agent triggers through `runner` from now on, the agent files read at once. Only the
    /// server does this; without a runner no trigger fires.
    pub fn set_agent_runner(&self, runner: Arc<dyn AgentRunner>) {
        if self.runner.set(runner).is_ok() {
            if let Err(e) = self.writer.state().end_interrupted_agent_runs() {
                eprintln!("agent runs left running: {e}");
            }
            self.reload_agents();
            self.wake.notify_all();
        }
    }

    fn runs_agents(&self) -> bool {
        self.runner.get().is_some()
    }

    /// Read the agent files again.
    pub fn reload_agents(&self) {
        let agents: Vec<Agent> = match self.writer.agents() {
            Ok(all) => all
                .into_iter()
                .filter_map(|(_, a)| a.ok())
                .filter(|a| a.enabled)
                .collect(),
            Err(_) => Vec::new(),
        };
        let now = self.clock.now_ms();
        self.plan().agents.set_agents(agents, now);
    }

    /// When agent `name`'s schedule trigger `trigger` next fires.
    pub fn agent_next_run(&self, name: &str, trigger: &str) -> Option<i64> {
        let key = (name.to_owned(), trigger.to_owned());
        self.plan().agents.next.get(&key).and_then(|(_, at)| *at)
    }

    /// Note `id` of `type_name` is about to be deleted: read what the delete triggers need of it
    /// while it is still in the index.
    fn before_delete(&self, id: &str, type_name: &str) {
        if !self.runs_agents() {
            return;
        }
        self.reload_agents();
        let triggers: Vec<(String, String, Option<Expr>)> = self
            .plan()
            .agents
            .event_triggers("delete", type_name)
            .into_iter()
            .map(|(a, t)| {
                let cond = match &t.when {
                    TriggerWhen::NoteEvent(e) => e.condition_expr(),
                    TriggerWhen::Schedule(_) | TriggerWhen::Mention(_) => None,
                };
                (a.name.clone(), t.name.clone(), cond)
            })
            .collect();
        if triggers.is_empty() {
            return;
        }
        let Some(title) = self.note_matches(id, type_name, None) else {
            return;
        };
        let matched = triggers
            .into_iter()
            .filter(|(_, _, c)| {
                c.as_ref()
                    .is_none_or(|c| self.note_matches(id, type_name, Some(c)).is_some())
            })
            .map(|(a, t, _)| (a, t))
            .collect();
        self.plan()
            .agents
            .deleting
            .insert(id.to_owned(), Deleting { title, matched });
    }

    /// Notes changed by `author`: keep them to match against the triggers.
    fn heard(&self, saved: &[Saved], author: &Author) {
        if !self.runs_agents() {
            return;
        }
        let at = self.clock.now_ms();
        let mut plan = self.plan();
        for s in saved.iter().flat_map(|s| std::iter::once(s).chain(&s.also)) {
            let deleting = plan.agents.deleting.remove(&s.id);
            // Scan the latest version, including refusals for agent-written calls.
            plan.agents.mentions.remove(&s.id);
            if s.event != Event::Delete {
                plan.agents
                    .mentions
                    .insert(s.id.clone(), s.type_name.clone());
            }
            if matches!(author, Author::Agent(_)) {
                continue;
            }
            let op = match s.event {
                Event::Create => "create",
                Event::Update => "update",
                Event::Delete => "delete",
            };
            plan.agents.heard.push(Heard {
                op,
                id: s.id.clone(),
                type_name: s.type_name.clone(),
                at,
                deleting,
            });
        }
        drop(plan);
        self.wake.notify_all();
    }

    /// The title of note `id`, of `type_name`, when it is in the index and matches `cond`.
    fn note_matches(&self, id: &str, type_name: &str, cond: Option<&Expr>) -> Option<String> {
        let mut filter = Expr::Binary(
            BinOp::Eq,
            Box::new(Expr::Name("id".to_owned())),
            Box::new(Expr::Lit(Lit::Str(id.to_owned()))),
        );
        if let Some(c) = cond {
            filter = Expr::Binary(BinOp::And, Box::new(filter), Box::new(c.clone()));
        }
        let q = Query {
            from: From::Types(vec![type_name.to_owned()]),
            include_archived: true,
            filter: Some(filter),
            search: None,
            sort: Vec::new(),
            group_by: None,
            select: Vec::new(),
            limit: Some(1),
        };
        let now = self.clock.now_ms() as f64 / 1000.0;
        let page = PageRequest {
            cursor: None,
            size: 1,
        };
        let got = self.writer.read_index(|index, schema| {
            let ctx = query::Context::new(index, schema).with_this(id).at(now);
            query::run(&q, &ctx, &page)
        });
        match got {
            Ok(p) => p.rows.into_iter().next().map(|r| r.title),
            Err(e) => {
                eprintln!("a trigger's where condition could not be read against note {id}: {e}");
                None
            }
        }
    }

    /// Read the agent files, take what is due and admit it: the runs to start now.
    pub(crate) fn agent_starts(&self) -> Vec<AgentJob> {
        if !self.runs_agents() {
            return Vec::new();
        }
        self.reload_agents();
        let now = self.clock.now_ms();
        let (mut jobs, events) = self.plan().agents.take_due(now);
        let called = self.plan().agents.take_mentions();
        let mut tell = Vec::new();
        // The runs of calls are admitted as they are recorded, under the same lock as their room.
        let mut called_runs = Vec::new();
        for id in called {
            called_runs.extend(self.mention_jobs(&id, now, &mut tell));
        }
        for e in events {
            let title = match e.op {
                "delete" => e.title.clone(),
                _ => {
                    let cond = match &e.trigger.when {
                        TriggerWhen::NoteEvent(n) => n.condition_expr(),
                        TriggerWhen::Schedule(_) | TriggerWhen::Mention(_) => None,
                    };
                    self.note_matches(&e.note, &e.type_name, cond.as_ref())
                }
            };
            let Some(title) = title else { continue };
            jobs.push(AgentJob {
                agent: e.agent.name.clone(),
                trigger: e.trigger.name.clone(),
                prompt: format!(
                    "{}\nEvent: {} of note {} (type {}, title {title:?})",
                    e.trigger.prompt, e.op, e.note, e.type_name
                ),
                title: run_title(&e.trigger.name, now, None),
                thread: None,
                origin: None,
            });
        }
        let mut starts: Vec<AgentJob> = called_runs
            .into_iter()
            .filter_map(|a| match a {
                Admitted::Start(j) => Some(j),
                _ => None,
            })
            .collect();
        let mut skipped = Vec::new();
        {
            let mut plan = self.plan();
            for job in jobs {
                match plan.agents.admit(job.clone(), now) {
                    Admitted::Start(j) => starts.push(j),
                    Admitted::Queued => {}
                    Admitted::Skipped(true) => {
                        skipped.push(job.clone());
                        tell.push(job);
                    }
                    Admitted::Skipped(false) => skipped.push(job),
                    // A trigger run has no thread, so it is dropped and never refused.
                    Admitted::Dropped | Admitted::Refused => {}
                }
            }
        }
        for job in &skipped {
            self.record_skipped(job);
        }
        for job in tell {
            self.notify_limit(&job);
        }
        starts
    }

    /// The runs that the calls committed in note `id` ask for (agents G19), each call id seen for
    /// the first time recorded in `agent_calls` with the thread opened for its run. A call that is
    /// not run (the agent may not be called from the note, is disabled or missing, or is over its
    /// hourly limit) is recorded with no thread; an agent over its limit is added to `tell` when
    /// the user is to hear of it. A call whose queue is full is refused and not recorded, so the
    /// next save of the note asks again. The note is left as it is. Answers the admitted runs,
    /// each [`Admitted::Start`] or [`Admitted::Queued`].
    fn mention_jobs(&self, id: &str, now: i64, tell: &mut Vec<AgentJob>) -> Vec<Admitted> {
        let Some(runner) = self.runner.get() else {
            return Vec::new();
        };
        let Ok(note) = self.writer.read(id) else {
            return Vec::new();
        };
        // Recheck the last persisted writer, not just the queued notification.
        let Ok(Some(version)) = self.writer.state().version(id, &note.version) else {
            return Vec::new();
        };
        let found = mentions::calls(&note.doc);
        if found.is_empty() {
            return Vec::new();
        }
        let state = self.writer.state();
        let callees = self.plan().agents.callees(&note.type_name);
        let title = note
            .doc
            .field("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let mut admits = Vec::new();
        for f in found {
            let call = &f.call;
            match state.agent_call(&call.call) {
                Ok(None) => {}
                Ok(Some(_)) => continue,
                Err(e) => {
                    eprintln!("call {} in note {id}: reading agent_calls: {e}", call.call);
                    continue;
                }
            }
            let record = |thread: Option<&str>| -> bool {
                match state.record_agent_call(&call.call, &call.agent, id, thread) {
                    Ok(new) => new,
                    Err(e) => {
                        eprintln!("call {} in note {id}: recording it: {e}", call.call);
                        false
                    }
                }
            };
            if state
                .agent_call_bound_elsewhere(&call.call, id, &call.agent)
                .unwrap_or(true)
            {
                let _ = state.refuse_agent_call_copy(&call.call, id, &call.agent);
                continue;
            }
            if matches!(version.author, Author::Agent(_))
                || !state
                    .consume_agent_call(&call.call, id, &call.agent)
                    .unwrap_or(false)
            {
                record(None);
                continue;
            }
            let Some((agent, trigger)) = callees.iter().find(|(a, _)| a.name == call.agent) else {
                eprintln!(
                    "call {} in note {id}: agent {:?} may not be called from a {} note, or is disabled or missing",
                    call.call, call.agent, note.type_name
                );
                record(None);
                continue;
            };
            let request = mentions::request(&call.label, agent);
            let context =
                (f.context != call.label.trim()).then(|| format!("Context: {}", f.context));
            let prompt = [
                trigger.prompt.trim(),
                request.as_str(),
                context.as_deref().unwrap_or(""),
                &format!(
                    "Called from note {id} (type {}, title {title:?}).",
                    note.type_name
                ),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
            let mut job = AgentJob {
                agent: agent.name.clone(),
                trigger: trigger.name.clone(),
                prompt,
                title: format!(
                    "{title}: {}",
                    mentions::first_chars(&request, MENTION_TITLE_CHARS)
                ),
                thread: None,
                origin: Some(MENTION_ORIGIN),
            };
            if self.plan().agents.over_limit(&agent.name, now) {
                let admitted = self.plan().agents.admit(job.clone(), now);
                self.record_skipped(&job);
                if let Admitted::Skipped(true) = admitted {
                    tell.push(job);
                }
                record(None);
                continue;
            }
            // Full queue: no thread is opened, and the call is not recorded (TV-86 F2).
            if self.plan().agents.room(&agent.name, now).is_err() {
                let _ = state.release_agent_call(&call.call);
                eprintln!(
                    "agent {:?}: call {} in note {id} is not run: its queue is full; the next save of the note asks again",
                    agent.name, call.call
                );
                continue;
            }
            let thread = match runner.open_thread(
                agent,
                MENTION_ORIGIN,
                &job.title,
                std::slice::from_ref(&note.id),
            ) {
                Ok(t) => t,
                Err(e) => {
                    // Not recorded: the next save tries again.
                    let _ = state.release_agent_call(&call.call);
                    eprintln!(
                        "agent {:?}: the thread for call {} in note {id} could not be opened: {e}",
                        agent.name, call.call
                    );
                    continue;
                }
            };
            if !record(Some(&thread)) {
                let by = Author::Agent(agent.name.clone());
                if let Err(e) = self.writer.delete(&thread, None, &by) {
                    eprintln!(
                        "agent {:?}: the unused thread {thread} stays: {e}",
                        agent.name
                    );
                }
                continue;
            }
            job.thread = Some(thread.clone());
            // Room is checked and the run admitted under one lock. Refused, the call is forgotten
            // and its thread goes, so the next save of the note asks again (TV-86 F2).
            let admitted = {
                let mut plan = self.plan();
                plan.agents
                    .room(&agent.name, now)
                    .map(|()| plan.agents.admit(job, now))
            };
            match admitted {
                Ok(a @ (Admitted::Start(_) | Admitted::Queued)) => admits.push(a),
                _ => {
                    eprintln!(
                        "agent {:?}: call {} in note {id} is not run: its queue is full; the next save of the note asks again",
                        agent.name, call.call
                    );
                    self.drop_thread(&agent.name, &thread);
                    if let Err(e) = state.forget_agent_call(&call.call) {
                        eprintln!("call {} in note {id}: forgetting it: {e}", call.call);
                    }
                }
            }
        }
        admits
    }

    /// Run notes sent to `agent` (TV-77 D1): open a thread of origin [`DISPATCH_ORIGIN`] about
    /// `notes` titled `title`, and start a run on it whose first message is `prompt`, now or once
    /// one of the agent's slots is free. The hourly limit and the queue are checked first, so a
    /// dispatch is refused rather than skipped or dropped, and no thread stays for a refused one.
    /// Answers the thread's id.
    pub fn dispatch(
        self: &Arc<Self>,
        agent: &str,
        notes: &[String],
        title: &str,
        prompt: &str,
    ) -> Result<String, DispatchRefused> {
        let runner = self.runner.get().ok_or(DispatchRefused::NoRunner)?.clone();
        let now = self.clock.now_ms();
        // The agent as its file says now, as a run reads it, not as the plan last loaded it.
        let a = match self.writer.agent(agent) {
            Ok(a) if a.enabled => a,
            _ => return Err(DispatchRefused::NoAgent),
        };
        self.plan().agents.room(agent, now)?;
        // Not under the plan's lock: the write that opens the thread runs the hooks, which read
        // the plan.
        let thread = runner
            .open_thread(&a, DISPATCH_ORIGIN, title, notes)
            .map_err(DispatchRefused::Thread)?;
        let job = AgentJob {
            agent: a.name.clone(),
            trigger: DISPATCH_TRIGGER.to_owned(),
            prompt: prompt.to_owned(),
            title: title.to_owned(),
            thread: Some(thread.clone()),
            origin: Some(DISPATCH_ORIGIN),
        };
        let admitted = {
            let mut plan = self.plan();
            plan.agents
                .room(agent, now)
                .map(|()| plan.agents.admit(job, now))
        };
        let refused = match admitted {
            Ok(Admitted::Start(job)) => {
                let s = self.clone();
                thread::Builder::new()
                    .name("agent-dispatch".into())
                    .spawn(move || s.agent_work(job))
                    .err()
                    .map(|e| {
                        // The slot taken for it is free again.
                        if let Some(lane) = self.plan().agents.lanes.get_mut(agent) {
                            lane.running = lane.running.saturating_sub(1);
                        }
                        DispatchRefused::Thread(format!("no thread to run it on: {e}"))
                    })
            }
            Ok(Admitted::Queued) => None,
            // `room` was checked under the same lock, so `admit` does not skip or refuse it.
            Ok(Admitted::Skipped(_)) => Some(DispatchRefused::OverLimit),
            Ok(Admitted::Dropped | Admitted::Refused) => Some(DispatchRefused::QueueFull),
            Err(e) => Some(e),
        };
        if let Some(e) = refused {
            self.drop_thread(&a.name, &thread);
            return Err(e);
        }
        Ok(thread)
    }

    /// Delete the thread opened for a run that does not go ahead.
    fn drop_thread(&self, agent: &str, thread: &str) {
        let by = Author::Agent(agent.to_owned());
        if let Err(e) = self.writer.delete(thread, None, &by) {
            eprintln!("agent {agent:?}: the unused thread {thread} stays: {e}");
        }
    }

    /// Run `job`, then whatever waits behind it for the same agent, until none does.
    pub(crate) fn agent_work(&self, mut job: AgentJob) {
        loop {
            self.run_agent_job(&job);
            let next = self.plan().agents.next_queued(&job.agent);
            match next {
                Some(j) => job = j,
                None => break,
            }
        }
        self.wake.notify_all();
    }

    /// Run one job: a new thread and one turn on it, recorded in `agent_runs`.
    fn run_agent_job(&self, job: &AgentJob) {
        let Some(runner) = self.runner.get() else {
            return;
        };
        // The agent as its file says now: disabled or gone since, it does not run.
        let agent = match self.writer.agent(&job.agent) {
            Ok(a) if a.enabled => a,
            _ => return,
        };
        let state = self.writer.state();
        let id = match state.start_agent_run(&agent.name, &job.trigger) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("agent {:?}: recording the run failed: {e}", agent.name);
                return;
            }
        };
        let origin = format!("trigger:{}", job.trigger);
        let opened = match &job.thread {
            Some(t) => Ok(t.clone()),
            None => runner.open_thread(&agent, &origin, &job.title, &[]),
        };
        let (outcome, log, thread) = match opened {
            Ok(thread) => {
                if let Err(e) = state.set_agent_run_thread(id, &thread) {
                    eprintln!("agent {:?}: recording the run's thread: {e}", agent.name);
                }
                let r = runner.run_turn(&agent, &thread, &job.prompt);
                (r.outcome, r.log, Some(thread))
            }
            Err(e) => (Err(e), Vec::new(), None),
        };
        let outcome = match outcome {
            Ok(()) => AgentRunOutcome::Succeeded,
            Err(m) => AgentRunOutcome::Failed(m),
        };
        eprintln!(
            "agent run {id}: agent={} trigger={} thread={} outcome={}",
            agent.name,
            job.trigger,
            thread.as_deref().unwrap_or("-"),
            outcome.as_str()
        );
        if let Err(e) = state.finish_agent_run(id, &outcome, &log.join("\n")) {
            eprintln!("agent {:?}: recording the run's end: {e}", agent.name);
        }
        let (word, message) = match &outcome {
            AgentRunOutcome::Failed(m) => ("failed", Some(m.as_str())),
            _ => ("succeeded", None),
        };
        let mut summary = format!("{}\u{2019}s {} run {word}", agent.label, job.trigger);
        if let Some(m) = message {
            summary.push_str(&format!(": {}", m.lines().next().unwrap_or("").trim()));
        }
        let mut entry = state::NewActivity::new(
            "agent.run",
            "automation",
            &Author::Agent(agent.name.clone()),
            summary,
        )
        .detail(serde_json::json!({
            "agent": agent.name,
            "trigger": job.trigger,
            "run": id,
            "thread": thread,
            "origin": job.origin.map_or_else(|| origin.clone(), str::to_owned),
            "outcome": word,
            "error": message.map(|m| state::one_line(m, 300)),
        }));
        if let Some(t) = &thread {
            entry = entry.subject("thread", t);
        }
        state.record_activity(entry);
        // What the run drafted into the Outbox has its notification; the next run starts another.
        self.writer.end_outbox_drafts(&agent.name);
        if let AgentRunOutcome::Failed(message) = &outcome {
            self.notify_agent_failure(&agent, &job.trigger, thread.as_deref(), message);
        }
    }

    /// The activity entry of a run skipped over the hourly limit (activity Y2).
    fn record_skipped(&self, job: &AgentJob) {
        self.writer.state().record_activity(
            state::NewActivity::new(
                "agent.run.skipped",
                "automation",
                &Author::Agent(job.agent.clone()),
                format!(
                    "{}\u{2019}s {} run skipped: {MAX_PER_HOUR} runs started in the last hour",
                    job.agent, job.trigger
                ),
            )
            .subject("agent", &job.agent)
            .detail(serde_json::json!({
                "agent": job.agent,
                "trigger": job.trigger,
                "limit": MAX_PER_HOUR,
            })),
        );
    }

    /// "<Agent>'s <trigger> run failed", linking to its thread.
    fn notify_agent_failure(
        &self,
        agent: &Agent,
        trigger: &str,
        thread: Option<&str>,
        message: &str,
    ) {
        let url = match thread {
            Some(t) => format!("/agents/{}/chat?thread={t}", agent.name),
            None => format!("/agents/{}/triggers", agent.name),
        };
        let n = NewNotification {
            title: clip(
                &format!("{}\u{2019}s {trigger} run failed", agent.label),
                state::MAX_TITLE,
            ),
            body: Some(clip(message.trim(), state::MAX_BODY)).filter(|b| !b.is_empty()),
            note_id: None,
            url: Some(url),
            source: format!("agent:{}", agent.name),
        };
        if let Err(e) = self.writer.state().add_notification(&n) {
            eprintln!(
                "agent {:?}: recording the failure notification: {e}",
                agent.name
            );
        }
    }

    /// Tell the user, once an hour, that an agent reached its hourly limit.
    fn notify_limit(&self, job: &AgentJob) {
        let label = self
            .plan()
            .agents
            .agents
            .get(&job.agent)
            .map_or_else(|| job.agent.clone(), |a| a.label.clone());
        let n = NewNotification {
            title: clip(
                &format!("{label} reached {MAX_PER_HOUR} trigger runs in an hour"),
                state::MAX_TITLE,
            ),
            body: Some(format!(
                "Further runs are skipped until the hour is up. The first skipped was its {} trigger.",
                job.trigger
            )),
            note_id: None,
            url: Some(format!("/agents/{}/triggers", job.agent)),
            source: format!("agent:{}", job.agent),
        };
        if let Err(e) = self.writer.state().add_notification(&n) {
            eprintln!(
                "agent {:?}: recording the limit notification: {e}",
                job.agent
            );
        }
    }
}

/// The writer's hooks with note event triggers added: each change is passed to the hooks it wraps
/// and then, once the write is durable, to the scheduler bound with [`TriggerBinding::bind`].
pub struct TriggerHooks {
    inner: Box<dyn Hooks>,
    scheduler: Arc<OnceLock<Weak<Scheduler>>>,
}

/// Binds a [`TriggerHooks`] to the scheduler once there is one.
#[derive(Clone)]
pub struct TriggerBinding(Arc<OnceLock<Weak<Scheduler>>>);

impl TriggerBinding {
    pub fn bind(&self, scheduler: &Arc<Scheduler>) {
        let _ = self.0.set(Arc::downgrade(scheduler));
    }
}

impl TriggerHooks {
    /// `inner` with note event triggers, and the binding that names their scheduler.
    pub fn wrap(inner: Box<dyn Hooks>) -> (Box<dyn Hooks>, TriggerBinding) {
        let slot = Arc::new(OnceLock::new());
        let hooks = TriggerHooks {
            inner,
            scheduler: slot.clone(),
        };
        (Box::new(hooks), TriggerBinding(slot))
    }

    fn scheduler(&self) -> Option<Arc<Scheduler>> {
        self.scheduler.get().and_then(Weak::upgrade)
    }
}

/// Lets go of a scheduler the hooks upgraded. The hooks run inside a write, with the writer's
/// locks held, so the scheduler must not drop here: its drop drops its host, which waits for the
/// script workers, and a worker in a run that writes waits for those locks. When this is the last
/// reference, as when its owner dropped during the write, the scheduler drops on a thread of its
/// own, which waits for the workers once the write has let go of the locks.
fn release(s: Arc<Scheduler>) {
    if let Some(last) = Arc::into_inner(s) {
        let spawned = thread::Builder::new()
            .name("scheduler-drop".into())
            .spawn(move || drop(last));
        if let Err(e) = spawned {
            eprintln!("scheduler: no thread to drop the scheduler on, so it dropped here: {e}");
        }
    }
}

impl Hooks for TriggerHooks {
    fn before(&self, change: &mut NoteChange, author: &Author) -> Result<(), String> {
        self.inner.before(change, author)?;
        if change.event == Event::Delete
            && !matches!(author, Author::Agent(_))
            && let Some(s) = self.scheduler()
        {
            s.before_delete(&change.id, &change.type_name);
            release(s);
        }
        Ok(())
    }

    fn side_effects(&self, changes: &[NoteChange]) -> writer::Result<Vec<NoteChange>> {
        self.inner.side_effects(changes)
    }

    fn after(&self, saved: &[Saved], author: &Author, schema: &Schema) {
        self.inner.after(saved, author, schema);
        if let Some(s) = self.scheduler() {
            s.heard(saved, author);
            release(s);
        }
    }

    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> Result<Option<Doc>, String> {
        self.inner.migrate(type_name, migration, id, doc)
    }

    fn asset_known(&self, id: &str) -> bool {
        self.inner.asset_known(id)
    }

    fn check_script(&self, role: ScriptRole, name: &str, source: &str) -> Result<(), String> {
        self.inner.check_script(role, name, source)
    }
}
