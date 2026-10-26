//! Sub agents (subagents U1 to U4): `start_subagent`, with which an agent hands a part of its
//! task to a throwaway sub agent or to another agent by name, and the rights a sub agent runs
//! with.
//!
//! The turn loop (`crate::ai_chat`) runs each call as a turn of its own on a thread of its own, a
//! sub agent session (`crate::agent_threads`, subagents U5), and answers the call with the sub
//! agent's last message. This module holds what that needs apart from the loop: the tool's
//! schema, the permissions a sub agent gets ([`narrow`] and [`intersect`], never more than the
//! parent's, subagents U3), the throwaway's prompt section, and the tool result.

use ai::{Content, Message, Role, ToolSpec};
use schema::{Agent, AgentPermissions};
use serde_json::{Value, json};

use crate::ai_chat::{Limits, ToolOut};

/// The tool's name.
pub const TOOL: &str = "start_subagent";

/// The memory tools that write the agent's own memories: a throwaway sub agent runs as its
/// parent and reads its memories, but writes none (subagents U1).
pub const MEMORY_WRITES: [&str; 3] = ["remember", "update_memory", "forget"];

/// The longest answer a sub agent's block shows, in characters.
const ANSWER_SHOWN: usize = 20_000;

/// `start_subagent`, for an agent whose permissions allow it, naming the agents it may delegate
/// to.
pub fn spec(agent: &Agent) -> ToolSpec {
    let named = &agent.permissions.subagents.agents;
    let max = agent.permissions.subagents.max;
    let delegate = if named.is_empty() {
        "This agent may start only throwaway sub agents: agent must be null.".to_owned()
    } else {
        format!(
            "agent names another agent to delegate to, with its own prompt, memories and permissions; it may be one of: {}.",
            named.join(", ")
        )
    };
    let switch = |what: &str| {
        json!({
            "type": ["object", "null"],
            "description": what,
            "properties": switches_schema(what),
            "required": switch_names(what),
            "additionalProperties": false,
        })
    };
    let names = |what: &str| json!({ "type": ["array", "null"], "items": { "type": "string" }, "description": what });
    crate::ai_chat::tool(
        TOOL,
        &format!(
            "Hand one part of your task to a sub agent, which works on it in a conversation of its own and answers with its last message, which this tool returns, with its status. Use it to break a large task down. Calls in one round run at the same time, at most {max} at once. A sub agent never has more rights than you: what it changes is proposed to the user here, or made, as your own changes would be. It cannot start sub agents itself, nor ask you or the user anything, so give it everything it needs in task and instructions. agent null starts a throwaway sub agent: you, with the task added to your instructions. {delegate}"
        ),
        json!({
            "task": { "type": "string", "description": "What the sub agent is to do, in a sentence or two. It is the sub agent's first message and its block's title." },
            "instructions": { "type": ["string", "null"], "description": "What else it needs to know: the context, the notes to use, the form of the answer; null for none." },
            "agent": { "type": ["string", "null"], "description": "The agent to delegate to by name, or null for a throwaway sub agent." },
            "permissions": {
                "type": ["object", "null"],
                "description": "Narrow the sub agent's permissions below yours; null keeps yours (for a named agent, what both of you have). Anything that would widen them is ignored, and the result says so.",
                "properties": {
                    "tools": names("Only these of your tools; null keeps them all."),
                    "types": names("Only these of your note types; null keeps them all."),
                    "confirm": switch("confirm"),
                    "web": switch("web"),
                    "browser": { "type": ["boolean", "null"], "description": "false turns browser automation off; null keeps yours." },
                    "run_code": { "type": ["boolean", "null"], "description": "false turns run_script off; null keeps yours." },
                },
                "required": ["tools", "types", "confirm", "web", "browser", "run_code"],
                "additionalProperties": false,
            },
            "model": { "type": ["string", "null"], "description": "Another model of your AI account for the sub agent; null for yours." },
        }),
    )
}

fn switch_names(what: &str) -> Vec<&'static str> {
    if what == "confirm" {
        schema::CONFIRM_SWITCHES.to_vec()
    } else {
        schema::WEB_SWITCHES.to_vec()
    }
}

fn switches_schema(what: &str) -> Value {
    let how = if what == "confirm" {
        "true makes the sub agent ask before this kind of change; null keeps yours."
    } else {
        "false turns this internet tool off; null keeps yours."
    };
    let props: serde_json::Map<String, Value> = switch_names(what)
        .into_iter()
        .map(|n| {
            (
                n.to_owned(),
                json!({ "type": ["boolean", "null"], "description": how }),
            )
        })
        .collect();
    Value::Object(props)
}

/// `parent`'s permissions narrowed by the call's `permissions` argument (subagents U3): a tools or
/// types list keeps only what the parent has, a confirmation switch only turns on, an internet
/// switch, browser automation and Run wscript code only turn off. What would widen them is
/// left out, one line each in the second value. Sub agents of its own it never has.
pub fn narrow(parent: &AgentPermissions, arg: &Value) -> (AgentPermissions, Vec<String>) {
    let mut p = parent.clone();
    p.subagents = schema::AgentSubagents::default();
    let mut ignored = Vec::new();
    if !arg.is_object() {
        return (p, ignored);
    }
    let list = |v: &Value| -> Option<Vec<String>> {
        v.as_array().map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
    };
    if let Some(tools) = list(&arg["tools"]) {
        let (kept, wider): (Vec<String>, Vec<String>) =
            tools.into_iter().partition(|t| parent.allows_tool(t));
        for t in wider {
            ignored.push(format!("the tool {t}, which you do not have"));
        }
        p.tools = Some(kept);
    }
    if let Some(types) = list(&arg["types"]) {
        let limits = Limits::of_permissions(parent);
        let (kept, wider): (Vec<String>, Vec<String>) =
            types.into_iter().partition(|t| limits.allows(t));
        for t in wider {
            ignored.push(format!("{t} notes, which you may not reach"));
        }
        p.types = Some(kept);
    }
    if let Some(c) = arg["confirm"].as_object() {
        for (name, on) in parent.confirm.switches() {
            match c.get(name).and_then(Value::as_bool) {
                Some(true) => {
                    p.confirm.set(name, true);
                }
                Some(false) if on => ignored.push(format!(
                    "making {name} changes without asking, which you must ask before"
                )),
                _ => {}
            }
        }
    }
    if let Some(wb) = arg["web"].as_object() {
        for (name, on) in parent.web.switches() {
            match wb.get(name).and_then(Value::as_bool) {
                Some(false) => {
                    p.web.set(name, false);
                }
                Some(true) if !on => {
                    ignored.push(format!("the Internet switch {name}, which is off for you"))
                }
                _ => {}
            }
        }
    }
    match arg["browser"].as_bool() {
        Some(false) => p.browser = false,
        Some(true) if !parent.browser => {
            ignored.push("Browser automation, which you do not have".to_owned());
        }
        _ => {}
    }
    match arg["run_code"].as_bool() {
        Some(false) => p.scripts.run_code = false,
        Some(true) if !parent.scripts.run_code => {
            ignored.push("Run wscript code, which you do not have".to_owned());
        }
        _ => {}
    }
    (p, ignored)
}

/// What a named agent runs with when `parent` delegates to it (subagents U3): a tool, note type,
/// skill, internet switch, browser automation, Run wscript code or the Outbox bypass only when
/// both have it, and each confirmation switch on when either has it on. Sub agents of its own it
/// never has.
pub fn intersect(own: &AgentPermissions, parent: &AgentPermissions) -> AgentPermissions {
    let lists = |a: &Option<Vec<String>>, b: &Option<Vec<String>>| match (a, b) {
        (None, None) => None,
        (Some(x), None) | (None, Some(x)) => Some(x.clone()),
        (Some(x), Some(y)) => Some(x.iter().filter(|t| y.contains(t)).cloned().collect()),
    };
    let mut p = own.clone();
    p.tools = lists(&own.tools, &parent.tools);
    p.skills = lists(&own.skills, &parent.skills);
    // An absent types list is every type but threads and memories, so a list meets the other
    // agent's reach, not its list.
    p.types = match (&own.types, &parent.types) {
        (None, None) => None,
        (Some(x), None) => {
            let l = Limits::of_permissions(parent);
            Some(x.iter().filter(|t| l.allows(t)).cloned().collect())
        }
        (None, Some(y)) => {
            let l = Limits::of_permissions(own);
            Some(y.iter().filter(|t| l.allows(t)).cloned().collect())
        }
        (Some(x), Some(y)) => Some(x.iter().filter(|t| y.contains(t)).cloned().collect()),
    };
    for ((name, mine), (_, theirs)) in own
        .confirm
        .switches()
        .into_iter()
        .zip(parent.confirm.switches())
    {
        p.confirm.set(name, mine || theirs);
    }
    for ((name, mine), (_, theirs)) in own.web.switches().into_iter().zip(parent.web.switches()) {
        p.web.set(name, mine && theirs);
    }
    p.outbox.send_without_review =
        own.outbox.send_without_review && parent.outbox.send_without_review;
    p.browser = own.browser && parent.browser;
    p.scripts.run_code = own.scripts.run_code && parent.scripts.run_code;
    p.subagents = schema::AgentSubagents::default();
    p
}

/// The section a throwaway sub agent's prompt ends with (subagents U1): what it is, its task and
/// the instructions it was given.
pub fn prompt_section(parent_label: &str, task: &str, instructions: Option<&str>) -> String {
    let mut s = format!(
        "\n## Sub agent\n\nYou are a sub agent: {parent_label} started you to do one part of its task, and reads your last message as your answer. Nobody reads this conversation as it happens and you cannot ask questions, so do the task with the tools you have, then answer with what you found or did, briefly and completely. A change you propose waits for the user in {parent_label}'s chat: say what you proposed in your answer.\n\nYour task: {task}\n"
    );
    if let Some(i) = instructions {
        s.push_str(&format!("\nInstructions: {i}\n"));
    }
    s
}

/// The first message a named agent gets when `parent_label` delegates `task` to it (subagents U1).
pub fn delegated_message(parent_label: &str, task: &str, instructions: Option<&str>) -> String {
    let mut s = format!(
        "{parent_label} hands you this task as a sub agent. Your last message is your answer to it, and nobody can answer questions meanwhile.\n\nTask: {task}"
    );
    if let Some(i) = instructions {
        s.push_str(&format!("\n\nInstructions: {i}"));
    }
    s
}

/// The last thing the assistant said in `messages`: a sub agent's answer.
pub fn answer_of(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|m| m.role == Role::Assistant)
        .find_map(|m| {
            let text: Vec<&str> = m
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            (!text.is_empty()).then(|| text.join("\n\n"))
        })
}

/// How a sub agent run ended, for its result.
pub struct Ended<'a> {
    pub thread: &'a str,
    pub agent: &'a str,
    pub delegated: Option<&'a str>,
    /// `done`, `failed` or `stopped`.
    pub outcome: &'a str,
    pub rounds: u64,
    pub tools: u64,
    pub ms: u64,
    pub answer: Option<String>,
    /// Why it failed.
    pub error: Option<String>,
    /// What it proposed, waiting for the user, one line each.
    pub proposed: Vec<String>,
    /// What its `permissions` argument asked for that would have widened them.
    pub ignored: Vec<String>,
}

/// The tool result of a sub agent run (subagents U1): its answer with a status line, what it
/// proposed and what was ignored; `summary.subagent` carries the same for its block.
pub fn result(e: Ended<'_>) -> ToolOut {
    let s = |n: u64, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let ran = format!(
        "{}, {}",
        s(e.rounds, "round", "rounds"),
        s(e.tools, "tool call", "tool calls")
    );
    let status = match e.outcome {
        "done" => format!("The sub agent finished: {ran}."),
        "stopped" => format!("The user stopped this sub agent before it finished: {ran}."),
        _ => format!(
            "The sub agent failed: {}. {ran}.",
            e.error.as_deref().unwrap_or("it did not finish")
        ),
    };
    let mut content = json!({
        "status": status,
        "outcome": e.outcome,
        "rounds": e.rounds,
        "tools": e.tools,
        "thread": e.thread,
        "answer": e.answer,
    });
    if !e.proposed.is_empty() {
        content["proposed"] = json!(e.proposed);
        content["proposed_message"] = json!(
            "These changes it proposed wait for the user in this chat; they are not made yet."
        );
    }
    if !e.ignored.is_empty() {
        content["ignored_permissions"] = json!(e.ignored);
        content["ignored_message"] =
            json!("A sub agent never has more than you: these were asked for and not given.");
    }
    let answer = e
        .answer
        .as_deref()
        .map(|a| crate::ai_chat::cut(a, ANSWER_SHOWN));
    let summary = json!({
        "notes": [],
        "message": if e.outcome == "done" { Value::Null } else { json!(status) },
        "subagent": {
            "thread": e.thread,
            "agent": e.agent,
            "delegated": e.delegated,
            "outcome": e.outcome,
            "rounds": e.rounds,
            "tools": e.tools,
            "ms": e.ms,
            "answer": answer,
            "proposed": e.proposed,
            "ignored": e.ignored,
        },
    });
    ToolOut {
        content: content.to_string(),
        is_error: e.outcome != "done",
        summary,
        proposal: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(wcl: &str) -> AgentPermissions {
        Agent::parse("a", wcl).unwrap().permissions
    }

    #[test]
    fn narrowing_keeps_only_what_the_parent_has() {
        let parent = perms(
            "permissions {\n  tools = [\"get_note\", \"propose_edit\"]\n  types = [\"task\"]\n  confirm {\n    notes = false\n  }\n  web {\n    search = false\n  }\n}\n",
        );
        let (p, ignored) = narrow(
            &parent,
            &json!({
                "tools": ["get_note", "web_search"],
                "types": ["task", "email"],
                "confirm": { "notes": true, "delete": false },
                "web": { "search": true, "read": false },
                "browser": true,
                "run_code": null,
            }),
        );
        assert_eq!(p.tools.as_deref(), Some(&["get_note".to_owned()][..]));
        assert_eq!(p.types.as_deref(), Some(&["task".to_owned()][..]));
        assert!(p.confirm.notes, "turning a switch on narrows");
        assert!(p.confirm.delete, "turning one off is ignored");
        assert!(!p.web.search && !p.web.read);
        assert!(!p.browser);
        assert_eq!(ignored.len(), 5, "{ignored:?}");
        assert!(
            ignored.iter().any(|i| i.contains("web_search")),
            "{ignored:?}"
        );
        assert!(ignored.iter().any(|i| i.contains("email")), "{ignored:?}");
        assert!(ignored.iter().any(|i| i.contains("delete")), "{ignored:?}");
        assert!(ignored.iter().any(|i| i.contains("search")), "{ignored:?}");
        assert!(ignored.iter().any(|i| i.contains("Browser")), "{ignored:?}");
        // Null keeps everything, and a sub agent never starts its own.
        let lead = perms("permissions {\n  subagents {\n    allow = true\n  }\n}\n");
        let (same, none) = narrow(&lead, &Value::Null);
        assert!(none.is_empty());
        assert!(!same.subagents.allow);
        assert_eq!(same.tools, None);
    }

    #[test]
    fn a_named_agent_gets_what_both_have() {
        let own = perms(
            "permissions {\n  tools = [\"get_note\", \"propose_edit\", \"search_notes\"]\n  browser = true\n  confirm {\n    notes = false\n    delete = false\n  }\n}\n",
        );
        let parent = perms(
            "permissions {\n  tools = [\"get_note\", \"search_notes\", \"query_notes\"]\n  types = [\"task\", \"agent_thread\"]\n  confirm {\n    notes = false\n  }\n  web {\n    read = false\n  }\n  subagents {\n    allow = true\n  }\n}\n",
        );
        let p = intersect(&own, &parent);
        assert_eq!(
            p.tools,
            Some(vec!["get_note".to_owned(), "search_notes".to_owned()])
        );
        // The parent's list meets the named agent's reach, which leaves threads out.
        assert_eq!(p.types, Some(vec!["task".to_owned()]));
        assert!(!p.confirm.notes, "off for both");
        assert!(p.confirm.delete, "on for the parent");
        assert!(!p.web.read && p.web.search);
        assert!(!p.browser);
        assert!(!p.subagents.allow);
    }

    #[test]
    fn the_answer_is_the_last_assistant_text() {
        let messages = vec![
            Message::user("Go"),
            Message {
                role: Role::Assistant,
                content: vec![Content::text("First.")],
                provider_state: None,
            },
            Message {
                role: Role::Assistant,
                content: vec![Content::text("The venue is the hall.")],
                provider_state: None,
            },
        ];
        assert_eq!(
            answer_of(&messages).as_deref(),
            Some("The venue is the hall.")
        );
        assert_eq!(answer_of(&[Message::user("x")]), None);
    }

    #[test]
    fn the_tool_is_a_strict_object() {
        let mut a = Agent::new("lead");
        a.permissions.subagents.allow = true;
        a.permissions.subagents.agents = vec!["writer".into()];
        let t = spec(&a);
        assert!(t.description.contains("writer"));
        let s = &t.input_schema;
        assert_eq!(s["additionalProperties"], false);
        let perms = &s["properties"]["permissions"];
        assert_eq!(perms["additionalProperties"], false);
        assert_eq!(
            perms["required"].as_array().unwrap().len(),
            perms["properties"].as_object().unwrap().len()
        );
        for k in ["confirm", "web"] {
            let x = &perms["properties"][k];
            assert_eq!(
                x["required"].as_array().unwrap().len(),
                x["properties"].as_object().unwrap().len()
            );
        }
    }
}
