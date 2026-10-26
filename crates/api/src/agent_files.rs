//! Agent files, `agents/<name>.wcl` (agents G1, G2, G4).
//!
//! - `GET /api/agents`: every agent's summary, the built-in Assistant first, then by name, with
//!   `running`, how many turns run on its threads now (agents G17), and `tools`, the name of
//!   every tool `permissions.tools` may name, for the agent screen.
//! - `GET /api/agents/{name}`: the parsed agent and its WCL, or the WCL alone when `Accept` asks
//!   for text.
//! - `PUT /api/agents/{name}` (admin): the agent file as WCL text, JSON `{ "wcl": "..." }`, or the
//!   JSON form `GET` answers (without `wcl`), which is written as canonical WCL. Every problem is
//!   a 400 `invalid` with its issues; disabling the built-in agent is a 400 `built_in`.
//! - `DELETE /api/agents/{name}` (admin): removes the file. Its threads stay. The built-in agent
//!   is a 400 `built_in`.
//!
//! Agent files are definitions, so changing one needs `admin`, as types and tasks do
//! (`auth::required_scope` answers `admin` for any write it does not list). A paired device's
//! `write` token may also write them, so the apps edit agents as the web app does (see
//! `auth::device_writes_agent`).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use schema::{
    Agent, AgentPermissions, Invalid, Issue, MentionTrigger, NoteEvent, Schedule, Trigger,
    TriggerWhen,
};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::Writer;

use crate::defs::{body, text_or_json};
use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// The name of every built-in tool the chat can offer, which `permissions.tools` may name, with
/// `run_script` (script-tools T1) and the tools that run saved scripts and tasks.
pub fn tool_names() -> Vec<String> {
    crate::ai_chat::tool_specs()
        .into_iter()
        .chain([
            crate::brave::tool_spec(),
            crate::ai_scripts::run_script_spec(),
        ])
        .chain(crate::ai_saved::specs())
        .map(|t| t.name)
        .collect()
}

/// Every name `permissions.tools` may name in `w`'s vault: the built-in tools and the vault's
/// script tools (script-tools T2).
pub(crate) fn known_tools(w: &Writer) -> Vec<String> {
    let mut known = tool_names();
    for (name, _) in w.tool_files().unwrap_or_default() {
        if !known.contains(&name) {
            known.push(name);
        }
    }
    known
}

/// The issues for each tool an agent names that `known` lacks. A skill's tool,
/// `<skill>__<tool>`, is taken as it is: the skill's tools are read when the agent opens it
/// (script-tools T4).
fn unknown_in(known: &[String], agent: &Agent) -> Vec<Issue> {
    agent
        .permissions
        .tools
        .iter()
        .flatten()
        .filter(|t| !known.contains(t) && !t.contains(crate::script_tools::SKILL_TOOL_SEPARATOR))
        .map(|t| {
            Issue::new(
                "permissions.tools",
                format!("there is no tool {t:?}; the tools are {}", known.join(", ")),
            )
        })
        .collect()
}

/// The check of the tools an agent file names, against the tools of `w`'s vault, for
/// [`Writer::check_agent`] and [`Writer::put_agent`].
pub(crate) fn tool_check(w: &Writer) -> impl FnOnce(&Agent) -> Vec<Issue> + use<> {
    let known = known_tools(w);
    move |agent| unknown_in(&known, agent)
}

/// The parsed agent as the routes answer it, and as `PUT` takes it back.
pub fn agent_json(a: &Agent) -> Value {
    let triggers: Vec<Value> = a.triggers.iter().map(trigger_json).collect();
    json!({
        "name": a.name,
        "label": a.label,
        "description": a.description,
        "icon": a.icon,
        "enabled": a.enabled,
        "account": a.account_ref(),
        "model": a.model,
        "effort": a.effort,
        "system_prompt": a.system_prompt,
        "built_in": a.is_built_in(),
        "permissions": permissions_json(&a.permissions),
        "triggers": triggers,
    })
}

fn permissions_json(p: &AgentPermissions) -> Value {
    let confirm: serde_json::Map<String, Value> = p
        .confirm
        .switches()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect();
    let web: serde_json::Map<String, Value> = p
        .web
        .switches()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect();
    let outbox: serde_json::Map<String, Value> = p
        .outbox
        .switches()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect();
    let scripts: serde_json::Map<String, Value> = p
        .scripts
        .switches()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect();
    json!({
        "tools": p.tools,
        "types": p.types,
        "confirm": confirm,
        "web": web,
        "outbox": outbox,
        "browser": p.browser,
        "skills": p.skills,
        "scripts": scripts,
        "subagents": {
            "allow": p.subagents.allow,
            "max": p.subagents.max,
            "agents": p.subagents.agents,
        },
    })
}

fn trigger_json(t: &Trigger) -> Value {
    match &t.when {
        TriggerWhen::Schedule(s) => json!({
            "name": t.name,
            "kind": "schedule",
            "schedule": s.cron,
            "timezone": s.timezone,
            "prompt": t.prompt,
        }),
        TriggerWhen::NoteEvent(e) => json!({
            "name": t.name,
            "kind": "event",
            "on": e.on,
            "type": e.types,
            "where": e.condition,
            "prompt": t.prompt,
        }),
        TriggerWhen::Mention(m) => json!({
            "name": t.name,
            "kind": "mention",
            "type": m.types,
            "prompt": t.prompt,
        }),
    }
}

/// What a note may call the agent by (agents G16): `null` when it is disabled, else `{types}`,
/// the types whose notes may call it, `null` for every type (also when it has no mention trigger,
/// since every enabled agent can be called).
fn mention_json(a: &Agent) -> Value {
    if !a.enabled {
        return Value::Null;
    }
    let mut types: Vec<&str> = Vec::new();
    for t in &a.triggers {
        if let TriggerWhen::Mention(m) = &t.when {
            if m.types.is_empty() {
                return json!({ "types": null });
            }
            types.extend(m.types.iter().map(String::as_str));
        }
    }
    // No mention trigger of its own: every type may call it (agents G16).
    if types.is_empty() {
        return json!({ "types": null });
    }
    types.sort_unstable();
    types.dedup();
    json!({ "types": types })
}

/// One line of `GET /api/agents`.
fn summary_json(name: &str, agent: &Result<Agent, Invalid>) -> Value {
    match agent {
        Ok(a) => json!({
            "name": a.name,
            "label": a.label,
            "description": a.description,
            "icon": a.icon,
            "account": a.account_ref(),
            "model": a.model,
            "enabled": a.enabled,
            "triggers": a.triggers.len(),
            "mention": mention_json(a),
            "built_in": a.is_built_in(),
        }),
        Err(e) => json!({
            "name": name,
            "label": name,
            "built_in": name == schema::ASSISTANT_AGENT,
            "error": e.to_string(),
        }),
    }
}

/// `GET /api/agents`.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let w2 = w.clone();
    let mut agents = blocking(w, |w| {
        let all = w.agents()?;
        Ok::<_, ApiError>(
            all.into_iter()
                .map(|(n, a)| {
                    let account = a
                        .as_ref()
                        .ok()
                        .and_then(|a| crate::ai_settings::agent_account_name(w, a));
                    (n, a, account)
                })
                .collect::<Vec<_>>(),
        )
    })
    .await?;
    agents.sort_by_key(|(name, _, _)| (name != schema::ASSISTANT_AGENT, name.clone()));
    let agents: Vec<Value> = agents
        .iter()
        .map(|(n, a, account)| {
            let mut v = summary_json(n, a);
            if a.is_ok() {
                v["account"] = json!(account);
            }
            v["running"] = json!(crate::agent_threads::running_of(n).len());
            v
        })
        .collect();
    let tools = blocking(w2, |w| Ok::<_, ApiError>(known_tools(w))).await?;
    Ok(Json(json!({ "agents": agents, "tools": tools })))
}

/// The response to a read or a write: the agent with its WCL, and the name of the account it
/// names, a legacy provider resolved to the account made for it (accounts A6).
fn agent_response(
    headers: &HeaderMap,
    agent: &Agent,
    wcl: String,
    account: Option<String>,
) -> Response {
    let mut out = agent_json(agent);
    out["account"] = json!(account);
    out["wcl"] = json!(wcl);
    text_or_json(headers, wcl, out)
}

/// `GET /api/agents/{name}`.
pub async fn read(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // A file that does not parse is a 400 `invalid` with its issues and its text, so the File
    // tab can show it to be fixed.
    let (wcl, agent, account) = blocking(w, move |w| {
        let wcl = w.read_agent(&name)?;
        let agent = Agent::parse(&name, &wcl).map_err(|e| {
            let mut err = ApiError::from(writer::Error::Invalid(e));
            err.body["wcl"] = json!(wcl);
            err
        })?;
        let account = crate::ai_settings::agent_account_name(w, &agent);
        Ok::<_, ApiError>((wcl, agent, account))
    })
    .await?;
    Ok(agent_response(&headers, &agent, wcl, account))
}

/// The JSON body of `PUT /api/agents/{name}`: `wcl`, or the form `GET` answers. `name`, `kind`
/// and `built_in` are what `GET` answers and are ignored.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentBody {
    wcl: Option<String>,
    #[serde(default, rename = "name")]
    _name: Option<Value>,
    #[serde(default, rename = "built_in")]
    _built_in: Option<Value>,
    label: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    enabled: Option<bool>,
    account: Option<String>,
    /// From a client of before accounts: kept as the file's legacy `provider`.
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    system_prompt: Option<String>,
    #[serde(default)]
    permissions: PermissionsForm,
    #[serde(default)]
    triggers: Vec<TriggerForm>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PermissionsForm {
    tools: Option<Vec<String>>,
    types: Option<Vec<String>>,
    #[serde(default)]
    confirm: BTreeMap<String, bool>,
    #[serde(default)]
    web: BTreeMap<String, bool>,
    #[serde(default)]
    outbox: BTreeMap<String, bool>,
    /// Browser automation (browser B6); absent is off.
    #[serde(default)]
    browser: bool,
    /// The skills offered (skills S6); absent is every valid skill.
    #[serde(default)]
    skills: Option<Vec<String>>,
    /// The script switches (script-tools T7); absent is every switch off.
    #[serde(default)]
    scripts: BTreeMap<String, bool>,
    /// Sub agents (subagents U2); absent is off.
    #[serde(default)]
    subagents: Option<SubagentsForm>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SubagentsForm {
    #[serde(default)]
    allow: bool,
    #[serde(default)]
    max: Option<i64>,
    #[serde(default)]
    agents: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerForm {
    name: String,
    /// `mention` makes a mention trigger; the other kinds are told apart by their fields.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    prompt: String,
    schedule: Option<String>,
    timezone: Option<String>,
    on: Option<Vec<String>>,
    #[serde(rename = "type")]
    types: Option<Vec<String>>,
    #[serde(rename = "where")]
    condition: Option<String>,
}

/// The JSON form as agent `name`. What the form cannot say at all, such as a trigger that is
/// neither a schedule nor a note event, is an issue; the rest is checked once the agent is
/// written out as WCL and read back.
fn from_form(name: &str, f: AgentBody) -> Result<Agent, Vec<Issue>> {
    let mut issues = Vec::new();
    let mut agent = Agent::new(name);
    if let Some(label) = f.label {
        agent.label = label;
    }
    agent.description = f.description;
    agent.icon = f.icon;
    agent.enabled = f.enabled.unwrap_or(true);
    agent.account = f.account.filter(|a| !a.trim().is_empty());
    agent.provider = f.provider;
    agent.model = f.model;
    agent.effort = f.effort;
    agent.system_prompt = f.system_prompt;
    let p = &mut agent.permissions;
    p.tools = f.permissions.tools;
    p.types = f.permissions.types;
    p.browser = f.permissions.browser;
    p.skills = f.permissions.skills;
    for (k, on) in f.permissions.confirm {
        if !p.confirm.set(&k, on) {
            issues.push(Issue::new(
                format!("permissions.confirm.{k}"),
                "is not a switch",
            ));
        }
    }
    for (k, on) in f.permissions.web {
        if !p.web.set(&k, on) {
            issues.push(Issue::new(
                format!("permissions.web.{k}"),
                "is not a switch",
            ));
        }
    }
    for (k, on) in f.permissions.outbox {
        if !p.outbox.set(&k, on) {
            issues.push(Issue::new(
                format!("permissions.outbox.{k}"),
                "is not a switch",
            ));
        }
    }
    for (k, on) in f.permissions.scripts {
        if !p.scripts.set(&k, on) {
            issues.push(Issue::new(
                format!("permissions.scripts.{k}"),
                "is not a switch",
            ));
        }
    }
    if let Some(sub) = f.permissions.subagents {
        p.subagents.allow = sub.allow;
        p.subagents.agents = sub.agents;
        if let Some(max) = sub.max {
            // Out of range, the file written out says so when it is read back.
            match u8::try_from(max) {
                Ok(m) if (1..=schema::SUBAGENTS_MAX).contains(&m) => p.subagents.max = m,
                _ => issues.push(Issue::new(
                    "permissions.subagents.max",
                    format!("must be a whole number from 1 to {}", schema::SUBAGENTS_MAX),
                )),
            }
        }
    }
    for t in f.triggers {
        let at = format!("trigger {}", t.name);
        if t.kind.as_deref() == Some("mention") {
            if t.schedule.is_some() || t.on.is_some() {
                issues.push(Issue::new(
                    at,
                    "has mention with schedule or on: a trigger is one kind only",
                ));
                continue;
            }
            agent.triggers.push(Trigger {
                name: t.name,
                prompt: if t.prompt.trim().is_empty() {
                    String::new()
                } else {
                    t.prompt
                },
                when: TriggerWhen::Mention(MentionTrigger {
                    types: t.types.unwrap_or_default(),
                }),
            });
            continue;
        }
        let when = match (t.schedule, t.on) {
            (Some(cron), None) => TriggerWhen::Schedule(Schedule {
                cron,
                timezone: t.timezone.unwrap_or_else(|| "UTC".to_owned()),
            }),
            (None, Some(on)) => TriggerWhen::NoteEvent(NoteEvent {
                on,
                types: t.types.unwrap_or_default(),
                condition: t.condition.filter(|c| !c.trim().is_empty()),
            }),
            (Some(_), Some(_)) => {
                issues.push(Issue::new(
                    at,
                    "has both schedule and on: a trigger is one or the other",
                ));
                continue;
            }
            (None, None) => {
                issues.push(Issue::new(
                    at,
                    "needs schedule, a cron expression, or on, a list of note changes",
                ));
                continue;
            }
        };
        agent.triggers.push(Trigger {
            name: t.name,
            prompt: t.prompt,
            when,
        });
    }
    if issues.is_empty() {
        Ok(agent)
    } else {
        Err(issues)
    }
}

/// `PUT /api/agents/{name}`: create or replace an agent file.
pub async fn put(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: AgentBody = body(&headers, &raw, |wcl| AgentBody {
        wcl: Some(wcl),
        ..AgentBody::default()
    })?;
    let source = match b.wcl {
        Some(wcl) => wcl,
        None => {
            let n = name.clone();
            from_form(&name, b)
                .map_err(|issues| {
                    writer::Error::Invalid(Invalid {
                        file: writer::agent_path(&n),
                        issues,
                    })
                })?
                .to_wcl()
        }
    };
    let (wcl, agent, account) = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        let check = tool_check(w);
        let (wcl, agent) = w.put_agent(&name, &source, check)?;
        let account = crate::ai_settings::agent_account_name(w, &agent);
        Ok::<_, ApiError>((wcl, agent, account))
    })
    .await?;
    Ok(agent_response(&headers, &agent, wcl, account))
}

/// `DELETE /api/agents/{name}`.
pub async fn delete(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.delete_agent(&n)
    })
    .await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}
