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
//!   failed job stays in `state.db` with its message.
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
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use schema::{Doc, Handler, Schema};
use serde_json::{Value as Json, json};
use state::Author;
use writer::{Event, Hooks, Input, Migration, NoteChange, Saved, Writer, doc_to_json, note_path};

use crate::{Host, Job, Kind, Limits, Vault, current_job};

/// The most handler runs one chain may hold (D9, D30).
pub const MAX_DEPTH: usize = 8;

/// Workers for the `after_*` host. The queue runs one job at a time.
const AFTER_WORKERS: usize = 2;

/// How long the queue thread sleeps before looking at `state.db` again without being woken.
const POLL: Duration = Duration::from_secs(1);

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
    /// Set when jobs were queued, to wake the queue thread.
    wake: (Mutex<bool>, Condvar),
}

impl Handlers {
    pub fn new(limits: Limits) -> Handlers {
        let binding: Binding = Arc::new(OnceLock::new());
        let vault = Arc::new(HandlerVault {
            binding: binding.clone(),
        });
        Handlers {
            shared: Arc::new(Shared {
                before: Host::new(vault.clone(), MAX_DEPTH + 2, limits.clone()),
                after: Host::new(vault, AFTER_WORKERS, limits),
                binding,
                active: Mutex::new(None),
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
    fn before(&self, change: &mut NoteChange) -> Result<(), String> {
        let writer = self.writer()?;
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
            job.args = vec![arg];
            let returned = self.before.run(job).result.map_err(|e| e.to_string())?;
            if change.event != Event::Delete
                && let Some(new) = returned_doc(returned, &doc, &change.type_name, &change.id)?
            {
                change.doc = Some(new);
            }
        }
        Ok(())
    }

    /// Queue the `after_*` handlers of every saved note.
    fn after(&self, saved: &[Saved], schema: &Schema) {
        let Ok(writer) = self.writer() else {
            return;
        };
        let chain = inherited();
        let mut queue = Vec::new();
        for s in saved {
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
        if let Err(e) = writer.state().queue_jobs(&queue) {
            eprintln!("queueing after_* handlers: {e}");
            return;
        }
        let (flag, cv) = &self.wake;
        *flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_one();
    }

    /// The queue thread: run queued jobs, oldest first, until the writer is gone.
    fn run_queue(&self) {
        loop {
            let Ok(writer) = self.writer() else {
                return;
            };
            match writer.state().queued_jobs() {
                Ok(jobs) => {
                    for job in jobs {
                        let (outcome, log) = self.run_job(&writer, &job.payload);
                        if let Err(message) = &outcome {
                            eprintln!("after_* handler job {} failed: {message}", job.id);
                        }
                        if let Err(e) = writer.state().finish_job(job.id, outcome, &log) {
                            eprintln!("recording handler job {}: {e}", job.id);
                        }
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

    /// Run one queued `after_*` job.
    fn run_job(&self, writer: &Writer, payload: &str) -> (Result<(), String>, String) {
        let job = (|| {
            let p: Json = serde_json::from_str(payload).map_err(|e| e.to_string())?;
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
            let script = text("script");
            let source = script_source(writer, &script)?;
            let mut job = Job::new(Kind::After, &script, &source, &text("function"));
            job.args = vec![json!({
                "id": id,
                "type": type_name,
                "event": event,
                "version": text("version"),
                "note": doc_to_json(&doc),
            })];
            job.allow_domains = list("allow_domains");
            job.secrets = list("secrets");
            job.context = json!({ "chain": p.get("chain").cloned().unwrap_or_default() });
            Ok::<_, String>(job)
        })();
        match job {
            Ok(job) => {
                let out = self.after.run(job);
                (out.result.map(|_| ()).map_err(|e| e.to_string()), out.log)
            }
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

    fn after(&self, saved: &[Saved], _author: &Author, schema: &Schema) {
        self.0.after(saved, schema);
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

    fn secret(&self, name: &str) -> Result<Option<String>, String> {
        Vault::secret(&*bound(&self.binding)?, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
