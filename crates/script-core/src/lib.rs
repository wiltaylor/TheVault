//! The wscript engine: kinds of run, jobs, resource limits, the [`Host`] and the [`Vault`] trait
//! scripts reach the vault through (D6, D9, D10, D30, notes/01).
//!
//! This crate names nothing server-only, so a device runs the same engine (D6). `crates/script`
//! adds the server's modules and implementations through [`Extensions`].
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
//! What a script can reach depends on the [`Kind`] of run. The engine registers `vault`, `value`,
//! `json`, `math` and `regex` for every kind; the [`Extensions`] a host is made with add the rest.
//! A module a kind of run lacks is a compile error, not a runtime one (D30).
//!
//! Notes cross the boundary as `Value`s in the JSON form of the notes API: `{ id, type, version,
//! note }`, where `note` is `{ fields, blocks }` (D32). Changes are recorded as made by the
//! handler named in the [`Job`] (D13, D30).

pub mod convert;
mod device;
pub mod run;
mod vault;
mod vault_module;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use wscript::std_modules::DynValue;
use wscript::{CompiledUnit, Context, Fault, HostError, IntoValue, Module, Vm, VmConfig};

pub use device::{DeviceAccess, DeviceData, DeviceExtensions, NoDevice, Switches, device_module};
pub use vault::Vault;
pub use vault_module::vault_module;
pub use vocab::{Archived, Author};

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
    /// A script run on a device, which alone reaches the `device` module (D6).
    Device,
}

impl Kind {
    /// Which registration runs this kind.
    fn registration(self) -> Registration {
        match self {
            Kind::Before | Kind::Migration => Registration::Contained,
            Kind::After => Registration::SideEffects,
            Kind::Task => Registration::Task,
            Kind::Device => Registration::Device,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Registration {
    Contained,
    SideEffects,
    Task,
    Device,
}

impl Registration {
    /// The kind [`Extensions::modules`] is asked about for this registration.
    fn kind(self) -> Kind {
        match self {
            Registration::Contained => Kind::Before,
            Registration::SideEffects => Kind::After,
            Registration::Task => Kind::Task,
            Registration::Device => Kind::Device,
        }
    }
}

/// What a host adds to the engine's modules: the server's `http`, `secrets` and `fs`, or a
/// device's `device` module.
pub trait Extensions {
    /// Add functions to the `vault` module every kind of run reaches.
    fn vault(&self, _module: &mut Module, _vault: &Arc<dyn Vault>) {}

    /// Modules for runs of `kind`, besides the engine's own. Asked once per registration:
    /// [`Kind::Before`] stands for migrations too, which reach the same modules.
    fn modules(&self, _kind: Kind, _vault: &Arc<dyn Vault>) -> Vec<Module> {
        Vec::new()
    }
}

/// No modules beyond the engine's own.
pub struct NoExtensions;

impl Extensions for NoExtensions {}

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
    /// Bytes of one file or asset a script reads.
    pub max_file: usize,
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
            max_file: 16 << 20,
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
    /// Directories the `fs` module may read. Ignored for kinds without `fs`.
    pub read_paths: Vec<String>,
    /// Directories the `fs` module may read and write. Ignored for kinds without `fs`.
    pub write_paths: Vec<String>,
    /// Who the run's changes are recorded as made by. `None` is handler `<name>`.
    pub author: Option<Author>,
    /// The IANA time zone `vault::today` reads the date in. `None` is UTC.
    pub timezone: Option<String>,
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
            read_paths: Vec::new(),
            write_paths: Vec::new(),
            author: None,
            timezone: None,
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
    /// The bytecode instructions the run used: 0 for one that never started, such as a script
    /// that does not compile.
    pub instructions: u64,
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

/// One context per registration: what `before_*` handlers and migrations reach, what `after_*`
/// handlers reach, what tasks reach, and what device scripts reach.
struct Contexts {
    contained: Context,
    side_effects: Context,
    task: Context,
    device: Context,
}

impl Contexts {
    fn for_kind(&self, kind: Kind) -> &Context {
        match kind.registration() {
            Registration::Contained => &self.contained,
            Registration::SideEffects => &self.side_effects,
            Registration::Task => &self.task,
            Registration::Device => &self.device,
        }
    }
}

impl Host {
    /// A host whose scripts reach `vault` and the engine's own modules, run on `workers` threads,
    /// bounded by `limits`.
    pub fn new(vault: Arc<dyn Vault>, workers: usize, limits: Limits) -> Host {
        Host::with_extensions(vault, workers, limits, &NoExtensions)
    }

    /// A host like [`Host::new`] whose scripts also reach what `extensions` adds.
    pub fn with_extensions(
        vault: Arc<dyn Vault>,
        workers: usize,
        limits: Limits,
        extensions: &dyn Extensions,
    ) -> Host {
        let context = |r: Registration| {
            let mut vault_module = vault_module::vault_module(vault.clone());
            extensions.vault(&mut vault_module, &vault);
            let mut ctx = Context::new()
                .module(wscript::std_modules::value())
                .module(wscript::std_modules::json())
                .module(wscript::std_modules::math())
                .module(wscript::std_modules::regex())
                .module(vault_module);
            for m in extensions.modules(r.kind(), &vault) {
                ctx = ctx.module(m);
            }
            ctx
        };
        let contexts = Arc::new(Contexts {
            contained: context(Registration::Contained),
            side_effects: context(Registration::SideEffects),
            task: context(Registration::Task),
            device: context(Registration::Device),
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

    /// Check that `source` compiles for `kind` of run and exports `function`, answering how many
    /// arguments it takes, without running it.
    pub fn entry(
        &self,
        kind: Kind,
        name: &str,
        source: &str,
        function: &str,
    ) -> Result<usize, Error> {
        let unit = compile(self.contexts.for_kind(kind), name, source)?;
        unit.exports
            .get(function)
            .map(|(_, sig)| sig.params.len())
            .ok_or_else(|| Error::Entry {
                script: name.to_owned(),
                message: format!("there is no function {function:?}"),
            })
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
        instructions: 0,
    }
}

fn worker(rx: &Mutex<Receiver<Request>>, contexts: &Contexts) {
    let mut engines: HashMap<Registration, Engine> = HashMap::new();
    loop {
        let next = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok(Request { job, limits, reply }) = next else {
            return;
        };
        let engine = engines
            .entry(job.kind.registration())
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
            author: job
                .author
                .clone()
                .unwrap_or_else(|| Author::Handler(job.name.clone())),
            deadline: Instant::now() + limits.timeout,
            job,
            limits: limits.clone(),
            log: String::new(),
            timed_out: false,
        });
        let called = self.vm.call_values(&unit, &function, args);
        let instructions = limits
            .instructions
            .saturating_sub(self.vm.fuel().unwrap_or(0));
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
            instructions,
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

/// What a host function returns: a fault that ends the run, or a value or error message the script
/// receives.
pub type Reply<T> = Fault<Result<T, String>>;

/// Run a host call that reaches the vault: within the deadline, as the run's author.
pub fn host_call<T>(
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
pub fn note_body(v: DynValue) -> Json {
    match convert::from_dyn(v) {
        Json::Object(mut o) if !o.contains_key("fields") && o.contains_key("note") => {
            o.remove("note").unwrap_or_default()
        }
        other => other,
    }
}
