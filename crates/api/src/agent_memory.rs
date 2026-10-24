//! Agent memory (agents G21): what an agent keeps from one conversation to the next, each memory
//! a note of the built-in type `agent_memory`.
//!
//! A memory note holds `title` (a short name), `agent` (the agent's name) and the content field
//! `body`, the memory's text as paragraphs. The text goes into the body with its markup
//! characters escaped, so it reads back as it was written; a body edited by hand reads back as
//! its paragraphs' text, one blank line apart.
//!
//! An agent's own tools, `remember`, `recall`, `update_memory` and `forget`, reach only its own
//! memories, and write as `Author::Agent(<agent>)`, so they never fire triggers (agents G11).
//! They change nothing but the agent's memory, so an unattended run is offered them whatever
//! its confirmation switches say (agents G12). The note tools leave `agent_memory` notes out
//! unless an agent's `permissions.types` names the type (`crate::ai_chat::Limits`).
//!
//! Each turn's system text gains [`prompt_section`] after the agent's prompt: the agent's
//! memories, newest first, one line each.
//!
//! The helpers here ([`memories`], [`read`], [`create`], [`update`], [`forget`]) are the ones
//! any other route or tool that lists or changes an agent's memories goes through, so the limits
//! hold for every writer.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use schema::Agent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use state::{Author, Scope};
use writer::{Input, Writer, doc_to_json};

use crate::ai_chat::{ApplyError, ToolOut, tool};
use crate::auth::Caller;
use crate::{ApiError, blocking};

/// The note type of a memory.
pub const MEMORY_TYPE: &str = writer::AGENT_MEMORY_TYPE;

/// The longest memory text, in characters.
pub const TEXT_LIMIT: usize = 8_000;

/// The longest memory title, in characters.
pub const TITLE_LIMIT: usize = 120;

/// The most memories one agent holds.
pub const MAX_MEMORIES: usize = 500;

/// How many memories `recall` answers when it is not told.
pub const RECALL_DEFAULT: usize = 20;

/// The most memories one `recall` answers.
pub const RECALL_LIMIT: usize = 50;

/// The longest the memory lines of the prompt section run, in characters.
pub const PROMPT_LIMIT: usize = 8_000;

/// How much of a memory's text its prompt line shows, in characters.
pub const LINE_TEXT: usize = 200;

/// The names of the own memory tools.
pub const TOOLS: [&str; 4] = ["remember", "recall", "update_memory", "forget"];

/// Whether `name` is one of the own memory tools.
pub fn is_memory_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// One memory, as the tools and routes answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Memory {
    pub id: String,
    /// The name of the agent it belongs to.
    pub agent: String,
    pub title: String,
    pub text: String,
    pub version: String,
    pub created: Option<String>,
    pub updated: Option<String>,
}

impl Memory {
    /// The memory in note `n`, or `None` when it is not an `agent_memory` note.
    fn of(n: &writer::Note) -> Option<Memory> {
        if n.type_name != MEMORY_TYPE {
            return None;
        }
        let doc = doc_to_json(&n.doc);
        let f = &doc["fields"];
        let s = |k: &str| f[k].as_str().map(str::to_owned);
        Some(Memory {
            id: n.id.clone(),
            agent: s("agent").unwrap_or_default(),
            title: s("title").unwrap_or_default(),
            text: text_of(&doc),
            version: n.version.clone(),
            created: s("created"),
            updated: s("updated"),
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "text": self.text,
            "created": self.created,
            "updated": self.updated,
        })
    }
}

/// The text of a memory note's `body`: each text block's text, unescaped, one blank line apart.
fn text_of(doc: &Value) -> String {
    fn walk(blocks: &Value, out: &mut Vec<String>) {
        for b in blocks.as_array().into_iter().flatten() {
            if let Some(t) = b["fields"]["text"].as_str() {
                let t = schema::unescape_markup(t);
                if !t.trim().is_empty() {
                    out.push(t);
                }
            }
            walk(&b["blocks"], out);
        }
    }
    let mut out = Vec::new();
    for b in doc["blocks"].as_array().into_iter().flatten() {
        if b["kind"] == "content" && b["labels"][0] == "body" {
            walk(&b["blocks"], &mut out);
        }
    }
    out.join("\n\n")
}

/// The `body` block holding `text`, a paragraph for each part between blank lines.
fn body_of(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| json!({ "kind": "paragraph", "fields": { "text": schema::escape_markup(p) } }))
        .collect();
    json!({ "kind": "content", "labels": ["body"], "blocks": paragraphs })
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_memory", message)
}

/// `title` trimmed, refused when empty or longer than [`TITLE_LIMIT`].
fn check_title(title: &str) -> Result<String, ApiError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(invalid("A memory needs a title."));
    }
    let n = t.chars().count();
    if n > TITLE_LIMIT {
        return Err(invalid(format!(
            "A memory's title is at most {TITLE_LIMIT} characters; this one is {n}."
        )));
    }
    Ok(t.to_owned())
}

/// `text` trimmed, refused when empty or longer than [`TEXT_LIMIT`].
fn check_text(text: &str) -> Result<String, ApiError> {
    let t = text.trim();
    if t.is_empty() {
        return Err(invalid("A memory needs some text."));
    }
    let n = t.chars().count();
    if n > TEXT_LIMIT {
        return Err(invalid(format!(
            "A memory's text is at most {TEXT_LIMIT} characters; this one is {n}. Split it into \
             smaller memories."
        )));
    }
    Ok(t.to_owned())
}

/// Every memory of agent `agent`, archived ones left out, newest first: the last changed first.
pub fn memories(w: &Writer, agent: &str) -> Result<Vec<Memory>, ApiError> {
    if !schema::is_name(agent) {
        return Err(ApiError::bad_request(format!(
            "{agent:?} is not an agent name"
        )));
    }
    let q = format!("from {MEMORY_TYPE} where agent == \"{agent}\"");
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = script_core::Vault::query_page(w, &q, query::MAX_PAGE, cursor.take())
            .map_err(ApiError::bad_request)?;
        for id in page["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["id"].as_str())
        {
            if let Ok(n) = w.read(id)
                && let Some(m) = Memory::of(&n)
            {
                out.push(m);
            }
        }
        match page["next"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
    Ok(out)
}

/// The memories of `all` whose title or text holds every word of `query`, ignoring case; all of
/// them for an empty query.
pub fn matching(all: Vec<Memory>, query: &str) -> Vec<Memory> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|m| {
            let hay = format!("{}\n{}", m.title, m.text).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Memory note `id`. Any other note, or none, is a 404.
pub fn read(w: &Writer, id: &str) -> Result<Memory, ApiError> {
    let missing = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("there is no memory {id}"),
        )
    };
    let n = w.read(id).map_err(|e| match e {
        writer::Error::Missing(_) => missing(),
        e => e.into(),
    })?;
    Memory::of(&n).ok_or_else(missing)
}

/// Create a memory of agent `agent`, as `author`. Refused past the limits: a title over
/// [`TITLE_LIMIT`], a text over [`TEXT_LIMIT`], or an agent that holds [`MAX_MEMORIES`] already.
pub fn create(
    w: &Writer,
    agent: &str,
    title: &str,
    text: &str,
    author: &Author,
) -> Result<Memory, ApiError> {
    let title = check_title(title)?;
    let text = check_text(text)?;
    let held = memories(w, agent)?.len();
    if held >= MAX_MEMORIES {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "memory_full",
            format!(
                "{agent} already holds {held} memories, the most an agent may. Forget some, or \
                 merge several into one with update_memory, before remembering more."
            ),
        ));
    }
    let doc = json!({
        "fields": { "title": title, "agent": agent },
        "blocks": [body_of(&text)],
    });
    let saved = w.create(MEMORY_TYPE, &Input::Json(doc), author)?;
    read(w, &saved.id)
}

/// Change memory `id`: its title and its text, `None` leaving a part as it is. With `base`, the
/// memory must still be at that version.
pub fn update(
    w: &Writer,
    id: &str,
    title: Option<&str>,
    text: Option<&str>,
    base: Option<&str>,
    author: &Author,
) -> Result<Memory, ApiError> {
    let title = title.map(check_title).transpose()?;
    let text = text.map(check_text).transpose()?;
    let n = w.read(id)?;
    if Memory::of(&n).is_none() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("there is no memory {id}"),
        ));
    }
    let mut doc = doc_to_json(&n.doc);
    if let Some(t) = title {
        doc["fields"]["title"] = json!(t);
    }
    if let Some(t) = text {
        let mut blocks: Vec<Value> = doc["blocks"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| !(b["kind"] == "content" && b["labels"][0] == "body"))
            .collect();
        blocks.push(body_of(&t));
        doc["blocks"] = Value::Array(blocks);
    }
    let base = base.unwrap_or(&n.version);
    w.update(id, &Input::Json(doc), Some(base), author)?;
    read(w, id)
}

/// Move memory `id` to the trash, where the user can restore it. With `base`, the memory must
/// still be at that version.
pub fn forget(w: &Writer, id: &str, base: Option<&str>, author: &Author) -> Result<(), ApiError> {
    let m = read(w, id)?;
    w.delete(id, Some(base.unwrap_or(&m.version)), author)?;
    Ok(())
}

/// The "Your memories" section of agent `agent`'s system text: each memory, newest first, as
/// `- <title> (<id>): <the first LINE_TEXT characters of its text>`, the lines stopping before
/// they pass [`PROMPT_LIMIT`] characters, with a line saying how many more there are. `None`
/// for an agent with no memories.
pub fn prompt_section(w: &Writer, agent: &str) -> Option<String> {
    let all = memories(w, agent).ok()?;
    if all.is_empty() {
        return None;
    }
    let mut lines = String::new();
    let mut shown = 0;
    for m in &all {
        let flat = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut text: String = flat.chars().take(LINE_TEXT).collect();
        if flat.chars().count() > LINE_TEXT {
            text.push('…');
        }
        let line = format!("- {} ({}): {text}\n", m.title, m.id);
        if lines.chars().count() + line.chars().count() > PROMPT_LIMIT {
            break;
        }
        lines.push_str(&line);
        shown += 1;
    }
    let mut out = String::from(
        "\n## Your memories\n\nWhat you chose to remember in earlier conversations, newest \
         first, each with its id and the start of its text. Read one in full with `recall`.\n\n",
    );
    out.push_str(&lines);
    let more = all.len() - shown;
    if more > 0 {
        out.push_str(&format!(
            "- {more} more {} not shown here: `recall` reads them.\n",
            if more == 1 {
                "memory is"
            } else {
                "memories are"
            }
        ));
    }
    Some(out)
}

/// The own memory tools' specs.
pub fn tool_specs() -> Vec<ai::ToolSpec> {
    vec![
        tool(
            "remember",
            "Remember something for later conversations: a new memory of your own, with a short title and its text. Your memories are listed in your instructions at the start of every conversation. Answers the memory's id.",
            json!({
                "title": { "type": "string", "description": "A short name for the memory, at most 120 characters." },
                "text": { "type": "string", "description": "What to remember, at most 8000 characters." },
            }),
        ),
        tool(
            "recall",
            "Read your own memories in full: those whose title or text holds every word of `query`, ignoring case, or all of them for an empty query, newest first.",
            json!({
                "query": { "type": "string", "description": "Words to look for; empty for every memory." },
                "limit": { "type": ["integer", "null"], "description": "How many memories to return, 1 to 50; null for 20." },
            }),
        ),
        tool(
            "update_memory",
            "Change one of your own memories by id: its title, its text, or both. null leaves a part as it is.",
            json!({
                "id": { "type": "string", "description": "The memory's id." },
                "title": { "type": ["string", "null"], "description": "The new title, or null to keep it." },
                "text": { "type": ["string", "null"], "description": "The new text, or null to keep it." },
            }),
        ),
        tool(
            "forget",
            "Forget one of your own memories by id. It moves to the trash, where the user can restore it.",
            json!({ "id": { "type": "string", "description": "The memory's id." } }),
        ),
    ]
}

/// The answer of a memory tool: `content` for the model, and the memories as the summary's notes.
fn out(content: Value, memories: &[&Memory]) -> ToolOut {
    let notes: Vec<Value> = memories
        .iter()
        .map(|m| json!({ "id": m.id, "title": m.title }))
        .collect();
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": notes }),
        proposal: None,
    }
}

fn failed(e: ApiError) -> ToolOut {
    ToolOut::error(
        e.body["message"]
            .as_str()
            .unwrap_or("the memory could not be changed")
            .to_owned(),
    )
}

/// Memory `id` when it is agent `agent`'s own.
fn own(w: &Writer, agent: &str, input: &Value) -> Result<Memory, ToolOut> {
    let id = crate::ai_chat::str_arg(input, "id")?;
    let m = read(w, id).map_err(failed)?;
    if m.agent != agent {
        return Err(ToolOut::error(format!(
            "Memory {id} is not yours: you can change only your own memories."
        )));
    }
    Ok(m)
}

/// An optional string argument: `None` when absent or null.
fn opt_str<'a>(input: &'a Value, name: &str) -> Result<Option<&'a str>, ToolOut> {
    match &input[name] {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        _ => Err(ToolOut::error(format!("{name} must be a string or null"))),
    }
}

/// Run own memory tool `name` for agent `agent`. Its writes are authored by the agent, and need
/// a caller that may write notes.
pub(crate) fn run(w: &Writer, scope: Scope, agent: &str, name: &str, input: &Value) -> ToolOut {
    let author = Author::Agent(agent.to_owned());
    let writes = name != "recall";
    if writes && !scope.allows(Scope::Write) {
        return ToolOut::error(
            "This chat cannot change memories: the token running it may only read.",
        );
    }
    let result = match name {
        "remember" => (|| {
            let title = input["title"].as_str().unwrap_or_default();
            let text = input["text"].as_str().unwrap_or_default();
            let m = create(w, agent, title, text, &author).map_err(failed)?;
            Ok(out(
                json!({ "id": m.id, "message": format!("Remembered as memory {}.", m.id) }),
                &[&m],
            ))
        })(),
        "recall" => (|| {
            let query = input["query"].as_str().unwrap_or_default();
            let limit = input["limit"]
                .as_u64()
                .map_or(RECALL_DEFAULT, |n| {
                    usize::try_from(n).unwrap_or(RECALL_LIMIT)
                })
                .clamp(1, RECALL_LIMIT);
            let all = memories(w, agent).map_err(failed)?;
            let held = all.len();
            let found = matching(all, query);
            let total = found.len();
            let kept: Vec<&Memory> = found.iter().take(limit).collect();
            let rows: Vec<Value> = kept.iter().map(|m| m.to_json()).collect();
            Ok(out(
                json!({ "memories": rows, "matched": total, "held": held }),
                &kept,
            ))
        })(),
        "update_memory" => (|| {
            let m = own(w, agent, input)?;
            let title = opt_str(input, "title")?;
            let text = opt_str(input, "text")?;
            if title.is_none() && text.is_none() {
                return Err(ToolOut::error(
                    "Nothing to change: send a new title, a new text, or both.",
                ));
            }
            let m = update(w, &m.id, title, text, None, &author).map_err(failed)?;
            Ok(out(
                json!({ "id": m.id, "title": m.title, "message": "The memory was changed." }),
                &[&m],
            ))
        })(),
        "forget" => (|| {
            let m = own(w, agent, input)?;
            forget(w, &m.id, Some(&m.version), &author).map_err(failed)?;
            Ok(out(
                json!({ "id": m.id, "message": "The memory was forgotten: it is in the trash, where the user can restore it." }),
                &[&m],
            ))
        })(),
        other => Err(ToolOut::error(format!("there is no tool {other:?}"))),
    };
    result.unwrap_or_else(|e| e)
}

// The routes of the agent screen's Memory tab (agents G21). The writes are authored as the
// caller, and go through the helpers above, so the limits hold for them too.

type Shared = State<Arc<Writer>>;

/// The query of `GET /api/agents/{name}/memories`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    query: Option<String>,
}

/// `GET /api/agents/{name}/memories?query=`: the agent's memories whose title or text holds every
/// word of `query`, all of them without one, newest first: `{memories, held, limit}`, `held` how
/// many the agent holds in all and `limit` the most it may.
pub async fn list(
    State(w): Shared,
    Path(name): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    blocking(w, move |w| {
        let all = memories(w, &name)?;
        let held = all.len();
        let found = matching(all, q.query.as_deref().unwrap_or_default());
        Ok::<_, ApiError>(json!({ "memories": found, "held": held, "limit": MAX_MEMORIES }))
    })
    .await
    .map(Json)
}

/// The body of `POST /api/agents/{name}/memories`.
#[derive(Debug, Deserialize)]
pub struct NewMemory {
    title: String,
    text: String,
}

/// `POST /api/agents/{name}/memories`: a new memory of the agent, from `{title, text}`.
pub async fn create_route(
    State(w): Shared,
    Caller(author): Caller,
    Path(name): Path<String>,
    Json(b): Json<NewMemory>,
) -> Result<(StatusCode, Json<Memory>), ApiError> {
    let m = blocking(w, move |w| create(w, &name, &b.title, &b.text, &author)).await?;
    Ok((StatusCode::CREATED, Json(m)))
}

/// The body of `PATCH /api/agents/{name}/memories/{id}`.
#[derive(Debug, Deserialize)]
pub struct MemoryChange {
    title: Option<String>,
    text: Option<String>,
    base_version: Option<String>,
}

/// Memory `id` when it is agent `name`'s; any other is a 404.
fn read_of(w: &Writer, name: &str, id: &str) -> Result<Memory, ApiError> {
    let m = read(w, id)?;
    if m.agent != name {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{name} has no memory {id}"),
        ));
    }
    Ok(m)
}

/// `PATCH /api/agents/{name}/memories/{id}`: change the memory's title, text or both, from
/// `{title?, text?, base_version?}`; with `base_version`, a 409 when it changed since.
pub async fn update_route(
    State(w): Shared,
    Caller(author): Caller,
    Path((name, id)): Path<(String, String)>,
    Json(b): Json<MemoryChange>,
) -> Result<Json<Memory>, ApiError> {
    blocking(w, move |w| {
        let m = read_of(w, &name, &id)?;
        update(
            w,
            &m.id,
            b.title.as_deref(),
            b.text.as_deref(),
            b.base_version.as_deref(),
            &author,
        )
    })
    .await
    .map(Json)
}

/// `DELETE /api/agents/{name}/memories/{id}`: the memory goes to the trash.
pub async fn delete_route(
    State(w): Shared,
    Caller(author): Caller,
    Path((name, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    blocking(w, move |w| {
        let m = read_of(w, &name, &id)?;
        forget(w, &m.id, Some(&m.version), &author)?;
        Ok::<_, ApiError>(json!({ "id": m.id, "deleted": true }))
    })
    .await
    .map(Json)
}

// Other agents' memories (agents G21): for an agent that may change agents, and only the
// memories of an agent whose file it could change (agents G20's no-escalation rule), because a
// memory steers the agent that reads it.

/// The names of the tools that reach other agents' memories.
pub const CROSS_TOOLS: [&str; 4] = [
    "list_agent_memories",
    "get_agent_memory",
    "propose_agent_memory_change",
    "propose_agent_memory_delete",
];

/// Whether `name` is one of the cross-agent memory tools.
pub fn is_cross_tool(name: &str) -> bool {
    CROSS_TOOLS.contains(&name)
}

/// The cross-agent memory tools' specs.
pub fn cross_tool_specs() -> Vec<ai::ToolSpec> {
    vec![
        tool(
            "list_agent_memories",
            "List another agent's memories: those whose title or text holds every word of `query`, ignoring case, or all of them for an empty query, newest first, each with its id, title, the start of its text and when it changed. You reach an agent's memories only when you could change its file; use recall for your own.",
            json!({
                "agent": { "type": "string", "description": "The other agent's name." },
                "query": { "type": "string", "description": "Words to look for; empty for every memory." },
            }),
        ),
        tool(
            "get_agent_memory",
            "Read one of another agent's memories in full by id.",
            json!({ "id": { "type": "string", "description": "The memory's id." } }),
        ),
        tool(
            "propose_agent_memory_change",
            "Propose a memory of another agent for the user to apply: a new one when `id` is null, else a change to its memory `id`, null leaving the title or text as it is. A memory steers the agent that reads it, so you reach only the memories of an agent whose file you could change. When the user lets agent changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "agent": { "type": "string", "description": "The other agent's name." },
                "id": { "type": ["string", "null"], "description": "The memory's id, or null for a new memory." },
                "title": { "type": ["string", "null"], "description": "The title, at most 120 characters; null keeps it." },
                "text": { "type": ["string", "null"], "description": "The text, at most 8000 characters; null keeps it." },
            }),
        ),
        tool(
            "propose_agent_memory_delete",
            "Propose forgetting one of another agent's memories for the user to apply. Applied, it moves to the trash, where the user can restore it. When the user lets agent changes go ahead without asking, it is done at once and the result says applied.",
            json!({ "id": { "type": "string", "description": "The memory's id." } }),
        ),
    ]
}

/// The refusal of a cross-agent tool on the calling agent's own memories.
pub const OWN_MEMORIES: &str = "These tools reach other agents' memories: use remember, recall, update_memory and forget for your own.";

/// Why agent `by` may not reach agent `target`'s memories (`found` as it stands, `None` when
/// there is no such agent), or `None` when it may: only when it could change `target`'s file
/// (agents G20), so it may change agents, `target` is another agent, and `target` has nothing
/// `by` lacks.
pub(crate) fn out_of_reach(by: &Agent, target: &str, found: Option<&Agent>) -> Option<String> {
    if target == by.name {
        return Some(OWN_MEMORIES.to_owned());
    }
    if !by.permissions.allows_tool("propose_agent_change") {
        return Some(format!(
            "The memories of agent {target} are out of reach: an agent reaches another agent's \
             memories only when it may change agents (the propose_agent_change tool)."
        ));
    }
    let Some(agent) = found else {
        return Some(format!("There is no agent {target}."));
    };
    let problems = crate::ai_agents::escalation(by, agent);
    if problems.is_empty() {
        return None;
    }
    Some(format!(
        "The memories of agent {target} are out of reach: an agent reaches another agent's \
         memories only when it could change that agent's file, and {target} has what this agent \
         lacks: {}.",
        problems.join("; ")
    ))
}

/// Agent `target` as it stands, when `by` may reach its memories.
fn reach(w: &Writer, by: &Agent, target: &str) -> Result<Agent, ToolOut> {
    if !schema::is_name(target) {
        return Err(ToolOut::error(format!("{target:?} is not an agent name")));
    }
    let found =
        crate::ai_agents::agent_named(w, target).map_err(|e| ToolOut::error(e.to_string()))?;
    if let Some(why) = out_of_reach(by, target, found.as_ref()) {
        return Err(ToolOut::error(why));
    }
    found.ok_or_else(|| ToolOut::error(format!("There is no agent {target}.")))
}

/// A memory's row in a listing: its text cut to [`LINE_TEXT`] characters.
fn row(m: &Memory) -> Value {
    let mut text: String = m.text.chars().take(LINE_TEXT).collect();
    if m.text.chars().count() > LINE_TEXT {
        text.push('…');
    }
    json!({ "id": m.id, "title": m.title, "text": text, "updated": m.updated })
}

/// The answer of a cross-agent memory tool, with the proposal it emits.
fn cross_out(content: Value, memories: &[&Memory], proposal: Option<Value>) -> ToolOut {
    let mut o = out(content, memories);
    o.proposal = proposal.map(|p| ("agent_memory_proposal", p));
    o
}

/// `ApiError` `e` as the model reads it.
fn checked<T>(r: Result<T, ApiError>) -> Result<T, ToolOut> {
    r.map_err(failed)
}

/// Run cross-agent memory tool `name` for agent `by`: the reads answer, the changes propose.
pub(crate) fn run_cross(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    name: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let str_arg = crate::ai_chat::str_arg;
    match name {
        "list_agent_memories" => {
            let target = str_arg(input, "agent")?;
            reach(w, by, target)?;
            let query = input["query"].as_str().unwrap_or_default();
            let all = checked(memories(w, target))?;
            let held = all.len();
            let found = matching(all, query);
            let rows: Vec<Value> = found.iter().map(row).collect();
            let refs: Vec<&Memory> = found.iter().collect();
            Ok(cross_out(
                json!({ "agent": target, "memories": rows, "matched": found.len(), "held": held }),
                &refs,
                None,
            ))
        }
        "get_agent_memory" => {
            let m = checked(read(w, str_arg(input, "id")?))?;
            reach(w, by, &m.agent)?;
            let mut content = m.to_json();
            content["agent"] = json!(m.agent);
            Ok(cross_out(content, &[&m], None))
        }
        "propose_agent_memory_change" => {
            let target = str_arg(input, "agent")?;
            let title = opt_str(input, "title")?;
            let text = opt_str(input, "text")?;
            let id = opt_str(input, "id")?
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let agent = reach(w, by, target)?;
            let Some(id) = id else {
                let (Some(title), Some(text)) = (title, text) else {
                    return Err(ToolOut::error(
                        "A new memory needs both a title and a text.",
                    ));
                };
                let (title, text) = (checked(check_title(title))?, checked(check_text(text))?);
                let held = checked(memories(w, target))?.len();
                if held >= MAX_MEMORIES {
                    return Err(ToolOut::error(format!(
                        "{target} already holds {held} memories, the most an agent may. Propose \
                         forgetting some first."
                    )));
                }
                let proposal = json!({
                    "id": call_id,
                    "kind": "agent_memory",
                    "op": "create",
                    "memory_agent": target,
                    "agent_label": agent.label,
                    "memory_id": null,
                    "title": title,
                    "text": text,
                    "before": null,
                    "proposer": by.name,
                });
                return Ok(cross_out(
                    json!({
                        "proposed": true,
                        "message": format!("The new memory of {target} is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it."),
                    }),
                    &[],
                    Some(proposal),
                ));
            };
            let m = checked(read(w, id))?;
            if m.agent != target {
                return Err(ToolOut::error(format!(
                    "Memory {id} belongs to agent {}, not {target}.",
                    m.agent
                )));
            }
            if title.is_none() && text.is_none() {
                return Err(ToolOut::error(
                    "Nothing to change: send a new title, a new text, or both.",
                ));
            }
            let title = match title {
                Some(t) => checked(check_title(t))?,
                None => m.title.clone(),
            };
            let text = match text {
                Some(t) => checked(check_text(t))?,
                None => m.text.clone(),
            };
            if title == m.title && text == m.text {
                return Ok(cross_out(
                    json!({
                        "proposed": false,
                        "message": "The proposed memory is the same as the current one: there is nothing to apply.",
                    }),
                    &[&m],
                    None,
                ));
            }
            let proposal = json!({
                "id": call_id,
                "kind": "agent_memory",
                "op": "change",
                "memory_agent": target,
                "agent_label": agent.label,
                "memory_id": m.id,
                "base_version": m.version,
                "title": title,
                "text": text,
                "before": { "title": m.title, "text": m.text },
                "proposer": by.name,
            });
            Ok(cross_out(
                json!({
                    "proposed": true,
                    "message": "The memory change is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it.",
                }),
                &[&m],
                Some(proposal),
            ))
        }
        "propose_agent_memory_delete" => {
            let m = checked(read(w, str_arg(input, "id")?))?;
            let agent = reach(w, by, &m.agent)?;
            let proposal = json!({
                "id": call_id,
                "kind": "agent_memory",
                "op": "delete",
                "memory_agent": m.agent,
                "agent_label": agent.label,
                "memory_id": m.id,
                "base_version": m.version,
                "title": m.title,
                "text": m.text,
                "before": { "title": m.title, "text": m.text },
                "proposer": by.name,
            });
            Ok(cross_out(
                json!({
                    "proposed": true,
                    "message": "Forgetting the memory is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.",
                }),
                &[&m],
                Some(proposal),
            ))
        }
        other => Err(ToolOut::error(format!("there is no tool {other:?}"))),
    }
}

/// Apply an `agent_memory_proposal` (agents G21) as `author`, when the memory is still at the
/// proposal's `base_version` and its proposer may still reach the agent's memories. Answers
/// `{op, memory_id, memory_agent, version}`, `op` `agent_memory_create`, `agent_memory_change`
/// or `agent_memory_delete`.
pub(crate) fn apply(w: &Writer, author: &Author, p: &Value) -> Result<Value, ApplyError> {
    let target = p["memory_agent"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApplyError::Invalid("the change has no memory_agent".into()))?;
    if let Some(by) = crate::ai_agents::proposer_of(w, p)? {
        let found = crate::ai_agents::agent_named(w, target)?;
        if let Some(why) = out_of_reach(&by, target, found.as_ref()) {
            return Err(ApplyError::Refused(why));
        }
    }
    let refused = |e: ApiError| {
        ApplyError::Invalid(
            e.body["message"]
                .as_str()
                .unwrap_or("the memory could not be changed")
                .to_owned(),
        )
    };
    let part = |k: &str| p[k].as_str().unwrap_or_default();
    let op = p["op"].as_str().unwrap_or_default();
    if op == "create" {
        let m = create(w, target, part("title"), part("text"), author).map_err(refused)?;
        return Ok(json!({
            "op": "agent_memory_create",
            "memory_id": m.id,
            "memory_agent": target,
            "version": m.version,
        }));
    }
    let id = p["memory_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApplyError::Invalid("the change has no memory_id".into()))?;
    let stale = || {
        ApplyError::Stale(format!(
            "the memory {id} changed since the proposal, so nothing was written"
        ))
    };
    let m = read(w, id).map_err(|_| stale())?;
    if m.agent != target || p["base_version"].as_str() != Some(m.version.as_str()) {
        return Err(stale());
    }
    match op {
        "change" => {
            let m = update(
                w,
                id,
                Some(part("title")),
                Some(part("text")),
                Some(&m.version),
                author,
            )
            .map_err(refused)?;
            Ok(json!({
                "op": "agent_memory_change",
                "memory_id": m.id,
                "memory_agent": target,
                "version": m.version,
            }))
        }
        "delete" => {
            forget(w, id, Some(&m.version), author).map_err(refused)?;
            Ok(json!({ "op": "agent_memory_delete", "memory_id": id, "memory_agent": target }))
        }
        other => Err(ApplyError::Invalid(format!(
            "{other:?} is not a memory change"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_goes_into_the_body_and_reads_back_as_written() {
        let text = "Likes *tea*, [not] coffee.\n\nSecond line\nwith a break.";
        let doc = json!({ "blocks": [body_of(text)] });
        assert_eq!(text_of(&doc), text);
    }

    #[test]
    fn matching_needs_every_word_in_the_title_or_text() {
        let m = |title: &str, text: &str| Memory {
            id: title.to_owned(),
            agent: "a".into(),
            title: title.into(),
            text: text.into(),
            version: String::new(),
            created: None,
            updated: None,
        };
        let all = vec![
            m("Tea", "Earl Grey, no milk"),
            m("Garden", "Water on Sundays"),
        ];
        let ids = |v: Vec<Memory>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(matching(all.clone(), "")), ["Tea", "Garden"]);
        assert_eq!(ids(matching(all.clone(), "tea MILK")), ["Tea"]);
        assert_eq!(ids(matching(all, "tea sundays")), Vec::<String>::new());
    }
}
