//! Agent files, `agents/<name>.wcl` (agents G1, G2, G4).
//!
//! - `GET /api/agents`: every agent's summary, the built-in Assistant first, then by name, and
//!   `tools`, the name of every tool `permissions.tools` may name, for the agent screen.
//! - `GET /api/agents/{name}`: the parsed agent and its WCL, or the WCL alone when `Accept` asks
//!   for text.
//! - `PUT /api/agents/{name}` (admin): the agent file as WCL text, JSON `{ "wcl": "..." }`, or the
//!   JSON form `GET` answers (without `wcl`), which is written as canonical WCL. Every problem is
//!   a 400 `invalid` with its issues; disabling the built-in agent is a 400 `built_in`.
//! - `DELETE /api/agents/{name}` (admin): removes the file. Its threads stay. The built-in agent
//!   is a 400 `built_in`.
//!
//! Agent files are definitions, so changing one needs `admin`, as types and tasks do
//! (`auth::required_scope` answers `admin` for any write it does not list).

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
use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

/// The name of every tool the chat can offer, which `permissions.tools` may name.
pub fn tool_names() -> Vec<String> {
    crate::ai_chat::tool_specs()
        .into_iter()
        .chain([crate::brave::tool_spec()])
        .map(|t| t.name)
        .collect()
}

/// The issues for each tool `agent` names that the server does not have.
pub(crate) fn unknown_tools(agent: &Agent) -> Vec<Issue> {
    let known = tool_names();
    agent
        .permissions
        .tools
        .iter()
        .flatten()
        .filter(|t| !known.contains(t))
        .map(|t| {
            Issue::new(
                "permissions.tools",
                format!("there is no tool {t:?}; the tools are {}", known.join(", ")),
            )
        })
        .collect()
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
        "provider": a.provider,
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
    json!({ "tools": p.tools, "types": p.types, "confirm": confirm, "web": web })
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

/// What a note may call the agent by (agents G16): `null` when it has no mention trigger or is
/// disabled, else `{types}`, the types whose notes may call it, `null` for every type.
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
    if types.is_empty() {
        return Value::Null;
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
            "provider": a.provider,
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
    let mut agents = blocking(w, |w| w.agents()).await?;
    agents.sort_by_key(|(name, _)| (name != schema::ASSISTANT_AGENT, name.clone()));
    let agents: Vec<Value> = agents.iter().map(|(n, a)| summary_json(n, a)).collect();
    Ok(Json(json!({ "agents": agents, "tools": tool_names() })))
}

/// The response to a read or a write: the agent with its WCL.
fn agent_response(headers: &HeaderMap, agent: &Agent, wcl: String) -> Response {
    let mut out = agent_json(agent);
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
    let (wcl, agent) = blocking(w, move |w| {
        let wcl = w.read_agent(&name)?;
        let agent = Agent::parse(&name, &wcl).map_err(|e| {
            let mut err = ApiError::from(writer::Error::Invalid(e));
            err.body["wcl"] = json!(wcl);
            err
        })?;
        Ok::<_, ApiError>((wcl, agent))
    })
    .await?;
    Ok(agent_response(&headers, &agent, wcl))
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
    agent.provider = f.provider;
    agent.model = f.model;
    agent.effort = f.effort;
    agent.system_prompt = f.system_prompt;
    let p = &mut agent.permissions;
    p.tools = f.permissions.tools;
    p.types = f.permissions.types;
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
    let (wcl, agent) = blocking(w, move |w| w.put_agent(&name, &source, unknown_tools)).await?;
    Ok(agent_response(&headers, &agent, wcl))
}

/// `DELETE /api/agents/{name}`.
pub async fn delete(State(w): Shared, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    blocking(w, move |w| w.delete_agent(&n)).await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}
