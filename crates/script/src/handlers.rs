//! Event handlers (D9, D30) and schema-change migrations (D6): the writer's [`Hooks`], run as
//! wscript.
//!
//! A type file registers handlers with `handler "<event>"` blocks (`docs/vault-format.md`
//! "Handlers"). Every handler function takes one `Value`, the note as `{ id, type, event, version,
//! note }` where `note` is `{ fields, blocks }`, and runs as the author `handler <script>`.
//!
//! - `before_create`, `before_update` and `before_delete` run inside the write, on the writer's
//!   [`Kind::Before`] host. Returning a note body (`{ fields, blocks? }`, or a whole note) replaces
//!   the note; leaving out `blocks` keeps the note's blocks. Returning `null` or unit leaves it as
//!   it is, and returning `Err(message)` rejects the write with that message. A `before_delete`
//!   handler can only reject. A note `version` is absent on create.
//! - `after_create`, `after_update` and `after_delete` are queued in `state.db` after the commit
//!   and run one at a time on a queue thread, as [`Kind::After`] runs with the handler's
//!   allowlist and secrets. Jobs still queued when the server stops run when it starts again. A
//!   failed job stays in `state.db` with its message. Their `Value` also carries `author`, the
//!   write's author as `{ kind, name }`, and `previous`, for an update the note before it as
//!   `{ fields, blocks }`, else `null` (TV-25).
//! - A migration function takes the note in the same form and returns the note body to write, or
//!   `null` to leave it. It cannot change other notes.
//!
//! A handler's `vault::create`, `vault::update` and `vault::delete` re-enter the write path (D30).
//! In a `before_*` handler they join the write in progress through the writer's `stage_*`
//! methods. In an `after_*` handler they are writes of their own, and every handler they set off
//! carries the chain that led to it. A chain of handler runs, each set off by the one before, is
//! stopped at [`MAX_DEPTH`] runs with an error naming the chain, and naming the cycle when a note
//! and event repeat in it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use schema::{Doc, Handler, Schema};
use serde_json::{Value as Json, json};
use state::Author;
use writer::{
    Event, Hooks, Input, Migration, NoteChange, Saved, ScriptRole, Writer, doc_to_json, note_path,
};

use crate::{Ai, Host, Job, Kind, Limits, RunContext, Vault, current_job};

/// The most handler runs one chain may hold (D9, D30).
pub const MAX_DEPTH: usize = 8;

/// Workers for the `after_*` host. The queue runs one job at a time.
const AFTER_WORKERS: usize = 2;

/// How long the queue thread sleeps before looking at `state.db` again without being woken.
const POLL: Duration = Duration::from_secs(1);

/// The `after_*` handler scripts that may run longer than [`Limits::timeout`], with their wall
/// clock limit: the Outbox's send handler uploads a message of up to 25 MB to Gmail (outbox X3,
/// X4).
const LONG_HANDLERS: [(&str, Duration); 1] = [("outbox_send", Duration::from_secs(5 * 60))];

/// One handler run in a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    type_name: String,
    id: String,
    /// The event, such as `before_update`.
    event: String,
    script: String,
}

impl Link {
    fn to_json(&self) -> Json {
        json!({
            "type": self.type_name,
            "id": self.id,
            "event": self.event,
            "script": self.script,
        })
    }

    fn from_json(v: &Json) -> Option<Link> {
        let field = |n: &str| v.get(n).and_then(Json::as_str).map(str::to_owned);
        Some(Link {
            type_name: field("type")?,
            id: field("id")?,
            event: field("event")?,
            script: field("script")?,
        })
    }

    /// The same note and event, whichever script ran.
    fn same_place(&self, other: &Link) -> bool {
        self.type_name == other.type_name && self.id == other.id && self.event == other.event
    }
}

impl std::fmt::Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.type_name, self.id, self.event)
    }
}

/// The error for a chain that would pass [`MAX_DEPTH`], naming the chain and any cycle in it.
fn depth_error(chain: &[Link]) -> String {
    let path = |links: &[Link]| {
        links
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" -> ")
    };
    for (i, a) in chain.iter().enumerate() {
        if let Some(j) = chain[i + 1..].iter().position(|b| a.same_place(b)) {
            return format!(
                "the handler chain stopped at its depth limit of {MAX_DEPTH}: it runs in a cycle, \
                 {}",
                path(&chain[i..=i + 1 + j])
            );
        }
    }
    format!(
        "the handler chain stopped at its depth limit of {MAX_DEPTH}: {}",
        path(chain)
    )
}

thread_local! {
    /// The chain of the `after_*` handler whose host call is running on this thread.
    static INHERITED: RefCell<Vec<Link>> = const { RefCell::new(Vec::new()) };
}

fn inherited() -> Vec<Link> {
    INHERITED.with(|c| c.borrow().clone())
}

/// Run `f` with `chain` as the chain every handler it sets off continues.
fn with_chain<T>(chain: Vec<Link>, f: impl FnOnce() -> T) -> T {
    let previous = INHERITED.with(|c| c.replace(chain));
    let out = f();
    INHERITED.with(|c| *c.borrow_mut() = previous);
    out
}

/// The writer the hosts' `vault` module reaches, set once the writer exists.
type Binding = Arc<OnceLock<Weak<Writer>>>;

fn bound(binding: &Binding) -> Result<Arc<Writer>, String> {
    binding
        .get()
        .and_then(Weak::upgrade)
        .ok_or_else(|| "the vault is not open".to_owned())
}

/// The handlers and migrations of one vault.
///
/// Make one, pass [`Handlers::hooks`] to [`Writer::with_hooks`], then call [`Handlers::start`]
/// with the writer.
pub struct Handlers {
    shared: Arc<Shared>,
}

struct Shared {
    binding: Binding,
    /// Runs `before_*` handlers and migrations. A chain nests one run inside another, so it has a
    /// worker for every run a chain can hold.
    before: Host,
    after: Host,
    /// The chain of `before_*` runs in the write in progress, `None` between writes. Writes run
    /// one at a time, so there is one.
    active: Mutex<Option<Vec<Link>>>,
    /// Live provenance is not part of the script-controlled payload.
    contexts: Mutex<HashMap<i64, RunContext>>,
    /// Set when jobs were queued, to wake the queue thread.
    wake: (Mutex<bool>, Condvar),
}

impl Handlers {
    /// The handlers of the home server, whose `after_*` handlers reach `ai` with the vault's
    /// credentials (ai D13). Make it outside an async runtime: its HTTP clients block.
    pub fn new(limits: Limits) -> Handlers {
        Handlers::with_ai(limits, true)
    }

    /// The handlers of a device's embedded server, where every `ai` call returns `Err` with kind
    /// `unavailable` (ai D13).
    pub fn on_device(limits: Limits) -> Handlers {
        Handlers::with_ai(limits, false)
    }

    fn with_ai(limits: Limits, ai: bool) -> Handlers {
        let binding: Binding = Arc::new(OnceLock::new());
        let vault = Arc::new(HandlerVault {
            binding: binding.clone(),
        });
        let after_ai = if ai {
            Ai::bound(binding.clone())
        } else {
            Ai::unavailable()
        };
        Handlers {
            shared: Arc::new(Shared {
                // `before_*` handlers and migrations have no `ai` module.
                before: Host::new(
                    vault.clone(),
                    MAX_DEPTH + 2,
                    limits.clone(),
                    Ai::unavailable(),
                ),
                after: Host::new(vault, AFTER_WORKERS, limits, after_ai),
                binding,
                active: Mutex::new(None),
                contexts: Mutex::new(HashMap::new()),
                wake: (Mutex::new(false), Condvar::new()),
            }),
        }
    }

    /// The writer's hooks.
    pub fn hooks(&self) -> Box<dyn Hooks> {
        Box::new(HandlerHooks(self.shared.clone()))
    }

    /// Bind the handlers to `writer` without running the `after_*` queue: jobs are queued in
    /// `state.db` and wait for a [`Handlers::start`].
    pub fn bind(&self, writer: &Arc<Writer>) {
        let _ = self.shared.binding.set(Arc::downgrade(writer));
    }

    /// Bind the handlers to `writer` and start running queued `after_*` jobs, those left from a
    /// previous run first.
    pub fn start(&self, writer: &Arc<Writer>) {
        self.bind(writer);
        let shared = self.shared.clone();
        thread::Builder::new()
            .name("handler-queue".into())
            .spawn(move || shared.run_queue())
            .expect("start the handler queue");
    }
}

/// The activity entry of a handler run that failed (activity Y2): the script, the event and the
/// note, and the first line of the error.
fn record_failure(
    writer: &Writer,
    script: &str,
    event: &str,
    type_name: &str,
    id: &str,
    message: &str,
) {
    let first = message.lines().next().unwrap_or("").trim();
    let mut e = state::NewActivity::new(
        "handler.failed",
        "automation",
        &Author::Handler(script.to_owned()),
        format!("Handler {script} ({event} of {type_name}) failed: {first}"),
    )
    .detail(json!({
        "script": script,
        "event": event,
        "type": type_name,
        "note": id,
        "error": state::one_line(message, 300),
    }));
    if !id.is_empty() {
        e = e.subject("note", id);
    }
    writer.state().record_activity(e);
}

impl Shared {
    fn writer(&self) -> Result<Arc<Writer>, String> {
        bound(&self.binding)
    }

    fn active(&self) -> MutexGuard<'_, Option<Vec<Link>>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Add `link` to the chain of the write in progress, or refuse it past the depth limit.
    fn enter(&self, link: Link) -> Result<ChainGuard<'_>, String> {
        let mut active = self.active();
        let top = active.is_none();
        let chain = active.get_or_insert_with(inherited);
        chain.push(link);
        if chain.len() > MAX_DEPTH {
            let message = depth_error(chain);
            chain.pop();
            if top {
                *active = None;
            }
            return Err(message);
        }
        Ok(ChainGuard { shared: self, top })
    }

    /// Run the `before_*` handlers of `change`, in file order.
    /// Before [`Handlers::start`], as during the start-up upgrade steps, no handler runs, as
    /// [`Shared::after`] queues none.
    fn before(&self, change: &mut NoteChange) -> Result<(), String> {
        let Ok(writer) = self.writer() else {
            return Ok(());
        };
        let event = format!("before_{}", event_name(change.event));
        let handlers =
            writer.read_index(|_, schema| handlers_of(schema, &change.type_name, &event));
        for h in handlers {
            let _link = self.enter(Link {
                type_name: change.type_name.clone(),
                id: change.id.clone(),
                event: event.clone(),
                script: h.script.clone(),
            })?;
            let doc = match &change.doc {
                Some(doc) => doc.clone(),
                None => writer.read(&change.id).map_err(|e| e.to_string())?.doc,
            };
            let version = match change.event {
                Event::Create => None,
                _ => writer
                    .current_version(&change.id)
                    .map_err(|e| e.to_string())?,
            };
            let arg = note_json(change, &event, version.as_deref(), &doc);
            let source = script_source(&writer, &h.script)?;
            let mut job = Job::new(Kind::Before, &h.script, &source, &h.function);
            if writer.incoming_agent_note(&change.type_name, &doc) {
                job.run_context().observe_outside_text();
            }
            job.args = vec![arg];
            let returned = self.before.run(job).result.map_err(|e| {
                let message = e.to_string();
                record_failure(
                    &writer,
                    &h.script,
                    &event,
                    &change.type_name,
                    &change.id,
                    &message,
                );
                message
            })?;
            if change.event != Event::Delete
                && let Some(new) = returned_doc(returned, &doc, &change.type_name, &change.id)?
            {
                change.doc = Some(new);
            }
        }
        Ok(())
    }

    /// Queue the `after_*` handlers of every saved note, written by `author`. A note whose change
    /// had its own author, such as one a `before_*` handler staged, gives its handlers that one.
    fn after(&self, saved: &[Saved], author: &Author, schema: &Schema) {
        let Ok(writer) = self.writer() else {
            return;
        };
        let chain = inherited();
        let mut queue = Vec::new();
        for s in saved {
            let by = s.author.as_ref().unwrap_or(author);
            let (kind, name) = by.parts();
            let event = format!("after_{}", event_name(s.event));
            for h in handlers_of(schema, &s.type_name, &event) {
                let mut links = chain.clone();
                links.push(Link {
                    type_name: s.type_name.clone(),
                    id: s.id.clone(),
                    event: event.clone(),
                    script: h.script.clone(),
                });
                let payload = json!({
                    "type": s.type_name,
                    "id": s.id,
                    "event": event,
                    "version": s.version,
                    "wcl": s.wcl,
                    "previous": s.previous,
                    "restored": s.restored,
                    "author": { "kind": kind, "name": name, "gmail_actions": matches!(by, Author::AgentMail(_)) },
                    "script": h.script,
                    "function": h.function,
                    "allow_domains": h.allow_domains,
                    "secrets": h.secrets,
                    "chain": links.iter().map(Link::to_json).collect::<Vec<_>>(),
                })
                .to_string();
                if links.len() > MAX_DEPTH {
                    let message = depth_error(&links);
                    eprintln!("handler not run: {message}");
                    record_failure(&writer, &h.script, &event, &s.type_name, &s.id, &message);
                    if let Err(e) = writer.state().record_failed_job(&payload, &message) {
                        eprintln!("recording a failed handler job: {e}");
                    }
                } else {
                    queue.push(payload);
                }
            }
        }
        if queue.is_empty() {
            return;
        }
        {
            // Publish the durable rows and live contexts atomically relative to
            // the queue worker's lookup. No context lock is held during work.
            let mut contexts = self.contexts.lock().unwrap_or_else(|e| e.into_inner());
            let parent = RunContext::current().unwrap_or_default();
            for payload in &queue {
                let context = parent.child();
                let queued = writer.publish_run_context(&context, |state, run_id| {
                    state.queue_jobs_with_provenance(std::slice::from_ref(payload), run_id)
                });
                match queued {
                    Ok(ids) => {
                        for id in ids {
                            contexts.insert(id, context.clone());
                        }
                    }
                    Err(e) => {
                        eprintln!("queueing after_* handlers: {e}");
                        let _ = writer
                            .state()
                            .record_failed_job(payload, "handler provenance publication failed");
                        writer.state().record_activity(state::NewActivity::server(
                            "handler.queue_failed",
                            "automation",
                            "Handler provenance publication failed; handler was not executed",
                        ));
                    }
                }
            }
        }
        let (flag, cv) = &self.wake;
        *flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_one();
    }

    /// The queue thread: run queued jobs, oldest first, until the writer is gone.
    fn run_queue(&self) {
        if let Ok(writer) = self.writer() {
            self.recover_sends(&writer, true);
            self.recover_deletes(&writer);
        }
        loop {
            let Ok(writer) = self.writer() else {
                return;
            };
            match writer.state().queued_jobs() {
                Ok(jobs) => {
                    for job in jobs {
                        let context = match self.handler_context(&writer, job.id) {
                            Ok(context) => context,
                            Err(error) => {
                                eprintln!("recovering handler provenance: {error}");
                                let held = writer.state().handler_recovery_subject(job.id).and_then(|subject| {
                                    writer.state().record_unavailable_run_for("", &subject)?;
                                    writer.state().finish_job(job.id, Err("held: provenance unavailable; release by editing the note as owner".into()), "")
                                });
                                if let Err(error) = held {
                                    eprintln!("holding handler job: {error}");
                                }
                                continue;
                            }
                        };
                        context.scope_recovered(|| {
                            let payload: Json =
                                serde_json::from_str(&job.payload).unwrap_or(Json::Null);
                            if payload["type"] == "outbox"
                                && writer
                                    .read(payload["id"].as_str().unwrap_or_default())
                                    .is_ok()
                            {
                                let id = payload["id"].as_str().unwrap_or_default();
                                // An old draft job must not merge its restriction
                                // into a new owner-authorized attempt for the item.
                                if context.is_restricted() {
                                    writer.state().record_activity(state::NewActivity::new(
                                        "run.outside_text_refused", "automation",
                                        &Author::Handler("outbox_send".into()),
                                        script_core::OUTSIDE_TEXT_REFUSAL,
                                    ));
                                    if let Err(error) = writer.state().finish_job(
                                        job.id, Err(script_core::OUTSIDE_TEXT_REFUSAL.into()), "",
                                    ) {
                                        eprintln!("holding restricted Outbox job: {error}");
                                    }
                                    return;
                                }
                                // A pre-Stop job must not contaminate a newer
                                // owner-authorized item by joining its old admission.
                                if context
                                    .stop_generation()
                                    .is_none_or(|g| writer.state().automation_commit(g).is_err())
                                {
                                    if let Ok(subject) =
                                        writer.state().handler_recovery_subject(job.id)
                                    {
                                        let held = writer.state().record_stale_run_for(id, &subject).and_then(|_| {
                                            writer.state().finish_job(job.id, Err("held: stale admission; release by editing the note as owner".into()), "")
                                        });
                                        if let Err(error) = held {
                                            eprintln!("holding stale handler job: {error}");
                                        }
                                    }
                                    return;
                                }
                                match writer.outbox_run_context(id) {
                                    Ok(item) => context.merge(&item),
                                    Err(error) => {
                                        let held = writer.state().handler_recovery_subject(job.id).and_then(|subject| {
                                            writer.state().record_unavailable_run_for(id, &subject)?;
                                            writer.state().finish_job(job.id, Err(format!("held: Outbox {id} provenance unavailable; release by editing the note as owner")), "")
                                        });
                                        if let Err(hold_error) = held {
                                            eprintln!("holding queued Outbox job: {error}; {hold_error}");
                                        }
                                        return;
                                    }
                                }
                            }
                            if payload["type"] == "outbox" {
                                let id = payload["id"].as_str().unwrap_or_default();
                                match writer.state().outbox_deadline(id) {
                                    Ok(Some(at)) if writer.now_ms() < at => return,
                                    Err(e) => {
                                        eprintln!("outbox deadline: {e}");
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            if payload["type"] == "outbox"
                                && payload["script"] == "outbox_send"
                                && let Some(id) = payload["id"].as_str()
                                && let Ok(note) = writer.read(id)
                                && doc_to_json(&note.doc)["fields"]["status"] == "ready"
                            {
                                self.recover_sends(&writer, false);
                                self.recover_deletes(&writer);
                            }
                            let (outcome, log) = self.run_job(&writer, &job.payload);
                            if let Err(message) = &outcome {
                                // A deadline faults the script before it can record its normal
                                // Outbox error. Preserve the delete target and expose manual retry.
                                if payload["type"] == "outbox"
                                    && payload["script"] == "outbox_send"
                                    && let Some(id) = payload["id"].as_str()
                                    && let Ok(note) = writer.read(id)
                                {
                                    let body = doc_to_json(&note.doc);
                                    let fields = &body["fields"];
                                    if fields["kind"] == "event"
                                        && fields["event_action"] == "delete"
                                        && fields["status"] == "sending"
                                        && fields["sent_at"].as_str().unwrap_or_default().is_empty()
                                    {
                                        let mut body = body;
                                        body["fields"]["status"] = json!("failed");
                                        body["fields"]["error"] = json!(message);
                                        if let Err(e) = writer.update(
                                            id,
                                            &Input::Json(body),
                                            Some(&note.version),
                                            &Author::Handler("outbox_send".into()),
                                        ) {
                                            eprintln!("recording failed Outbox deletion: {e}");
                                        }
                                    }
                                }
                                eprintln!("after_* handler job {} failed: {message}", job.id);
                                let p: Json =
                                    serde_json::from_str(&job.payload).unwrap_or(Json::Null);
                                let s = |k: &str| p[k].as_str().unwrap_or_default().to_owned();
                                record_failure(
                                    &writer,
                                    &s("script"),
                                    &s("event"),
                                    &s("type"),
                                    &s("id"),
                                    message,
                                );
                            }
                            if let Err(e) = writer.state().finish_job(job.id, outcome, &log) {
                                eprintln!("recording handler job {}: {e}", job.id);
                            } else {
                                self.contexts
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&job.id);
                            }
                        });
                        writer.prune_run_contexts();
                    }
                }
                Err(e) => eprintln!("reading the handler queue: {e}"),
            }
            drop(writer);
            let (flag, cv) = &self.wake;
            let mut woken = flag.lock().unwrap_or_else(|e| e.into_inner());
            if !*woken {
                woken = cv
                    .wait_timeout(woken, POLL)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            *woken = false;
        }
    }

    fn handler_context(&self, writer: &Writer, id: i64) -> state::Result<RunContext> {
        let mut contexts = self.contexts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(context) = contexts.get(&id) {
            return Ok(context.clone());
        }
        let run_id = writer.state().handler_provenance(id)?.unwrap_or_default();
        let subject = writer.state().handler_recovery_subject(id)?;
        let context = writer.recover_run_context_for(&run_id, &subject)?;
        contexts.insert(id, context.clone());
        Ok(context)
    }

    // The queue is serial, but SMS receipts and event write-back jobs are asynchronous.
    // Preserve those live waits before a claim, not across a server restart.
    fn recover_sends(&self, writer: &Writer, startup: bool) {
        let Ok(notes) = writer.list_with(Some("outbox"), writer::Archived::Include) else {
            return;
        };
        let handlers = writer.read_index(|_, schema| handlers_of(schema, "outbox", "after_update"));
        let Some(handler) = handlers.iter().find(|h| h.script == "outbox_send") else {
            return;
        };
        for summary in notes {
            let Ok(note) = writer.read(&summary.id) else {
                continue;
            };
            let body = doc_to_json(&note.doc);
            let fields = &body["fields"];
            let text = |k: &str| fields[k].as_str().unwrap_or_default();
            if text("status") != "sending"
                || !matches!(text("kind"), "email" | "sms" | "event")
                || (text("kind") == "event" && text("event_action") == "delete")
            {
                continue;
            }
            let context = match writer.outbox_run_context(&note.id) {
                Ok(context) => context,
                Err(error) => {
                    eprintln!("recovering Outbox send provenance: {error}");
                    continue;
                }
            };
            context.scope_recovered(|| {
                if text("kind") == "sms" && text("sent_at").is_empty() {
                    // The handler may auto-select the phone without storing it in `device`.
                    let command = writer.state().list_devices().ok().and_then(|devices| {
                        devices.iter().find_map(|device| {
                            writer
                                .state()
                                .device_command(&device.id, text("sent_ref"))
                                .ok()
                        })
                    });
                    if let Some(command) = command {
                        match command.status {
                            state::CommandStatus::Done | state::CommandStatus::Failed => {
                                if let Err(e) = writer.finish_outbox_sms(&command) {
                                    eprintln!("recovering SMS receipt: {e}");
                                }
                                return;
                            }
                            state::CommandStatus::Pending | state::CommandStatus::Delivered
                                if !startup =>
                            {
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                if !startup && text("kind") == "event" && text("sent_at").is_empty() {
                    // Failure to read the queue is not evidence that a write-back was abandoned.
                    let Ok(jobs) = writer.state().queued_jobs() else {
                        return;
                    };
                    if jobs.iter().any(|job| {
                        let p: Json = serde_json::from_str(&job.payload).unwrap_or(Json::Null);
                        p["script"] == "google_event"
                            && (p["id"] == fields["event"] || p["id"] == fields["result"])
                    }) {
                        return;
                    }
                }
                let payload = json!({
                    "type": "outbox", "id": note.id, "version": note.version,
                    "wcl": note.wcl, "event": "after_update", "script": "outbox_send",
                    "function": "reconcile_send", "allow_domains": handler.allow_domains,
                    "secrets": handler.secrets,
                })
                .to_string();
                let (result, _) = self.run_job(writer, &payload);
                if let Err(e) = result {
                    eprintln!("recovering interrupted Outbox send: {e}");
                }
            });
        }
    }

    // Runs before replay and before a new claim, on the single queue thread. Recovery
    // reads Google but never repeats DELETE without a user's ready transition.
    fn recover_deletes(&self, writer: &Writer) {
        let Ok(notes) = writer.list_with(Some("outbox"), writer::Archived::Include) else {
            return;
        };
        let handlers = writer.read_index(|_, schema| handlers_of(schema, "outbox", "after_update"));
        let Some(handler) = handlers.iter().find(|h| h.script == "outbox_send") else {
            return;
        };
        for summary in notes {
            let Ok(note) = writer.read(&summary.id) else {
                continue;
            };
            let body = doc_to_json(&note.doc);
            let fields = &body["fields"];
            if fields["status"] != "sending"
                || fields["kind"] != "event"
                || fields["event_action"] != "delete"
            {
                continue;
            }
            let context = match writer.outbox_run_context(&note.id) {
                Ok(context) => context,
                Err(error) => {
                    eprintln!("recovering Outbox delete provenance: {error}");
                    continue;
                }
            };
            context.scope_recovered(|| {
                let payload = json!({
                    "type": "outbox", "id": note.id, "version": note.version,
                    "wcl": note.wcl, "event": "after_update", "script": "outbox_send",
                    "function": "reconcile_delete", "allow_domains": handler.allow_domains,
                    "secrets": handler.secrets,
                })
                .to_string();
                let (result, _) = self.run_job(writer, &payload);
                if let Err(error) = result {
                    // A host deadline bypasses script Result handling.
                    if let Ok(current) = writer.read(&note.id) {
                        let mut body = doc_to_json(&current.doc);
                        if body["fields"]["status"] == "sending" {
                            body["fields"]["status"] = json!("failed");
                            body["fields"]["error"] =
                                json!(format!("The delete was interrupted; retry. {error}"));
                            if let Err(e) = writer.update(
                                &note.id,
                                &Input::Json(body),
                                Some(&current.version),
                                &Author::Handler("outbox_send".into()),
                            ) {
                                eprintln!("recording interrupted Outbox deletion: {e}");
                            }
                        }
                    }
                }
            });
        }
    }

    /// Run one queued `after_*` job.
    fn run_job(&self, writer: &Writer, payload: &str) -> (Result<(), String>, String) {
        let job = (|| {
            let p: Json = serde_json::from_str(payload).map_err(|e| e.to_string())?;
            // A crash can happen after creating the event but before linking `result` on
            // its Outbox item. The durable chain still identifies that interrupted send.
            if p["script"] == "google_event"
                && p["chain"].as_array().is_some_and(|chain| {
                    chain.iter().any(|link| {
                        link["type"] == "outbox"
                            && link["id"]
                                .as_str()
                                .and_then(|id| writer.read(id).ok())
                                .is_some_and(|note| {
                                    let body = doc_to_json(&note.doc);
                                    body["fields"]["status"] == "failed"
                                        && body["fields"]["interrupted_at"]
                                            .as_str()
                                            .is_some_and(|s| !s.is_empty())
                                })
                    })
                })
            {
                return Ok(None);
            }
            let text = |n: &str| {
                p.get(n)
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let list = |n: &str| -> Vec<String> {
                p.get(n)
                    .and_then(Json::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Json::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let (type_name, id, event) = (text("type"), text("id"), text("event"));
            let doc = Input::Wcl(text("wcl"))
                .parse(&note_path(&type_name, &id))
                .map_err(|e| e.to_string())?;
            writer.expose_run_note(&type_name, &doc);
            let previous = match p.get("previous").and_then(Json::as_str) {
                Some(wcl) => {
                    let old = Input::Wcl(wcl.to_owned())
                        .parse(&note_path(&type_name, &id))
                        .map_err(|e| e.to_string())?;
                    writer.expose_run_note(&type_name, &old);
                    doc_to_json(&old)
                }
                None => Json::Null,
            };
            let script = text("script");
            let source = script_source(writer, &script)?;
            let target_trashed = if type_name == "outbox" && script == "outbox_send" {
                writer.read(&id).ok().is_some_and(|current| {
                    let body = doc_to_json(&current.doc);
                    let fields = &body["fields"];
                    let target = ["sent_ref", "event", "result"]
                        .iter()
                        .filter_map(|key| fields[*key].as_str())
                        .find(|id| !id.is_empty());
                    target.is_some_and(|id| writer.state().trash_entry(id).ok().flatten().is_some())
                })
            } else {
                false
            };
            let mut job = Job::new(Kind::After, &script, &source, &text("function"));
            // Bind privilege to the executing source, never to a user-claimable name.
            if type_name == "outbox"
                && event == "after_update"
                && script == "google_gmail"
                && text("function") == "copy_sent"
                && source == include_str!("../../writer/defaults/google/google_gmail.ws")
            {
                let body = doc_to_json(&doc);
                let fields = &body["fields"];
                if fields["kind"] == "email"
                    && fields["status"] == "sending"
                    && let Some(id) = fields["sent_ref"].as_str().filter(|id| !id.is_empty())
                {
                    job.author = Some(Author::GmailSentCopy(id.to_owned()));
                }
            }
            job.args = vec![json!({
                "id": id,
                "type": type_name,
                "event": event,
                "version": text("version"),
                "note": doc_to_json(&doc),
                "previous": previous,
                "restored": p.get("restored").cloned().unwrap_or(Json::Bool(false)),
                "author": p.get("author").cloned().unwrap_or(Json::Null),
                "target_trashed": target_trashed,
            })];
            job.allow_domains = list("allow_domains");
            job.secrets = list("secrets");
            job.timeout = LONG_HANDLERS
                .iter()
                .find(|(name, _)| *name == script)
                .map(|(_, limit)| *limit);
            job.context = json!({ "chain": p.get("chain").cloned().unwrap_or_default() });
            Ok::<_, String>(Some(job))
        })();
        match job {
            Ok(Some(job)) => {
                let out = self.after.run(job);
                (out.result.map(|_| ()).map_err(|e| e.to_string()), out.log)
            }
            Ok(None) => (Ok(()), String::new()),
            Err(message) => (Err(message), String::new()),
        }
    }

    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> Result<Option<Doc>, String> {
        let writer = self.writer()?;
        if !schema::is_name(&migration.script) {
            return Err(format!("{:?} is not a valid script name", migration.script));
        }
        let source = script_source(&writer, &migration.script)?;
        let mut job = Job::new(
            Kind::Migration,
            &migration.script,
            &source,
            &migration.function,
        );
        job.args = vec![json!({
            "id": id,
            "type": type_name,
            "event": "migrate",
            "version": writer.current_version(id).map_err(|e| e.to_string())?,
            "note": doc_to_json(doc),
        })];
        let returned = self.before.run(job).result.map_err(|e| e.to_string())?;
        returned_doc(returned, doc, type_name, id)
    }
}

/// Takes a link back off the chain of the write in progress when its handler ends.
struct ChainGuard<'a> {
    shared: &'a Shared,
    top: bool,
}

impl Drop for ChainGuard<'_> {
    fn drop(&mut self) {
        let mut active = self.shared.active();
        if self.top {
            *active = None;
        } else if let Some(chain) = active.as_mut() {
            chain.pop();
        }
    }
}

struct HandlerHooks(Arc<Shared>);

impl Hooks for HandlerHooks {
    fn before(&self, change: &mut NoteChange, _author: &Author) -> Result<(), String> {
        self.0.before(change)
    }

    fn after(&self, saved: &[Saved], author: &Author, schema: &Schema) {
        self.0.after(saved, author, schema);
    }

    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> Result<Option<Doc>, String> {
        self.0.migrate(type_name, migration, id, doc)
    }

    fn check_script(&self, role: ScriptRole, name: &str, source: &str) -> Result<(), String> {
        let kind = match role {
            ScriptRole::Before => Kind::Before,
            ScriptRole::After => Kind::After,
            ScriptRole::Task => Kind::Task,
            ScriptRole::Device => Kind::Device,
        };
        self.0
            .after
            .check(kind, name, source)
            .map_err(|e| e.to_string())
    }
}

fn event_name(e: Event) -> &'static str {
    match e {
        Event::Create => "create",
        Event::Update => "update",
        Event::Delete => "delete",
    }
}

fn handlers_of(schema: &Schema, type_name: &str, event: &str) -> Vec<Handler> {
    schema
        .type_def(type_name)
        .map(|t| {
            t.handlers
                .iter()
                .filter(|h| h.event == event)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn script_source(writer: &Writer, script: &str) -> Result<String, String> {
    writer
        .vault()
        .read_text(&format!("scripts/{script}.ws"))
        .map_err(|_| format!("there is no script {script:?} in scripts/"))
}

fn note_json(change: &NoteChange, event: &str, version: Option<&str>, doc: &Doc) -> Json {
    json!({
        "id": change.id,
        "type": change.type_name,
        "event": event,
        "version": version,
        "note": doc_to_json(doc),
    })
}

/// The note a handler or migration returned, or `None` when it returned `null`. A whole note is
/// taken for its body, and a body without `blocks` keeps the blocks of `doc`.
fn returned_doc(
    returned: Json,
    doc: &Doc,
    type_name: &str,
    id: &str,
) -> Result<Option<Doc>, String> {
    let mut body = match returned {
        Json::Null => return Ok(None),
        Json::Object(mut o) if !o.contains_key("fields") && o.contains_key("note") => {
            o.remove("note").unwrap_or_default()
        }
        other => other,
    };
    if let Json::Object(o) = &mut body
        && !o.contains_key("blocks")
    {
        o.insert("blocks".into(), doc_to_json(doc)["blocks"].clone());
    }
    Input::Json(body)
        .parse(&note_path(type_name, id))
        .map(Some)
        .map_err(|e| format!("the script returned something that is not a note: {e}"))
}

/// The vault as the handler hosts' scripts see it. A note change from a `before_*` handler joins
/// the write in progress; one from a migration is refused; any other is a write of its own that
/// continues the chain of the job making it.
struct HandlerVault {
    binding: Binding,
}

/// What a script receives for a change that joined the write in progress. It has no version
/// until the write commits.
fn staged(c: NoteChange) -> Json {
    json!({
        "id": c.id,
        "type": c.type_name,
        "version": null,
        "note": c.doc.as_ref().map(doc_to_json),
    })
}

enum Route {
    Stage,
    Refuse,
    Write(Vec<Link>),
}

fn route() -> Route {
    match current_job(|j| (j.kind, j.context.clone())) {
        Some((Kind::Before, _)) => Route::Stage,
        Some((Kind::Migration, _)) => Route::Refuse,
        Some((_, context)) => Route::Write(
            context
                .get("chain")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(Link::from_json).collect())
                .unwrap_or_default(),
        ),
        None => Route::Write(Vec::new()),
    }
}

const MIGRATION_WRITE: &str =
    "a migration changes the note it is given by returning it; it cannot change other notes";

impl Vault for HandlerVault {
    fn get(&self, id: &str) -> Result<Json, String> {
        Vault::get(&*bound(&self.binding)?, id)
    }

    fn query(&self, text: &str, max_rows: usize) -> Result<Vec<Json>, String> {
        Vault::query(&*bound(&self.binding)?, text, max_rows)
    }

    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String> {
        let w = bound(&self.binding)?;
        match route() {
            Route::Stage => w
                .stage_create(type_name, &Input::Json(note), author)
                .map(staged)
                .map_err(|e| e.to_string()),
            Route::Refuse => Err(MIGRATION_WRITE.into()),
            Route::Write(chain) => {
                with_chain(chain, || Vault::create(&*w, type_name, note, author))
            }
        }
    }

    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String> {
        let w = bound(&self.binding)?;
        match route() {
            Route::Stage => w
                .stage_update(id, &Input::Json(note), Some(version), author)
                .map(staged)
                .map_err(|e| e.to_string()),
            Route::Refuse => Err(MIGRATION_WRITE.into()),
            Route::Write(chain) => {
                with_chain(chain, || Vault::update(&*w, id, note, version, author))
            }
        }
    }

    fn delete(&self, id: &str, version: &str, author: &Author) -> Result<Json, String> {
        let w = bound(&self.binding)?;
        match route() {
            Route::Stage => w
                .stage_delete(id, Some(version), author)
                .map(staged)
                .map_err(|e| e.to_string()),
            Route::Refuse => Err(MIGRATION_WRITE.into()),
            Route::Write(chain) => with_chain(chain, || Vault::delete(&*w, id, version, author)),
        }
    }

    fn query_page(&self, text: &str, size: usize, cursor: Option<String>) -> Result<Json, String> {
        Vault::query_page(&*bound(&self.binding)?, text, size, cursor)
    }

    fn search(
        &self,
        text: &str,
        limit: usize,
        archived: writer::Archived,
    ) -> Result<Vec<Json>, String> {
        Vault::search(&*bound(&self.binding)?, text, limit, archived)
    }

    fn set_archived(&self, id: &str, archived: bool, author: &Author) -> Result<Json, String> {
        let w = bound(&self.binding)?;
        match route() {
            Route::Stage => {
                let version = w
                    .current_version(id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("note {id} does not exist"))?;
                w.stage_set_archived(id, archived, Some(&version), author)
                    .map(staged)
                    .map_err(|e| e.to_string())
            }
            Route::Refuse => Err(MIGRATION_WRITE.into()),
            Route::Write(chain) => {
                with_chain(chain, || Vault::set_archived(&*w, id, archived, author))
            }
        }
    }

    fn restore(&self, id: &str, author: &Author) -> Result<Json, String> {
        self.side_write("vault::restore", |w| Vault::restore(w, id, author))
    }

    fn links(&self, id: &str) -> Result<Vec<Json>, String> {
        Vault::links(&*bound(&self.binding)?, id)
    }

    fn backlinks(&self, id: &str) -> Result<Vec<Json>, String> {
        Vault::backlinks(&*bound(&self.binding)?, id)
    }

    fn assets(&self) -> Result<Vec<Json>, String> {
        Vault::assets(&*bound(&self.binding)?)
    }

    fn asset(&self, id: &str) -> Result<(Json, Vec<u8>), String> {
        Vault::asset(&*bound(&self.binding)?, id)
    }

    fn upload_asset(&self, filename: &str, bytes: Vec<u8>) -> Result<Json, String> {
        self.side_write("vault::upload_asset", |w| {
            Vault::upload_asset(w, filename, bytes)
        })
    }

    fn types(&self) -> Result<Vec<Json>, String> {
        Vault::types(&*bound(&self.binding)?)
    }

    fn taxonomies(&self) -> Result<Vec<Json>, String> {
        Vault::taxonomies(&*bound(&self.binding)?)
    }

    fn add_term(&self, taxonomy: &str, path: &str, author: &Author) -> Result<Json, String> {
        self.side_write("vault::add_term", |w| {
            Vault::add_term(w, taxonomy, path, author)
        })
    }

    fn daily(&self, date: &str, author: &Author) -> Result<Json, String> {
        self.side_write("vault::daily", |w| Vault::daily(w, date, author))
    }

    fn secret(&self, name: &str) -> Result<Option<String>, String> {
        Vault::secret(&*bound(&self.binding)?, name)
    }

    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        // Not a vault change, but nothing may outlast a write a dry run or a migration takes.
        self.side_write("secrets::set", |w| Vault::set_secret(w, name, value))
    }

    fn notify(&self, source: &str, title: &str, body: &str, note: &str) -> Result<Json, String> {
        // Inside the write path, which a dry run also takes, nothing may outlast the write.
        self.side_write("vault::notify", |w| {
            Vault::notify(w, source, title, body, note)
        })
    }

    fn device_command(
        &self,
        source: &str,
        device: &str,
        name: &str,
        payload: Json,
    ) -> Result<Json, String> {
        self.side_write("vault::device_command", |w| {
            Vault::device_command(w, source, device, name, payload)
        })
    }

    fn protected(&self) -> Vec<std::path::PathBuf> {
        bound(&self.binding)
            .map(|w| Vault::protected(&*w))
            .unwrap_or_default()
    }

    fn now_ms(&self) -> i64 {
        bound(&self.binding).map_or(0, |w| w.now_ms())
    }
}

impl HandlerVault {
    /// A write that is not a note change a `before_*` handler can stage: refused inside the write
    /// path and in a migration, a write of its own continuing the chain otherwise.
    fn side_write<T>(
        &self,
        what: &str,
        f: impl FnOnce(&Writer) -> Result<T, String>,
    ) -> Result<T, String> {
        let w = bound(&self.binding)?;
        match route() {
            Route::Stage => Err(format!(
                "{what} cannot run in a before_* handler; use an after_* handler"
            )),
            Route::Refuse => Err(MIGRATION_WRITE.into()),
            Route::Write(chain) => with_chain(chain, || f(&w)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopened_handler_queue_retains_staleness_and_unreadable_work() {
        for unreadable in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let vault = store::Vault::init(dir.path()).unwrap();
            let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
            let writer = Writer::open(vault, state).unwrap();
            let context = RunContext::default();
            context.restrict();
            let run_id = context.persist(writer.state()).unwrap();
            let generation = context.stop_generation().unwrap();
            let reference = if unreadable {
                "missing-handler-run"
            } else {
                &run_id
            };
            let id = writer
                .state()
                .queue_jobs_with_provenance(&["{}".into()], reference)
                .unwrap()[0];
            drop(context);
            drop(writer);
            let vault = store::Vault::open(dir.path()).unwrap();
            let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
            let writer = Arc::new(Writer::open(vault, state).unwrap());
            writer.state().set_automation_stopped(true).unwrap();
            writer.state().set_automation_stopped(false).unwrap();
            let handlers = Handlers::on_device(Limits::default());
            handlers.bind(&writer);
            let recovered = handlers.shared.handler_context(&writer, id);
            let kind = if unreadable {
                assert!(recovered.is_err());
                let snapshot = writer.state().load_run_provenance(reference);
                assert!(snapshot.is_restricted());
                assert!(
                    writer
                        .state()
                        .automation_commit(snapshot.stop_generation().unwrap())
                        .is_err()
                );
                "run.recovery_unavailable"
            } else {
                let recovered = recovered.unwrap();
                assert!(recovered.is_restricted());
                assert_eq!(recovered.stop_generation(), Some(generation));
                assert!(writer.state().automation_commit(generation).is_err());
                "run.recovery_stale"
            };
            assert!(
                writer
                    .state()
                    .queued_jobs()
                    .unwrap()
                    .iter()
                    .any(|job| job.id == id)
            );
            assert!(
                writer
                    .state()
                    .activity(&state::ActivityFilter::default())
                    .unwrap()
                    .items
                    .iter()
                    .any(|entry| entry.kind == kind && entry.subject.as_deref() == Some(reference))
            );
        }
    }

    #[test]
    fn recovered_send_and_failed_delete_writeback_use_each_items_context() {
        let dir = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
        let writer = Arc::new(Writer::open(vault, state).unwrap());
        writer
            .upgrade_shipped_defaults(writer::Upkeep::Home)
            .unwrap();
        std::fs::write(dir.path().join("scripts/outbox_send.ws"), r#"use vault
fn reconcile_send(note: Value) -> Result[(), string] {
    vault::patch(note.get("id").unwrap().as_string().unwrap(), Value::Map(#{ "status": Value::String("failed"), "error": Value::String("reconciled") }), "")?
    Ok(())
}
fn reconcile_delete(note: Value) -> Result[(), string] { Err("injected delete failure") }
"#).unwrap();
        let send = RunContext::default();
        let delete = RunContext::default();
        for (context, kind, action) in [(&send, "email", "update"), (&delete, "event", "delete")] {
            context.scope(|| {
                writer.create("outbox", &Input::Json(json!({"fields": {
                "title":"Recover", "kind":kind, "status":"sending", "event_action":action,
                "to":["ann@example.com"]
            }})), &Author::Session("test".into())).unwrap()
            });
        }
        send.restrict();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let capture = observed.clone();
        writer.set_commit_hook(Some(Box::new(move |_| {
            capture.lock().unwrap().push(RunContext::current().unwrap());
            std::ops::ControlFlow::Continue(())
        })));
        let handlers = Handlers::on_device(Limits::default());
        handlers.bind(&writer);
        let ambient = RunContext::default();
        ambient.scope(|| {
            handlers.shared.recover_sends(&writer, true);
            handlers.shared.recover_deletes(&writer);
            assert_eq!(RunContext::current(), Some(ambient.clone()));
        });
        let observed = observed.lock().unwrap();
        assert!(observed.iter().all(|context| context != &send));
        assert!(observed.iter().any(|context| context == &delete));
        assert!(
            observed
                .iter()
                .all(|context| context == &send || context == &delete)
        );
        assert_ne!(send, delete);
        assert!(!delete.is_restricted());
        assert!(!ambient.is_restricted());
        assert!(RunContext::current().is_none());
    }

    fn link(id: &str, event: &str) -> Link {
        Link {
            type_name: "task".into(),
            id: id.into(),
            event: event.into(),
            script: "s".into(),
        }
    }

    #[test]
    fn the_depth_error_names_the_cycle() {
        let chain = [
            link("A", "after_create"),
            link("B", "before_update"),
            link("C", "before_update"),
            link("B", "before_update"),
            link("C", "before_update"),
        ];
        assert_eq!(
            depth_error(&chain),
            "the handler chain stopped at its depth limit of 8: it runs in a cycle, \
             task B before_update -> task C before_update -> task B before_update"
        );
        let straight = [link("A", "after_update"), link("B", "after_update")];
        assert_eq!(
            depth_error(&straight),
            "the handler chain stopped at its depth limit of 8: \
             task A after_update -> task B after_update"
        );
    }
}
