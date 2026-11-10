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
//! Each agent runs at most [`MAX_RUNNING`] runs at once. Up to [`MAX_QUEUED`] wait in memory.
//! Trigger overflow waits in state.db with capacity backoff, including across restarts.
//! A person-requested run can displace a trigger back to disk, never discard it.
//! At most [`MAX_PER_HOUR`] runs start per agent per hour. The durable queue caps waiting jobs
//! at 10,000 per agent with visible drops and at most one owner notification an hour.
//! Jobs expire after seven days. Run failures get three retries. Every run is recorded
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
    /// True only when no tool call was attempted. A partial turn must never be replayed.
    pub no_side_effects: bool,
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
    fn run_turn(
        &self,
        agent: &Agent,
        thread: &str,
        prompt: &str,
        scope: Option<state::Scope>,
        caller: Option<&Author>,
    ) -> TurnReport;
}

/// One run to start: an agent's trigger, the first message and the thread's title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentJob {
    pub op: Option<String>,
    /// Durable trigger queue identity. Person-requested calls keep their existing lane.
    pub durable: Option<i64>,
    pub note: Option<(String, String)>,
    /// Caller ceiling for explicitly dispatched runs. Autonomous runs use the agent scope.
    pub scope: Option<state::Scope>,
    pub caller: Option<Author>,
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
    /// Durable work displaced by a person-requested run, returned to disk after unlocking.
    displaced: Vec<AgentJob>,
    /// Persistence errors keep their exact payload in memory as well as the durable event.
    persist_retry: Vec<AgentJob>,
}

/// What became of a run asked for.
enum Admitted {
    Start(Box<AgentJob>),
    Queued,
    /// Over the hourly limit; `true` when the user is to be told.
    Skipped(bool),
    /// The queue is full and the run has no thread: it is dropped.
    Dropped,
    /// The queue is full of runs with threads and this run has one: it is refused, and the
    /// thread it came with is to be deleted.
    Refused,
}

#[cfg(test)]
mod persistence_retry_tests {
    use super::*;

    #[test]
    fn persistence_retry_buffer_is_capped_and_discards_oldest() {
        let mut plan = AgentPlan::default();
        for i in 0..=state::TRIGGER_QUEUE_CAP {
            plan.retry_persistence(AgentJob {
                scope: None, caller: None,
                op: Some("create".to_owned()), durable: None, note: Some((i.to_string(), "memo".to_owned())),
                agent: "clerk".to_owned(), trigger: "opened".to_owned(), prompt: i.to_string(),
                title: String::new(), thread: None, origin: None,
            });
        }
        assert_eq!(plan.persist_retry.len(), 10_000);
        assert_eq!(plan.persist_retry.first().unwrap().prompt, "1");
        assert_eq!(plan.persist_retry.last().unwrap().prompt, "10000");
    }
}

impl AgentPlan {
    fn retry_persistence(&mut self, job: AgentJob) {
        if self.persist_retry.len() >= state::TRIGGER_QUEUE_CAP as usize {
            let oldest = self.persist_retry.remove(0);
            eprintln!("trigger persistence retry buffer full: discarding oldest in-memory retry for agent {:?}, trigger {:?}; durable event remains unacknowledged", oldest.agent, oldest.trigger);
        }
        self.persist_retry.push(job);
    }
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
                op: None,
                durable: None,
                note: None,
                agent: agent.clone(),
                trigger: trigger.clone(),
                prompt: t.prompt.clone(),
                title: run_title(trigger, now, Some(&schedule.timezone)),
                thread: None,
                origin: None,
                scope: None,
                caller: None,
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
            .chain((!self.persist_retry.is_empty()).then_some(now + 30_000))
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
        lane.started.len() + lane.queue.len() >= MAX_PER_HOUR
    }

    /// Start `job` now, queue it behind its agent's running runs, or skip it over the hourly
    /// limit. With the queue full, a run with no thread is dropped, and a run with a thread makes
    /// room by dropping the newest run with no thread, or is refused when there is none.
    fn admit(&mut self, job: AgentJob, now: i64) -> Admitted {
        let lane = self.lanes.entry(job.agent.clone()).or_default();
        while lane.started.front().is_some_and(|t| *t <= now - HOUR_MS) {
            lane.started.pop_front();
        }
        if lane.started.len() + lane.queue.len() >= MAX_PER_HOUR {
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
            return Admitted::Start(Box::new(job));
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
                if dropped.durable.is_some() {
                    self.displaced.push(dropped);
                } else {
                    eprintln!(
                        "agent {:?}: {MAX_QUEUED} runs are waiting, so the newest run with no thread, of trigger {:?}, was dropped",
                        dropped.agent, dropped.trigger
                    );
                }
            }
        }
        lane.queue.push_back(job);
        Admitted::Queued
    }

    /// The next waiting run of `agent`, or `None`, freeing its slot, when none waits.
    fn next_queued(&mut self, agent: &str, now: i64) -> Option<AgentJob> {
        let lane = self.lanes.get_mut(agent)?;
        match lane.queue.pop_front() {
            Some(j) => {
                while lane.started.front().is_some_and(|t| *t <= now - HOUR_MS) {
                    lane.started.pop_front();
                }
                lane.started.push_back(now);
                Some(j)
            }
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
        if let Err(e) = self.writer.state().recover_trigger_jobs() {
            eprintln!("recovering trigger queue: {e}");
        }
        if let Ok(events) = self.writer.state().trigger_events() {
            for payload in events {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) else {
                    continue;
                };
                let op = match v["op"].as_str() {
                    Some("create") => "create",
                    Some("update") => "update",
                    Some("delete") => "delete",
                    _ => continue,
                };
                let deleting = v["deleting"].as_object().map(|_| Deleting {
                    title: v["deleting"]["title"].as_str().unwrap_or("").to_owned(),
                    matched: serde_json::from_value(v["deleting"]["matched"].clone())
                        .unwrap_or_default(),
                });
                self.plan().agents.heard.push(Heard {
                    op,
                    id: v["id"].as_str().unwrap_or("").to_owned(),
                    type_name: v["type"].as_str().unwrap_or("").to_owned(),
                    at: v["at"].as_i64().unwrap_or(0),
                    deleting,
                });
            }
        }
        if self.runner.set(runner).is_ok() {
            if let Err(e) = self.writer.state().end_interrupted_agent_runs() {
                eprintln!("agent runs left running: {e}");
            }
            self.reload_agents();
            if let Ok(starts) = self
                .writer
                .state()
                .recent_agent_starts(self.clock.now_ms() - HOUR_MS)
            {
                let mut plan = self.plan();
                for (agent, at) in starts {
                    plan.agents
                        .lanes
                        .entry(agent)
                        .or_default()
                        .started
                        .push_back(at);
                }
            }
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
            if matches!(author, Author::Agent(_) | Author::AgentMail(_)) {
                continue;
            }
            let op = match s.event {
                Event::Create => "create",
                Event::Update => "update",
                Event::Delete => "delete",
            };
            let payload = serde_json::json!({
                "op": op, "id": s.id, "type": s.type_name, "at": at,
                "deleting": deleting.as_ref().map(|d| serde_json::json!({"title": d.title, "matched": d.matched})),
            }).to_string();
            if let Err(e) = self.writer.state().save_trigger_event(&s.id, at, &payload) {
                eprintln!("persisting trigger event: {e}");
            }
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
        jobs.extend(std::mem::take(&mut self.plan().agents.persist_retry));
        let called = self.plan().agents.take_mentions();
        let mut tell = Vec::new();
        // The runs of calls are admitted as they are recorded, under the same lock as their room.
        let mut called_runs = Vec::new();
        for id in called {
            called_runs.extend(self.mention_jobs(&id, now, &mut tell));
        }
        let mut processed: Vec<String> = jobs
            .iter()
            .filter_map(|job| job.note.as_ref().map(|(id, _)| id.clone()))
            .collect();
        for e in events {
            processed.push(e.note.clone());
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
                op: Some(e.op.to_owned()),
                durable: None,
                note: Some((e.note.clone(), e.type_name.clone())),
                agent: e.agent.name.clone(),
                trigger: e.trigger.name.clone(),
                prompt: format!(
                    "{}\nEvent: {} of note {} (type {}, title {title:?})",
                    e.trigger.prompt, e.op, e.note, e.type_name
                ),
                title: run_title(&e.trigger.name, now, None),
                thread: None,
                origin: None,
                scope: None,
                caller: None,
            });
        }
        let mut starts: Vec<AgentJob> = called_runs
            .into_iter()
            .filter_map(|a| match a {
                Admitted::Start(j) => Some(*j),
                _ => None,
            })
            .collect();
        let mut failed_notes = Vec::new();
        for job in jobs {
            if let Err(error) = self.persist_trigger(&job, now, false) {
                eprintln!("persisting trigger: {error}");
                if let Some((id, _)) = &job.note {
                    failed_notes.push(id.clone());
                }
                self.plan().agents.retry_persistence(job);
            }
        }
        for note in processed {
            if failed_notes.contains(&note) {
                continue;
            }
            let _ = self
                .writer
                .state()
                .acknowledge_trigger_event(&note, now - DEBOUNCE_MS);
        }
        if failed_notes.is_empty() {
            let _ = self.writer.state().prune_trigger_events(now - DEBOUNCE_MS);
        }
        let state = self.writer.state();
        match state.due_trigger_jobs(now) {
            Ok(queued) => {
                for row in queued {
                    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&row.payload)
                    else {
                        let _ = state.finish_trigger_job(
                            row.id,
                            "failed",
                            Some("invalid queue payload"),
                        );
                        continue;
                    };
                    let job = AgentJob {
                        scope: None, caller: None,
                        op: payload["op"].as_str().map(str::to_owned),
                        durable: Some(row.id),
                        note: row
                            .note
                            .map(|id| (id, payload["type"].as_str().unwrap_or("").to_owned())),
                        agent: row.agent,
                        trigger: row.trigger,
                        prompt: payload["prompt"].as_str().unwrap_or("").to_owned(),
                        title: payload["title"].as_str().unwrap_or("").to_owned(),
                        thread: None,
                        origin: None,
                    };
                    let mut plan = self.plan();
                    // Never admit into a full lane: durable overflow stays on disk.
                    let blocked = plan.agents.over_limit(&job.agent, now)
                        || plan.agents.lanes.get(&job.agent).is_some_and(|l| {
                            l.running >= MAX_RUNNING && l.queue.len() >= MAX_QUEUED
                        });
                    if blocked {
                        let _ = state.retry_trigger_job(row.id, now, None);
                    } else if state.claim_trigger_job(row.id).unwrap_or(false) {
                        match plan.agents.admit(job, now) {
                            Admitted::Start(job) => starts.push(*job),
                            Admitted::Queued => {}
                            _ => {
                                let _ = state.retry_trigger_job(row.id, now, None);
                            }
                        }
                    }
                }
            }
            Err(e) => eprintln!("reading trigger queue: {e}"),
        }
        for job in tell {
            self.notify_limit(&job);
        }
        self.return_displaced_triggers();
        starts
    }

    fn return_displaced_triggers(&self) {
        let jobs = std::mem::take(&mut self.plan().agents.displaced);
        for job in jobs {
            if let Some(id) = job.durable {
                let _ = self
                    .writer
                    .state()
                    .retry_trigger_job(id, self.clock.now_ms(), None);
            }
        }
    }

    fn persist_trigger(
        &self,
        job: &AgentJob,
        due: i64,
        backfill: bool,
    ) -> Result<bool, state::Error> {
        let payload = serde_json::json!({
            "prompt": job.prompt, "title": job.title,
            "type": job.note.as_ref().map(|(_, t)| t),
            "op": job.op,
        })
        .to_string();
        let state = self.writer.state();
        let enqueue = if backfill {
            state::State::enqueue_backfill
        } else {
            state::State::enqueue_trigger
        };
        let inserted = enqueue(
            state,
            &job.agent,
            &job.trigger,
            job.note.as_ref().map(|(id, _)| id.as_str()),
            &payload,
            due,
        )?
        .is_some();
        if state
            .trigger_drop_notice(&job.agent, self.clock.now_ms())
            .unwrap_or(false)
        {
            let _ = state.add_notification(&NewNotification {
                title: format!("{} trigger queue is full", job.agent),
                body: Some("The 10,000 waiting job cap was reached. New jobs were refused. The dropped count is shown with the agent's triggers.".to_owned()),
                note_id: None,
                url: Some(format!("/agents/{}/triggers", job.agent)),
                source: format!("agent:{}", job.agent),
            });
        }
        Ok(inserted)
    }

    /// Shared selection/execution guard. TV-137 adds writer.agent_may_read here when
    /// integrating its Jev gate, so a queued note is checked again just before it runs.
    fn trigger_note_allowed(
        &self,
        id: &str,
        ty: &str,
        event: &schema::NoteEvent,
        op: &str,
    ) -> bool {
        event.matches(op, ty)
            && (!is_agent_type(ty) || !event.types.is_empty())
            && (op == "delete"
                || self
                    .note_matches(id, ty, event.condition_expr().as_ref())
                    .is_some())
    }

    /// Newest matching notes without any trigger record. Also used when confirming the
    /// preview, so edits, deletions, new records and safety decisions take effect.
    pub fn trigger_backfill_candidates(
        &self,
        name: &str,
        trigger: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let agent = self.writer.agent(name).map_err(|e| e.to_string())?;
        if !agent.enabled {
            return Err("agent is disabled".to_owned());
        }
        let t = agent.trigger(trigger).ok_or("trigger not found")?;
        let TriggerWhen::NoteEvent(e) = &t.when else {
            return Err("backfill needs a note event trigger".to_owned());
        };
        let op = if e.on.iter().any(|op| op == "create") {
            "create"
        } else if e.on.iter().any(|op| op == "update") {
            "update"
        } else {
            return Err("backfill needs create or update in on".to_owned());
        };
        let mut notes = self.writer.list(None).map_err(|e| e.to_string())?;
        notes.reverse();
        let mut found = Vec::new();
        for note in notes {
            if !self.trigger_note_allowed(&note.id, &note.type_name, e, op)
                || self
                    .writer
                    .state()
                    .trigger_record_exists(name, trigger, &note.id)
                    .map_err(|e| e.to_string())?
            {
                continue;
            }
            found.push((note.id, note.type_name));
        }
        Ok(found)
    }

    pub fn trigger_backfill(
        &self,
        name: &str,
        trigger: &str,
        notes: &[(String, String)],
    ) -> Result<usize, String> {
        let current = self.trigger_backfill_candidates(name, trigger)?;
        let agent = self.writer.agent(name).map_err(|e| e.to_string())?;
        let t = agent.trigger(trigger).ok_or("trigger not found")?;
        let mut count = 0;
        for (id, ty) in notes.iter().take(1000).filter(|n| current.contains(n)) {
            let title = self
                .writer
                .title(id)
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let job = AgentJob {
                scope: None, caller: None,
                op: Some(
                    match &t.when {
                        TriggerWhen::NoteEvent(e) if e.on.iter().any(|op| op == "create") => {
                            "create"
                        }
                        _ => "update",
                    }
                    .to_owned(),
                ),
                durable: None,
                note: Some((id.clone(), ty.clone())),
                agent: name.to_owned(),
                trigger: trigger.to_owned(),
                prompt: format!(
                    "{}\nEvent: backfill of note {id} (type {ty}, title {title:?})",
                    t.prompt
                ),
                title: run_title(trigger, self.clock.now_ms(), None),
                thread: None,
                origin: None,
            };
            count += usize::from(
                self.persist_trigger(&job, self.clock.now_ms(), true)
                    .map_err(|e| e.to_string())?,
            );
        }
        self.wake.notify_all();
        Ok(count)
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
                || !self.call_versions_trusted(id, &call.call, &call.agent)
                || !state
                    .consume_agent_call(&call.call, id, &call.agent, &version.author)
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
                op: None,
                durable: None,
                note: None,
                agent: agent.name.clone(),
                trigger: trigger.name.clone(),
                prompt,
                title: format!(
                    "{title}: {}",
                    mentions::first_chars(&request, MENTION_TITLE_CHARS)
                ),
                thread: None,
                origin: Some(MENTION_ORIGIN),
                scope: None,
                caller: None,
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

    /// A later issuer save cannot launder an earlier foreign write. Use persisted
    /// history rather than pending scans, which are coalesced and lost on restart.
    fn call_versions_trusted(&self, note: &str, call: &str, agent: &str) -> bool {
        let state = self.writer.state();
        let Ok(Some((issued_ms, principal))) = state.agent_call_issuance(call, note, agent) else {
            return false;
        };
        let Ok(versions) = state.history(note) else {
            return false;
        };
        for version in versions.into_iter().filter(|v| v.at_ms >= issued_ms) {
            let (kind, name) = version.author.parts();
            if format!("{kind}:{name}") == principal {
                continue;
            }
            let Ok(doc) = schema::Doc::parse(&version.content, note) else {
                return false;
            };
            if mentions::calls(&doc)
                .iter()
                .any(|found| found.call.call == call)
            {
                return false;
            }
        }
        true
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
        scope: state::Scope,
        caller: Author,
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
            op: None,
            durable: None,
            note: None,
            agent: a.name.clone(),
            trigger: DISPATCH_TRIGGER.to_owned(),
            prompt: prompt.to_owned(),
            title: title.to_owned(),
            thread: Some(thread.clone()),
            origin: Some(DISPATCH_ORIGIN),
            scope: Some(scope),
            caller: Some(caller),
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
                    .spawn(move || s.agent_work(*job))
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
        self.return_displaced_triggers();
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
            let next = self
                .plan()
                .agents
                .next_queued(&job.agent, self.clock.now_ms());
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
            _ => {
                if let Some(id) = job.durable {
                    let _ = self.writer.state().finish_trigger_job(
                        id,
                        "cancelled",
                        Some("agent missing or disabled"),
                    );
                }
                return;
            }
        };
        let state = self.writer.state();
        if let Some(id) = job.durable {
            match state.begin_trigger_job(id, self.clock.now_ms()) {
                Ok(true) => {}
                Ok(false) => return,
                Err(e) => {
                    eprintln!("starting durable trigger: {e}");
                    return;
                }
            }
            let matches = agent
                .trigger(&job.trigger)
                .is_some_and(|t| match (&job.note, &t.when) {
                    (Some((note, ty)), TriggerWhen::NoteEvent(e)) => job
                        .op
                        .as_deref()
                        .is_some_and(|op| self.trigger_note_allowed(note, ty, e, op)),
                    (None, TriggerWhen::Schedule(_)) => true,
                    _ => false,
                });
            if !matches {
                let _ = state.finish_trigger_job(
                    id,
                    "cancelled",
                    Some("note no longer matches trigger"),
                );
                return;
            }
        }
        let id = match state.start_agent_run(&agent.name, &job.trigger) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("agent {:?}: recording the run failed: {e}", agent.name);
                if let Some(queued) = job.durable {
                    let _ =
                        state.retry_trigger_job(queued, self.clock.now_ms(), Some(&e.to_string()));
                }
                return;
            }
        };
        let origin = format!("trigger:{}", job.trigger);
        let opened = match &job.thread {
            Some(t) => Ok(t.clone()),
            None => runner.open_thread(&agent, &origin, &job.title, &[]),
        };
        let (outcome, log, thread, no_side_effects) = match opened {
            Ok(thread) => {
                if let Err(e) = state.set_agent_run_thread(id, &thread) {
                    eprintln!("agent {:?}: recording the run's thread: {e}", agent.name);
                }
                let r =
                    runner.run_turn(&agent, &thread, &job.prompt, job.scope, job.caller.as_ref());
                (r.outcome, r.log, Some(thread), r.no_side_effects)
            }
            Err(e) => (Err(e), Vec::new(), None, true),
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
        if let Some(queued) = job.durable {
            let result = match &outcome {
                AgentRunOutcome::Succeeded => state.finish_trigger_job(queued, "succeeded", None),
                AgentRunOutcome::Failed(message) if no_side_effects => {
                    state.retry_trigger_job(queued, self.clock.now_ms(), Some(message))
                }
                AgentRunOutcome::Failed(message) => {
                    state.finish_trigger_job(queued, "failed", Some(message))
                }
                AgentRunOutcome::Running => unreachable!(),
            };
            if let Err(e) = result {
                eprintln!("finishing trigger queue job: {e}");
            }
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
            && !matches!(author, Author::Agent(_) | Author::AgentMail(_))
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
