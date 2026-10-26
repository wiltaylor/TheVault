//! Agents and wscript (script-tools T1 to T6): `run_script`, the script tools of the vault and of
//! the skills an agent has opened, and the tools that read and propose tool files.
//!
//! - `run_script { code, function, args }` compiles `code` and calls its `function` (`main`
//!   unless named) with `args`, JSON text of an array. It is offered only to an agent whose file
//!   sets `permissions { scripts { run_code = true } }`, which only an admin writes, and runs with
//!   everything a task reaches: `http` to any host, every secret by name (`*`), no `fs`
//!   directories, five minutes. Its changes are made at once as the agent.
//! - `run_saved_script`, `run_task`, `list_scripts` and `list_tasks` run the vault's own scripts
//!   and tasks by name, for an agent with `run_saved` or `run_code` (`crate::ai_saved`).
//! - A vault tool, `tools/<name>.wcl`, is offered under its name, and a skill's tool under
//!   `<skill>__<tool>` from the round after the agent calls `use_skill` on the skill, for the rest
//!   of the thread. Both run as `crate::script_tools` describes, writing as the agent.
//! - `list_tools` and `get_tool` read; `propose_tool_change` and `propose_tool_delete` check a
//!   change and hand it to the user as a `tool_proposal` (the `tools` confirmation switch, admin
//!   scope), which `POST /api/ai/apply` applies through [`apply_tool`].
//!
//! A script's changes are made at once, so a turn whose scope cannot write notes (a `read`
//! caller's, `crate::ai_chat`'s `change_scope`) is offered none of these script tools.
//!
//! Every script run answers the model `{ ok, result | error, kind, log, duration_ms,
//! instructions, script }`, and its `tool_result` summary carries `script`, which
//! `crate::ai_activity` records as an `ai.script` entry with the version that ran.

use std::sync::Arc;
use std::time::Duration;

use ai::{Content, Message, Role, ToolCall, ToolSpec};
use schema::{Agent, ScriptTool};
use script::{ALL_SECRETS, Host, Job, Kind};
use serde_json::{Value, json};
use state::Author;
use writer::Writer;

use crate::ai_chat::{ApplyError, ToolOut, str_arg, tool};
use crate::script_tools::{
    self, Checked, Ran, check_vault, hash, host_for, name_issues, skill_tool_name, skill_tool_names,
};

/// The tool that runs the agent's own wscript.
pub const RUN_SCRIPT: &str = "run_script";

/// `run_script`'s wall-clock limit (script-tools T1), as the Outbox's send handler's.
pub const CODE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The built-in skill that teaches wscript (script-tools T5).
pub const WSCRIPT_SKILL: &str = "wscript";

/// The tools that change a tool file, under the `tools` confirmation switch.
pub const PROPOSE_TOOLS: [&str; 2] = ["propose_tool_change", "propose_tool_delete"];

/// The most characters of a run's log a result carries.
const LOG_LIMIT: usize = 20_000;

/// The most characters of a run's log the activity log keeps.
const ACTIVITY_LOG: usize = 2_000;

/// The text the model reads before writing a tool file.
const FORMAT: &str = "A script tool is a file tools/<name>.wcl, <name> a lowercase identifier ([a-z][a-z0-9_]*) that is not a built-in tool's name, and the script it names, scripts/<script>.ws. Fields: description (required: what the model reads, what the tool does and when to call it), script (required: the script's name), function (required: the exported function, which takes ONE argument, a Value map of the parameters, and returns Result[Value, string] or any value), allow_domains (list of host names the http module may reach, [\"*\"] any), secrets (list of secret names it may read and write), timeout (seconds, default 30, at most 300). Each parameter is a block `param <name> { kind = \"string|int|float|bool|json|list\"; of = \"string|int|float|bool\" (lists only); description = \"...\"; required = true }`; an optional parameter left out arrives as null, a json one parsed. The script runs with the vault, value, json, math, regex, http, ai, secrets and fs (no directories) modules, and its changes are made as the agent that calls it. The wscript skill (use_skill wscript) teaches the language.";

fn spec_of(name: &str, description: &str, properties: Value) -> ToolSpec {
    tool(name, description, properties)
}

/// The read and propose tools, in every agent's list like any other.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        spec_of(
            "list_tools",
            "List the vault's script tools: each tool's name, description, script, function, parameters, hosts, secrets, timeout, and whether it is valid with its problems. Only valid tools are offered as tools.",
            json!({}),
        ),
        spec_of(
            "get_tool",
            "Read one script tool: its tool file's WCL text as stored, the tool parsed, its problems, the source of the script it names, and the tool-file vocabulary. Read it before proposing a change.",
            json!({ "name": { "type": "string", "description": "The tool's name." } }),
        ),
        spec_of(
            "propose_tool_change",
            "Propose a script tool for the user to apply: a new tool under a new name or a change to an existing one, its tool file and, when given, the whole source of the script it names, written together. It is checked as the tool write would check it, and the user sees the WCL and the script diff and applies or discards it. When the user lets tool changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The tool's name: an existing tool to change, or a new name ([a-z][a-z0-9_]*) to create one." },
                "wcl": { "type": "string", "description": "The whole tool file as WCL text." },
                "script_source": { "type": ["string", "null"], "description": "The whole wscript source of the script the tool names, written as scripts/<script>.ws; null to leave the script as it is." },
            }),
        ),
        spec_of(
            "propose_tool_delete",
            "Propose deleting a script tool for the user to apply. Its script is deleted too when no handler, task, device file or other tool names it. When the user lets tool changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({ "name": { "type": "string", "description": "The tool's name." } }),
        ),
    ]
}

/// `run_script`.
pub fn run_script_spec() -> ToolSpec {
    spec_of(
        RUN_SCRIPT,
        "Run wscript code on the vault: `code` is a whole wscript unit (with its `use` lines), and its function `function` (`main` when null) is called with `args`, each element arriving as a `Value` parameter (declare them `name: Value`). It runs with full access: every vault function, http to any host, the ai module, every script secret by name, for up to 5 minutes and 50 million instructions. Changes are made at once, as you, and recorded in history. Use it for bulk changes and lookups the other tools cannot do; read the wscript skill (use_skill wscript) before writing code, and do a dry run that changes nothing first. Answers the function's return value as JSON, its log (vault::log, print), the time it took and the instructions it used, or the compile or run-time error to fix.",
        json!({
            "code": { "type": "string", "description": "The whole wscript unit." },
            "function": { "type": ["string", "null"], "description": "The function to call; null for main." },
            "args": { "type": ["string", "null"], "description": "JSON text of an array, one element per argument of the function, each a Value; null for none." },
        }),
    )
}

/// What a script tool call runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Code,
    /// `run_saved_script`, `run_task`, `list_scripts` and `list_tasks` (`crate::ai_saved`).
    Saved,
    Task,
    ListScripts,
    ListTasks,
    Vault(String),
    Skill {
        skill: String,
        tool: String,
    },
}

/// The script tools a round offers: their specs, and what each name runs.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScriptTools {
    pub specs: Vec<ToolSpec>,
    pub targets: Vec<(String, Target)>,
}

impl ScriptTools {
    pub(crate) fn target(&self, name: &str) -> Option<&Target> {
        self.targets.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }

    fn add(&mut self, spec: ToolSpec, target: Target) {
        if self.target(&spec.name).is_some() {
            return;
        }
        self.targets.push((spec.name.clone(), target));
        self.specs.push(spec);
    }
}

fn tool_spec(name: &str, t: &ScriptTool) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: t.description.clone(),
        input_schema: t.input_schema(),
    }
}

/// The script tools `agent` is offered this round on `w`'s vault, with `opened` the skills it has
/// called `use_skill` on in the thread. None without a script host: the apps' turns run on the
/// home server.
pub(crate) fn offered(w: &Writer, agent: &Agent, opened: &[String]) -> ScriptTools {
    let mut out = ScriptTools::default();
    let Some(host) = host_for(w) else {
        return out;
    };
    let p = &agent.permissions;
    if p.scripts.run_code && p.allows_tool(RUN_SCRIPT) {
        out.add(run_script_spec(), Target::Code);
    }
    if p.scripts.runs_saved() && crate::script_tools::scheduler_for(w).is_some() {
        for spec in crate::ai_saved::specs() {
            let target = match spec.name.as_str() {
                crate::ai_saved::RUN_SAVED => Target::Saved,
                crate::ai_saved::RUN_TASK => Target::Task,
                crate::ai_saved::LIST_SCRIPTS => Target::ListScripts,
                _ => Target::ListTasks,
            };
            if p.allows_tool(&spec.name) {
                out.add(spec, target);
            }
        }
    }
    for c in script_tools::all_vault(w, Some(&host)) {
        if let Some(t) = c.tool.as_ref().filter(|_| c.valid())
            && p.allows_tool(&t.name)
        {
            out.add(tool_spec(&t.name, t), Target::Vault(t.name.clone()));
        }
    }
    if !crate::ai_skills::uses_skills(agent) {
        return out;
    }
    for skill in opened {
        if !p.allows_skill(skill) {
            continue;
        }
        let Ok(s) = w.skill(skill) else { continue };
        if !s.valid() {
            continue;
        }
        for tool in skill_tool_names(&s) {
            let Some(name) = skill_tool_name(skill, &tool) else {
                continue;
            };
            if !p.allows_tool(&name) {
                continue;
            }
            match script_tools::check_skill(w, Some(&host), skill, &tool) {
                Ok(c) if c.valid() => {
                    if let Some(t) = &c.tool {
                        out.add(
                            tool_spec(&name, t),
                            Target::Skill {
                                skill: skill.clone(),
                                tool,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The skills `use_skill` opened in `messages`: each call answered without an error, in order.
pub(crate) fn opened_skills(messages: &[Message]) -> Vec<String> {
    let mut calls: Vec<(String, String)> = Vec::new();
    let mut ok: Vec<String> = Vec::new();
    for m in messages {
        for c in &m.content {
            match c {
                Content::ToolCall { id, name, input } if m.role == Role::Assistant => {
                    if name == "use_skill"
                        && let Some(s) = input["name"].as_str()
                    {
                        calls.push((id.clone(), s.trim().to_owned()));
                    }
                }
                Content::ToolResult { id, is_error, .. } if !is_error => ok.push(id.clone()),
                _ => {}
            }
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (id, skill) in calls {
        if ok.contains(&id) && !out.contains(&skill) {
            out.push(skill);
        }
    }
    out
}

/// What the system text says about wscript when the agent can run it, so it reaches for the
/// `wscript` skill.
pub(crate) fn prompt_note(tools: &ScriptTools, skills: bool) -> Option<String> {
    if tools.specs.is_empty() {
        return None;
    }
    let code = tools.target(RUN_SCRIPT).is_some();
    let saved = tools.target(crate::ai_saved::RUN_SAVED).is_some()
        || tools.target(crate::ai_saved::RUN_TASK).is_some();
    let mut s = String::from("\n## Scripts\n\n");
    if code {
        s.push_str(
            "You can run wscript code on the vault with run_script, with full access: use it for \
             bulk changes and lookups, and do a dry run that changes nothing first. ",
        );
    }
    if saved {
        s.push_str(
            "You can run the vault's saved scripts by name with run_saved_script and start its \
             scheduled tasks now with run_task; list_scripts and list_tasks name them. ",
        );
    }
    if !code && !saved {
        s.push_str("Some of your tools are wscript script tools. ");
    }
    if skills {
        s.push_str(
            "Before writing wscript, or a script tool, read the wscript skill with use_skill: \
             it is the language and the vault API.\n",
        );
    } else {
        s.push('\n');
    }
    Some(s)
}

fn cut(s: &str, limit: usize) -> String {
    let total = s.chars().count();
    if total <= limit {
        return s.to_owned();
    }
    let kept: String = s.chars().take(limit).collect();
    format!("{kept}\n[log cut: {} more characters]", total - limit)
}

/// The model's answer and the summary of a run of tool `name`.
fn answer(name: &str, ran: &Ran) -> ToolOut {
    answer_as(name, ran, None)
}

/// [`answer`], with `outcome` in place of `ok` for a run that went well, such as a task still
/// running.
fn answer_as(name: &str, ran: &Ran, outcome: Option<&'static str>) -> ToolOut {
    let mut content = ran.to_json();
    content["log"] = json!(cut(&ran.log, LOG_LIMIT));
    let (outcome, kind, error) = match &ran.result {
        Ok(_) => (outcome.unwrap_or("ok"), None, None),
        Err((k, m)) => ("error", Some(*k), Some(m.clone())),
    };
    match kind {
        Some("instruction_limit") => {
            content["message"] = json!(
                "The script ran more than its limit of 50 million instructions, which stops runaway loops. Make one run do less, such as a page of notes at a time, or look for a loop that never ends."
            );
        }
        Some("timeout") => {
            content["message"] = json!(
                "The script ran past its time limit. Make one run do less, such as a page of notes at a time."
            );
        }
        Some("compile") => {
            content["message"] =
                json!("The code does not compile. Fix the errors and run it again.");
        }
        _ => {}
    }
    let summary_script = json!({
        "tool": name,
        "ref": ran.script,
        "outcome": outcome,
        "error_kind": kind,
        "error": error.as_deref().map(|e| state::one_line(e, 300)),
        "ms": ran.ms,
        "instructions": ran.instructions,
        "log": cut(&ran.log, ACTIVITY_LOG),
    });
    let mut summary = json!({ "notes": [], "script": summary_script });
    if let Some(e) = &error {
        summary["message"] = json!(state::one_line(e, 300));
    }
    ToolOut {
        content: content.to_string(),
        is_error: ran.result.is_err(),
        summary,
        proposal: None,
    }
}

/// `run_script`: the agent's own code, run with full access as the agent.
fn run_code(host: &Host, agent: &Agent, input: &Value) -> Ran {
    match code_job(agent, input) {
        Ok((job, script)) => script_tools::run(host, job, script),
        Err(refused) => refused,
    }
}

/// The arguments of a call's `args`: JSON text of an array (or the array), none when null or
/// blank. Refused with `script` as the reference otherwise.
pub(crate) fn parse_args(args: &Value, script: &Value) -> Result<Vec<Value>, Ran> {
    let refuse = |m: String| Err(Ran::refused("arguments", m, script.clone()));
    match args {
        Value::Null => Ok(Vec::new()),
        Value::Array(a) => Ok(a.clone()),
        Value::String(s) if s.trim().is_empty() => Ok(Vec::new()),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(a)) => Ok(a),
            Ok(_) => refuse("args must be JSON text of an array, one element per argument".into()),
            Err(e) => refuse(format!("args is not JSON text: {e}")),
        },
        _ => refuse("args must be JSON text of an array".into()),
    }
}

/// The job of `run_script` call `input` by `agent` and its script reference: full access, five
/// minutes, written as the agent. A call that cannot run is the refusal.
fn code_job(agent: &Agent, input: &Value) -> Result<(Job, Value), Ran> {
    let code = input["code"].as_str().unwrap_or_default();
    let script = json!({ "kind": "code", "hash": hash(code) });
    if code.trim().is_empty() {
        return Err(Ran::refused(
            "arguments",
            "code must be a wscript unit".into(),
            script,
        ));
    }
    let function = input["function"]
        .as_str()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or("main");
    let args = parse_args(&input["args"], &script)?;
    let mut job = Job::new(Kind::Task, RUN_SCRIPT, code, function);
    job.args = args;
    job.allow_domains = vec!["*".to_owned()];
    job.secrets = vec![ALL_SECRETS.to_owned()];
    job.author = Some(Author::Agent(agent.name.clone()));
    job.timeout = Some(CODE_TIMEOUT);
    Ok((job, script))
}

/// Run script tool call `call` of `agent`, which `target` names. Blocks until the script ends.
pub(crate) fn call(w: &Writer, agent: &Agent, target: &Target, call: &ToolCall) -> ToolOut {
    if let Some(e) = &call.error {
        return ToolOut::error(e.clone());
    }
    let Some(host) = host_for(w) else {
        return ToolOut::error("scripts run on the home server, which this one is not");
    };
    let author = Author::Agent(agent.name.clone());
    let scheduler = || script_tools::scheduler_for(w);
    let ran = match target {
        Target::Code => run_code(&host, agent, &call.input),
        Target::Saved => crate::ai_saved::run_saved(w, &host, agent, &call.input),
        Target::ListScripts => return crate::ai_saved::list_scripts(w, &host),
        Target::ListTasks => {
            return match scheduler() {
                Some(s) => crate::ai_saved::list_tasks(&s),
                None => ToolOut::error("tasks run on the home server, which this one is not"),
            };
        }
        Target::Task => {
            let Some(s) = scheduler() else {
                return ToolOut::error("tasks run on the home server, which this one is not");
            };
            let t = crate::ai_saved::run_task(&s, &call.input);
            let mut o = answer_as(&call.name, &t.ran, t.started.then_some("started"));
            if let Ok(mut c) = serde_json::from_str::<Value>(&o.content) {
                c["run_id"] = json!(t.run_id);
                c["started"] = json!(t.started);
                o.content = c.to_string();
            }
            return o;
        }
        Target::Vault(name) => script_tools::run_vault_tool(w, &host, name, &call.input, &author),
        Target::Skill { skill, tool } => {
            script_tools::run_skill_tool(w, &host, skill, tool, &call.input, &author)
        }
    };
    answer(&call.name, &ran)
}

// ---- reading and proposing tool files ---------------------------------------------------------

fn out(content: Value, names: Vec<Value>, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [], "tools": names }),
        proposal: proposal.map(|p| ("tool_proposal", p)),
    }
}

fn host(w: &Writer) -> Option<Arc<Host>> {
    host_for(w)
}

/// `list_tools`.
pub(crate) fn list(w: &Writer) -> Result<ToolOut, ToolOut> {
    let h = host(w);
    let all = script_tools::all_vault(w, h.as_deref());
    let names = all.iter().map(|c| json!(c.name)).collect();
    let tools: Vec<Value> = all.iter().map(Checked::summary).collect();
    Ok(out(json!({ "tools": tools }), names, None))
}

/// `get_tool`.
pub(crate) fn get(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = w
        .read_tool(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let c = check_vault(w, host(w).as_deref(), name, &wcl, None);
    let mut content = c.detail();
    content["format"] = json!(FORMAT);
    Ok(out(content, vec![json!(name)], None))
}

/// `propose_tool_change`: check the tool file as the tool write would and hand it, with the
/// script, to the user.
pub(crate) fn propose_change(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = input["wcl"]
        .as_str()
        .ok_or_else(|| ToolOut::error("wcl must be the whole tool file as WCL text"))?;
    let script_after = input["script_source"].as_str();
    let (after, tool) = w.check_tool(name, wcl, name_issues).map_err(|e| {
        let message = e.to_string();
        let details = crate::ApiError::from(e).body;
        let mut o = ToolOut::error(message.clone());
        o.content = json!({
            "error": message,
            "issues": details,
            "message": "The tool was not proposed. Fix the issues and propose it again; get_tool answers the tool-file vocabulary.",
        })
        .to_string();
        o
    })?;
    let before = w.read_tool(name).ok();
    let script_before = w.read_script(&tool.script).ok();
    let script_after = script_after.filter(|s| Some(*s) != script_before.as_deref());
    if before.as_deref() == Some(after.as_str()) && script_after.is_none() {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed tool and script are the same as the current ones: there is nothing to apply.",
            }),
            vec![json!(name)],
            None,
        ));
    }
    let checked = check_vault(
        w,
        host(w).as_deref(),
        name,
        &after,
        script_after.or(script_before.as_deref()),
    );
    let exists = before.is_some();
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": exists,
        "problems": checked.problems,
        "message": format!(
            "The tool is valid WCL and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it.{}",
            if checked.problems.is_empty() {
                String::new()
            } else {
                " Once applied it would not be offered until its problems are fixed.".to_owned()
            }
        ),
    });
    let proposal = json!({
        "id": call_id,
        "kind": "tool",
        "op": if exists { "change" } else { "create" },
        "name": name,
        "exists": exists,
        "before": before,
        "after": after,
        "script": tool.script,
        "script_exists": script_before.is_some(),
        "script_before": script_before,
        "script_after": script_after,
        "script_delete": false,
        "problems": checked.problems,
        "proposer": by.name,
    });
    Ok(out(content, vec![json!(name)], Some(proposal)))
}

/// `propose_tool_delete`.
pub(crate) fn propose_delete(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let before = w
        .read_tool(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let takes = w
        .tool_delete_takes_script(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let script = ScriptTool::parse(name, &before).ok().map(|t| t.script);
    let script_before = takes.as_deref().and_then(|s| w.read_script(s).ok());
    let proposal = json!({
        "id": call_id,
        "kind": "tool",
        "op": "delete",
        "name": name,
        "exists": true,
        "before": before,
        "after": null,
        "script": script,
        "script_exists": script_before.is_some(),
        "script_before": script_before,
        "script_after": null,
        "script_delete": takes.is_some(),
        "problems": [],
        "proposer": by.name,
    });
    let script_line = match &takes {
        Some(s) => format!(" Its script {s} goes with it: nothing else names it."),
        None => match &script {
            Some(s) => format!(" Its script {s} stays: something else names it."),
            None => String::new(),
        },
    };
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "script_deleted": takes,
            "message": format!("The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.{script_line}"),
        }),
        vec![json!(name)],
        Some(proposal),
    ))
}

/// Apply a `tool_proposal`, when the tool file and the script are still the proposal's
/// `before` and `script_before`. Answers `{op, tool_name, script_name}`.
pub(crate) fn apply_tool(w: &Writer, p: &Value) -> Result<Value, ApplyError> {
    let name = p["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApplyError::Invalid("the change has no name".into()))?;
    let stale = || {
        ApplyError::Stale(format!(
            "the tool {name} or its script changed since the proposal, so nothing was written"
        ))
    };
    if w.read_tool(name).ok().as_deref() != p["before"].as_str() {
        return Err(stale());
    }
    let script = p["script"].as_str().filter(|s| schema::is_name(s));
    if p["op"] == "delete" {
        let deleted = w.delete_tool(name)?;
        return Ok(json!({ "op": "tool_delete", "tool_name": name, "script_name": deleted }));
    }
    let after = p["after"]
        .as_str()
        .ok_or_else(|| ApplyError::Invalid("the change has no after".into()))?;
    let script_after = p["script_after"].as_str();
    if script_after.is_some()
        && let Some(s) = script
        && w.read_script(s).ok().as_deref() != p["script_before"].as_str()
    {
        return Err(stale());
    }
    w.put_tool(name, after, script_after, name_issues)?;
    let op = if p["before"].is_string() {
        "tool_change"
    } else {
        "tool_create"
    };
    Ok(json!({
        "op": op,
        "tool_name": name,
        "script_name": script_after.and(script),
    }))
}

/// What the model is told a made tool change did.
pub(crate) fn made_line(done: &Value) -> String {
    let name = done["tool_name"].as_str().unwrap_or_default();
    let script = done["script_name"].as_str();
    match (done["op"].as_str().unwrap_or_default(), script) {
        ("tool_create", Some(s)) => format!("The tool {name} was created, with its script {s}."),
        ("tool_create", None) => format!("The tool {name} was created."),
        ("tool_delete", Some(s)) => format!("The tool {name} and its script {s} were deleted."),
        ("tool_delete", None) => format!("The tool {name} was deleted."),
        (_, Some(s)) => format!("The tool {name} and its script {s} were changed."),
        _ => format!("The tool {name} was changed."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(calls: &[(&str, &str)]) -> Message {
        Message {
            role: Role::Assistant,
            content: calls
                .iter()
                .map(|(id, skill)| Content::ToolCall {
                    id: (*id).to_owned(),
                    name: "use_skill".to_owned(),
                    input: json!({ "name": skill }),
                })
                .collect(),
            provider_state: None,
        }
    }

    #[test]
    fn opened_skills_are_the_use_skill_calls_answered_without_an_error() {
        let messages = vec![
            assistant(&[("a", "brand"), ("b", "missing")]),
            Message::tool_results(vec![
                Content::tool_result("a".to_owned(), "{}".to_owned(), false),
                Content::tool_result("b".to_owned(), "{}".to_owned(), true),
            ]),
            assistant(&[("c", "brand"), ("d", "pdf")]),
            Message::tool_results(vec![
                Content::tool_result("c".to_owned(), "{}".to_owned(), false),
                Content::tool_result("d".to_owned(), "{}".to_owned(), false),
            ]),
        ];
        assert_eq!(opened_skills(&messages), ["brand", "pdf"]);
        assert!(opened_skills(&messages[..1]).is_empty());
    }

    #[test]
    fn run_script_has_full_access_five_minutes_and_writes_as_the_agent() {
        let agent = Agent::new("scripter");
        let input =
            json!({ "code": "fn go(n: Value) -> Value { n }", "function": "go", "args": "[3]" });
        let (job, script) = code_job(&agent, &input).unwrap();
        assert_eq!(job.timeout, Some(Duration::from_secs(300)));
        assert_eq!(CODE_TIMEOUT, Duration::from_secs(5 * 60));
        assert_eq!(job.allow_domains, ["*"]);
        assert_eq!(job.secrets, [ALL_SECRETS]);
        assert!(job.read_paths.is_empty() && job.write_paths.is_empty());
        assert_eq!(job.author, Some(Author::Agent("scripter".into())));
        assert_eq!(job.kind, Kind::Task);
        assert_eq!(job.function, "go");
        assert_eq!(job.args, [json!(3)]);
        assert_eq!(script["kind"], "code");
        assert_eq!(script["hash"], hash("fn go(n: Value) -> Value { n }"));
        // The defaults: main, no arguments.
        let (job, _) = code_job(
            &agent,
            &json!({ "code": "fn main() {}", "function": null, "args": null }),
        )
        .unwrap();
        assert_eq!(job.function, "main");
        assert!(job.args.is_empty());
        let bad = code_job(
            &agent,
            &json!({ "code": "fn main() {}", "args": "{\"a\": 1}" }),
        )
        .unwrap_err();
        assert_eq!(bad.result.unwrap_err().0, "arguments");
    }

    #[test]
    fn a_run_answers_its_log_cut_and_its_script_in_the_summary() {
        let ran = Ran {
            result: Err(("instruction_limit", "stopped".to_owned())),
            log: "x".repeat(LOG_LIMIT + 10),
            ms: 7,
            instructions: 50,
            script: json!({ "kind": "code", "hash": "ab" }),
        };
        let o = answer(RUN_SCRIPT, &ran);
        assert!(o.is_error);
        let c: Value = serde_json::from_str(&o.content).unwrap();
        assert_eq!(c["kind"], "instruction_limit");
        assert!(c["message"].as_str().unwrap().contains("50 million"));
        assert!(c["log"].as_str().unwrap().contains("[log cut: 10 more"));
        assert_eq!(o.summary["script"]["ref"]["hash"], "ab");
        assert_eq!(o.summary["script"]["outcome"], "error");
        assert_eq!(o.summary["script"]["instructions"], 50);
    }
}
