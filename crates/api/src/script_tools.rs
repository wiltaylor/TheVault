//! Script tools (script-tools T2, T4, T6): `tools/<name>.wcl` files naming a script function an
//! agent calls as a tool, the same in a skill's folder, and the routes that manage them.
//!
//! - `GET /api/tools`: every tool file, each `{ name, valid, problems, description, script,
//!   function, params, allow_domains, secrets, timeout }`. A tool is valid when its file parses,
//!   its name is not a built-in tool's, and its script exists, compiles as a task and exports the
//!   function taking one argument; `problems` says why not.
//! - `GET /api/tools/{name}`: the same with `wcl`, `tool` and the script's `script_source`; the
//!   WCL alone when `Accept` asks for text.
//! - `PUT /api/tools/{name}` (admin): the tool file as WCL text, or JSON `{ wcl, script_source? }`,
//!   which writes the script it names in the same commit. A file that does not parse, or a
//!   built-in tool's name, is a 400 `invalid`; a missing or broken script is not refused, and
//!   shows in `problems`.
//! - `DELETE /api/tools/{name}` (admin): removes the file, and its script when nothing else names
//!   it, answering `script_deleted`.
//! - `POST /api/tools/{name}/run` (admin): `{ params }`, runs the tool as the caller would an
//!   agent's call and answers `{ ok, result, error, kind, log, duration_ms, instructions,
//!   script }`.
//!
//! Every run reads the tool and its script at one vault generation, and answers a script
//! reference: `{ kind: "vault", tool, script, hash, tool_hash, generation }` for a vault tool,
//! `{ kind: "skill", skill, tool, path, hash }` for a skill's, and `{ kind: "code", hash }` for an
//! agent's own code (`crate::ai_scripts`), `hash` the SHA-256 of the script source that ran.
//!
//! Scripts run on the home server's script host, the one the scheduled tasks run on. The apps'
//! embedded servers have none: their turns run on the home server, and their tool routes only
//! read.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use scheduler::Scheduler;
use schema::{Issue, ScriptTool};
use script::{Error as ScriptError, Host, Job, Kind};
use serde::Deserialize;
use serde_json::{Value, json};
use state::Author;
use writer::Writer;

use crate::defs::{body, text_or_json};
use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// The separator of a skill tool's name: `<skill>__<tool>` (script-tools T4).
pub const SKILL_TOOL_SEPARATOR: &str = "__";

/// The longest tool name a provider takes.
const MAX_TOOL_NAME: usize = 64;

// ---- the script host --------------------------------------------------------------------------

type Hosts = Mutex<Vec<(Weak<Writer>, Weak<Scheduler>)>>;

fn hosts() -> &'static Hosts {
    static HOSTS: OnceLock<Hosts> = OnceLock::new();
    HOSTS.get_or_init(Mutex::default)
}

/// Make `scheduler`'s script host the one agents' scripts on `writer`'s vault run on, and
/// `scheduler` the one `run_task` starts tasks on. Only the home server registers one
/// (script-tools T1).
pub(crate) fn register(writer: &Arc<Writer>, scheduler: &Arc<Scheduler>) {
    let mut all = hosts().lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|(w, s)| w.strong_count() > 0 && s.strong_count() > 0);
    all.push((Arc::downgrade(writer), Arc::downgrade(scheduler)));
}

/// The scheduler of `w`'s vault, when this server runs scripts for agents.
pub(crate) fn scheduler_for(w: &Writer) -> Option<Arc<Scheduler>> {
    let all = hosts().lock().unwrap_or_else(|e| e.into_inner());
    all.iter()
        .find(|(writer, _)| std::ptr::eq(writer.as_ptr(), w))
        .and_then(|(_, s)| s.upgrade())
}

/// The script host of `w`'s vault, when this server runs scripts for agents.
pub(crate) fn host_for(w: &Writer) -> Option<Arc<Host>> {
    scheduler_for(w).map(|s| s.host().clone())
}

// ---- checking ---------------------------------------------------------------------------------

/// Every tool name the server has itself, which a script tool may not take.
pub(crate) fn built_in_names() -> Vec<String> {
    let mut names = crate::agent_files::tool_names();
    names.extend(crate::ai_browser::tool_specs().into_iter().map(|t| t.name));
    names.push(crate::ai_skills::INSTALL_TOOL.to_owned());
    names
}

/// The issue for a tool named as a built-in tool.
pub(crate) fn name_issues(tool: &ScriptTool) -> Vec<Issue> {
    if built_in_names().contains(&tool.name) {
        vec![Issue::new(
            "name",
            format!(
                "{:?} is a tool the server has itself: name the tool something else",
                tool.name
            ),
        )]
    } else {
        Vec::new()
    }
}

/// Whether the script compiles as a task and exports `function` taking one argument: the
/// problem, if not. Kept by source, so a turn does not compile every tool's script each round.
fn entry_problem(
    w: &Writer,
    host: Option<&Host>,
    script: &str,
    source: &str,
    function: &str,
) -> Option<String> {
    type Cache = Mutex<HashMap<u64, Option<String>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mut h = DefaultHasher::new();
    (script, source, function, host.is_some()).hash(&mut h);
    let key = h.finish();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(p) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return p.clone();
    }
    let problem = match host {
        Some(host) => match host.entry(Kind::Task, script, source, function) {
            Ok(1) => None,
            Ok(n) => Some(format!(
                "function {function:?} of script {script:?} takes {n} arguments: a tool's function takes one, the map of its parameters"
            )),
            Err(ScriptError::Entry { .. }) => {
                Some(format!("script {script:?} has no function {function:?}"))
            }
            Err(e) => Some(e.to_string()),
        },
        // No script host (an app's embedded server): only whether it compiles.
        None => w.check_tool_script(script, source).err(),
    };
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 256 {
        c.clear();
    }
    c.insert(key, problem.clone());
    problem
}

/// One tool file as checked: the tool when it parses, its script's source when it exists, and
/// every problem that keeps it from being offered.
#[derive(Debug, Clone)]
pub(crate) struct Checked {
    pub name: String,
    pub wcl: String,
    pub tool: Option<ScriptTool>,
    pub script_source: Option<String>,
    pub problems: Vec<String>,
}

impl Checked {
    pub(crate) fn valid(&self) -> bool {
        self.tool.is_some() && self.problems.is_empty()
    }

    /// The list entry of `GET /api/tools`.
    pub(crate) fn summary(&self) -> Value {
        let mut v = match &self.tool {
            Some(t) => t.to_json(),
            None => json!({
                "name": self.name,
                "description": null,
                "script": null,
                "function": null,
                "params": [],
                "allow_domains": [],
                "secrets": [],
                "timeout": null,
            }),
        };
        v["valid"] = json!(self.valid());
        v["problems"] = json!(self.problems);
        v
    }

    /// The answer of `GET /api/tools/{name}`.
    pub(crate) fn detail(&self) -> Value {
        json!({
            "name": self.name,
            "wcl": self.wcl,
            "valid": self.valid(),
            "problems": self.problems,
            "tool": self.tool.as_ref().map(|_| self.summary()),
            "script": self.tool.as_ref().map(|t| t.script.clone()),
            "script_source": self.script_source,
        })
    }
}

/// Check tool `name`, whose file is `wcl`, against a script read by `read_script`.
pub(crate) fn check_with(
    w: &Writer,
    host: Option<&Host>,
    name: &str,
    wcl: &str,
    file: &str,
    builtin_clash: bool,
    read_script: impl FnOnce(&str) -> Option<String>,
) -> Checked {
    let mut out = Checked {
        name: name.to_owned(),
        wcl: wcl.to_owned(),
        tool: None,
        script_source: None,
        problems: Vec::new(),
    };
    let tool = match ScriptTool::parse_at(name, wcl, file) {
        Ok(t) => t,
        Err(e) => {
            out.problems = e.issues.iter().map(ToString::to_string).collect();
            return out;
        }
    };
    if builtin_clash {
        out.problems
            .extend(name_issues(&tool).iter().map(ToString::to_string));
    }
    match read_script(&tool.script) {
        None => out
            .problems
            .push(format!("there is no script {:?}", tool.script)),
        Some(source) => {
            if let Some(p) = entry_problem(w, host, &tool.script, &source, &tool.function) {
                out.problems.push(p);
            }
            out.script_source = Some(source);
        }
    }
    out.tool = Some(tool);
    out
}

/// Check vault tool `name`, its file `wcl`, with its script as it is now or as `script`.
pub(crate) fn check_vault(
    w: &Writer,
    host: Option<&Host>,
    name: &str,
    wcl: &str,
    script: Option<&str>,
) -> Checked {
    check_with(
        w,
        host,
        name,
        wcl,
        &writer::tool_path(name),
        true,
        |s| match script {
            Some(source) => Some(source.to_owned()),
            None => w.read_script(s).ok(),
        },
    )
}

/// Every vault tool, checked.
pub(crate) fn all_vault(w: &Writer, host: Option<&Host>) -> Vec<Checked> {
    match w.tool_files() {
        Ok(files) => files
            .into_iter()
            .map(|(name, wcl)| check_vault(w, host, &name, &wcl, None))
            .collect(),
        Err(e) => {
            eprintln!("the tools could not be listed: {e}");
            Vec::new()
        }
    }
}

/// The tool files of skill `skill`, by tool name: `tools/<tool>.wcl` in its folder.
pub(crate) fn skill_tool_names(skill: &writer::Skill) -> Vec<String> {
    skill
        .files
        .iter()
        .filter_map(|f| {
            f.path
                .strip_prefix("tools/")
                .and_then(|p| p.strip_suffix(".wcl"))
                .filter(|n| schema::is_name(n))
                .map(str::to_owned)
        })
        .collect()
}

/// The name a skill's tool is offered under: `<skill>__<tool>`, or `None` when it would be too
/// long for a provider.
pub(crate) fn skill_tool_name(skill: &str, tool: &str) -> Option<String> {
    let n = format!("{skill}{SKILL_TOOL_SEPARATOR}{tool}");
    (n.len() <= MAX_TOOL_NAME).then_some(n)
}

/// Tool `tool` of skill `skill`, checked against the skill's own scripts.
pub(crate) fn check_skill(
    w: &Writer,
    host: Option<&Host>,
    skill: &str,
    tool: &str,
) -> Result<Checked, String> {
    let path = format!("tools/{tool}.wcl");
    let wcl = w.skill_text(skill, &path).map_err(|e| e.to_string())?;
    let file = format!("skills/{skill}/{path}");
    Ok(check_with(w, host, tool, &wcl, &file, false, |s| {
        w.skill_text(skill, &format!("scripts/{s}.ws")).ok()
    }))
}

// ---- running ----------------------------------------------------------------------------------

/// How a script run ended, for a tool's answer and the activity log.
#[derive(Debug, Clone)]
pub(crate) struct Ran {
    /// The function's return value, or the error's kind and message.
    pub result: Result<Value, (&'static str, String)>,
    pub log: String,
    pub ms: u64,
    pub instructions: u64,
    /// The script reference: which version of which script ran.
    pub script: Value,
}

impl Ran {
    /// A failure before the script ran.
    pub(crate) fn refused(kind: &'static str, message: String, script: Value) -> Ran {
        Ran {
            result: Err((kind, message)),
            log: String::new(),
            ms: 0,
            instructions: 0,
            script,
        }
    }

    /// The JSON answer: `{ ok, result, error, kind, log, duration_ms, instructions, script }`.
    pub(crate) fn to_json(&self) -> Value {
        let (ok, result, error, kind) = match &self.result {
            Ok(v) => (true, v.clone(), Value::Null, Value::Null),
            Err((k, m)) => (false, Value::Null, json!(m), json!(k)),
        };
        json!({
            "ok": ok,
            "result": result,
            "error": error,
            "kind": kind,
            "log": self.log,
            "duration_ms": self.ms,
            "instructions": self.instructions,
            "script": self.script,
        })
    }
}

/// The SHA-256 hex of `text`.
pub(crate) fn hash(text: &str) -> String {
    writer::content_hash(text)
}

/// The kind of a script error, as answers name it.
pub(crate) fn error_kind(e: &ScriptError) -> &'static str {
    match e {
        ScriptError::Compile { .. } => "compile",
        ScriptError::Entry { .. } => "entry",
        ScriptError::InstructionLimit { .. } => "instruction_limit",
        ScriptError::CallDepth { .. } => "call_depth",
        ScriptError::Timeout { .. } => "timeout",
        ScriptError::Fault { .. } => "fault",
        ScriptError::Returned { .. } => "returned",
        ScriptError::Closed => "closed",
    }
}

/// Run `job` on `host` and report it with `script` as its reference.
pub(crate) fn run(host: &Host, job: Job, script: Value) -> Ran {
    let began = Instant::now();
    let out = host.run(job);
    let ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ran {
        result: out.result.map_err(|e| (error_kind(&e), e.to_string())),
        log: out.log,
        ms,
        instructions: out.instructions,
        script,
    }
}

/// A job running checked tool `c` with `input` as `author`, or the refusal when it cannot run.
fn tool_job(
    c: &Checked,
    input: &Value,
    author: &Author,
    job_name: &str,
) -> Result<Job, (&'static str, String)> {
    let Some(tool) = c.tool.as_ref().filter(|_| c.valid()) else {
        return Err((
            "invalid",
            format!("the tool {} cannot run: {}", c.name, c.problems.join("; ")),
        ));
    };
    let source = c.script_source.clone().unwrap_or_default();
    let args = tool.arguments(input).map_err(|e| ("arguments", e))?;
    let mut job = Job::new(Kind::Task, job_name, &source, &tool.function);
    job.args = vec![args];
    job.allow_domains = tool.allow_domains.clone();
    job.secrets = tool.secrets.clone();
    job.author = Some(author.clone());
    job.timeout = Some(Duration::from_secs(tool.timeout));
    Ok(job)
}

/// Run vault tool `name` with `input` as `author`: the tool and its script as they are at one
/// generation.
pub(crate) fn run_vault_tool(
    w: &Writer,
    host: &Host,
    name: &str,
    input: &Value,
    author: &Author,
) -> Ran {
    let snap = match w.tool_snapshot(name) {
        Ok(s) => s,
        Err(e) => {
            return Ran::refused(
                "invalid",
                e.to_string(),
                json!({ "kind": "vault", "tool": name }),
            );
        }
    };
    let c = check_vault(w, Some(host), name, &snap.wcl, snap.script.as_deref());
    let script = json!({
        "kind": "vault",
        "tool": name,
        "script": c.tool.as_ref().map(|t| t.script.clone()),
        "hash": c.script_source.as_deref().map(hash),
        "tool_hash": hash(&snap.wcl),
        "generation": snap.generation,
    });
    let job_name = c.tool.as_ref().map_or(name, |t| t.script.as_str());
    match tool_job(&c, input, author, job_name) {
        Ok(job) => run(host, job, script),
        Err((kind, message)) => Ran::refused(kind, message, script),
    }
}

/// Run tool `tool` of skill `skill` with `input` as `author`.
pub(crate) fn run_skill_tool(
    w: &Writer,
    host: &Host,
    skill: &str,
    tool: &str,
    input: &Value,
    author: &Author,
) -> Ran {
    let base = json!({ "kind": "skill", "skill": skill, "tool": tool });
    let c = match check_skill(w, Some(host), skill, tool) {
        Ok(c) => c,
        Err(e) => return Ran::refused("invalid", e, base),
    };
    let mut script = base;
    if let Some(t) = &c.tool {
        script["path"] = json!(format!("scripts/{}.ws", t.script));
    }
    script["hash"] = json!(c.script_source.as_deref().map(hash));
    let job_name = c
        .tool
        .as_ref()
        .map_or_else(|| tool.to_owned(), |t| t.script.clone());
    match tool_job(&c, input, author, &job_name) {
        Ok(job) => run(host, job, script),
        Err((kind, message)) => Ran::refused(kind, message, script),
    }
}

// ---- routes -----------------------------------------------------------------------------------

/// `GET /api/tools`.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let tools = blocking(w, |w| {
        let host = host_for(w);
        Ok::<_, ApiError>(
            all_vault(w, host.as_deref())
                .iter()
                .map(Checked::summary)
                .collect::<Vec<_>>(),
        )
    })
    .await?;
    Ok(Json(json!({ "tools": tools })))
}

/// `GET /api/tools/{name}`.
pub async fn read(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let c = blocking(w, move |w| {
        let wcl = w.read_tool(&name)?;
        let host = host_for(w);
        Ok::<_, ApiError>(check_vault(w, host.as_deref(), &name, &wcl, None))
    })
    .await?;
    Ok(text_or_json(&headers, c.wcl.clone(), c.detail()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolBody {
    wcl: String,
    #[serde(default)]
    script_source: Option<String>,
}

/// `PUT /api/tools/{name}`.
pub async fn put(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: ToolBody = body(&headers, &raw, |wcl| ToolBody {
        wcl,
        script_source: None,
    })?;
    let c = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        let (text, _) = w.put_tool(&name, &b.wcl, b.script_source.as_deref(), name_issues)?;
        let host = host_for(w);
        Ok::<_, ApiError>(check_vault(w, host.as_deref(), &name, &text, None))
    })
    .await?;
    Ok(text_or_json(&headers, c.wcl.clone(), c.detail()))
}

/// `DELETE /api/tools/{name}`.
pub async fn delete(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    let script = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.delete_tool(&n)
    })
    .await?;
    Ok(Json(
        json!({ "name": name, "deleted": true, "script_deleted": script }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBody {
    #[serde(default)]
    params: Value,
}

/// `POST /api/tools/{name}/run`.
pub async fn run_route(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
    Json(b): Json<RunBody>,
) -> Result<Json<Value>, ApiError> {
    let ran = blocking(w, move |w| {
        let Some(host) = host_for(w) else {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "tools run on the home server: run it in the web app",
            ));
        };
        w.read_tool(&name)?;
        Ok(run_vault_tool(w, &host, &name, &b.params, &by))
    })
    .await?;
    Ok(Json(ran.to_json()))
}
