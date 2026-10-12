//! The wscript host: handlers, tasks, migrations, host functions and resource limits (D6, D9,
//! D10, D30, notes/01).
//!
//! A [`Host`] owns a pool of worker threads. wscript VMs are not `Send`, so each worker owns its
//! VMs and every script runs on a worker; [`Host::run`] blocks until the run ends (notes/01).
//!
//! Every run is bounded by [`Limits`]:
//!
//! - an instruction limit and a call-depth limit, which the VM enforces;
//! - a wall-clock timeout, which the host enforces at every host call, since the VM does not meter
//!   time spent in one;
//! - size limits on HTTP responses, query results and the run log.
//!
//! What a script can reach depends on the [`Kind`] of run:
//!
//! | Module | Functions | `before_*` handlers | `after_*` handlers, tasks, migrations |
//! |--------|-----------|---------------------|---------------------------------------|
//! | `vault` | `get(id)`, `query(text)`, `create(type, note)`, `update(id, note, version)`, `delete(id, version)`, `log(message)` | yes | yes |
//! | `http` | `get(url, headers)`, `post(url, headers, body)`, `request(method, url, headers, body)` | no | `after_*` and tasks |
//! | `secrets` | `get(name)` | no | `after_*` and tasks |
//! | `value`, `json`, `math`, `regex` | wscript's standard modules | yes | yes |
//!
//! wscript's `fs`, `process` and `time` modules are never registered, so a script that uses them
//! does not compile (notes/01). A module a kind of run lacks fails the same way: HTTP in a
//! `before_*` handler is a compile error, not a runtime one (D30).
//!
//! Notes cross the boundary as `Value`s in the JSON form of the notes API: `{ id, type, version,
//! note }`, where `note` is `{ fields, blocks }` (D32). Changes go through `writer` and are
//! recorded as made by the handler named in the [`Job`] (D13, D30). HTTP calls reach only the
//! hosts on the job's allowlist and never follow redirects (D10). A script reads only the secrets
//! its job names, and cannot list them (D10).

mod convert;
mod handlers;
mod http;
mod run;
mod vault;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use state::Author;
use wscript::std_modules::DynValue;
use wscript::{CompiledUnit, Context, Fault, HostError, IntoValue, Module, Vm, VmConfig};

pub use handlers::{Handlers, MAX_DEPTH};
pub use http::Response;
pub use vault::Vault;

/// Errors from running a script.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// The script does not compile, including when it uses a module this kind of run lacks.
    #[error("script {script:?} does not compile: {message}")]
    Compile { script: String, message: String },
    /// The script has no such function, or the function takes a different number of arguments.
    #[error("script {script:?}: {message}")]
    Entry { script: String, message: String },
    #[error("script {script:?} stopped: it ran more than its limit of {limit} instructions")]
    InstructionLimit { script: String, limit: u64 },
    #[error("script {script:?} stopped: its calls nested deeper than its limit of {limit}")]
    CallDepth { script: String, limit: usize },
    #[error("script {script:?} stopped: it ran longer than its {limit:?} time limit")]
    Timeout { script: String, limit: Duration },
    /// The script faulted at run time.
    #[error("script {script:?} failed: {message}")]
    Fault { script: String, message: String },
    /// The entry function returned `Err(message)`. For a `before_*` handler this is a rejection
    /// (D9).
    #[error("{message}")]
    Returned { script: String, message: String },
    #[error("the script host has shut down")]
    Closed,
}

/// The kind of run, which decides the modules a script can use (D30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A `before_*` handler, inside the write path: no HTTP and no secrets.
    Before,
    /// An `after_*` handler, from the queue after the commit.
    After,
    /// A scheduled task (D10).
    Task,
    /// A schema change migration (D6): no HTTP and no secrets.
    Migration,
}

impl Kind {
    fn side_effects(self) -> bool {
        matches!(self, Kind::After | Kind::Task)
    }
}

/// Bounds on one run (notes/01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Bytecode instructions. Time inside host calls is not counted, which `timeout` covers.
    pub instructions: u64,
    /// Nested script calls.
    pub call_depth: usize,
    /// Wall-clock time for the whole run.
    pub timeout: Duration,
    /// Bytes of one HTTP response body.
    pub max_http_response: usize,
    /// Rows one `vault::query` may return.
    pub max_query_rows: usize,
    /// Bytes of log a run keeps.
    pub max_log: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            instructions: 50_000_000,
            call_depth: 256,
            timeout: Duration::from_secs(30),
            max_http_response: 1 << 20,
            max_query_rows: 10_000,
            max_log: 64 << 10,
        }
    }
}

/// One script run.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// The script's name, the stem of `scripts/<name>.ws`. Changes the run makes are recorded as
    /// made by handler `<name>`.
    pub name: String,
    /// The script's source.
    pub source: String,
    /// The function to call.
    pub function: String,
    /// The function's arguments, each passed as a `Value`.
    pub args: Vec<Json>,
    pub kind: Kind,
    /// Hosts the `http` module may reach. Ignored for kinds without HTTP.
    pub allow_domains: Vec<String>,
    /// Secrets the `secrets` module may read. Ignored for kinds without secrets.
    pub secrets: Vec<String>,
    /// A wall-clock limit replacing [`Limits::timeout`], such as a task's `timeout`.
    pub timeout: Option<Duration>,
    /// What the caller needs to know about the run from inside a host call, through
    /// [`current_job`]. The host does not read it.
    pub context: Json,
}

impl Job {
    /// A job calling `function` of script `name` with no arguments, no hosts and no secrets.
    pub fn new(kind: Kind, name: &str, source: &str, function: &str) -> Job {
        Job {
            name: name.to_owned(),
            source: source.to_owned(),
            function: function.to_owned(),
            args: Vec::new(),
            kind,
            allow_domains: Vec::new(),
            secrets: Vec::new(),
            timeout: None,
            context: Json::Null,
        }
    }
}

/// Look at the job of the run in progress on this thread, or `None` outside a run. A [`Vault`]
/// uses it to tell what kind of run is calling it.
pub fn current_job<T>(f: impl FnOnce(&Job) -> T) -> Option<T> {
    run::peek(|r| f(&r.job))
}

/// How a run ended, with what it logged either way.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// What the function returned, as JSON: `null` for unit.
    pub result: Result<Json, Error>,
    /// `vault::log`, `print` and `println` output, up to [`Limits::max_log`] bytes.
    pub log: String,
}

/// A run's extra time to finish after its deadline before [`Host::run`] stops waiting for it.
/// Host calls end themselves at the deadline, so only a host bug uses it.
const GRACE: Duration = Duration::from_secs(5);

/// Compiled units a worker keeps before it starts over with fresh VMs. A VM keeps every unit it
/// has run, so this bounds its memory.
const MAX_UNITS: usize = 64;

const WORKER_STACK: usize = 8 << 20;

/// The wscript host of one vault.
pub struct Host {
    contexts: Arc<Contexts>,
    jobs: Mutex<Option<Sender<Request>>>,
    limits: Limits,
}

struct Request {
    job: Job,
    limits: Limits,
    reply: Sender<Outcome>,
}

/// The two registrations: without and with HTTP and secrets.
struct Contexts {
    contained: Context,
    side_effects: Context,
}

impl Contexts {
    fn for_kind(&self, kind: Kind) -> &Context {
        if kind.side_effects() {
            &self.side_effects
        } else {
            &self.contained
        }
    }
}

impl Host {
    /// A host whose scripts reach `vault`, run on `workers` threads, bounded by `limits`.
    pub fn new(vault: Arc<dyn Vault>, workers: usize, limits: Limits) -> Host {
        let http = http::client();
        let common = || {
            Context::new()
                .module(wscript::std_modules::value())
                .module(wscript::std_modules::json())
                .module(wscript::std_modules::math())
                .module(wscript::std_modules::regex())
                .module(vault_module(vault.clone()))
        };
        let contexts = Arc::new(Contexts {
            contained: common(),
            side_effects: common()
                .module(http::module(http))
                .module(secrets_module(vault.clone())),
        });
        let (tx, rx) = mpsc::channel::<Request>();
        let rx = Arc::new(Mutex::new(rx));
        for n in 0..workers.max(1) {
            let rx = rx.clone();
            let contexts = contexts.clone();
            thread::Builder::new()
                .name(format!("script-{n}"))
                .stack_size(WORKER_STACK)
                .spawn(move || worker(&rx, &contexts))
                .expect("start a script worker");
        }
        Host {
            contexts,
            jobs: Mutex::new(Some(tx)),
            limits,
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Check that `source` compiles for `kind` of run, without running it.
    pub fn check(&self, kind: Kind, name: &str, source: &str) -> Result<(), Error> {
        compile(self.contexts.for_kind(kind), name, source).map(|_| ())
    }

    /// Run `job` on a worker and wait for it to end.
    pub fn run(&self, job: Job) -> Outcome {
        let limits = Limits {
            timeout: job.timeout.unwrap_or(self.limits.timeout),
            ..self.limits.clone()
        };
        let name = job.name.clone();
        let wait = limits.timeout + GRACE;
        let (reply, answer) = mpsc::channel();
        let sent = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|tx| {
                tx.send(Request {
                    job,
                    limits: limits.clone(),
                    reply,
                })
                .is_ok()
            });
        if !sent {
            return failed(Error::Closed);
        }
        match answer.recv_timeout(wait) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => failed(Error::Timeout {
                script: name,
                limit: limits.timeout,
            }),
            Err(RecvTimeoutError::Disconnected) => failed(Error::Closed),
        }
    }

    /// Stop taking jobs. Workers finish the run in hand and exit.
    pub fn shutdown(&self) {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn failed(e: Error) -> Outcome {
    Outcome {
        result: Err(e),
        log: String::new(),
    }
}

fn worker(rx: &Mutex<Receiver<Request>>, contexts: &Contexts) {
    let mut engines: HashMap<bool, Engine> = HashMap::new();
    loop {
        let next = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok(Request { job, limits, reply }) = next else {
            return;
        };
        let side = job.kind.side_effects();
        let engine = engines
            .entry(side)
            .or_insert_with(|| Engine::new(contexts.for_kind(job.kind).clone()));
        let outcome = engine.execute(job, limits);
        let _ = reply.send(outcome);
    }
}

/// One VM and the units it has run, for one registration, on one thread.
struct Engine {
    ctx: Context,
    vm: Vm,
    units: HashMap<u64, (String, CompiledUnit)>,
}

impl Engine {
    fn new(ctx: Context) -> Engine {
        let vm = new_vm(&ctx);
        Engine {
            ctx,
            vm,
            units: HashMap::new(),
        }
    }

    fn unit(&mut self, name: &str, source: &str) -> Result<CompiledUnit, Error> {
        let mut h = DefaultHasher::new();
        source.hash(&mut h);
        let key = h.finish();
        if let Some((src, unit)) = self.units.get(&key)
            && src == source
        {
            return Ok(unit.clone());
        }
        let unit = compile(&self.ctx, name, source)?;
        if self.units.len() >= MAX_UNITS {
            self.units.clear();
            self.vm = new_vm(&self.ctx);
        }
        self.units.insert(key, (source.to_owned(), unit.clone()));
        Ok(unit)
    }

    fn execute(&mut self, job: Job, limits: Limits) -> Outcome {
        let script = job.name.clone();
        let unit = match self.unit(&script, &job.source) {
            Ok(u) => u,
            Err(e) => return failed(e),
        };
        let entry = |message: String| {
            failed(Error::Entry {
                script: script.clone(),
                message,
            })
        };
        let Some((_, sig)) = unit.exports.get(&job.function) else {
            return entry(format!("there is no function {:?}", job.function));
        };
        if sig.params.len() != job.args.len() {
            return entry(format!(
                "function {:?} takes {} argument(s), but the host passes {}",
                job.function,
                sig.params.len(),
                job.args.len()
            ));
        }
        let mut args = Vec::with_capacity(job.args.len());
        for a in &job.args {
            match convert::to_dyn(a.clone()).into_value(&unit.defs) {
                Ok(v) => args.push(v),
                Err(e) => return entry(e.message),
            }
        }
        let function = job.function.clone();
        self.vm.set_fuel(Some(limits.instructions));
        self.vm.set_call_depth_limit(limits.call_depth);
        run::start(run::Run {
            author: Author::Handler(job.name.clone()),
            deadline: Instant::now() + limits.timeout,
            job,
            limits: limits.clone(),
            log: String::new(),
            timed_out: false,
        });
        let called = self.vm.call_values(&unit, &function, args);
        let run = run::finish().expect("the run started above");
        let result = match called {
            Ok(v) => match convert::returned(v, &unit.defs) {
                Ok(Ok(json)) => Ok(json),
                Ok(Err(message)) => Err(Error::Returned { script, message }),
                Err(message) => Err(Error::Fault { script, message }),
            },
            Err(e) => Err(fault(script, e, run.timed_out, &limits)),
        };
        Outcome {
            result,
            log: run.log,
        }
    }
}

fn new_vm(ctx: &Context) -> Vm {
    Vm::with_config(
        ctx,
        VmConfig {
            out: Box::new(run::print),
            ..VmConfig::default()
        },
    )
}

fn fault(script: String, e: wscript::Error, timed_out: bool, limits: &Limits) -> Error {
    if timed_out {
        return Error::Timeout {
            script,
            limit: limits.timeout,
        };
    }
    let message = match e {
        wscript::Error::Runtime(f) => {
            if f.message == "fuel exhausted" {
                return Error::InstructionLimit {
                    script,
                    limit: limits.instructions,
                };
            }
            if f.message.starts_with("stack overflow") {
                return Error::CallDepth {
                    script,
                    limit: limits.call_depth,
                };
            }
            f.message
        }
        other => other.to_string(),
    };
    Error::Fault { script, message }
}

fn compile(ctx: &Context, name: &str, source: &str) -> Result<CompiledUnit, Error> {
    ctx.compile(source).map_err(|e| Error::Compile {
        script: name.to_owned(),
        message: match e {
            wscript::Error::Compile(diags) => diags
                .iter()
                .filter(|d| d.severity == wscript::Severity::Error)
                .map(|d| format!("[{}] {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; "),
            other => other.to_string(),
        },
    })
}

type Reply<T> = Fault<Result<T, String>>;

/// Run a host call that reaches the vault: within the deadline, as the run's author.
fn host_call<T>(
    f: impl FnOnce(&Author, &Limits) -> Result<T, String>,
) -> Result<Result<T, String>, HostError> {
    let (author, limits) = run::with(|r| {
        r.remaining()?;
        Ok((r.author.clone(), r.limits.clone()))
    })?;
    let out = f(&author, &limits);
    run::with(|r| r.remaining())?;
    Ok(out)
}

/// The note body of a `Value` a script passes: `{ fields, blocks }`, or a whole note from
/// `vault::get` with its body under `note`.
fn note_body(v: DynValue) -> Json {
    match convert::from_dyn(v) {
        Json::Object(mut o) if !o.contains_key("fields") && o.contains_key("note") => {
            o.remove("note").unwrap_or_default()
        }
        other => other,
    }
}

fn vault_module(v: Arc<dyn Vault>) -> Module {
    let mut m = Module::new("vault");
    m.doc("Read, query and change notes. Changes go through the write path (D30)");
    let vv = v.clone();
    m.fn_named("get", ["id"], move |id: String| -> Reply<DynValue> {
        Fault(host_call(|_, _| vv.get(&id).map(convert::to_dyn)))
    });
    let vv = v.clone();
    m.fn_named(
        "query",
        ["text"],
        move |text: String| -> Reply<Vec<DynValue>> {
            Fault(host_call(|_, l| {
                vv.query(&text, l.max_query_rows)
                    .map(|rows| rows.into_iter().map(convert::to_dyn).collect())
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "create",
        ["type_name", "note"],
        move |type_name: String, note: DynValue| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.create(&type_name, note_body(note), a)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "update",
        ["id", "note", "version"],
        move |id: String, note: DynValue, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.update(&id, note_body(note), &version, a)
                    .map(convert::to_dyn)
            }))
        },
    );
    m.fn_named(
        "delete",
        ["id", "version"],
        move |id: String, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                v.delete(&id, &version, a).map(convert::to_dyn)
            }))
        },
    );
    m.fn_named("log", ["message"], |message: String| -> Fault<()> {
        Fault(run::with(|r| {
            r.remaining()?;
            r.log(&message, true);
            Ok(())
        }))
    });
    m
}

fn secrets_module(v: Arc<dyn Vault>) -> Module {
    let mut m = Module::new("secrets");
    m.doc("Read the secrets this script was granted, by name (D10)");
    m.fn_named("get", ["name"], move |name: String| -> Reply<String> {
        let granted = run::with(|r| {
            r.remaining()?;
            Ok(r.job.secrets.contains(&name))
        });
        Fault(granted.map(|granted| {
            if !granted {
                return Err(format!("secret {name:?} is not granted to this script"));
            }
            v.secret(&name)?
                .ok_or_else(|| format!("there is no secret named {name:?}"))
        }))
    });
    m
}
