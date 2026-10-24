//! The chat's agent tools (agents G20): `list_agents`, `get_agent`, `propose_agent_change` and
//! `propose_agent_delete`, and the apply of an `agent_proposal`.
//!
//! A proposal is checked with the writer's dry run ([`Writer::check_agent`], with the tool names
//! the server has), so it meets the refusals `PUT /api/agents/{name}` gives, and then against the
//! agent proposing it ([`escalation`]): an agent never writes an agent that could do more than
//! itself, and never changes or deletes its own file. Each refusal goes back to the model as an
//! `is_error` result. The proposal records its `proposer`, so `POST /api/ai/apply` checks it again
//! against that agent as it stands then. Applied, a change goes through [`Writer::put_agent`] and
//! a delete through [`Writer::delete_agent`], after checking the file is still the `before` the
//! proposal showed.

use std::sync::OnceLock;

use schema::{ASSISTANT_AGENT, Agent};
use serde_json::{Value, json};
use writer::Writer;

use crate::ai_chat::{ApplyError, Limits, ToolOut, VAULT_FORMAT, str_arg};

/// The refusal of a change to the proposing agent's own file.
pub const OWN_FILE: &str = "An agent cannot change its own file.";

/// An agent tool's answer: `content` for the model, and the agent names for the summary.
fn out(content: Value, names: Vec<Value>, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [], "agents": names }),
        proposal: proposal.map(|p| ("agent_proposal", p)),
    }
}

/// A refusal from the writer as an `is_error` result: its message, the issues the API would
/// answer, and what the model should do.
fn refused(e: writer::Error, then: &str) -> ToolOut {
    let message = e.to_string();
    let details = crate::ApiError::from(e).body;
    let mut out = ToolOut::error(message.clone());
    out.content = json!({ "error": message, "issues": details, "message": then }).to_string();
    out
}

/// The "Agent files" section of `docs/vault-format.md`, which an agent file is written from.
pub fn agent_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        let start = VAULT_FORMAT
            .find("\n## Agent files\n")
            .unwrap_or(VAULT_FORMAT.len());
        let end = VAULT_FORMAT[start..]
            .find("\n## Device files\n")
            .map_or(VAULT_FORMAT.len(), |e| start + e);
        format!("{}\n", VAULT_FORMAT[start..end].trim())
    })
}

/// `list_agents`: each agent's name, label, description, provider, model, whether it is enabled
/// and built in, and its trigger count. A file that does not parse shows its error.
pub(crate) fn list(w: &Writer) -> Result<ToolOut, ToolOut> {
    let mut agents = w.agents().map_err(|e| ToolOut::error(e.to_string()))?;
    agents.sort_by_key(|(name, _)| (name != ASSISTANT_AGENT, name.clone()));
    let rows: Vec<Value> = agents
        .iter()
        .map(|(name, a)| match a {
            Ok(a) => json!({
                "name": a.name,
                "label": a.label,
                "description": a.description,
                "provider": a.provider,
                "model": a.model,
                "enabled": a.enabled,
                "builtin": a.is_built_in(),
                "triggers": a.triggers.len(),
            }),
            Err(e) => json!({
                "name": name,
                "builtin": name == ASSISTANT_AGENT,
                "error": e.to_string(),
            }),
        })
        .collect();
    let names = agents.iter().map(|(n, _)| json!(n)).collect();
    Ok(out(json!({ "agents": rows }), names, None))
}

/// `get_agent`: the file as stored, the agent parsed, and the agent-file vocabulary.
pub(crate) fn get(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = w
        .read_agent(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let (agent, error) = match Agent::parse(name, &wcl) {
        Ok(a) => (crate::agent_files::agent_json(&a), Value::Null),
        Err(e) => (Value::Null, json!(e.to_string())),
    };
    Ok(out(
        json!({
            "name": name,
            "wcl": wcl,
            "agent": agent,
            "error": error,
            "format": agent_format_text(),
        }),
        vec![json!(name)],
        None,
    ))
}

/// What `target` could do that `by` cannot (agents G20), one line each; empty when `target`
/// stays within `by`'s permissions. An absent tools or types list counts as every tool or type,
/// so an agent with a list cannot write an agent without one.
pub fn escalation(by: &Agent, target: &Agent) -> Vec<String> {
    let (bp, tp) = (&by.permissions, &target.permissions);
    let mut problems = Vec::new();
    match (&bp.tools, &tp.tools) {
        (Some(_), None) => problems.push(
            "it would allow every tool, and this agent may use only the tools its permissions list"
                .to_owned(),
        ),
        (Some(_), Some(tools)) => {
            for t in tools.iter().filter(|t| !bp.allows_tool(t)) {
                problems.push(format!(
                    "it would allow the tool {t}, which this agent does not have"
                ));
            }
        }
        (None, _) => {}
    }
    let limits = Limits::of(by);
    match (&bp.types, &tp.types) {
        (Some(_), None) => problems.push(
            "it would reach every note type, and this agent reaches only the types its permissions list"
                .to_owned(),
        ),
        (_, Some(types)) => {
            for t in types.iter().filter(|t| !limits.allows(t)) {
                problems.push(format!(
                    "it would reach {t} notes, which this agent may not"
                ));
            }
        }
        (None, None) => {}
    }
    for ((name, on), (_, mine)) in tp.confirm.switches().into_iter().zip(bp.confirm.switches()) {
        if !on && mine {
            problems.push(format!(
                "it would turn off the confirmation of {name} changes, which this agent must ask before making"
            ));
        }
    }
    for ((name, on), (_, mine)) in tp.web.switches().into_iter().zip(bp.web.switches()) {
        if on && !mine {
            problems.push(format!(
                "it would turn on the Internet switch {name}, which is off for this agent"
            ));
        }
    }
    // The Outbox bypass (outbox X11): only an agent that sends without review may give it.
    for ((name, on), (_, mine)) in tp.outbox.switches().into_iter().zip(bp.outbox.switches()) {
        if on && !mine {
            problems.push(format!(
                "it would turn on the Outbox switch {name}, which is off for this agent"
            ));
        }
    }
    // Browser automation (browser B6): only an agent that has it may give it.
    if tp.browser && !bp.browser {
        problems.push(
            "it would turn on Browser automation (permissions.browser), which is off for this agent"
                .to_owned(),
        );
    }
    // Skills (skills S6): only those this agent is offered itself; an absent list is every skill.
    match (&bp.skills, &tp.skills) {
        (Some(_), None) => problems.push(
            "it would offer every skill, and this agent is offered only the skills its permissions list"
                .to_owned(),
        ),
        (_, Some(skills)) => {
            for s in skills.iter().filter(|s| !bp.allows_skill(s)) {
                problems.push(format!(
                    "it would offer the skill {s}, which this agent is not offered"
                ));
            }
        }
        (None, None) => {}
    }
    problems
}

/// The `is_error` result of a change that would give `name` more than `by` has.
fn escalation_error(name: &str, problems: &[String]) -> ToolOut {
    let message = format!(
        "The change was not proposed: it would give agent {name} more than this agent has. {}.",
        capitalise(&problems.join("; "))
    );
    let mut out = ToolOut::error(message.clone());
    out.content = json!({
        "error": message,
        "problems": problems,
        "message": "An agent may only give another agent tools, note types, skills and switches it has itself. Propose a change within them, or ask the user to make it on the agent screen.",
    })
    .to_string();
    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// `propose_agent_change`: check the agent file as the agent write would, without writing it,
/// and against the proposing agent `by`, and hand it to the user.
pub(crate) fn propose_change(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = input["wcl"]
        .as_str()
        .ok_or_else(|| ToolOut::error("wcl must be the whole agent file as WCL text"))?;
    if name == by.name {
        return Err(ToolOut::error(OWN_FILE));
    }
    let before = w.read_agent(name).ok();
    let (after, agent) = w
        .check_agent(name, wcl, crate::agent_files::unknown_tools)
        .map_err(|e| {
            refused(
                e,
                "The agent was not proposed. Fix the issues and propose it again; get_agent answers the agent-file vocabulary.",
            )
        })?;
    let problems = escalation(by, &agent);
    if !problems.is_empty() {
        return Err(escalation_error(name, &problems));
    }
    let names = vec![json!(name)];
    if before.as_deref() == Some(after.as_str()) {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed agent file is the same as the current one: there is nothing to apply.",
            }),
            names,
            None,
        ));
    }
    let exists = before.is_some();
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": exists,
        "message": if exists {
            "The agent change is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it."
        } else {
            "The new agent is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who applies or discards it."
        },
    });
    let proposal = json!({
        "id": call_id,
        "kind": "agent",
        "op": if exists { "change" } else { "create" },
        "name": name,
        "label": agent.label,
        "exists": exists,
        "before": before,
        "after": after,
        "issues": [],
        "proposer": by.name,
    });
    Ok(out(content, names, Some(proposal)))
}

/// `propose_agent_delete`: check the agent exists, is not built in and is not the proposing
/// agent `by`, without deleting it, and hand the delete to the user.
pub(crate) fn propose_delete(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let before = w
        .read_agent(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let label = Agent::parse(name, &before).map_or_else(|_| name.to_owned(), |a| a.label);
    if name == ASSISTANT_AGENT {
        let message = format!("{label} is built in and cannot be deleted.");
        let mut out = ToolOut::error(message.clone());
        out.content = json!({ "error": message, "code": "built_in" }).to_string();
        return Err(out);
    }
    if name == by.name {
        return Err(ToolOut::error(OWN_FILE));
    }
    let proposal = json!({
        "id": call_id,
        "kind": "agent",
        "op": "delete",
        "name": name,
        "label": label,
        "before": before,
        "proposer": by.name,
    });
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it. The agent's threads stay.",
        }),
        vec![json!(name)],
        Some(proposal),
    ))
}

/// Agent `name` as it stands now, Assistant included; `None` when there is no such agent.
pub(crate) fn agent_named(w: &Writer, name: &str) -> Result<Option<Agent>, ApplyError> {
    if name == ASSISTANT_AGENT {
        return crate::ai_settings::assistant(w)
            .map(Some)
            .map_err(|e| ApplyError::Invalid(e.body["message"].as_str().unwrap_or("").into()));
    }
    match w.agent(name) {
        Ok(a) => Ok(Some(a)),
        Err(writer::Error::Missing(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The agent that proposed `p`, as it stands now: its `proposer`, else the agent the proposal
/// names. `None` for a change no agent proposed.
pub(crate) fn proposer_of(w: &Writer, p: &Value) -> Result<Option<Agent>, ApplyError> {
    let Some(name) = [&p["proposer"], &p["agent"]]
        .into_iter()
        .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
    else {
        return Ok(None);
    };
    match agent_named(w, name)? {
        Some(a) => Ok(Some(a)),
        None => Err(ApplyError::Refused(format!(
            "the agent {name} that proposed this no longer exists, so nothing was written"
        ))),
    }
}

/// Apply an `agent_proposal` (agents G20), when the agent file is still the proposal's `before`
/// and the change stays within the proposing agent's permissions as they are now.
pub(crate) fn apply_agent(w: &Writer, p: &Value) -> Result<Value, ApplyError> {
    let name = p["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApplyError::Invalid("the change has no name".into()))?;
    let by = proposer_of(w, p)?;
    if by.as_ref().is_some_and(|a| a.name == name) {
        return Err(ApplyError::Refused(OWN_FILE.to_owned()));
    }
    let before = p["before"].as_str();
    if w.read_agent(name).ok().as_deref() != before {
        return Err(ApplyError::Stale(format!(
            "the agent {name} changed since the proposal, so nothing was written"
        )));
    }
    if p["op"] == "delete" {
        w.delete_agent(name)?;
        return Ok(json!({ "op": "agent_delete", "agent_name": name }));
    }
    let after = p["after"]
        .as_str()
        .ok_or_else(|| ApplyError::Invalid("the change has no after".into()))?;
    if let Some(by) = &by {
        let (_, agent) = w.check_agent(name, after, crate::agent_files::unknown_tools)?;
        let problems = escalation(by, &agent);
        if !problems.is_empty() {
            return Err(ApplyError::Refused(format!(
                "it would give agent {name} more than {} has: {}",
                by.label,
                problems.join("; ")
            )));
        }
    }
    w.put_agent(name, after, crate::agent_files::unknown_tools)?;
    let op = if before.is_some() {
        "agent_change"
    } else {
        "agent_create"
    };
    Ok(json!({ "op": op, "agent_name": name }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(wcl: &str) -> Agent {
        Agent::parse("a", wcl).unwrap()
    }

    #[test]
    fn agent_format_holds_the_agent_file_section() {
        let t = agent_format_text();
        assert!(t.starts_with("## Agent files"), "{}", &t[..40]);
        assert!(!t.contains("## Device files"));
    }

    #[test]
    fn escalation_compares_tools_types_and_switches() {
        let all = agent("");
        let some = agent(
            "permissions {\n  tools = [\"get_note\"]\n  types = [\"task\"]\n  web {\n    search = false\n  }\n}\n",
        );
        assert!(escalation(&all, &some).is_empty());
        assert!(escalation(&some, &some).is_empty());
        let p = escalation(&some, &all);
        assert_eq!(p.len(), 3, "{p:?}");
        let more = agent(
            "permissions {\n  tools = [\"get_note\", \"propose_edit\"]\n  types = [\"task\"]\n  confirm {\n    notes = false\n  }\n  web {\n    search = false\n  }\n}\n",
        );
        let p = escalation(&some, &more);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("propose_edit"));
        assert!(p[1].contains("notes"));
        let threads = agent("permissions {\n  types = [\"agent_thread\"]\n}\n");
        assert_eq!(escalation(&all, &threads).len(), 1);
    }

    #[test]
    fn escalation_refuses_the_outbox_bypass_the_proposer_lacks() {
        let sender = agent("permissions {\n  outbox {\n    send_without_review = true\n  }\n}\n");
        let all = agent("");
        let p = escalation(&all, &sender);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("send_without_review"), "{p:?}");
        assert!(escalation(&sender, &sender).is_empty());
        assert!(escalation(&sender, &all).is_empty());
    }

    #[test]
    fn escalation_refuses_browser_automation_the_proposer_lacks() {
        let driver = agent("permissions {\n  browser = true\n}\n");
        let all = agent("");
        let p = escalation(&all, &driver);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("Browser automation"), "{p:?}");
        assert!(escalation(&driver, &driver).is_empty());
        assert!(escalation(&driver, &all).is_empty());
    }

    #[test]
    fn escalation_refuses_skills_the_proposer_is_not_offered() {
        let all = agent("");
        let one = agent("permissions {\n  skills = [\"pdf-forms\"]\n}\n");
        let two = agent("permissions {\n  skills = [\"pdf-forms\", \"brand\"]\n}\n");
        assert!(escalation(&all, &one).is_empty());
        assert!(escalation(&two, &one).is_empty());
        let p = escalation(&one, &two);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("brand"), "{p:?}");
        let p = escalation(&one, &all);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("every skill"), "{p:?}");
    }
}
