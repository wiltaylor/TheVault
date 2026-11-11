//! `POST /api/ai/chat`: the chat panel's conversation turn (ai D10, D11, D14).
//!
//! The body is `{ account?, model?, messages, note_id?, agent?, thread?, update_skills?, compact? }`: the transcript so
//! far, ending with the user's new message, in the neutral form of [`ai::Message`]. `account` is
//! an AI account's id or name (accounts A5); `provider` (`anthropic`, `openai` or `chatgpt`) is
//! still taken for one release and names the account the migration made for it. Account and
//! model default to the agent's, then to Settings > AI's default account and its model. The turn
//! is run by `agent`, the built-in Assistant when it is absent (agents G4, G6): its account and
//! model when it names them and the body names neither (the chat box's choice wins over the
//! agent's, and an account named without a model runs on that account's model; a run not started
//! from the chat box, such as a trigger's or a mention's, sends none), its prompt, effort, tools,
//! note types ([`Limits`]) and confirmation and internet switches; a named agent is the author of
//! what the tools write (agents G11). With `thread`, a note of the agent's (agents G7), the server
//! reads the transcript from the thread, `messages` holding only the new message, and writes the
//! turn to it after each round (`crate::agent_threads`). `note_id` names the note open beside the panel: its id, type and title (never
//! its body) join the system prompt, so "this note" resolves through `get_note`.
//!
//! The answer is a server-sent event stream. The server runs the tool loop, at most
//! [`MAX_ROUNDS`] model calls per user turn, over read-only tools; `propose_edit`,
//! `propose_create`, `propose_archive`, `propose_delete`, `propose_type_change`,
//! `propose_type_delete`, `propose_taxonomy_change`, `propose_taxonomy_delete`, `propose_agent_change` and
//! `propose_agent_delete`, and the skill tools `propose_skill_change`, `propose_skill_file_change`,
//! `propose_skill_file_delete` and `propose_skill_delete` (`crate::ai_skill_changes`, skills S9)
//! check a change and hand it to the user (ai D14, D18, D20, D31, agents
//! G20; an agent change is also checked against the turn's agent, `crate::ai_agents`). When
//! the agent's permissions turn a kind's confirmation off and the turn's scope allows the write
//! (`write` for notes, `admin` for types, taxonomies, agents and skills; [`change_scope`]
//! caps direct changes at both caller and agent scope), the tool makes
//! the change itself with [`apply_change`], as the caller, through the writer calls the panel's
//! Apply reaches, and sends `applied` in place of the proposal (ai D22). Nothing here ever purges
//! the trash (ai D21). With a Brave Search key stored, `web_search` searches the web
//! (`crate::brave`, ai D26). An agent with `permissions.scripts.run_code` runs its own wscript
//! with `run_script`, and every agent may call the vault's valid script tools and, once it has
//! opened a skill, the skill's own (`crate::ai_scripts`, script-tools T1 to T4); the tool files
//! are proposed with `propose_tool_change` and `propose_tool_delete` under the `tools` switch.
//! `get_script` reads a saved script, and `propose_script_change` and `propose_script_delete`
//! propose one that always waits for the user's Apply (`crate::ai_script_changes`, TV-70).
//! `get_task` reads a scheduled task, and `propose_task_change` and `propose_task_delete` propose
//! one, never with `fs` paths, that always waits for the user's Apply (`crate::ai_task_changes`,
//! TV-71).
//! An agent with `permissions.subagents.allow` hands parts of a task to sub agents with
//! `start_subagent` on a thread (`crate::ai_subagents`, subagents U1 to U8): each runs as a turn
//! of its own on a session thread, the calls of one round at once, and the call's result is its
//! answer; its proposals and changes come out in this turn's stream with `subagent`, the call id,
//! and `sub_thread`, its session. `notify_user` raises a notification under the bell at once,
//! with no proposal and in unattended runs too, within per-turn, hourly and daily limits
//! (`crate::ai_notify`, TV-32 S1). On the thread of a call from a note or of notes sent to the
//! agent, `ask_user` puts a question to the user and ends the turn after its round
//! (`crate::ai_ask`, TV-77 D3).
//! Events:
//!
//! | Event | Data |
//! |---|---|
//! | `text` | `{delta}`: answer text as it arrives |
//! | `tool` | `{id, name, input}`: a tool the model called, before it runs |
//! | `tool_result` | `{id, summary, is_error}`: `summary` holds the ids and titles returned (`notes`), never bodies, a `message` on an error, for `read_asset` the `asset` read (id, filename, media type, size, kind), never its bytes, and for `fetch_url` and `download_asset` the `web` address read (url, host, media type, size, kind, and a page's title or a download's filename and asset id), never its content (ai D27) |
//! | `proposal` | a note change for the user to apply, by `op` (ai D14, D20): `update` `{id, op, note_id, title, base_version, before, after, ops}`; `create` `{id, op, type, title, after}`; `archive`, `unarchive` and `delete` `{id, op, note_id, title, base_version}`; and a file to store, `download` `{id, op, url, filename, media_type, size}` (ai D27) |
//! | `type_proposal` | `{id, name, exists, before, after, issues}`: a type file's new WCL for the user to apply (ai D18) |
//! | `taxonomy_proposal` | a taxonomy change for the user to apply (ai D31), `kind` always `taxonomy`: `create` and `change` `{id, kind, op, name, exists, before, after, renames, issues, affected}`, `renames` a list of `{from, to}` term paths and `affected` the number of notes they rewrite; `delete` `{id, kind, op, name, before}` |
//! | `agent_proposal` | an agent change for the user to apply (agents G20), `kind` always `agent` and `proposer` the agent that proposed it: `create` and `change` `{id, kind, op, name, label, exists, before, after, issues, proposer}`; `delete` `{id, kind, op, name, label, before, proposer}` |
//! | `agent_memory_proposal` | a change to another agent's memories for the user to apply (agents G21), `kind` always `agent_memory`: `{id, kind, op, memory_agent, agent_label, memory_id, base_version, title, text, before, proposer}`, `op` `create` (`memory_id`, `base_version` and `before` null), `change` (`title` and `text` the new ones, `before` `{title, text}`) or `delete` |
//! | `skill_proposal` | a skill change for the user to apply (skills S9), `kind` always `skill`: `{id, kind, op, name, path?, exists, before, after, issues, base, proposer, ...}`, `op` `create`, `change`, `file`, `file_delete` or `delete` (`crate::ai_skill_changes`) |
//! | `tool_proposal` | a script tool change for the user to apply (script-tools T3), `kind` always `tool`: `{id, kind, op, name, exists, before, after, script, script_exists, script_before, script_after, script_delete, problems, proposer}`, `op` `create`, `change` or `delete` (`crate::ai_scripts`); `runs_as` and `warnings` as in `script_proposal`, for the script the tool names, the tool itself left out |
//! | `script_proposal` | a saved script change for the user to apply (TV-70), `kind` always `script`, never made at once: `{id, kind, op, name, exists, before, after, runs_as, warnings, proposer}`, `op` `create`, `change` or `delete`, `runs_as` each handler, task, device file and tool that runs the script `{kind, name, file, event, allow_domains, secrets, read_paths, write_paths, schedule, timezone, paused}`, and `warnings` a line for each that may reach any host (`crate::ai_script_changes`) |
//! | `task_proposal` | a scheduled task change for the user to apply (TV-71), `kind` always `task`, never made at once: `{id, kind, op, name, exists, before, after, script, script_exists, script_before, script_after, task, next_runs, runs_as, warnings, proposer}`, `op` `create`, `change` or `delete`, `task` the task as the scheduler reads it `{label, description, schedule, schedule_text, timezone, script, function, allow_domains, secrets, read_paths, write_paths, retries, retry_delay_ms, timeout_ms, paused}`, `next_runs` its next three runs (none while paused or for a delete), `runs_as` what else runs its script as in `script_proposal`, and `warnings` a line for any host, for secrets with hosts, for a secret that does not exist, and, when the script changes, for each other run that may reach any host (`crate::ai_task_changes`) |
//! | `applied` | a change the tool made itself (ai D22): `{id, op, note_id, title, version}` for `update`, `create`, `archive`, `unarchive` and `delete` (whose version is the one that went to the trash); `{id, op, type_name}` for `type_change`, `type_create` and `type_delete`; `{id, op, taxonomy_name, notes}` for `taxonomy_change`, `taxonomy_create` and `taxonomy_delete` (ai D31); `{id, op, agent_name}` for `agent_create`, `agent_change` and `agent_delete` (agents G20); `{id, op, memory_id, memory_agent, version}` for `agent_memory_create`, `agent_memory_change` and `agent_memory_delete` (agents G21); `{id, op, skill_name, path?}` for `skill_create`, `skill_change`, `skill_file`, `skill_file_delete` and `skill_delete` (skills S9); `{id, op, asset_id, filename, media_type, size, url}` for `download` (ai D27) |
//! | `subagent` | `{id, call, thread, turn, agent, delegated, task, n, status, rounds, tools, started_ms, ended_ms, tool}`: a sub agent's block as it stands (subagents U6), sent as it starts, at each of its model calls and tool calls, and as it ends; `id` is the `start_subagent` call, `thread` its session, `status` `running`, `done`, `failed` or `stopped`, `turn` its turn in the turn routes (null for a sub agent of an unattended run), and `tool` the tool it runs now |
//! | `question` | `{id, text}`: the agent asked the user `text` with the `ask_user` call `id` (TV-77 D3); the turn ends after this round, and the thread reads `needs_you` until the user's next message |
//! | `context` | `{used, window}` after each model call: the tokens the call took in and gave back, and the account's context window |
//! | `compacting` | `{reason}`: a thread's compaction started, `auto` because its last call filled `compact_at` of the window, or `command` for `/compact` (`compact: true`) |
//! | `compacted` | `{reason, upto, summary, before, after}`: the model now reads `summary` in place of the thread's first `upto` messages; `before` is the tokens it summarized and `after` the summary's. `/compact` then ends with `done` |
//! | `compact_failed` | `{message}`: a compaction due on its own failed; the turn goes on with the thread as it was |
//! | `skills_updated` | `{count, skills, message}`: first in an `update_skills` turn (skills S6), the new snapshot's skill count and names and the user message the server added, a notice and the text the model reads (`crate::ai_skills`) |
//! | `usage` | `{account, name, kind, model, input, output}`: one model call's tokens, with the account's id, name and kind |
//! | `refusal` | `{account, name, kind, model, category, explanation, messages}`: the provider refused; ends the stream |
//! | `error` | `{kind, message, account, model, messages}`: the turn failed, `account` the name of the account it went to when one was chosen; ends the stream |
//! | `done` | `{stop, messages}`: the turn is over |
//!
//! `messages` is the turn's new transcript items, for the client to append and send back next
//! turn: assistant messages with their `provider_state`, and the user messages carrying tool
//! results. They are never edited afterwards. A comment goes out every [`HEARTBEAT`]. When the
//! client of `POST /api/ai/chat` goes away the loop stops, and the provider call in flight is
//! dropped, which closes its connection. The turns form (`crate::ai_turns`, ai D23) runs its
//! turns through the same [`start`], holding their events itself, so a turn on a thread runs on
//! with no client reading until it ends or is stopped (agents G17), and `POST /api/ai/apply` applies a proposal with [`apply_change`]. An agent
//! trigger's run (`crate::agent_runs`, agents G12) is a turn with no client, [`run_unattended`].
//! What it does with a change tool whose confirmation switch is on is its [`Held`] mode, from
//! its thread's origin (TV-77 D2): a trigger's run is offered none, nor any tool of
//! [`ALWAYS_PROPOSE`]; a call from a note or a dispatch keeps them, and their proposals wait in
//! the thread for the user's Apply.

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ai::{
    Accounts, AiError, Content, Credential, Credentials, ErrorKind, Event as AiEvent, Message,
    Outcome, ProviderState, Request, Role, Settings, Stop, ToolCall, ToolSpec, Usage,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, HeaderName};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Extension, Json};
use schema::Agent;
use script_core::Vault;
use serde::Deserialize;
use serde_json::{Value, json};
use state::{Author, Scope};
use tokio::sync::{mpsc, watch};
use writer::{Input, Op, TypeChange, Writer, doc_to_json};

use crate::agent_threads::{self, Busy, Stored, Thread};
use crate::ai_retry::CapacityRetry;
use crate::ai_taxonomies::apply_taxonomy;
use crate::auth::{Caller, GrantedScope};
use crate::events::HEARTBEAT;
use crate::{ApiError, agent_memory, ai_assets};

/// The most model calls one user turn makes (ai D14).
pub const MAX_ROUNDS: usize = 50;

/// The most rows `search_notes` returns.
pub const SEARCH_LIMIT: usize = 20;

/// The most rows `query_notes` returns.
pub const QUERY_LIMIT: usize = 50;

/// The longest text `get_note` returns in one value before cutting it.
pub const BODY_LIMIT: usize = 40_000;

/// The longest tool result sent back to the model.
pub const RESULT_LIMIT: usize = 100_000;

/// Fields the server sets itself, left out of a proposal's changes.
const SERVER_FIELDS: [&str; 2] = ["created", "updated"];

/// The request body.
#[derive(Debug, Deserialize)]
pub struct ChatBody {
    /// The AI account, by id or name (accounts A5).
    #[serde(default)]
    account: Option<String>,
    /// `anthropic`, `openai` or `chatgpt`: the account of before accounts, for one release.
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    messages: Vec<Message>,
    #[serde(default)]
    note_id: Option<String>,
    /// The agent whose turn this is (agents G4, G6): its model, prompt, tools and switches.
    /// Absent means the built-in Assistant, with the caller as the author of what the tools
    /// write; named, the agent is the author (agents G11).
    #[serde(default)]
    agent: Option<String>,
    /// The thread note the turn belongs to (agents G7): the server reads its transcript, so
    /// `messages` holds only the user's new message, and writes the turn to it as it streams.
    #[serde(default)]
    thread: Option<String>,
    /// `/update-skills` (skills S6): take a fresh snapshot of the skills the agent is offered
    /// for the thread's system text. The turn's user message is the server's
    /// ([`crate::ai_skills::UPDATED_TEXT`]); `messages` holds none of the user's.
    #[serde(default)]
    update_skills: bool,
    /// `/compact`: summarize the thread for the model now, and end the turn. `messages` holds
    /// none of the user's.
    #[serde(default)]
    compact: bool,
}

impl ChatBody {
    /// The agent the body names, if any.
    pub(crate) fn agent(&self) -> Option<&str> {
        self.agent
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// The thread the body names, if any.
    pub(crate) fn thread(&self) -> Option<&str> {
        self.thread
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

/// One event a turn sends: its name and its JSON data as text.
#[derive(Debug, Clone)]
pub(crate) struct Sent {
    pub(crate) name: String,
    pub(crate) data: String,
}

impl Sent {
    /// The event as a server-sent event.
    pub(crate) fn event(&self) -> Event {
        Event::default().event(&self.name).data(&self.data)
    }
}

/// What an unattended turn (an agent trigger's run, agents G12) reports: how it ended and its
/// trail lines.
#[derive(Debug, Default)]
pub(crate) struct Unattended {
    pub(crate) tool_called: bool,
    pub(crate) ended: Option<Result<(), String>>,
    pub(crate) log: Vec<String>,
}

pub(crate) type Report = Arc<std::sync::Mutex<Unattended>>;

/// The one HTTP client chat calls share.
fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(ai::client)
}

/// `POST /api/ai/chat`.
pub async fn chat(
    State(w): State<Arc<Writer>>,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Json(body): Json<ChatBody>,
) -> Result<impl IntoResponse, ApiError> {
    // No one stops this form of turn but the client, by going away.
    crate::auth::require_in_app_agent_caller(&author)?;
    let (_, never) = watch::channel(false);
    let (rx, _) = start(w, author, scope, body, never).await?;
    Ok(sse(receive(rx)))
}

/// Run one unattended turn of agent `agent` on its thread `thread`, whose user message is
/// `prompt`, and wait for it to end (agents G12). Explicit dispatch retains the caller identity
/// and scope ceiling. Autonomous runs use the agent's unattended scope. The thread's origin sets
/// its [`Held`] mode: a trigger's run is offered no change tool whose confirmation switch is on,
/// so nothing waits for a user, while a call from a note or a dispatch keeps those changes as
/// proposals in the thread (TV-77 D2). A model call that fails with a transient error is made
/// again after each of `waits` (`crate::ai_retry`).
pub(crate) async fn run_unattended(
    w: Arc<Writer>,
    agent: &Agent,
    thread: &str,
    prompt: &str,
    waits: crate::ai_retry::Waits,
    caller_scope: Option<Scope>,
    caller: Option<Author>,
) -> Unattended {
    let report: Report = Arc::default();
    let retry = CapacityRetry::new(waits, w.clone(), thread, report.clone());
    let body = ChatBody {
        account: None,
        provider: None,
        model: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![Content::text(prompt)],
            provider_state: None,
        }],
        note_id: None,
        agent: Some(agent.name.clone()),
        thread: Some(thread.to_owned()),
        update_skills: false,
        compact: false,
    };
    let author = caller.unwrap_or_else(|| Author::Agent(agent.name.clone()));
    let (_stop, never) = watch::channel(false);
    let started = start_with(
        w,
        author,
        change_scope(
            caller_scope.unwrap_or_else(|| unattended_scope(agent)),
            agent,
        ),
        body,
        never,
        Some((report.clone(), retry)),
    )
    .await;
    match started {
        Ok((mut rx, _)) => while rx.recv().await.is_some() {},
        Err(e) => {
            return Unattended {
                ended: Some(Err(e.body["message"]
                    .as_str()
                    .unwrap_or("the turn could not start")
                    .to_owned())),
                log: Vec::new(),
                tool_called: false,
            };
        }
    }
    let mut r = std::mem::take(&mut *report.lock().unwrap_or_else(|e| e.into_inner()));
    if r.ended.is_none() {
        r.ended = Some(Err("the turn stopped before it ended".to_owned()));
    }
    r
}

/// The scope an unattended run of `agent` has: `write` for notes, and `admin` when its
/// permissions allow a type, taxonomy or agent change (agents G12, G20).
pub(crate) fn unattended_scope(agent: &Agent) -> Scope {
    let p = &agent.permissions;
    let admin = [
        "propose_type_change",
        "propose_type_delete",
        "propose_taxonomy_change",
        "propose_taxonomy_delete",
        "propose_agent_change",
        "propose_agent_delete",
    ]
    .iter()
    .chain(crate::ai_skill_changes::PROPOSE_TOOLS.iter())
    .chain(crate::ai_scripts::PROPOSE_TOOLS.iter())
    .chain(crate::ai_script_changes::PROPOSE_TOOLS.iter())
    .chain(["propose_device_file_change"].iter())
    .chain(crate::ai_task_changes::PROPOSE_TOOLS.iter())
    .any(|t| p.allows_tool(t));
    if admin { Scope::Admin } else { Scope::Write }
}

/// Direct changes cannot exceed either the caller's or the agent's authority.
fn change_scope(caller: Scope, agent: &Agent) -> Scope {
    let agent = unattended_scope(agent);
    if agent.allows(caller) { caller } else { agent }
}

/// The confirmation switch that governs change tool `name`, or `None` for a tool that changes
/// nothing.
fn change_switch(name: &str) -> Option<&'static str> {
    Some(match name {
        "propose_edit" | "propose_fields" | "propose_create" => "notes",
        "propose_archive" => "archive",
        "propose_delete" => "delete",
        "propose_type_change" | "propose_type_delete" => "types",
        "propose_taxonomy_change" | "propose_taxonomy_delete" => "taxonomies",
        "propose_agent_change" | "propose_agent_delete" => "agents",
        "propose_agent_memory_change" | "propose_agent_memory_delete" => "agent_memories",
        "propose_skill_change"
        | "propose_skill_file_change"
        | "propose_skill_file_delete"
        | "propose_skill_delete" => "skills",
        "propose_tool_change" | "propose_tool_delete" => "tools",
        "download_asset" | "browser_save" | "browser_capture" => "downloads",
        _ => return None,
    })
}

/// The change tools that always propose, whatever the agent's switches say: their change waits
/// for the user's Apply in every run. A trigger's run ([`Held::Drop`]) leaves them out, as no one
/// would see the proposal; a chat, a call from a note and a dispatch offer them (TV-77 F11). A
/// new tool of that kind is added here, and [`Direct::covers`] never covers its events.
const ALWAYS_PROPOSE: &[&str] = &[
    "propose_device_file_change",
    crate::ai_script_changes::PROPOSE_TOOLS[0],
    crate::ai_script_changes::PROPOSE_TOOLS[1],
    crate::ai_task_changes::PROPOSE_TOOLS[0],
    crate::ai_task_changes::PROPOSE_TOOLS[1],
];

/// What a turn does with a change tool whose change would only be proposed: one whose
/// confirmation switch is on, or one of [`ALWAYS_PROPOSE`] (agents G12, TV-77 D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Offer it: the change waits in the thread as a proposal for the user's Apply. A chat, and
    /// an unattended run a person asked for: a call from a note or a dispatch.
    Propose,
    /// Leave it out, and say which changes are held back ([`withheld_note`]): a trigger's run,
    /// whose thread no one looks at.
    Drop,
}

/// The origins of the unattended runs a person asked for, which keep proposals ([`Held::Propose`]).
const PROPOSING_ORIGINS: [&str; 2] = [scheduler::MENTION_ORIGIN, scheduler::DISPATCH_ORIGIN];

impl Held {
    /// The mode of a turn, unattended or not, on a thread of origin `origin`. An unattended run
    /// of any other origin, a trigger's (`trigger:<name>`) or one not known here, drops them.
    fn of(unattended: bool, origin: &str) -> Held {
        if !unattended || PROPOSING_ORIGINS.contains(&origin) {
            Held::Propose
        } else {
            Held::Drop
        }
    }
}

/// The tools a turn offers: those the internet switches leave (ai D28), `web_search` only with a
/// Brave key (ai D26), only those the agent may use (agents G4), the browser tools with
/// `browser`, the agent's browser permission and a browser box, whatever its tools list says
/// (browser B6), the skill tools only when the agent uses skills, with
/// `install_skill_in_browser` beside the browser tools (skills S6), `ask_user` only when `asks`
/// (`crate::ai_ask`, TV-77 D3), and, when `drop` gives what the turn makes ([`Held::Drop`]), no
/// change tool that would only propose (agents G12).
fn turn_tools(
    web: ai::Web,
    brave: bool,
    allowed: Option<&[String]>,
    drop: Option<Direct>,
    browser: bool,
    skills: bool,
    asks: bool,
) -> Vec<ToolSpec> {
    let mut tools = offered_tools(web);
    if !asks {
        tools.retain(|t| t.name != crate::ai_ask::TOOL);
    }
    if web.search && brave {
        tools.push(crate::brave::tool_spec());
    }
    if let Some(allowed) = allowed {
        tools.retain(|t| allowed.contains(&t.name));
    }
    if !skills {
        tools.retain(|t| !crate::ai_skills::is_skill_tool(&t.name));
    }
    if browser {
        tools.extend(crate::ai_browser::tool_specs());
        if skills {
            tools.push(crate::ai_skills::install_spec());
        }
    }
    if let Some(direct) = drop {
        tools.retain(|t| {
            !ALWAYS_PROPOSE.contains(&t.name.as_str())
                && change_switch(&t.name).is_none_or(|s| direct.makes(s))
        });
    }
    tools
}

/// Each confirmation switch: its key, its label on the agent's Permissions tab, and the change
/// it guards, in the tab's order.
const SWITCHES: [(&str, &str, &str); 9] = [
    (
        "notes",
        "Ask before creating and editing notes",
        "create or edit notes",
    ),
    ("archive", "Ask before archiving notes", "archive notes"),
    (
        "delete",
        "Ask before moving notes to the trash",
        "move notes to the trash",
    ),
    (
        "types",
        "Ask before changing types",
        "change or delete types",
    ),
    (
        "downloads",
        "Ask before downloading files into the vault",
        "store files in the vault",
    ),
    (
        "taxonomies",
        "Ask before changing taxonomies",
        "change or delete taxonomies",
    ),
    (
        "agents",
        "Ask before changing agents",
        "change agents or their memories",
    ),
    ("skills", "Ask before changing skills", "change skills"),
    (
        "tools",
        "Ask before changing script tools",
        "change script tools",
    ),
];

/// For a trigger's run ([`Held::Drop`]): the changes `agent` may make with someone present that
/// this run is not offered, because their switch asks first or their tool always proposes, and
/// no one sees a proposal (agents G12), as a section of the system text. `None` when nothing is
/// held back.
fn withheld_note(agent: &Agent, direct: Direct) -> Option<String> {
    let names: std::collections::BTreeSet<String> = tool_specs()
        .into_iter()
        .chain(crate::ai_web::tool_specs())
        .chain(crate::ai_browser::tool_specs())
        .map(|t| t.name)
        .collect();
    let lines: Vec<String> = SWITCHES
        .iter()
        .filter(|(key, ..)| {
            !direct.makes(key)
                && names.iter().any(|n| {
                    change_switch(n).is_some_and(|s| s == *key) && agent.permissions.allows_tool(n)
                })
        })
        .map(|(_, label, what)| format!("- {what}: your \"{label}\" switch is on"))
        .chain(
            ALWAYS_PROPOSE
                .iter()
                .filter(|n| agent.permissions.allows_tool(n))
                .map(|n| format!("- {n}: its change always waits for the user's Apply")),
        )
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "\n## This run is unattended\n\nNo one is present to apply a proposal, so this run has \
         no tools for the changes that would only be proposed:\n\n{}\n\nWhen the task needs one \
         of them, do not say you have no tool for it. Say which change you would make, and that \
         it needs the user: in a chat with you or a call from a note, where it waits for their \
         Apply, or with that switch turned off on your Permissions tab.\n",
        lines.join("\n")
    ))
}

/// The tools a trigger's run of `agent` is offered, by name (agents G12): [`Held::Drop`]. A call
/// from a note or a dispatch is offered the change tools that would only propose too.
pub(crate) fn trigger_tools(w: &Writer, agent: &Agent) -> Vec<String> {
    let direct = Direct::of(confirm_of(agent), unattended_scope(agent));
    let brave = crate::brave::Search::load(w.state()).is_some();
    turn_tools(
        web_of(agent),
        brave,
        agent.permissions.tools.as_deref(),
        Some(direct),
        agent.permissions.browser && crate::ai_browser::BrowserBox::from_env().is_some(),
        crate::ai_skills::uses_skills(agent),
        false,
    )
    .into_iter()
    .map(|t| t.name)
    // An unattended run may start sub agents when permitted (subagents U8).
    .chain(
        agent
            .permissions
            .subagents
            .allow
            .then(|| crate::ai_subagents::TOOL.to_owned()),
    )
    .chain(
        crate::ai_scripts::offered(w, agent, &[])
            .specs
            .into_iter()
            .map(|t| t.name),
    )
    .collect()
}

/// Check `body` and start its turn, which sends its events to the receiver returned and stops
/// when the receiver goes or `stop` turns true. The flag returned turns false when the turn is
/// over. Both chat forms start their turns here.
pub(crate) async fn start(
    w: Arc<Writer>,
    author: Author,
    scope: Scope,
    body: ChatBody,
    stop: watch::Receiver<bool>,
) -> Result<(mpsc::Receiver<Sent>, Arc<AtomicBool>), ApiError> {
    start_with(w, author, scope, body, stop, None).await
}

/// [`start`], for an unattended turn when `unattended` is given: where it reports, and how its
/// model calls are retried.
async fn start_with(
    w: Arc<Writer>,
    author: Author,
    scope: Scope,
    body: ChatBody,
    stop: watch::Receiver<bool>,
    unattended: Option<(Report, CapacityRetry)>,
) -> Result<(mpsc::Receiver<Sent>, Arc<AtomicBool>), ApiError> {
    let (report, retry) = unattended.unzip();
    let account = match (
        body.account
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty()),
        body.provider
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty()),
    ) {
        (Some(a), _) => Some(a.to_owned()),
        (None, Some(p)) if schema::AGENT_PROVIDERS.contains(&p) => Some(p.to_owned()),
        (None, Some(p)) => {
            return Err(ApiError::bad_request(format!(
                "unknown AI provider {p:?}: name an account with `account`"
            )));
        }
        (None, None) => None,
    };
    if body.messages.is_empty() && !body.update_skills && !body.compact {
        return Err(ApiError::bad_request(
            "send the conversation, ending with the user's message",
        ));
    }
    let ChatBody {
        model,
        messages,
        note_id,
        agent,
        thread,
        update_skills,
        compact,
        ..
    } = body;
    let writer = w.clone();
    let by = author.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        prepare(
            &writer,
            &by,
            agent,
            thread,
            messages,
            update_skills,
            compact,
        )
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))??;
    let named = prepared.named;
    let actor = if named {
        Author::Agent(prepared.agent.name.clone())
    } else {
        author.clone()
    };
    // An unattended turn runs on a thread its trigger or call opened (activity Y2).
    let origin = match (&report, &prepared.origin) {
        (Some(_), Some(o)) => o.clone(),
        _ => "user".to_owned(),
    };
    // A turn on a call's or a dispatch's thread may ask the user, the user's reply turn too
    // (TV-77 D3): keyed on the thread's own origin, which a reply turn's `origin` is not.
    let asks = prepared
        .origin
        .as_deref()
        .is_some_and(crate::ai_ask::asks_on);
    let cfg = TurnCfg {
        owner: author,
        author: actor,
        scope,
        agent_name: named.then(|| prepared.agent.name.clone()),
        unattended: report.is_some(),
        held: Held::of(report.is_some(), &origin),
        asks,
        retry,
        sub: None,
        origin,
        notified: None,
    };
    Ok(spawn_turn(
        w, prepared, cfg, account, model, note_id, stop, report,
    ))
}

/// Who a turn runs as and for whom, apart from its agent and thread.
struct TurnCfg {
    /// Who started it: the caller, or the agent of an unattended run.
    owner: Author,
    /// The author of what its tools write.
    author: Author,
    scope: Scope,
    /// The agent its proposals name, so their apply is authored by it.
    agent_name: Option<String>,
    /// Nobody follows it as it runs: it reports its end and is not listed as a turn (agents
    /// G12, subagents U8).
    unattended: bool,
    /// What it does with a change that would only be proposed (TV-77 D2).
    held: Held,
    /// Whether it offers `ask_user` (TV-77 D3).
    asks: bool,
    /// How an unattended turn retries a model call that failed with a transient error.
    retry: Option<CapacityRetry>,
    /// For a sub agent's turn (subagents U1).
    sub: Option<SubMark>,
    /// Where it came from, for the activity log.
    origin: String,
    /// The notifications sent by the turn that started a sub agent, which it shares; a turn that
    /// starts none has its own (TV-74).
    notified: Option<Arc<crate::ai_notify::NotifyCount>>,
}

/// What a sub agent's turn is (subagents U1).
#[derive(Debug, Clone)]
struct SubMark {
    parent: String,
    /// A throwaway sub agent runs as its parent and writes no memories of its own.
    throwaway: bool,
    /// Its number in the parent thread: a throwaway's changes are authored `<agent>/sub:<n>`.
    n: usize,
}

/// Start the turn `prepared` holds, which sends its events to the receiver returned and stops
/// when the receiver goes or `stop` turns true. The flag returned turns false when it is over.
#[allow(clippy::too_many_arguments)]
fn spawn_turn(
    w: Arc<Writer>,
    prepared: Prepared,
    cfg: TurnCfg,
    account: Option<String>,
    model: Option<String>,
    note_id: Option<String>,
    stop: watch::Receiver<bool>,
    report: Option<Report>,
) -> (mpsc::Receiver<Sent>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel::<Sent>(64);
    let activity = crate::ai_activity::TurnActivity::new(
        w.clone(),
        cfg.author.clone(),
        &prepared.agent.name,
        prepared.log.as_ref().map(|l| l.id.clone()),
        &cfg.origin,
    );
    let turn = Turn {
        writer: w,
        author: cfg.author,
        owner: cfg.owner,
        scope: cfg.scope,
        out: Out { tx, stop },
        images: ai_assets::Images::default(),
        agent_name: cfg.agent_name,
        log: prepared.log.map(std::sync::Mutex::new),
        report,
        unattended: cfg.unattended,
        held: cfg.held,
        asks: cfg.asks,
        retry: cfg.retry,
        sub: cfg.sub,
        activity,
        skills: prepared.skills,
        skills_updated: prepared.skills_updated,
        compact: prepared.compact,
        notified: cfg.notified.unwrap_or_default(),
    };
    let agent = prepared.agent;
    let history = prepared.history;
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    tokio::spawn(async move {
        turn.run(agent, account, model, history, note_id).await;
        flag.store(false, Ordering::SeqCst);
    });
    (rx, running)
}

/// A turn's agent and the transcript it starts from.
struct Prepared {
    agent: Agent,
    /// Whether the body named the agent.
    named: bool,
    /// The messages the first request sends.
    history: Vec<Message>,
    log: Option<ThreadLog>,
    /// The thread's origin, for the activity log.
    origin: Option<String>,
    /// The skills the system text lists (skills S6).
    skills: crate::ai_skills::Snapshot,
    /// For a `/update-skills` turn, the `skills_updated` event's data.
    skills_updated: Option<Value>,
    /// The compaction to make before the turn's first call, if one is due.
    compact: Option<PendingCompact>,
}

/// A compaction due before a turn: why, how many of the thread's messages it covers, and the
/// compaction it replaces, through which the model saw the older ones.
struct PendingCompact {
    reason: &'static str,
    upto: usize,
    previous: Option<agent_threads::Compaction>,
}

/// What the model is asked for when a thread is compacted.
const COMPACT_PROMPT: &str = "Summarize the conversation so far, so that you can carry on from your summary alone once the earlier messages are dropped from what you are sent. Keep: what the user wants and asked for, the decisions made, the facts found with the ids and titles of the notes they came from, the changes proposed or made and how each ended, the open questions, and what you were about to do next. Write it as working notes for yourself, short but complete. Do not call any tool.";

/// The agent a turn runs as: the one named, or the built-in Assistant. A disabled agent takes
/// no turn.
pub(crate) fn turn_agent(w: &Writer, name: Option<&str>) -> Result<Agent, ApiError> {
    let agent = match name {
        Some(n) => w.agent(n).map_err(|e| match e {
            writer::Error::Missing(m) => ApiError::new(StatusCode::NOT_FOUND, "not_found", m),
            e => e.into(),
        })?,
        None => crate::ai_settings::assistant(w)?,
    };
    if !agent.enabled {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "agent_disabled",
            format!("{} is disabled: enable it to chat with it", agent.label),
        ));
    }
    Ok(agent)
}

/// Load the turn's agent, and for a thread its transcript, which the user's new message joins
/// at once, after the lines on the outcomes the model has not heard of (ai D29), so a closed tab
/// loses nothing. The skills the system text lists are the thread's snapshot, taken on its
/// first turn or fresh with `update_skills`, whose user message is the server's (skills S6).
fn prepare(
    w: &Writer,
    author: &Author,
    agent: Option<String>,
    thread: Option<String>,
    messages: Vec<Message>,
    update_skills: bool,
    compact: bool,
) -> Result<Prepared, ApiError> {
    let named = agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let agent = turn_agent(w, named.as_deref())?;
    prepare_for(
        w,
        author,
        agent,
        named.is_some(),
        thread,
        messages,
        update_skills,
        compact,
    )
}

/// [`prepare`] for `agent` as it is given, which a sub agent's turn runs with its own rights
/// (subagents U3); `named` says whether the body named it.
#[allow(clippy::too_many_arguments)]
fn prepare_for(
    w: &Writer,
    author: &Author,
    agent: Agent,
    named: bool,
    thread: Option<String>,
    mut messages: Vec<Message>,
    update_skills: bool,
    compact: bool,
) -> Result<Prepared, ApiError> {
    let browser = agent.permissions.browser && crate::ai_browser::BrowserBox::from_env().is_some();
    let fresh = || crate::ai_skills::snapshot(w, &agent, browser);
    let updated = |s: &crate::ai_skills::Snapshot| {
        let message = Message {
            role: Role::User,
            content: vec![
                Content::Notice {
                    kind: crate::ai_skills::UPDATED_NOTICE.to_owned(),
                    text: format!("Skills updated ({})", s.skills.len()),
                },
                Content::text(crate::ai_skills::UPDATED_TEXT),
            ],
            provider_state: None,
        };
        let names: Vec<&str> = s.skills.iter().map(|l| l.name.as_str()).collect();
        let data = json!({ "count": names.len(), "skills": names, "message": message });
        (message, data)
    };
    let Some(id) = thread
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        if compact {
            return Err(ApiError::bad_request(
                "/compact works on a thread: send the turn with its thread",
            ));
        }
        // With no thread, nothing keeps a snapshot: each turn lists the skills as they are.
        let skills = fresh();
        let mut skills_updated = None;
        if update_skills {
            let (message, data) = updated(&skills);
            messages.push(message);
            skills_updated = Some(data);
        }
        return Ok(Prepared {
            agent,
            named,
            history: messages,
            log: None,
            origin: None,
            skills,
            skills_updated,
            compact: None,
        });
    };
    let thread = Thread::read(w, &id)?;
    if thread.agent != agent.name {
        return Err(ApiError::bad_request(format!(
            "thread {id} belongs to agent {:?}, not {:?}",
            thread.agent, agent.name
        )));
    }
    let busy = Busy::take(&thread.id, &agent.name)?;
    let origin = Some(thread.origin.clone());
    // Sending in an archived thread takes it out of the archive (agents G15).
    if thread.archived {
        w.set_archived(&thread.id, false, Some(&thread.version), author)?;
    }
    let mut transcript = thread.transcript.clone();
    let lines = transcript.take_outcome_lines();
    // The thread's snapshot, taken now on its first turn or when the user asked.
    let skills = match transcript.skills.clone() {
        Some(s) if !update_skills => s,
        _ => fresh(),
    };
    let mut skills_updated = None;
    // A message from the user answers a question the agent asked (TV-77 D3); a `/compact` or a
    // `/update-skills` turn alone answers nothing.
    let answers = !messages.is_empty();
    let mut new = messages;
    if update_skills {
        let (message, data) = updated(&skills);
        new.push(message);
        skills_updated = Some(data);
    }
    if !lines.is_empty()
        && let Some(first) = new.iter_mut().find(|m| m.role == Role::User)
    {
        first.content.insert(0, Content::text(lines.join("\n")));
    }
    let question = new
        .iter()
        .rev()
        .flat_map(|m| m.content.iter())
        .find_map(|c| match c {
            Content::Text { text }
                if !agent_threads::is_outcome_lines(text)
                    && !agent_threads::is_attachments(text) =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .unwrap_or_default();
    let unnamed = thread.title.trim().is_empty() || thread.title == agent_threads::DEFAULT_TITLE;
    let title = if unnamed && !update_skills {
        agent_threads::title_of(&question)
    } else {
        None
    };
    // A compaction, asked for or due because the last call filled enough of the window, covers
    // every message before this turn's.
    let upto = transcript.messages.len();
    // Unreadable settings compact nothing on their own; the turn reports them when it starts.
    let settings = Settings::load(w.state()).unwrap_or_default();
    let full = transcript.context.is_some_and(|c| {
        c.used.saturating_mul(100)
            >= c.window
                .saturating_mul(u64::from(settings.context.compact_at()))
    });
    let covered = transcript.compaction.as_ref().map_or(0, |c| c.upto);
    let reason = if compact {
        Some("command")
    } else if settings.context.auto_compact && full && upto > covered {
        Some("auto")
    } else {
        None
    };
    let pending = reason.map(|reason| PendingCompact {
        reason,
        upto,
        previous: transcript.compaction.clone(),
    });
    let mut history = transcript.messages.clone();
    history.extend(new);
    let base = history.clone();
    agent_threads::save(w, &thread.id, &agent.label, None, |t| {
        t.transcript.messages = base.clone();
        t.transcript.take_outcome_lines();
        t.transcript.skills = Some(skills.clone());
        if let Some(title) = &title {
            t.title = title.clone();
        }
        // The answer clears the question in the same write that adds it (TV-77 D3).
        if answers {
            t.question.clear();
        }
    })?;
    let log = ThreadLog {
        id: thread.id,
        label: agent.label.clone(),
        base: history.clone(),
        steps: serde_json::Map::new(),
        proposals: Vec::new(),
        subagents: Vec::new(),
        subs_before: transcript.subagents.len(),
        sub_changes: 0,
        saved: (0, 0, 0),
        context: None,
        question: None,
        _busy: busy,
    };
    Ok(Prepared {
        agent,
        named: true,
        history: transcript.model_view(&history),
        log: Some(log),
        origin,
        skills,
        skills_updated,
        compact: pending,
    })
}

/// The events of a proposal for the user, or of a change a tool made.
const PROPOSAL_EVENTS: [&str; 11] = [
    "proposal",
    "type_proposal",
    "taxonomy_proposal",
    "agent_proposal",
    "agent_memory_proposal",
    "skill_proposal",
    "tool_proposal",
    "device_file_proposal",
    "script_proposal",
    "task_proposal",
    "applied",
];

/// What a turn on a thread has added, written to the thread note as the turn streams.
struct ThreadLog {
    id: String,
    label: String,
    /// The transcript the turn started from, the user's new message included.
    base: Vec<Message>,
    /// The tool steps of this turn, by call id.
    steps: serde_json::Map<String, Value>,
    /// The proposals and changes of this turn.
    proposals: Vec<Stored>,
    /// The sub agent sessions this turn started (subagents U5).
    subagents: Vec<agent_threads::SubSession>,
    /// How many sub agent sessions the thread had before the turn.
    subs_before: usize,
    /// Bumped with each change to `subagents`.
    sub_changes: usize,
    /// How many messages, steps and proposals, and sub agent changes the last write held.
    saved: (usize, usize, usize),
    /// How full the context was after the turn's last call.
    context: Option<agent_threads::ContextUse>,
    /// The question the turn asked the user with `ask_user`, written with its last round
    /// (TV-77 D3).
    question: Option<String>,
    _busy: Busy,
}

/// The events of `rx` as a server-sent event answer, with a heartbeat every [`HEARTBEAT`].
pub(crate) fn sse<S>(events: S) -> impl IntoResponse
where
    S: futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    let body = Sse::new(events).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat"));
    (
        [
            (CACHE_CONTROL, "no-cache"),
            (HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        body,
    )
}

/// The events a turn sends, in order, ending when it does.
pub(crate) fn receive(
    rx: mpsc::Receiver<Sent>,
) -> impl futures_util::Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        let e = rx.recv().await?;
        Some((Ok::<_, Infallible>(e.event()), rx))
    })
}

/// Where the events go. A send fails once the client has gone or the turn was stopped.
struct Out {
    tx: mpsc::Sender<Sent>,
    /// Turns true when the turn is stopped (`DELETE /api/ai/chat/turns/{turn}`).
    stop: watch::Receiver<bool>,
}

impl Out {
    async fn send(&self, name: &str, data: Value) -> bool {
        if self.gone() {
            return false;
        }
        let e = Sent {
            name: name.to_owned(),
            data: data.to_string(),
        };
        tokio::select! {
            sent = self.tx.send(e) => sent.is_ok(),
            _ = self.stopped() => false,
        }
    }

    fn gone(&self) -> bool {
        self.tx.is_closed() || *self.stop.borrow()
    }

    /// Resolves once the turn is stopped. With no one left to stop it, never.
    async fn stopped(&self) {
        let mut stop = self.stop.clone();
        if stop.wait_for(|s| *s).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Resolves once the client has gone or the turn is stopped.
    async fn closed(&self) {
        tokio::select! {
            _ = self.tx.closed() => {}
            _ = self.stopped() => {}
        }
    }
}

/// What a turn needs before the first call: the model and its credential, and the prompt.
struct Setup {
    model: String,
    cred: Credential,
    system: String,
    /// The changes the tools make themselves this turn.
    direct: Direct,
    /// Brave Search, when its key is stored: `web_search` is offered only then (ai D26).
    web_search: Option<crate::brave::Search>,
    /// Whether the web tools may reach the local network (`ai_settings.fetch_private`, ai D27).
    fetch_private: bool,
    /// Which internet tools the agent is given (ai D28, agents G4).
    web: ai::Web,
    /// The agent's effort, when it names one.
    effort: Option<ai::Effort>,
    /// The tools the agent may use; `None` is every tool the server offers (agents G4).
    tools: Option<Vec<String>>,
    /// The account's context window in tokens.
    window: u64,
    /// The note types the agent's tools reach (agents G4, G7).
    limits: Limits,
    /// The agent whose turn it is, which the agent tools check a change against (agents G20).
    agent: Arc<Agent>,
    /// The browser box, when the server has one and the agent may use it (browser B6).
    browser: Option<crate::ai_browser::BrowserBox>,
}

/// The note types an agent's note tools reach (agents G4): those its `permissions.types` names,
/// or every type. Agent threads and agent memories are left out unless the list names their type,
/// so an agent does not read chats back as vault content (agents G7), and reaches memories only
/// through its memory tools (agents G21).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Limits {
    types: Option<Vec<String>>,
    change_types: Option<Vec<String>>,
    change_fields: std::collections::BTreeMap<String, Vec<String>>,
    parent: Option<String>,
}

impl Limits {
    pub(crate) fn of(agent: &Agent) -> Limits {
        Limits::of_permissions(&agent.permissions)
    }

    pub(crate) fn of_permissions(p: &schema::AgentPermissions) -> Limits {
        Limits {
            types: p.types.clone(),
            change_types: p.change_types.clone(),
            change_fields: p.change_fields.clone(),
            parent: None,
        }
    }

    /// Whether the agent may read and change notes of type `name`.
    pub(crate) fn allows(&self, name: &str) -> bool {
        match &self.types {
            None => name != agent_threads::THREAD_TYPE && name != agent_memory::MEMORY_TYPE,
            Some(t) => t.iter().any(|n| n == name),
        }
    }

    pub(crate) fn allows_change(&self, name: &str) -> bool {
        self.allows(name)
            && self
                .change_types
                .as_ref()
                .is_none_or(|types| types.iter().any(|t| t == name))
    }

    fn check_whole_type(&self, name: &str) -> Result<(), ApplyError> {
        if !self.allows_change(name) || self.change_fields.contains_key(name) {
            return Err(ApplyError::Refused(format!(
                "This agent may not rewrite {name} notes through a definition change."
            )));
        }
        Ok(())
    }

    fn check_event(&self, w: &Writer, event: &str, p: &Value) -> Result<(), ApplyError> {
        // Legacy read ceilings did not restrict definition operations.
        if matches!(event, "type_proposal" | "taxonomy_proposal")
            && self.change_types.is_none()
            && self.change_fields.is_empty()
        {
            return Ok(());
        }
        match event {
            "proposal" => self.check_change(w, p),
            "type_proposal" => self.check_whole_type(p["name"].as_str().unwrap_or_default()),
            "taxonomy_proposal" => {
                if p["op"] == "delete" {
                    if self.types.is_some()
                        || self.change_types.is_some()
                        || !self.change_fields.is_empty()
                    {
                        return Err(ApplyError::Refused(
                            "A restricted agent may not delete taxonomies.".into(),
                        ));
                    }
                    return Ok(());
                }
                let renames = crate::ai_taxonomies::renames_of(p)?;
                if renames.is_empty() {
                    return Ok(());
                }
                let check = w.check_taxonomy(
                    p["name"].as_str().unwrap_or_default(),
                    p["after"].as_str().unwrap_or_default(),
                    &renames,
                )?;
                for id in check.notes {
                    self.check_whole_type(&w.read(&id)?.type_name)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Check the actual current note and replacement, never the proposal's claimed diff.
    fn check_change(&self, w: &Writer, p: &Value) -> Result<(), ApplyError> {
        let op = p["op"].as_str().unwrap_or("update");
        if op == "download" {
            return Ok(());
        }
        let current = if op == "create" {
            None
        } else {
            Some(w.read(p["note_id"].as_str().unwrap_or_default())?)
        };
        let t = current
            .as_ref()
            .map(|n| n.type_name.as_str())
            .unwrap_or_else(|| p["type"].as_str().unwrap_or_default());
        if t == agent_threads::THREAD_TYPE && matches!(op, "create" | "update") {
            return Err(ApplyError::Refused(
                "Agent transcripts are written only by the agent runtime.".into(),
            ));
        }
        if !self.allows_change(t) {
            return Err(ApplyError::Refused(format!(
                "This agent may not change {t} notes."
            )));
        }
        if let Some(fields) = self.change_fields.get(t) {
            if op != "update" {
                return Err(ApplyError::Refused(format!(
                    "This agent may only edit allowed fields of {t} notes."
                )));
            }
            let before = doc_to_json(&current.as_ref().expect("update has current note").doc);
            let after = &p["after"];
            let empty = serde_json::Map::new();
            let bf = before["fields"].as_object().unwrap_or(&empty);
            let af = after["fields"].as_object().unwrap_or(&empty);
            if before["blocks"] != after["blocks"]
                || bf.keys().chain(af.keys()).any(|k| {
                    !SERVER_FIELDS.contains(&k.as_str())
                        && !fields.contains(k)
                        && bf.get(k) != af.get(k)
                })
            {
                return Err(ApplyError::Refused(format!(
                    "This agent may only change allowed fields of {t} notes."
                )));
            }
        }
        Ok(())
    }

    /// `rows` without the notes of types the agent may not read, and how many went.
    fn keep(&self, rows: &mut Vec<Value>) -> usize {
        let before = rows.len();
        rows.retain(|r| self.allows(r["type"].as_str().unwrap_or_default()));
        before - rows.len()
    }

    /// The refusal of a call that names a note, or a type of note, the agent may not reach.
    fn check(&self, w: &Writer, call: &ToolCall) -> Option<ToolOut> {
        let input = &call.input;
        let (type_name, verb) = match call.name.as_str() {
            "propose_create" => (input["type"].as_str()?.trim().to_owned(), "change"),
            "get_note" | "propose_edit" | "propose_fields" | "propose_archive"
            | "propose_delete" => {
                let id = input["id"].as_str()?.trim();
                if !w.agent_may_read(id) {
                    return Some(ToolOut::error(format!("Note {id} is held from agents.")));
                }
                let n = w.read(id).ok()?;
                let verb = if call.name == "get_note" {
                    "read"
                } else {
                    "change"
                };
                (n.type_name, verb)
            }
            _ => return None,
        };
        (!(if verb == "read" {
            self.allows(&type_name)
        } else {
            self.allows_change(&type_name)
        }))
        .then(|| ToolOut::error(format!("This agent may not {verb} {type_name} notes.")))
    }
}

/// The kinds of change the tools make themselves rather than propose (ai D22): those whose
/// confirmation the agent's permissions turned off and whose write the turn's scope
/// ([`change_scope`]) allows. Lacking the scope, a tool proposes, whatever the switch says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Direct {
    notes: bool,
    archive: bool,
    delete: bool,
    types: bool,
    /// `download_asset` stores the file at once (ai D27).
    downloads: bool,
    /// Taxonomy changes and deletes (ai D31).
    taxonomies: bool,
    /// Agent creates, changes and deletes (agents G20).
    agents: bool,
    /// Changes to other agents' memories (agents G21), under the agents switch: memories are
    /// notes, so `write` makes them.
    memories: bool,
    /// Skill creates, changes and deletes (skills S9).
    skills: bool,
    /// Script tool creates, changes and deletes (script-tools T3).
    tools: bool,
}

impl Direct {
    fn of(confirm: ai::Confirm, scope: Scope) -> Direct {
        let write = scope.allows(Scope::Write);
        Direct {
            notes: !confirm.notes && write,
            archive: !confirm.archive && write,
            delete: !confirm.delete && write,
            types: !confirm.types && scope.allows(Scope::Admin),
            downloads: !confirm.downloads && write,
            taxonomies: !confirm.taxonomies && scope.allows(Scope::Admin),
            agents: !confirm.agents && scope.allows(Scope::Admin),
            memories: !confirm.agents && write,
            skills: !confirm.skills && scope.allows(Scope::Admin),
            tools: !confirm.tools && scope.allows(Scope::Admin),
        }
    }

    /// Whether changes under confirmation switch `switch` are made rather than proposed.
    fn makes(self, switch: &str) -> bool {
        match switch {
            "notes" => self.notes,
            "archive" => self.archive,
            "delete" => self.delete,
            "types" => self.types,
            "downloads" => self.downloads,
            "taxonomies" => self.taxonomies,
            "agents" => self.agents,
            "agent_memories" => self.memories,
            "skills" => self.skills,
            "tools" => self.tools,
            _ => false,
        }
    }

    /// Whether the change in event `event` with data `p` is made rather than proposed.
    fn covers(self, event: &str, p: &Value) -> bool {
        match event {
            "type_proposal" => self.types && !type_handlers_changed(p),
            "taxonomy_proposal" => self.taxonomies,
            // Turning on an agent's Run wscript code waits for an admin's Apply, whatever the
            // switch says (script-tools T1), and so do Run saved scripts and tasks and granting
            // sub agents (subagents U2).
            "agent_proposal" => {
                self.agents
                    && p["grants_run_code"] != true
                    && p["grants_run_saved"] != true
                    && p["grants_subagents"] != true
            }
            "tool_proposal" => {
                self.tools && !p["script_after"].is_string() && !tool_binding_changed(p)
            }
            // The tools of [`ALWAYS_PROPOSE`]: always the user's Apply (TV-32 Q2 O1).
            "script_proposal" | "task_proposal" | "device_file_proposal" => false,
            "agent_memory_proposal" => self.memories,
            "skill_proposal" => {
                self.skills
                    && !p["path"].as_str().is_some_and(|s| {
                        let s = s.to_ascii_lowercase();
                        s == "tools"
                            || s.starts_with("tools/")
                            || s == "scripts"
                            || s.starts_with("scripts/")
                    })
            }
            "proposal" => match p["op"].as_str().unwrap_or("update") {
                "update" | "create" => self.notes,
                "archive" | "unarchive" => self.archive,
                "delete" => self.delete,
                "download" => self.downloads,
                _ => false,
            },
            _ => false,
        }
    }
}

/// The agent's confirmation switches as the chat reads them (ai D22).
fn confirm_of(agent: &Agent) -> ai::Confirm {
    let c = agent.permissions.confirm;
    ai::Confirm {
        notes: c.notes,
        archive: c.archive,
        delete: c.delete,
        types: c.types,
        downloads: c.downloads,
        taxonomies: c.taxonomies,
        agents: c.agents,
        skills: c.skills,
        tools: c.tools,
    }
}

/// The agent's internet switches as the chat reads them (ai D28).
fn web_of(agent: &Agent) -> ai::Web {
    let w = agent.permissions.web;
    ai::Web {
        search: w.search,
        read: w.read,
        download: w.download,
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    w: &Writer,
    scope: Scope,
    agent: &Agent,
    account: Option<String>,
    model: Option<String>,
    note_id: Option<String>,
    skills: Option<&str>,
    held: Held,
    asks: bool,
) -> Result<Setup, AiError> {
    let state = w.state();
    let settings = Settings::load(state)?;
    // The request's choice, made in the chat box, wins; without one the agent's account and
    // model, and then the default account and its model (agents G1, accounts A5). A model alone
    // runs on the agent's account; an account alone on that account's own model, since the
    // agent's model belongs to the agent's account. An agent naming an account that does not
    // exist runs on the default account.
    let accounts = Accounts::load(state)?;
    let model = model.filter(|m| !m.trim().is_empty());
    let own = agent.account_ref();
    let picked = accounts.pick(account.as_deref(), own)?.clone();
    let model = match (account.is_some(), model) {
        (_, Some(m)) => Some(m.trim().to_owned()),
        (true, None) => picked.model().map(str::to_owned),
        (false, None) => agent
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| picked.model().map(str::to_owned)),
    };
    let cred = Credentials::resolve(state, &picked)?;
    let model = match model {
        Some(m) => m,
        None => ai::default_model_blocking(&ai::blocking_client(), &cred, None)?,
    };
    // The agent's prompt, or the shipped one (ai D17, agents G6); the open note follows it.
    let mut system = agent
        .system_prompt
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(ai::CHAT_PROMPT)
        .to_owned();
    if !system.ends_with('\n') {
        system.push('\n');
    }
    // The agent's memories follow its prompt, so the prompt itself stays a stable prefix for
    // prompt caching (agents G21).
    if let Some(memories) = agent_memory::prompt_section(w, &agent.name) {
        system.push_str(&memories);
    }
    if let Some(id) = note_id
        .as_deref()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .filter(|id| w.agent_may_read(id))
        && let Ok(n) = w.read(&id)
    {
        let doc = doc_to_json(&n.doc);
        let title = doc["fields"]["title"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        system.push_str(&format!(
            "\n## The open note\n\nThe user has note {id} open beside this chat: type `{}`, \
             title {title:?}. \"This note\" means it; read it with get_note.\n",
            n.type_name
        ));
    }
    if note_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty() && !w.agent_may_read(id))
    {
        system.push_str("\nThis note is held from agents; the user can review and release it.\n");
    }
    // What the agent can do with wscript, so it opens the wscript skill first (script-tools T5).
    let scripts = if change_scope(scope, agent).allows(Scope::Write) {
        crate::ai_scripts::offered(w, agent, &[])
    } else {
        crate::ai_scripts::ScriptTools::default()
    };
    let skills_offered = crate::ai_skills::uses_skills(agent)
        && agent
            .permissions
            .allows_skill(crate::ai_scripts::WSCRIPT_SKILL);
    if let Some(note) = crate::ai_scripts::prompt_note(&scripts, skills_offered) {
        system.push_str(&note);
    }
    let direct = Direct::of(confirm_of(agent), change_scope(scope, agent));
    // A trigger's run is offered no change tool that would only propose (agents G12): say
    // which, so the model names the switch rather than saying it has no tool (TV-34).
    if held == Held::Drop
        && let Some(note) = withheld_note(agent, direct)
    {
        system.push_str(&note);
    }
    // The rule for ask_user, which a replaced system prompt would not hold (TV-77 F8), when the
    // agent may call it.
    if asks && agent.permissions.allows_tool(crate::ai_ask::TOOL) {
        system.push_str(crate::ai_ask::prompt_section());
    }
    // The skills come last, the thread's snapshot, so what comes before stays a stable prefix
    // (skills S6).
    if let Some(section) = skills {
        system.push_str(section);
    }
    Ok(Setup {
        window: picked.window(),
        model,
        cred,
        system,
        direct,
        web_search: crate::brave::Search::load(state),
        fetch_private: settings.fetch_private,
        web: web_of(agent),
        effort: agent.effort.as_deref().and_then(|e| e.parse().ok()),
        tools: agent.permissions.tools.clone(),
        limits: Limits::of(agent),
        agent: Arc::new(agent.clone()),
        browser: agent
            .permissions
            .browser
            .then(crate::ai_browser::BrowserBox::from_env)
            .flatten(),
    })
}

/// How one model call ended.
enum Called {
    Done {
        text: String,
        calls: Vec<ToolCall>,
        stop: Stop,
        state: Option<ProviderState>,
        usage: Usage,
    },
    Failed {
        text: String,
        error: AiError,
    },
    /// The client went away.
    Gone,
}

/// A tool's answer.
pub(crate) struct ToolOut {
    pub(crate) content: String,
    pub(crate) is_error: bool,
    pub(crate) summary: Value,
    /// An event sent before the result, its name and data: a proposal for the user to act on,
    /// or the change the tool made (ai D22).
    pub(crate) proposal: Option<(&'static str, Value)>,
}

impl ToolOut {
    pub(crate) fn error(message: impl Into<String>) -> ToolOut {
        let message = message.into();
        ToolOut {
            content: json!({ "error": message }).to_string(),
            is_error: true,
            summary: json!({ "notes": [], "message": message }),
            proposal: None,
        }
    }
}

struct Turn {
    writer: Arc<Writer>,
    author: Author,
    /// Who started the turn: the sub agent turns it starts are theirs to read and stop
    /// (subagents U6).
    owner: Author,
    /// The scope the caller's credentials grant, which bounds what the tools write (ai D22).
    scope: Scope,
    out: Out,
    /// The images prepared this turn, by asset id (ai D19).
    images: ai_assets::Images,
    /// The agent the request named, which its proposals carry so their apply is authored by it
    /// (agents G11).
    agent_name: Option<String>,
    /// The thread the turn writes to (agents G7).
    log: Option<std::sync::Mutex<ThreadLog>>,
    /// For an unattended turn (agents G12): how it ended and its trail lines.
    report: Option<Report>,
    /// Nobody follows the turn as it runs (agents G12), nor a sub agent of such a turn
    /// (subagents U8).
    unattended: bool,
    /// What the turn does with a change that would only be proposed, as does a sub agent of it,
    /// whose proposals wait in this turn's thread (TV-77 D2).
    held: Held,
    /// Whether the turn offers `ask_user`: on a thread of a call from a note or of notes sent to
    /// the agent, never in a sub agent's turn (TV-77 D3).
    asks: bool,
    /// How an unattended turn, and a sub agent of one, retries a model call that failed with a
    /// transient error (`crate::ai_retry`).
    retry: Option<CapacityRetry>,
    /// For a sub agent's turn (subagents U1).
    sub: Option<SubMark>,
    /// The turn's entries in the activity log (activity Y2).
    activity: crate::ai_activity::TurnActivity,
    /// The skills the system text lists (skills S6).
    skills: crate::ai_skills::Snapshot,
    /// For a `/update-skills` turn, the `skills_updated` event's data, sent first.
    skills_updated: Option<Value>,
    /// The compaction to make before the first call.
    compact: Option<PendingCompact>,
    /// The notifications this turn and its sub agent turns have sent (TV-74).
    notified: Arc<crate::ai_notify::NotifyCount>,
}

impl Turn {
    async fn run(
        self,
        agent: Agent,
        account: Option<String>,
        model: Option<String>,
        history: Vec<Message>,
        note_id: Option<String>,
    ) {
        let mut added = Vec::new();
        let name = agent.name.clone();
        self.activity.start();
        if let Some(data) = self.skills_updated.clone() {
            self.out.send("skills_updated", data).await;
        }
        self.rounds(agent, account, model, history, note_id, &mut added)
            .await;
        // What the turn added before it stopped, the client gone or not.
        self.persist(&added).await;
        // A turn that ended otherwise was stopped, or its client went away.
        self.activity.end("stopped", None);
        // The items the turn drafted into Review share one notification, which the writer raised
        // as each was written (outbox X10); the agent's next turn starts another. The Outbox
        // tools draft as the agent, the Assistant included.
        self.writer.end_outbox_drafts(&name);
    }

    /// Keep what an unattended turn reports.
    fn report(&self, f: impl FnOnce(&mut Unattended)) {
        if let Some(r) = &self.report {
            f(&mut r.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        }
    }

    fn log(&self) -> Option<std::sync::MutexGuard<'_, ThreadLog>> {
        self.log
            .as_ref()
            .map(|l| l.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// Send event `name`, first keeping it in the thread: a step's summary, a proposal as
    /// pending, a change the agent made as made. A proposal carries the agent and the thread, so
    /// its apply is authored by the agent and recorded in the thread.
    async fn emit(&self, name: &'static str, mut data: Value) -> bool {
        let proposal = PROPOSAL_EVENTS.contains(&name);
        if proposal && let Some(a) = &self.agent_name {
            data["agent"] = json!(a);
            // A throwaway sub agent's change is applied as it, `<agent>/sub:<n>` (subagents U4).
            if let Some(sub) = self.sub.as_ref().filter(|s| s.throwaway) {
                data["sub"] = json!(sub.n);
            }
        }
        self.keep(name, proposal, data, true).await
    }

    /// Send event `name` of a sub agent's in this turn's stream (subagents U4): its proposals and
    /// changes as this turn's own, kept in its thread, with the agent and number it names.
    async fn emit_sub(&self, name: &str, data: Value) -> bool {
        let Some(name) = PROPOSAL_EVENTS.iter().find(|e| **e == name) else {
            return false;
        };
        // The sub agent's own turn put it in the activity log.
        self.keep(name, true, data, false).await
    }

    /// [`Turn::emit`]'s work once the data is whole: the activity entry when `record` says so,
    /// the thread's copy, and the event.
    async fn keep(
        &self,
        name: &'static str,
        proposal: bool,
        mut data: Value,
        record: bool,
    ) -> bool {
        if record && name == "tool_result" {
            self.activity.tool_answered(
                data["id"].as_str().unwrap_or_default(),
                &data["summary"],
                data["is_error"] == true,
            );
        } else if record && proposal {
            self.activity.change(name, &data);
        }
        if let Some(mut log) = self.log() {
            if name == "tool_result"
                && let Some(id) = data["id"].as_str()
            {
                let step = json!({ "summary": data["summary"], "is_error": data["is_error"] });
                log.steps.insert(id.to_owned(), step);
            } else if proposal {
                data["thread"] = json!(log.id);
                let status = if name == "applied" {
                    agent_threads::Status::Made
                } else {
                    agent_threads::Status::Pending
                };
                log.proposals.push(Stored {
                    event: name.to_owned(),
                    data: data.clone(),
                    status,
                    result: None,
                    error: None,
                    reported: false,
                });
            }
        }
        self.out.send(name, data).await
    }

    /// Write the thread's transcript as it stands: the messages it started from and `added`,
    /// with this turn's steps and proposals. Nothing new since the last write writes nothing.
    async fn persist(&self, added: &[Message]) {
        let (id, label, messages, steps, proposals, context, subagents, question) = {
            let Some(mut log) = self.log() else { return };
            let now = (
                added.len(),
                log.steps.len() + log.proposals.len(),
                log.sub_changes,
            );
            if now == log.saved {
                return;
            }
            log.saved = now;
            let mut messages = log.base.clone();
            messages.extend(added.iter().cloned());
            (
                log.id.clone(),
                log.label.clone(),
                messages,
                log.steps.clone(),
                log.proposals.clone(),
                log.context,
                log.subagents.clone(),
                log.question.clone(),
            )
        };
        let w = self.writer.clone();
        let saved = tokio::task::spawn_blocking(move || {
            agent_threads::save(&w, &id, &label, None, |t| {
                t.transcript.messages = messages.clone();
                if context.is_some() {
                    t.transcript.context = context;
                }
                if let Some(q) = &question {
                    t.question = q.clone();
                }
                for (k, v) in &steps {
                    t.transcript.steps.insert(k.clone(), v.clone());
                }
                for p in &proposals {
                    if !t.transcript.proposals.iter().any(|q| q.id() == p.id()) {
                        t.transcript.proposals.push(p.clone());
                    }
                }
                for sub in &subagents {
                    match t
                        .transcript
                        .subagents
                        .iter_mut()
                        .find(|x| x.call == sub.call)
                    {
                        Some(x) => *x = sub.clone(),
                        None => t.transcript.subagents.push(sub.clone()),
                    }
                }
            })
            .map(|_| ())
            .map_err(|e| format!("thread {id} was not written: {}", e.body))
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("{e}"),
            Err(e) => eprintln!("a thread was not written: {e}"),
        }
    }

    async fn rounds(
        &self,
        agent: Agent,
        account: Option<String>,
        model: Option<String>,
        history: Vec<Message>,
        note_id: Option<String>,
        added: &mut Vec<Message>,
    ) {
        let w = self.writer.clone();
        let scope = self.scope;
        let held = self.held;
        let asks = self.asks;
        let skills = self.skills.section().map(str::to_owned);
        let uses_skills = crate::ai_skills::uses_skills(&agent);
        let mut setup = match tokio::task::spawn_blocking(move || {
            setup(
                &w,
                scope,
                &agent,
                account,
                model,
                note_id,
                skills.as_deref(),
                held,
                asks,
            )
        })
        .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                self.failed(&e, &[]).await;
                return;
            }
            Err(e) => {
                let e = AiError::new(ErrorKind::Unavailable, e.to_string());
                self.failed(&e, &[]).await;
                return;
            }
        };
        if crate::auth::is_agent_token(&self.owner) {
            setup.direct.agents = false;
            setup.direct.skills = false;
        }
        // Only the tools the agent may use (agents G4); a call naming another is not run.
        let mut tools = turn_tools(
            setup.web,
            setup.web_search.is_some(),
            setup.tools.as_deref(),
            (self.held == Held::Drop).then_some(setup.direct),
            setup.browser.is_some(),
            uses_skills,
            self.asks,
        );
        // A throwaway sub agent reads its parent's memories but writes none (subagents U1).
        if self.sub.as_ref().is_some_and(|s| s.throwaway) {
            tools.retain(|t| !crate::ai_subagents::MEMORY_WRITES.contains(&t.name.as_str()));
        }
        // Sub agents (subagents U1, U2): with the permission, on a thread, which keeps their
        // sessions, and never to a sub agent, whatever its file says.
        if self.sub.is_none() && self.log.is_some() && setup.agent.permissions.subagents.allow {
            tools.push(crate::ai_subagents::spec(&setup.agent));
        }
        let base_offered: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let mut history = history;
        if let Some(pending) = &self.compact {
            match self.compact(&setup, pending).await {
                Ok(view) => history = view,
                // `/compact` is the whole turn: it ends here, or with why it failed.
                Err(e) if pending.reason == "command" => {
                    self.failed(&e, &[]).await;
                    return;
                }
                // A compaction due on its own that fails leaves the thread as it was.
                Err(e) => {
                    let data = json!({ "message": e.message });
                    if !self.out.send("compact_failed", data).await {
                        return;
                    }
                }
            }
            if pending.reason == "command" {
                self.done(&Stop::EndTurn, &[]).await;
                return;
            }
        }
        // The images read_asset has sent this turn (ai D19).
        let mut images_sent = 0usize;
        for round in 1..=MAX_ROUNDS {
            if self.out.gone() {
                return;
            }
            let mut messages = history.clone();
            messages.extend(added.iter().cloned());
            // The script tools this round: run_script, the vault's tools and those of the skills
            // the thread has opened so far, read afresh each round (script-tools T2, T4).
            let scripts = self.script_tools(&setup, &messages).await;
            let mut round_tools = tools.clone();
            round_tools.extend(scripts.specs.iter().cloned());
            let mut offered = base_offered.clone();
            offered.extend(scripts.specs.iter().map(|t| t.name.clone()));
            let messages = self.hydrate(messages).await;
            let req = Request {
                model: setup.model.clone(),
                system: Some(setup.system.clone()),
                messages,
                tools: round_tools,
                schema: None,
                effort: setup.effort,
                max_tokens: None,
            };
            let mut began = Instant::now();
            let trail = |began: Instant, usage: Usage, outcome: Outcome| {
                let line = ai::trail_line(
                    &setup.cred.name,
                    setup.cred.kind,
                    &setup.model,
                    usage,
                    outcome,
                    began.elapsed(),
                    setup.cred.custom_host.as_deref(),
                );
                eprintln!("{line}");
                self.report(|r| r.log.push(line));
                self.activity.called(
                    &setup.cred.name,
                    setup.cred.kind.as_str(),
                    &setup.model,
                    usage.input,
                    usage.output,
                );
            };
            // An unattended turn makes a call that failed with a transient error again, after
            // each of its waits (`crate::ai_retry`). The same request goes again: the tool calls
            // of earlier rounds are answered in it, and this round's run only once its call has
            // ended, so a failed call has run none. Its partial text is dropped.
            let mut retries = 0;
            let called = loop {
                let called = tokio::select! {
                    _ = self.out.closed() => Called::Gone,
                    c = self.call(&setup.cred, &req) => c,
                };
                let Called::Failed { error, .. } = &called else {
                    break called;
                };
                let Some((retry, wait)) = self
                    .retry
                    .as_ref()
                    .filter(|_| error.is_transient())
                    .and_then(|r| Some((r, r.wait(retries)?)))
                else {
                    break called;
                };
                trail(began, error.usage, Outcome::of_error(error));
                retry.note(retries, wait, error).await;
                tokio::select! {
                    _ = self.out.closed() => return,
                    _ = retry.sleep(wait) => {}
                }
                retries += 1;
                began = Instant::now();
            };
            let (text, calls, stop, state) = match called {
                Called::Gone => return,
                Called::Failed { text, error } => {
                    trail(began, error.usage, Outcome::of_error(&error));
                    let mut content = Vec::new();
                    if !text.is_empty() {
                        content.push(Content::text(text));
                    }
                    if error.kind == ErrorKind::Refusal {
                        let stop = Stop::Refusal {
                            category: error.category.as_deref().map(str::to_owned),
                            explanation: error.explanation.clone(),
                        };
                        self.refused(&setup, content, &stop, added).await;
                    } else {
                        if !content.is_empty() {
                            content.push(Content::Notice {
                                kind: "error".into(),
                                text: "The answer stopped here.".into(),
                            });
                            added.push(assistant(content, None));
                        }
                        self.failed(&error, added).await;
                    }
                    return;
                }
                Called::Done {
                    text,
                    calls,
                    stop,
                    state,
                    usage,
                } => {
                    trail(began, usage, Outcome::of_stop(&stop));
                    self.context(usage.input + usage.output, setup.window).await;
                    (text, calls, stop, state)
                }
            };
            let mut content = Vec::new();
            if !text.is_empty() {
                content.push(Content::text(text));
            }
            match &stop {
                Stop::Refusal { .. } => {
                    self.refused(&setup, content, &stop, added).await;
                    return;
                }
                Stop::EndTurn | Stop::MaxTokens => {
                    if stop == Stop::MaxTokens {
                        content.push(Content::Notice {
                            kind: "cut_off".into(),
                            text: "The answer reached the output limit and is cut off.".into(),
                        });
                    }
                    added.push(assistant(content, state));
                    self.done(&stop, added).await;
                    return;
                }
                Stop::ToolUse => {}
            }
            for c in &calls {
                content.push(Content::ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    input: c.input.clone(),
                });
            }
            added.push(assistant(content, state));
            if round == MAX_ROUNDS {
                // The model asked for another round of tools, which would need a 13th call: answer
                // its calls as not run, so the transcript stays whole, and stop.
                let mut results: Vec<Content> = calls
                    .iter()
                    .map(|c| {
                        Content::tool_result(
                            c.id.clone(),
                            format!(
                                "Not run: this question reached the limit of {MAX_ROUNDS} rounds of tool calls."
                            ),
                            true,
                        )
                    })
                    .collect();
                results.push(Content::Notice {
                    kind: "round_limit".into(),
                    text: format!(
                        "Stopped after {MAX_ROUNDS} rounds of looking things up. Ask again to go on."
                    ),
                });
                added.push(Message::tool_results(results));
                self.done(&stop, added).await;
                return;
            }
            let mut results = Vec::with_capacity(calls.len());
            // The sub agents this round starts, run together once the other calls are answered
            // (subagents U1): where each one's result goes, and its call.
            let mut subs: Vec<(usize, ToolCall)> = Vec::new();
            // The question this round's ask_user calls put to the user (TV-77 D3).
            let mut asked: Option<String> = None;
            for c in calls {
                // Mark before execution, including scripts and subagents, so a later error
                // can never cause a trigger retry to repeat an already attempted change.
                self.report(|r| r.tool_called = true);
                self.activity.tool_called(&c.id, &c.name, &c.input);
                if !self
                    .out
                    .send(
                        "tool",
                        json!({ "id": c.id, "name": c.name, "input": c.input }),
                    )
                    .await
                {
                    return;
                }
                if c.error.is_none() && !offered.contains(&c.name) {
                    let out = ToolOut::error(format!("there is no tool {:?}", c.name));
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": out.summary, "is_error": true }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::tool_result(c.id, out.content, true));
                    continue;
                }
                if c.error.is_none() && c.name == crate::ai_subagents::TOOL {
                    subs.push((results.len(), c.clone()));
                    results.push(Content::tool_result(c.id, String::new(), false));
                    continue;
                }
                // A script tool (script-tools T1, T2, T4): it runs as the agent, its changes made
                // at once.
                if c.error.is_none()
                    && let Some(target) = scripts.target(&c.name)
                {
                    let w = self.writer.clone();
                    let agent = setup.agent.clone();
                    let target = target.clone();
                    let call = c.clone();
                    let got = tokio::task::spawn_blocking(move || {
                        crate::ai_scripts::call(&w, &agent, &target, &call)
                    })
                    .await
                    .unwrap_or_else(|e| ToolOut::error(e.to_string()));
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: Vec::new(),
                    });
                    continue;
                }
                if c.name == "read_asset" && c.error.is_none() {
                    let read = self.read_asset(&c, images_sent).await;
                    if !read.parts.is_empty() {
                        images_sent += 1;
                    }
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": read.summary, "is_error": read.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&read.content, RESULT_LIMIT),
                        is_error: read.is_error,
                        parts: read.parts,
                    });
                    continue;
                }
                // The skill tools (skills S6): only offered, and so only run, when the agent
                // uses skills.
                if crate::ai_skills::is_skill_tool(&c.name) && c.error.is_none() {
                    let read = self.skill_tool(&c, &setup, images_sent).await;
                    if !read.parts.is_empty() {
                        images_sent += 1;
                    }
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": read.summary, "is_error": read.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&read.content, RESULT_LIMIT),
                        is_error: read.is_error,
                        parts: read.parts,
                    });
                    continue;
                }
                // web_search (ai D26): only offered, and so only run, with a Brave key.
                if c.name == "web_search"
                    && c.error.is_none()
                    && setup.web.search
                    && let Some(search) = &setup.web_search
                {
                    let got = crate::brave::web_search(search, &c.input).await;
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: Vec::new(),
                    });
                    continue;
                }
                // The browser tools (browser B5): only offered, and so only run, with a box and
                // the agent's permission.
                if crate::ai_browser::is_browser_tool(&c.name)
                    && c.error.is_none()
                    && let Some(b) = &setup.browser
                {
                    let got = if c.name == crate::ai_skills::INSTALL_TOOL {
                        crate::ai_skills::install(self.writer.clone(), b, &setup.agent, &c.input)
                            .await
                    } else {
                        self.browser_tool(&c, b, &setup, images_sent).await
                    };
                    if !got.parts.is_empty() {
                        images_sent += 1;
                    }
                    if let Some(line) = &got.trail {
                        eprintln!("{line}");
                        self.report(|r| r.log.push(line.clone()));
                    }
                    if let Some((name, p)) = got.event
                        && !self.emit(name, p).await
                    {
                        return;
                    }
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: got.parts,
                    });
                    continue;
                }
                let web_allowed = match c.name.as_str() {
                    "fetch_url" => setup.web.read,
                    "download_asset" => setup.web.download,
                    _ => false,
                };
                if web_allowed && c.error.is_none() {
                    let got = self.web_tool(&c, &setup, images_sent).await;
                    if !got.parts.is_empty() {
                        images_sent += 1;
                    }
                    if let Some((name, p)) = got.event
                        && !self.emit(name, p).await
                    {
                        return;
                    }
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: got.parts,
                    });
                    continue;
                }
                // ask_user (TV-77 D3): the question is kept for the thread, and the turn ends once
                // this round's other calls are answered.
                if c.name == crate::ai_ask::TOOL && c.error.is_none() {
                    let got = match crate::ai_ask::question(&c.input) {
                        Ok(q) => match crate::ai_ask::join(asked.as_deref(), &q) {
                            // Two questions in one round are asked together, as one text that
                            // fits the thread's field and the bell (TV-88 M2).
                            Some(joined) => {
                                if !self
                                    .out
                                    .send("question", json!({ "id": c.id, "text": q }))
                                    .await
                                {
                                    return;
                                }
                                asked = Some(joined);
                                ToolOut {
                                    content: crate::ai_ask::RESULT.to_owned(),
                                    is_error: false,
                                    summary: json!({ "notes": [] }),
                                    proposal: None,
                                }
                            }
                            None => ToolOut::error(format!(
                                "Not asked: with the question this round already asks, it is longer than {} characters. Ask one shorter question.",
                                crate::ai_ask::MAX_QUESTION
                            )),
                        },
                        Err(out) => out,
                    };
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::tool_result(c.id, got.content, got.is_error));
                    continue;
                }
                // notify_user (TV-32 S1): sent at once, within the agent's limits, also unattended.
                if c.name == crate::ai_notify::TOOL && c.error.is_none() {
                    let w = self.writer.clone();
                    let limits = setup.limits.clone();
                    let agent = setup.agent.clone();
                    let input = c.input.clone();
                    let sent = self.notified.clone();
                    let got = tokio::task::spawn_blocking(move || {
                        crate::ai_notify::notify(&w, scope, &agent, &limits, &sent, &input)
                    })
                    .await
                    .unwrap_or_else(|e| ToolOut::error(e.to_string()));
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: Vec::new(),
                    });
                    continue;
                }
                let w = self.writer.clone();
                let author = self.author.clone();
                // The capability is scoped to this call, never inherited by a sub agent.
                let gmail_actions =
                    !self.unattended && self.sub.is_none() && setup.agent.permissions.gmail_actions;
                let call = c.clone();
                let direct = setup.direct;
                let held = self.held;
                let mut limits = setup.limits.clone();
                limits.parent = self.sub.as_ref().map(|s| s.parent.clone());
                let agent = setup.agent.clone();
                let got = tokio::task::spawn_blocking(move || {
                    run_tool(
                        &w,
                        &author,
                        scope,
                        direct,
                        held,
                        &limits,
                        &agent,
                        &call,
                        gmail_actions,
                    )
                })
                .await
                .unwrap_or_else(|e| ToolOut::error(e.to_string()));
                if let Some(line) = got.summary["gmail_skipped"].as_str() {
                    self.report(|r| r.log.push(line.to_owned()));
                }
                if let Some((name, p)) = got.proposal
                    && !self.emit(name, p).await
                {
                    return;
                }
                if !self
                    .emit(
                        "tool_result",
                        json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                    )
                    .await
                {
                    return;
                }
                results.push(Content::ToolResult {
                    id: c.id,
                    content: cut(&got.content, RESULT_LIMIT),
                    is_error: got.is_error,
                    parts: Vec::new(),
                });
            }
            if !subs.is_empty() {
                for (at, c, got) in self.subagents(&setup, subs).await {
                    if !self
                        .emit(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results[at] = Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: Vec::new(),
                    };
                }
                if self.out.gone() {
                    return;
                }
            }
            added.push(Message::tool_results(results));
            // A question for the user ends the turn after its round (TV-77 D3): the thread keeps
            // it, written with the round, and reads needs_you.
            if let Some(q) = asked {
                if let Some(mut log) = self.log() {
                    log.question = Some(q);
                }
                self.done(&Stop::EndTurn, added).await;
                return;
            }
            self.persist(added).await;
        }
    }

    /// The script tools a round offers: those of the vault, `run_script` when the agent may run
    /// code, and the tools of every skill opened in the thread, its whole transcript when it has
    /// one (a compaction keeps no tool calls) and `messages` besides.
    async fn script_tools(
        &self,
        setup: &Setup,
        messages: &[Message],
    ) -> crate::ai_scripts::ScriptTools {
        // A script's changes are made at once, so a turn whose scope cannot write notes, a
        // `read` caller's, is offered none ([`change_scope`]).
        if !change_scope(self.scope, &setup.agent).allows(Scope::Write) {
            return crate::ai_scripts::ScriptTools::default();
        }
        let mut seen: Vec<Message> = self.log().map(|l| l.base.clone()).unwrap_or_default();
        seen.extend(messages.iter().cloned());
        let w = self.writer.clone();
        let agent = setup.agent.clone();
        tokio::task::spawn_blocking(move || {
            let opened = crate::ai_scripts::opened_skills(&seen);
            crate::ai_scripts::offered(&w, &agent, &opened)
        })
        .await
        .unwrap_or_default()
    }

    /// `messages` with every image part loaded from the vault, for a provider request (ai D19).
    async fn hydrate(&self, mut messages: Vec<Message>) -> Vec<Message> {
        let has_images = messages.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, Content::ToolResult { parts, .. } if !parts.is_empty()))
        });
        if !has_images {
            return messages;
        }
        let w = self.writer.clone();
        let images = self.images.clone();
        let unloaded = messages.clone();
        tokio::task::spawn_blocking(move || {
            ai_assets::hydrate(&w, &mut messages, &images);
            messages
        })
        .await
        .unwrap_or(unloaded)
    }

    /// `read_asset`, with the turn's image count (ai D19).
    async fn read_asset(&self, call: &ToolCall, images_sent: usize) -> ai_assets::Read {
        let w = self.writer.clone();
        let images = self.images.clone();
        let input = call.input.clone();
        let may_send = images_sent < ai_assets::IMAGE_LIMIT;
        tokio::task::spawn_blocking(move || ai_assets::read_asset(&w, &input, may_send, &images))
            .await
            .unwrap_or_else(|e| ai_assets::Read {
                content: json!({ "error": e.to_string() }).to_string(),
                is_error: true,
                parts: Vec::new(),
                summary: json!({ "notes": [], "message": e.to_string() }),
            })
    }

    /// `use_skill` or `read_skill_file`, with the turn's image count (skills S6).
    async fn skill_tool(
        &self,
        call: &ToolCall,
        setup: &Setup,
        images_sent: usize,
    ) -> ai_assets::Read {
        let w = self.writer.clone();
        let images = self.images.clone();
        let input = call.input.clone();
        let name = call.name.clone();
        let agent = setup.agent.clone();
        let browser = setup.browser.is_some();
        let may_send = images_sent < ai_assets::IMAGE_LIMIT;
        tokio::task::spawn_blocking(move || {
            if name == "use_skill" {
                crate::ai_skills::use_skill(&w, &agent, &input, browser)
            } else {
                crate::ai_skills::read_skill_file(&w, &agent, &input, may_send, &images)
            }
        })
        .await
        .unwrap_or_else(|e| ai_assets::Read {
            content: json!({ "error": e.to_string() }).to_string(),
            is_error: true,
            parts: Vec::new(),
            summary: json!({ "notes": [], "message": e.to_string() }),
        })
    }

    /// `fetch_url` or `download_asset`, with the turn's image count (ai D27).
    async fn web_tool(
        &self,
        call: &ToolCall,
        setup: &Setup,
        images_sent: usize,
    ) -> crate::ai_web::WebOut {
        let f = crate::ai_web::fetcher(setup.fetch_private);
        if call.name == "fetch_url" {
            let may_send = images_sent < ai_assets::IMAGE_LIMIT;
            crate::ai_web::fetch_url(f, &call.id, &call.input, may_send, &self.images).await
        } else {
            let direct = setup.direct.downloads;
            crate::ai_web::download_asset(self.writer.clone(), f, direct, &call.id, &call.input)
                .await
        }
    }

    /// `browser_run`, `browser_file`, `browser_save` or `browser_capture`, with the turn's image count (browser B5).
    async fn browser_tool(
        &self,
        call: &ToolCall,
        b: &crate::ai_browser::BrowserBox,
        setup: &Setup,
        images_sent: usize,
    ) -> crate::ai_browser::BrowserOut {
        match call.name.as_str() {
            "browser_run" => crate::ai_browser::run(b, &call.input).await,
            "browser_capture" => {
                let direct = setup.direct.downloads;
                crate::ai_browser::capture(self.writer.clone(), b, direct, &call.id, &call.input)
                    .await
            }
            "browser_file" => {
                let may_send = images_sent < ai_assets::IMAGE_LIMIT;
                crate::ai_browser::file(b, &call.id, &call.input, may_send, &self.images).await
            }
            _ => {
                let direct = setup.direct.downloads;
                crate::ai_browser::save(self.writer.clone(), b, direct, &call.id, &call.input).await
            }
        }
    }

    /// Say how full the context is after a call, and keep it for the thread.
    async fn context(&self, used: u64, window: u64) {
        if let Some(mut log) = self.log() {
            log.context = Some(agent_threads::ContextUse { used, window });
        }
        self.out
            .send("context", json!({ "used": used, "window": window }))
            .await;
    }

    /// Summarize the thread's messages before `pending.upto` with one model call, keep the
    /// summary in the thread, and answer the messages the model is sent from now on.
    async fn compact(
        &self,
        setup: &Setup,
        pending: &PendingCompact,
    ) -> Result<Vec<Message>, AiError> {
        let (id, label, full) = {
            let Some(log) = self.log() else {
                return Err(AiError::invalid("only a thread is compacted"));
            };
            (log.id.clone(), log.label.clone(), log.base.clone())
        };
        let older = &full[..pending.upto.min(full.len())];
        let mut messages = agent_threads::model_view(pending.previous.as_ref(), older);
        if messages.is_empty() {
            return Err(AiError::invalid("there is nothing to compact yet"));
        }
        if !self
            .out
            .send("compacting", json!({ "reason": pending.reason }))
            .await
        {
            return Err(AiError::new(ErrorKind::Unavailable, "the turn was stopped"));
        }
        messages.push(Message::user(COMPACT_PROMPT));
        let req = Request {
            model: setup.model.clone(),
            system: Some(setup.system.clone()),
            messages: self.hydrate(messages).await,
            tools: Vec::new(),
            schema: None,
            effort: setup.effort,
            max_tokens: None,
        };
        let began = Instant::now();
        let mut stream = ai::stream(http(), &setup.cred, &req).await?;
        let mut summary = String::new();
        let mut usage = Usage::default();
        let mut ended = false;
        while let Some(e) = stream.next().await {
            match e? {
                AiEvent::TextDelta(d) => summary.push_str(&d),
                AiEvent::Usage(u) => usage = u,
                AiEvent::Done { .. } => {
                    ended = true;
                    break;
                }
                AiEvent::ToolCall(_) => {}
            }
        }
        self.keep_usage(&setup.cred, stream.plan_usage()).await;
        let line = ai::trail_line(
            &setup.cred.name,
            setup.cred.kind,
            &setup.model,
            usage,
            if ended {
                Outcome::Ok
            } else {
                Outcome::Error(ErrorKind::Provider)
            },
            began.elapsed(),
            setup.cred.custom_host.as_deref(),
        );
        eprintln!("{line}");
        self.report(|r| r.log.push(line));
        let summary = summary.trim().to_owned();
        if !ended || summary.is_empty() {
            return Err(AiError::new(
                ErrorKind::Provider,
                format!("{} gave no summary", setup.cred.name),
            ));
        }
        let compaction = agent_threads::Compaction {
            summary: summary.clone(),
            upto: older.len(),
            at_ms: crate::ai_settings::now_ms(),
            before: usage.input,
            reason: pending.reason.to_owned(),
        };
        let context = agent_threads::ContextUse {
            used: usage.output,
            window: setup.window,
        };
        let w = self.writer.clone();
        let kept = compaction.clone();
        tokio::task::spawn_blocking(move || {
            agent_threads::save(&w, &id, &label, None, |t| {
                t.transcript.compaction = Some(kept.clone());
                t.transcript.context = Some(context);
            })
        })
        .await
        .map_err(|e| AiError::new(ErrorKind::Unavailable, e.to_string()))?
        .map_err(|e| {
            AiError::new(
                ErrorKind::Unavailable,
                e.body["message"]
                    .as_str()
                    .unwrap_or("the thread was not written")
                    .to_owned(),
            )
        })?;
        if let Some(mut log) = self.log() {
            log.context = Some(context);
        }
        let data = json!({
            "reason": pending.reason,
            "upto": compaction.upto,
            "summary": summary,
            "before": compaction.before,
            "after": usage.output,
        });
        self.out.send("compacted", data).await;
        self.out
            .send(
                "context",
                json!({ "used": context.used, "window": context.window }),
            )
            .await;
        Ok(agent_threads::model_view(Some(&compaction), &full))
    }

    /// One model call, its text and usage streamed to the client as they arrive.
    async fn call(&self, cred: &Credential, req: &Request) -> Called {
        let mut stream = match ai::stream(http(), cred, req).await {
            Ok(s) => s,
            Err(error) => {
                return Called::Failed {
                    text: String::new(),
                    error,
                };
            }
        };
        self.keep_usage(cred, stream.plan_usage()).await;
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut usage = Usage::default();
        while let Some(e) = stream.next().await {
            match e {
                Err(error) => return Called::Failed { text, error },
                Ok(AiEvent::TextDelta(d)) => {
                    if !self.out.send("text", json!({ "delta": d })).await {
                        return Called::Gone;
                    }
                    text.push_str(&d);
                }
                Ok(AiEvent::ToolCall(c)) => calls.push(c),
                Ok(AiEvent::Usage(u)) => {
                    usage = u;
                    let data = json!({
                        "account": cred.account, "name": cred.name, "kind": cred.kind,
                        "model": req.model, "input": u.input, "output": u.output,
                    });
                    if !self.out.send("usage", data).await {
                        return Called::Gone;
                    }
                }
                Ok(AiEvent::Done {
                    stop,
                    provider_state,
                }) => {
                    return Called::Done {
                        text,
                        calls,
                        stop,
                        state: provider_state,
                        usage,
                    };
                }
            }
        }
        Called::Failed {
            text,
            error: AiError::new(
                ErrorKind::Provider,
                format!("{} stopped answering", cred.name),
            ),
        }
    }

    /// Keep the plan usage a ChatGPT answer reported as its account's latest (accounts A13).
    async fn keep_usage(&self, cred: &Credential, usage: Option<&ai::PlanUsage>) {
        let Some(usage) = usage.cloned() else {
            return;
        };
        let w = self.writer.clone();
        let id = cred.account.clone();
        let _ = tokio::task::spawn_blocking(move || {
            crate::ai_settings::keep_usage(w.state(), &id, Some(&usage));
        })
        .await;
    }

    async fn done(&self, stop: &Stop, added: &[Message]) {
        self.activity.end("done", Some(stop.as_str()));
        self.persist(added).await;
        self.report(|r| r.ended = Some(Ok(())));
        self.out
            .send("done", json!({ "stop": stop.as_str(), "messages": added }))
            .await;
    }

    async fn failed(&self, e: &AiError, added: &[Message]) {
        self.activity.end("error", Some(e.kind.as_str()));
        self.persist(added).await;
        self.report(|r| r.ended = Some(Err(e.message.clone())));
        self.out
            .send(
                "error",
                json!({
                    "kind": e.kind.as_str(),
                    "message": e.message,
                    "account": e.account,
                    "model": e.model,
                    "messages": added,
                }),
            )
            .await;
    }

    /// The provider refused (ai D11): the partial answer stays, marked cut off.
    async fn refused(
        &self,
        setup: &Setup,
        mut content: Vec<Content>,
        stop: &Stop,
        added: &mut Vec<Message>,
    ) {
        let (category, explanation) = match stop {
            Stop::Refusal {
                category,
                explanation,
            } => (category.clone(), explanation.clone()),
            _ => (None, None),
        };
        let mut notice = format!("{} {} refused", setup.cred.name, setup.model);
        if let Some(c) = &category {
            notice.push_str(&format!(" ({c})"));
        }
        notice.push('.');
        if let Some(x) = &explanation {
            notice.push(' ');
            notice.push_str(x);
        }
        content.push(Content::Notice {
            kind: "refusal".into(),
            text: notice.clone(),
        });
        added.push(assistant(content, None));
        self.activity
            .end("refused", Some(category.as_deref().unwrap_or("refusal")));
        self.persist(added).await;
        self.report(|r| r.ended = Some(Err(notice.clone())));
        self.out
            .send(
                "refusal",
                json!({
                    "account": setup.cred.account,
                    "name": setup.cred.name,
                    "kind": setup.cred.kind,
                    "model": setup.model,
                    "category": category,
                    "explanation": explanation,
                    "messages": added,
                }),
            )
            .await;
    }
}

/// A sub agent started and running (subagents U1).
struct SubRun {
    rx: mpsc::Receiver<Sent>,
    turn: crate::ai_turns::SubTurn,
    session: agent_threads::SubSession,
    /// What the call's `permissions` asked for that would have widened them.
    ignored: Vec<String>,
    began: Instant,
}

/// A refusal from the routes' code as a tool error.
fn api_tool_error(e: &ApiError) -> ToolOut {
    ToolOut::error(
        e.body["message"]
            .as_str()
            .unwrap_or("the sub agent could not start")
            .to_owned(),
    )
}

impl Turn {
    /// Run the sub agents of `calls`, each by where its result goes: at most the agent's `max`
    /// of them, all at once, and a tool error for each call past it (subagents U1). Each result
    /// comes back in order.
    async fn subagents(
        &self,
        setup: &Setup,
        calls: Vec<(usize, ToolCall)>,
    ) -> Vec<(usize, ToolCall, ToolOut)> {
        let max = usize::from(setup.agent.permissions.subagents.max);
        let mut out = Vec::new();
        let mut runs = Vec::new();
        for (k, (at, c)) in calls.into_iter().enumerate() {
            if k >= max {
                let e = ToolOut::error(format!(
                    "Not started: this agent runs at most {max} sub agents at once, and this round asked for more. Start it in a later round."
                ));
                out.push((at, c, e));
                continue;
            }
            match self.start_sub(setup, &c).await {
                Ok(run) => runs.push((at, c, run)),
                Err(e) => out.push((at, c, e)),
            }
        }
        let done =
            futures_util::future::join_all(runs.into_iter().map(|(at, c, run)| async move {
                let got = self.follow_sub(&c, run).await;
                (at, c, got)
            }))
            .await;
        out.extend(done);
        out.sort_by_key(|(at, _, _)| *at);
        out
    }

    /// Start the sub agent call `c` asks for (subagents U1, U3, U5): its rights, its session, a
    /// thread whose parent is this one, and its turn, as the parent's caller.
    async fn start_sub(&self, setup: &Setup, c: &ToolCall) -> Result<SubRun, ToolOut> {
        let input = &c.input;
        let task = str_arg(input, "task")?.to_owned();
        let opt = |k: &str| {
            input[k]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let instructions = opt("instructions");
        let named = opt("agent");
        let model = opt("model");
        let parent = setup.agent.clone();
        let (parent_thread, n) = match self.log() {
            Some(l) => (l.id.clone(), l.subs_before + l.subagents.len() + 1),
            None => return Err(ToolOut::error("Sub agents run only in a thread.")),
        };
        if let Some(name) = &named
            && !parent.permissions.subagents.allows_agent(name)
        {
            let may = &parent.permissions.subagents.agents;
            return Err(ToolOut::error(if may.is_empty() {
                format!(
                    "This agent may not delegate to {name}: it may start only throwaway sub agents, with agent null."
                )
            } else {
                format!(
                    "This agent may not delegate to {name}: it may delegate only to {}.",
                    may.join(", ")
                )
            }));
        }
        let w = self.writer.clone();
        let parent_turn = self.activity.turn_id().to_owned();
        let call = c.id.clone();
        let narrowing = input["permissions"].clone();
        let account = setup.cred.account.clone();
        let parent_model = setup.model.clone();
        let author = match &named {
            Some(name) => Author::Agent(name.clone()),
            None => Author::Agent(format!("{}/sub:{n}", parent.name)),
        };
        let by = author.clone();
        let sub_task = task.clone();
        let from = parent_thread.clone();
        let ready = tokio::task::spawn_blocking(move || {
            let task = sub_task;
            let (mut agent, ignored, account, model) = match &named {
                Some(name) => {
                    let own = turn_agent(&w, Some(name)).map_err(|e| api_tool_error(&e))?;
                    let both =
                        crate::ai_subagents::intersect(&own.permissions, &parent.permissions);
                    let (p, ignored) = crate::ai_subagents::narrow(&both, &narrowing);
                    let mut a = own;
                    a.permissions = p;
                    // A named agent that names its account runs on it, with its model; any
                    // other on the parent's account and model (subagents U3).
                    let (account, model) = if a.account_ref().is_some() {
                        (None, model)
                    } else {
                        (Some(account), model.or(Some(parent_model)))
                    };
                    (a, ignored, account, model)
                }
                None => {
                    let (p, ignored) = crate::ai_subagents::narrow(&parent.permissions, &narrowing);
                    let mut a = (*parent).clone();
                    a.permissions = p;
                    let base = parent
                        .system_prompt
                        .as_deref()
                        .filter(|p| !p.trim().is_empty())
                        .unwrap_or(ai::CHAT_PROMPT);
                    let section = crate::ai_subagents::prompt_section(
                        &parent.label,
                        &task,
                        instructions.as_deref(),
                    );
                    a.system_prompt = Some(format!("{}\n{section}", base.trim_end()));
                    (a, ignored, Some(account), model.or(Some(parent_model)))
                }
            };
            agent.permissions.subagents = schema::AgentSubagents::default();
            let text = match &named {
                Some(_) => crate::ai_subagents::delegated_message(
                    &parent.label,
                    &task,
                    instructions.as_deref(),
                ),
                None => task.clone(),
            };
            let origin = agent_threads::sub_origin(&from);
            let t = agent_threads::create(
                &w,
                &agent.name,
                agent_threads::title_of(&task).as_deref(),
                &origin,
            )
            .map_err(|e| api_tool_error(&e))?;
            let parent_ref = agent_threads::Parent {
                thread: from,
                turn: parent_turn,
                call,
            };
            agent_threads::save(&w, &t.id, &agent.label, None, |t| {
                t.transcript.parent = Some(parent_ref.clone());
            })
            .map_err(|e| api_tool_error(&e))?;
            let prepared = prepare_for(
                &w,
                &by,
                agent,
                true,
                Some(t.id.clone()),
                vec![Message::user(&text)],
                false,
                false,
            )
            .map_err(|e| api_tool_error(&e))?;
            Ok::<_, ToolOut>((prepared, ignored, account, model, t.id, origin))
        })
        .await
        .map_err(|e| ToolOut::error(e.to_string()))??;
        let (prepared, ignored, account, model, thread, origin) = ready;
        let agent = prepared.agent.name.clone();
        let delegated = c.input["agent"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let (stop, stopped) = watch::channel(false);
        let cfg = TurnCfg {
            owner: self.owner.clone(),
            author,
            scope: self.scope,
            agent_name: Some(agent.clone()),
            unattended: self.unattended,
            held: self.held,
            // A sub agent answers its parent, never the user.
            asks: false,
            retry: self.retry.as_ref().map(|r| r.for_sub(n)),
            sub: Some(SubMark {
                parent: setup.agent.name.clone(),
                throwaway: delegated.is_none(),
                n,
            }),
            origin,
            notified: Some(self.notified.clone()),
        };
        let (rx, running) = spawn_turn(
            self.writer.clone(),
            prepared,
            cfg,
            account,
            model,
            None,
            stopped,
            None,
        );
        // A sub agent of an unattended run, which nobody follows, is not listed as a turn: its
        // session is read again until it ends, as a trigger's thread is.
        let turn = crate::ai_turns::SubTurn::start(
            self.owner.clone(),
            &agent,
            &thread,
            stop,
            running,
            !self.unattended,
        );
        let session = agent_threads::SubSession {
            call: c.id.clone(),
            thread,
            agent,
            delegated,
            task,
            n,
            status: "running".to_owned(),
            rounds: 0,
            tools: 0,
            started_ms: crate::ai_settings::now_ms(),
            ended_ms: None,
        };
        self.activity.subagent_start(&session, &parent_thread);
        self.sub_update(&session, None, turn.id()).await;
        Ok(SubRun {
            rx,
            turn,
            session,
            ignored,
            began: Instant::now(),
        })
    }

    /// Follow sub agent `run` to its end (subagents U1, U4, U6): its rounds and tools on its
    /// block, its proposals and changes in this turn's stream, its events kept for its session
    /// pane, and its answer as the call's result. Stopping this turn stops it.
    async fn follow_sub(&self, c: &ToolCall, run: SubRun) -> ToolOut {
        let SubRun {
            mut rx,
            turn,
            mut session,
            ignored,
            began,
        } = run;
        let mut tool: Option<String> = None;
        let mut ended: Option<(&'static str, Option<String>)> = None;
        let mut answer = None;
        let mut proposed = Vec::new();
        let mut gone = false;
        loop {
            let e = tokio::select! {
                e = rx.recv() => e,
                _ = self.out.closed(), if !gone => {
                    gone = true;
                    turn.stop();
                    continue;
                }
            };
            let Some(e) = e else { break };
            turn.push(e.clone());
            let data: Value = serde_json::from_str(&e.data).unwrap_or(Value::Null);
            match e.name.as_str() {
                "tool" => {
                    session.tools += 1;
                    tool = data["name"].as_str().map(str::to_owned);
                    self.sub_update(&session, tool.as_deref(), turn.id()).await;
                }
                "usage" => {
                    session.rounds += 1;
                    self.sub_update(&session, tool.as_deref(), turn.id()).await;
                }
                "done" => {
                    let messages: Vec<Message> =
                        serde_json::from_value(data["messages"].clone()).unwrap_or_default();
                    answer = crate::ai_subagents::answer_of(&messages);
                    ended = Some(("done", None));
                }
                "error" => {
                    let why = data["message"].as_str().unwrap_or("it failed").to_owned();
                    ended = Some(("failed", Some(why)));
                }
                "refusal" => {
                    let why = format!("{} refused", data["name"].as_str().unwrap_or("the model"));
                    ended = Some(("failed", Some(why)));
                }
                name if PROPOSAL_EVENTS.contains(&name) => {
                    if name != "applied" {
                        proposed.push(format!(
                            "{}: waiting for the user",
                            agent_threads::proposal_what(name, &data)
                        ));
                    }
                    // Raised in this chat, under the sub agent's block, and applied from here
                    // (subagents U4); its outcome is recorded in the sub agent's session too.
                    let mut d = data;
                    d["subagent"] = json!(c.id);
                    d["sub_thread"] = json!(session.thread);
                    self.emit_sub(name, d).await;
                }
                _ => {}
            }
        }
        // A turn that ends with no end of its own was stopped: by the user, or with this one.
        let (outcome, error) = ended.unwrap_or(("stopped", None));
        session.status = outcome.to_owned();
        session.ended_ms = Some(crate::ai_settings::now_ms());
        let ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.activity.subagent_end(&session, ms);
        self.sub_update(&session, None, turn.id()).await;
        turn.finish();
        crate::ai_subagents::result(crate::ai_subagents::Ended {
            thread: &session.thread,
            agent: &session.agent,
            delegated: session.delegated.as_deref(),
            outcome,
            rounds: session.rounds,
            tools: session.tools,
            ms,
            answer,
            error,
            proposed,
            ignored,
        })
    }

    /// Keep sub agent `s` as it stands in the thread, and tell the client: the `subagent` event,
    /// with the tool it runs now and its turn's id in the turn routes.
    async fn sub_update(
        &self,
        s: &agent_threads::SubSession,
        tool: Option<&str>,
        turn: Option<&str>,
    ) {
        if let Some(mut log) = self.log() {
            match log.subagents.iter_mut().find(|x| x.call == s.call) {
                Some(x) => *x = s.clone(),
                None => log.subagents.push(s.clone()),
            }
            log.sub_changes += 1;
        }
        let mut data = json!(s);
        data["id"] = json!(s.call);
        data["tool"] = json!(tool);
        data["turn"] = json!(turn);
        self.out.send("subagent", data).await;
    }
}

fn assistant(content: Vec<Content>, state: Option<ProviderState>) -> Message {
    Message {
        role: Role::Assistant,
        content,
        provider_state: state,
    }
}

pub(crate) fn tool(name: &str, description: &str, properties: Value) -> ToolSpec {
    let required: Vec<&String> = properties
        .as_object()
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

/// The chat's tools: read-only ones, and proposals that write nothing unless Settings > AI turned
/// their confirmation off (ai D14, D18, D20, D22), then the web tools (ai D27).
/// The tools a turn offers: [`tool_specs`] without the internet tools Settings > AI switched
/// off (ai D28). `web_search` is added apart, only with a Brave Search key.
fn offered_tools(web: ai::Web) -> Vec<ToolSpec> {
    tool_specs()
        .into_iter()
        .filter(|t| match t.name.as_str() {
            "fetch_url" => web.read,
            "download_asset" => web.download,
            _ => true,
        })
        .collect()
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool("decide", "Ask a tool-free decision model typed questions about untrusted state. Returns typed answers and probabilities. Makes no changes. Set account and model to null to use defaults.", json!({
            "state": {"description": "Untrusted context", "type": ["string", "object", "array"]},
            "questions": {"type": "object", "description": "Question ids mapped to type (noul, choice, score), instructions and criteria."},
            "account": {"type": ["string", "null"]}, "model": {"type": ["string", "null"]}
        })),
        tool(
            "search_notes",
            "Full-text search over every note's title and text. Returns rows of id, type, title and fields, best match first.",
            json!({
                "text": { "type": "string", "description": "The words to look for." },
                "limit": { "type": "integer", "description": "How many rows to return, 1 to 20." },
            }),
        ),
        tool(
            "query_notes",
            "Run a query in the vault query language, such as `from task where status == \"Todo\" sort due asc`. It is not SQL: `==`, `&&`, `||`, double-quoted text, `sort` and the functions query_format lists. Returns at most 50 rows and the total that matched.",
            json!({ "query": { "type": "string", "description": "The query text." } }),
        ),
        tool(
            "get_note",
            "Read one note by id: its type, its version and the whole note as JSON, body included, and `assets`, the files it references (id, filename, media_type, size) for read_asset. Use an id exactly as a row of query_notes, search_notes or backlinks gave it, or as a link holds it. Never make an id up.",
            json!({ "id": { "type": "string", "description": "The note's 26-character id, copied from a row, a link or an earlier result." } }),
        ),
        tool(
            "read_asset",
            "Read an attached file by its asset id. An image (PNG, JPEG, GIF, WebP) comes back as the image itself, scaled down when large; a PDF, Word document (docx), spreadsheet (xlsx, xls, ods, csv) or text file as its text, cut at 40000 characters; anything else as its id, filename, media type and size. At most 8 images per question.",
            json!({ "id": { "type": "string", "description": "The asset's 26-character id." } }),
        ),
        tool(
            "find_assets",
            "Find stored files (assets) by name: those whose file name contains `text`, ignoring case, newest first. Empty text lists the newest. Each is its id, filename, media_type, size and created time, for read_asset or for a note's asset field.",
            json!({
                "text": { "type": "string", "description": "Part of the file name; empty for the newest assets." },
                "limit": { "type": "integer", "description": "How many assets to return, 1 to 50; 20 is usual." },
            }),
        ),
        tool(
            "backlinks",
            "The notes that link to a note, with the field or mention that links them.",
            json!({ "id": { "type": "string", "description": "The note's 26-character id." } }),
        ),
        tool(
            "list_types",
            "List every note type: its name, label, plural, description and whether it is built in.",
            json!({}),
        ),
        tool(
            "get_type",
            "Read one type: its type file's WCL text as stored, and a parsed summary of its fields (with kinds) and its templates (with their tabs and slots).",
            json!({ "name": { "type": "string", "description": "The type's name, such as `task`." } }),
        ),
        tool(
            "value_format",
            "How each field value is written: dates, datetimes with their offset, times, date ranges, durations, URLs, note and asset references, taxonomy terms, locations and colours, with the JSON form of a note and which fields only the server writes. Read it before writing a field you are not sure of, and after a note is refused.",
            json!({}),
        ),
        tool(
            "query_format",
            "The whole query language: clauses and their order, field names and values, operators, every function, what SQL habits to avoid, worked examples, and query views. Read it before writing a query that is more than a type and a simple condition, and after a query syntax error.",
            json!({}),
        ),
        tool(
            "type_format",
            "The vocabulary of type files: their items, every field kind and handlers. Templates, their tabs and the content blocks they hold are in block_format. Read it before proposing a type change.",
            json!({}),
        ),
        tool(
            "block_format",
            "The vocabulary of templates and content: templates and their tabs and slots, every content block and the inline markup of their text. Read it before proposing a template change or writing a note's content blocks.",
            json!({}),
        ),
        tool(
            "propose_type_change",
            "Propose a type file for the user to apply: a change to an existing type, such as its fields or templates, or a new type under a new name. It is checked as the type write would check it, and the user sees a diff of the WCL and applies or discards it. When the user lets type changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The type's name: an existing type to change, or a new name to create a type." },
                "wcl": { "type": "string", "description": "The whole new type file as WCL text, with every field and template it keeps." },
            }),
        ),
        tool(
            "propose_type_delete",
            "Propose deleting a type for the user to apply. A built-in type cannot be deleted, nor a type that still has notes (delete or move them first), nor one another type's reference field targets (change that type first). When the user lets type changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({ "name": { "type": "string", "description": "The type's name." } }),
        ),
        tool(
            "list_taxonomies",
            "List every taxonomy: its name, label, description, whether it is open (notes may add terms), whether it is built in, and how many terms it holds.",
            json!({}),
        ),
        tool(
            "get_taxonomy",
            "Read one taxonomy: its taxonomy file's WCL text as stored, and every term as a path from its root, such as `Active/In progress`.",
            json!({ "name": { "type": "string", "description": "The taxonomy's name, such as `status`." } }),
        ),
        tool(
            "taxonomy_format",
            "The vocabulary of taxonomy files: the label, description, open taxonomies and nested terms, and the taxonomy field kind. Read it before proposing a taxonomy change.",
            json!({}),
        ),
        tool(
            "propose_taxonomy_change",
            "Propose a taxonomy file for the user to apply: a change to an existing taxonomy, or a new taxonomy under a new name. Renaming or moving a term needs a rename entry, so the notes holding it move with it; dropping a term a note holds is refused. It is checked as the taxonomy write would check it, and the user sees a diff of the WCL and applies or discards it. When the user lets taxonomy changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The taxonomy's name: an existing taxonomy to change, or a new name to create one." },
                "wcl": { "type": "string", "description": "The whole new taxonomy file as WCL text, with every term it keeps." },
                "renames": {
                    "type": "array",
                    "description": "Terms renamed or moved, old path to new path; empty when none are. A term's children move with it.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "from": { "type": "string", "description": "The term's path now." },
                            "to": { "type": "string", "description": "Its path in the new file." },
                        },
                        "required": ["from", "to"],
                        "additionalProperties": false,
                    },
                },
            }),
        ),
        tool(
            "propose_taxonomy_delete",
            "Propose deleting a taxonomy for the user to apply. A taxonomy a type's field uses cannot be deleted: change those types first. When the user lets taxonomy changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({ "name": { "type": "string", "description": "The taxonomy's name." } }),
        ),
        tool(
            "list_agents",
            "List every agent: its name, label, description, AI account, model, whether it is enabled, whether it is built in, and how many triggers it has.",
            json!({}),
        ),
        tool(
            "get_agent",
            "Read one agent: its agent file's WCL text as stored, the agent parsed (permissions and triggers included), and the vocabulary of agent files. Read it before proposing an agent change.",
            json!({ "name": { "type": "string", "description": "The agent's name, such as `assistant`." } }),
        ),
        tool(
            "propose_agent_change",
            "Propose an agent file for the user to apply: a change to an existing agent, or a new agent under a new name. It is checked as the agent write would check it, and refused when it would give the agent a tool, a note type, a confirmation switch turned off, an Internet switch turned on, the Outbox switch send_without_review turned on, Browser automation turned on, Run wscript code or sub agents that you do not have yourself; you cannot change your own file. The user sees a diff of the WCL and applies or discards it. When the user lets agent changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The agent's name: an existing agent to change, or a new name (lowercase letters, digits and underscores, starting with a letter) to create one." },
                "wcl": { "type": "string", "description": "The whole new agent file as WCL text, with everything it keeps." },
            }),
        ),
        tool(
            "propose_agent_delete",
            "Propose deleting an agent for the user to apply. Its threads stay. The built-in Assistant cannot be deleted, and you cannot delete yourself. When the user lets agent changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({ "name": { "type": "string", "description": "The agent's name." } }),
        ),
        tool(
            "propose_create",
            "Propose a new note for the user to create. It is checked, and the user sees the new note and creates or discards it. When the user lets note changes go ahead without asking, it is created at once and the result says applied, with its id and version.",
            json!({
                "type": { "type": "string", "description": "The new note's type, such as `task`." },
                "note": {
                    "type": "string",
                    "description": "The whole new note as JSON text, {\"fields\": {...}, \"blocks\": [...]}.",
                },
            }),
        ),
        tool(
            "propose_archive",
            "Propose archiving a note, or taking it out of the archive, for the user to apply. When the user lets archiving go ahead without asking, it is done at once and the result says applied.",
            json!({
                "id": { "type": "string", "description": "The note's id." },
                "archived": { "type": "boolean", "description": "true to archive the note, false to take it out of the archive." },
            }),
        ),
        tool(
            "propose_delete",
            "Propose deleting a note for the user to apply. Applied, it moves the note to the trash, where the user can restore it; nothing empties the trash. When the user lets deletes go ahead without asking, the note moves to the trash at once and the result says applied.",
            json!({ "id": { "type": "string", "description": "The note's id." } }),
        ),
        tool(
            "propose_edit",
            "Propose a change to one note for the user to apply. It is checked, and the user sees a diff and applies or discards it. When the user lets note changes go ahead without asking, it is written at once and the result says applied, with the new version.",
            json!({
                "id": { "type": "string", "description": "The note's id." },
                "base_version": { "type": "string", "description": "The version get_note returned." },
                "note": {
                    "type": "string",
                    "description": "The whole new note as JSON text, {\"fields\": {...}, \"blocks\": [...]}, with every field and block it keeps.",
                },
            }),
        ),
        tool(
            "propose_fields",
            "Propose setting some fields of one note, keeping every other field and the whole body as they are: the way to tick `processed`, change a status or a due date, or fix a title without resending the note. It is checked and shown to the user like propose_edit, and written at once when the user lets note changes go ahead without asking. A content field (the body) is a block: change it with propose_edit.",
            json!({
                "id": { "type": "string", "description": "The note's id." },
                "base_version": { "type": "string", "description": "The version get_note returned." },
                "fields": {
                    "type": "string",
                    "description": "JSON text of an object: each field to set and its new value, such as {\"processed\": true, \"due\": \"2026-12-01\"}. null removes a field.",
                },
            }),
        ),
    ]
    .into_iter()
    .chain(crate::ai_skills::tool_specs())
    .chain(crate::ai_skill_changes::tool_specs())
    .chain(crate::ai_scripts::tool_specs())
    .chain(crate::ai_script_changes::tool_specs())
    .chain(crate::ai_device_changes::tool_specs())
    .chain(crate::ai_task_changes::tool_specs())
    .chain(agent_memory::tool_specs())
    .chain(agent_memory::cross_tool_specs())
    .chain(crate::ai_outbox::tool_specs())
    .chain([crate::ai_notify::tool_spec(), crate::ai_ask::tool_spec()])
    .chain(crate::ai_web::tool_specs())
    .collect()
}

/// `s` cut to `limit` characters, with a marker saying how much was left out.
pub(crate) fn cut(s: &str, limit: usize) -> String {
    let total = s.chars().count();
    if total <= limit {
        return s.to_owned();
    }
    let kept: String = s.chars().take(limit).collect();
    format!("{kept}\n[truncated: {} more characters]", total - limit)
}

/// Every string in `v` cut to `limit` characters.
fn cut_strings(v: &mut Value, limit: usize) {
    match v {
        Value::String(s) => {
            if s.chars().count() > limit {
                *s = cut(s, limit);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| cut_strings(x, limit)),
        Value::Object(o) => o.values_mut().for_each(|x| cut_strings(x, limit)),
        _ => {}
    }
}

/// The ids and titles of `rows`, for a `tool_result` summary.
fn notes_of(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .map(|r| json!({ "id": r["id"], "title": r["title"] }))
        .collect()
}

pub(crate) fn str_arg<'a>(input: &'a Value, name: &str) -> Result<&'a str, ToolOut> {
    input[name]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolOut::error(format!("{name} must be a non-empty string")))
}

/// What a listing tells the model of the rows an agent's types left out (agents G4).
fn left_out_of(n: usize) -> Value {
    if n == 0 {
        return json!({});
    }
    json!({ "left_out": n, "left_out_message": format!(
        "{n} more notes matched, of types this agent may not read."
    ) })
}

fn rows_out(rows: Vec<Value>, extra: Value) -> ToolOut {
    let summary = json!({ "notes": notes_of(&rows) });
    let mut content = json!({ "rows": rows });
    if let (Some(c), Some(e)) = (content.as_object_mut(), extra.as_object()) {
        c.extend(e.clone());
    }
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary,
        proposal: None,
    }
}

/// Run one tool call. A call the model got wrong is answered as an error, not run (ai D5).
/// A proposal whose confirmation is off is applied here, in place of being sent (ai D22). The
/// Outbox tools draft at once, as the agent, with a caller that has `write` (outbox X9). A
/// proposal that is not made here is refused in a trigger's run ([`Held::Drop`]): no one would
/// apply it, so it must not wait (TV-84).
#[allow(clippy::too_many_arguments)]
fn run_tool(
    w: &Writer,
    author: &Author,
    scope: Scope,
    direct: Direct,
    held: Held,
    limits: &Limits,
    agent: &Agent,
    call: &ToolCall,
    gmail_actions: bool,
) -> ToolOut {
    // The memory tools reach only the agent's own memories, and write as the agent (agents G21).
    if call.error.is_none() && agent_memory::is_memory_tool(&call.name) {
        return agent_memory::run(w, scope, &agent.name, &call.name, &call.input);
    }
    if call.error.is_none() && crate::ai_outbox::is_draft_tool(&call.name) {
        return crate::ai_outbox::draft(w, scope, agent, limits, &call.name, &call.input);
    }
    let mut out = propose_or_read(w, author, limits, agent, call);
    if let Some((event, p)) = &mut out.proposal
        && matches!(*event, "proposal" | "type_proposal" | "taxonomy_proposal")
    {
        p["proposer"] = json!(agent.name);
        if let Some(parent) = &limits.parent {
            p["proposer_parent"] = json!(parent);
            p["proposer_limits"] = serde_json::to_value(limits).expect("limits are JSON");
        }
        if let Err(e) = limits.check_event(w, event, p) {
            return ToolOut::error(e.to_string());
        }
    }
    match &out.proposal {
        Some((event, p)) if direct.covers(event, p) => {
            let (event, p) = (*event, p.clone());
            // A change made without the user's Apply to an Outbox item is the agent's, whoever
            // runs the turn, so the writer's review rules hold for it (outbox X2).
            let email = event == "proposal"
                && (p["type"] == "email"
                    || p["note_id"]
                        .as_str()
                        .and_then(|id| w.read(id).ok())
                        .is_some_and(|n| n.type_name == "email"));
            let author = if email && gmail_actions {
                Author::AgentMail(agent.name.clone())
            } else if email || touches_outbox(w, event, &p) {
                Author::Agent(agent.name.clone())
            } else {
                author.clone()
            };
            let mut made = make(w, &author, event, &p, out.summary);
            if email && !gmail_actions && !made.is_error {
                let notice = "Gmail action skipped: the note change was applied only in the vault. Gmail archive/trash needs a user Apply or an opted-in foreground chat, never a trigger or mention run.";
                made.summary["gmail_skipped"] = json!(notice);
                let summary_message = made.summary["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                made.summary["message"] = json!(format!("{summary_message} {notice}").trim());
                if let Ok(mut content) = serde_json::from_str::<Value>(&made.content) {
                    content["message"] = json!(format!(
                        "{} {notice}",
                        content["message"].as_str().unwrap_or_default()
                    ));
                    content["gmail_skipped"] = json!(notice);
                    if summary_message.is_empty() {
                        made.summary["message"] = content["message"].clone();
                    }
                    made.content = content.to_string();
                }
            }
            made
        }
        Some(_) if held == Held::Drop => ToolOut::error(HELD_REFUSAL),
        _ => out,
    }
}

/// What a trigger's run is told when a change would wait for an apply (TV-84). Such a change is
/// refused, not proposed: a trigger's thread has no one to apply it, and the proposal would stay
/// pending where nobody sees it. A grant of run code, run saved scripts or sub agents is the
/// case the agent tools reach.
const HELD_REFUSAL: &str = "This run is unattended, so the change was not proposed: no one is \
    here to apply a proposal, and a change that turns on run code, run saved scripts or sub \
    agents needs an admin's Apply in a chat. Say which change you would make, and that it \
    needs the user.";

/// Whether note proposal `p` creates or changes an Outbox item.
fn touches_outbox(w: &Writer, event: &str, p: &Value) -> bool {
    if event != "proposal" {
        return false;
    }
    if p["type"] == writer::OUTBOX_TYPE {
        return true;
    }
    p["note_id"]
        .as_str()
        .and_then(|id| w.read(id).ok())
        .is_some_and(|n| n.type_name == writer::OUTBOX_TYPE)
}

/// Make the change a tool proposed, as the caller: the `applied` event, and a result telling the
/// model it was made and its new version. A failure writes nothing and goes back to the model.
fn make(w: &Writer, author: &Author, event: &str, p: &Value, summary: Value) -> ToolOut {
    let done = match apply_change(w, author, event, p) {
        Ok(done) => done,
        Err(ApplyError::Write(writer::Error::Conflict {
            id,
            current_version,
            ..
        })) => {
            let base = p["base_version"].as_str().unwrap_or_default();
            return conflict(&id, base, &current_version, "write it again");
        }
        Err(e) => {
            let message = e.to_string();
            let details = ApiError::from(e).body;
            let mut out = ToolOut::error(message.clone());
            out.content = json!({
                "error": message,
                "details": details,
                "message": "Nothing was written. Fix the problem and try again.",
            })
            .to_string();
            return out;
        }
    };
    let op = done["op"].as_str().unwrap_or_default().to_owned();
    let version = done["version"].as_str().unwrap_or_default().to_owned();
    let name = done["type_name"].as_str().unwrap_or_default().to_owned();
    let made = match op.as_str() {
        "update" => format!("The edit was written to the note. Its new version is {version}."),
        "create" => format!(
            "The note was created with id {}. Its version is {version}.",
            done["note_id"].as_str().unwrap_or_default()
        ),
        "archive" => format!("The note was archived. Its new version is {version}."),
        "unarchive" => {
            format!("The note was taken out of the archive. Its new version is {version}.")
        }
        "delete" => "The note was moved to the trash, where the user can restore it.".to_owned(),
        "type_create" => format!("The type {name} was created."),
        "type_delete" => format!("The type {name} was deleted."),
        "taxonomy_create" | "taxonomy_change" | "taxonomy_delete" => {
            let name = done["taxonomy_name"].as_str().unwrap_or_default();
            match op.as_str() {
                "taxonomy_create" => format!("The taxonomy {name} was created."),
                "taxonomy_delete" => format!("The taxonomy {name} was deleted."),
                _ => format!(
                    "The taxonomy {name} was changed; {} notes were rewritten for its renames.",
                    done["notes"].as_u64().unwrap_or(0)
                ),
            }
        }
        "agent_create" | "agent_change" | "agent_delete" => {
            let name = done["agent_name"].as_str().unwrap_or_default();
            match op.as_str() {
                "agent_create" => format!("The agent {name} was created."),
                "agent_delete" => format!("The agent {name} was deleted; its threads stay."),
                _ => format!("The agent {name} was changed."),
            }
        }
        "agent_memory_create" | "agent_memory_change" | "agent_memory_delete" => {
            let id = done["memory_id"].as_str().unwrap_or_default();
            let of = done["memory_agent"].as_str().unwrap_or_default();
            match op.as_str() {
                "agent_memory_create" => format!("The memory {id} of agent {of} was created."),
                "agent_memory_delete" => format!(
                    "The memory {id} of agent {of} was moved to the trash, where the user can restore it."
                ),
                _ => format!("The memory {id} of agent {of} was changed."),
            }
        }
        o if o.starts_with("skill_") => crate::ai_skill_changes::made_line(&done),
        o if o.starts_with("tool_") => crate::ai_scripts::made_line(&done),
        _ => format!("The type {name} was changed."),
    };
    let mut content = done.clone();
    content["applied"] = json!(true);
    content["message"] = json!(format!(
        "{made} Settings > AI lets this kind of change go ahead without asking, so the user did \
         not confirm it; it is done and recorded in the history."
    ));
    let mut applied = done;
    applied["id"] = p["id"].clone();
    if event == "proposal" {
        applied["title"] = p["title"].clone();
    }
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary,
        proposal: Some(("applied", applied)),
    }
}

/// Why [`apply_change`] wrote nothing.
#[derive(Debug)]
pub enum ApplyError {
    /// The change is not one a chat proposes, or lacks a member it needs.
    Invalid(String),
    /// The type or taxonomy changed since the proposal.
    Stale(String),
    /// The write refused it: a conflict, a validation failure, a missing note.
    Write(writer::Error),
    /// A download got nothing (ai D27): the address rule, the size limit, the server.
    Fetch(ai::web::FetchError),
    /// The proposing agent may not make the change (agents G20): it would give another agent
    /// more than it has, or it is the agent's own file.
    Refused(String),
    /// A save from the browser box got nothing (browser B5): no box, or the box refused.
    Browser(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Invalid(m)
            | ApplyError::Stale(m)
            | ApplyError::Refused(m)
            | ApplyError::Browser(m) => f.write_str(m),
            ApplyError::Write(e) => e.fmt(f),
            ApplyError::Fetch(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<writer::Error> for ApplyError {
    fn from(e: writer::Error) -> ApplyError {
        ApplyError::Write(e)
    }
}

impl From<ApplyError> for ApiError {
    fn from(e: ApplyError) -> ApiError {
        match e {
            ApplyError::Invalid(m) => ApiError::bad_request(m),
            ApplyError::Stale(m) => ApiError::new(StatusCode::CONFLICT, "stale", m),
            ApplyError::Refused(m) => ApiError::new(StatusCode::FORBIDDEN, "not_permitted", m),
            ApplyError::Browser(m) => ApiError::new(StatusCode::BAD_GATEWAY, "browser_failed", m),
            ApplyError::Write(e) => e.into(),
            ApplyError::Fetch(e) => {
                use ai::web::FetchError;
                let (status, code) = match &e {
                    FetchError::LocalNetwork => (StatusCode::FORBIDDEN, "local_network"),
                    FetchError::BadUrl(_) => (StatusCode::BAD_REQUEST, "bad_request"),
                    _ => (StatusCode::BAD_GATEWAY, "fetch_failed"),
                };
                ApiError::new(status, code, e.to_string())
            }
        }
    }
}

/// Apply a chat change as `author`, through the writer calls the panel's Apply reaches (ai D22).
/// `event` is `proposal`, `type_proposal` or `taxonomy_proposal`, and `p` its data as the client
/// receives it:
///
/// - `update`: the note `note_id` replaced by `after` at `base_version`;
/// - `create`: a new note of `type` from `after`;
/// - `archive` and `unarchive`: the note `note_id` at `base_version`;
/// - `delete`: the note `note_id` at `base_version`, moved to the trash, never purged (ai D21);
/// - `download`: the file at `url` downloaded and stored as a new asset called `filename`,
///   through the guarded fetcher (ai D27), answering `{op, asset_id, filename, media_type,
///   size, url}`. It waits for the download, so call this off the async threads; with `source`
///   `browser`, the file at `path` in the browser box, when the proposing agent still has
///   browser automation (browser B5);
/// - a type proposal: `after` as the type `name`, when the type file is still `before`; a new
///   type through the type write, a change planned and then applied as one commit;
/// - a taxonomy proposal (ai D31): when the taxonomy file is still `before`, `after` as the
///   taxonomy `name` with its `renames`, or for `op` `delete` the taxonomy deleted.
/// - an agent proposal (agents G20): when the agent file is still `before` and the change stays
///   within its `proposer`'s permissions, `after` as the agent `name`, or for `op` `delete` the
///   agent deleted.
/// - an agent memory proposal (agents G21): when the memory is still at `base_version` and its
///   `proposer` may still reach `memory_agent`'s memories, the memory created, changed or moved
///   to the trash, answering `{op, memory_id, memory_agent, version}`.
///
/// Returns `{op, note_id, version}` for a note, `{op, type_name}` for a type (`type_create` or
/// `type_change`) and `{op, taxonomy_name, notes}` for a taxonomy (`taxonomy_create`,
/// `taxonomy_change` or `taxonomy_delete`, `notes` the number rewritten). A failure writes
/// nothing.
pub fn apply_change(
    w: &Writer,
    author: &Author,
    event: &str,
    p: &Value,
) -> Result<Value, ApplyError> {
    apply_change_with_admin(w, author, event, p, false)
}

/// Check current agent files and any saved delegation ceiling. Call under the write lock.
pub(crate) fn authorize_proposal(
    w: &Writer,
    author: &Author,
    event: &str,
    p: &Value,
) -> Result<(), ApplyError> {
    let proposer = p["proposer"]
        .as_str()
        .or(match author {
            Author::Agent(name) | Author::AgentMail(name) => {
                Some(name.split("/sub:").next().unwrap_or(name))
            }
            _ => None,
        })
        .or_else(|| p["agent"].as_str());
    if let Some(name) = proposer {
        let agent = crate::ai_agents::agent_named(w, name)?.ok_or_else(|| {
            ApplyError::Refused(format!("The proposing agent {name} no longer exists."))
        })?;
        if !agent.enabled {
            return Err(ApplyError::Refused(
                "The proposing agent is disabled.".into(),
            ));
        }
        Limits::of(&agent).check_event(w, event, p)?;
        if let Some(parent) = p["proposer_parent"].as_str() {
            let parent = crate::ai_agents::agent_named(w, parent)?
                .ok_or_else(|| ApplyError::Refused("The parent agent no longer exists.".into()))?;
            if !parent.enabled {
                return Err(ApplyError::Refused("The parent agent is disabled.".into()));
            }
            Limits::of(&parent).check_event(w, event, p)?;
            let ceiling: Limits = serde_json::from_value(p["proposer_limits"].clone())
                .map_err(|e| ApplyError::Invalid(format!("invalid delegation limits: {e}")))?;
            ceiling.check_event(w, event, p)?;
        }
    }
    Ok(())
}

/// `admin` is the authenticated Apply caller's scope, never the proposal's author or fields.
pub(crate) fn apply_change_with_admin(
    w: &Writer,
    author: &Author,
    event: &str,
    p: &Value,
    admin: bool,
) -> Result<Value, ApplyError> {
    // A type, taxonomy or agent change is recorded as the author's (activity Y2).
    let _acting = writer::acting_as(author);
    let member = |name: &str| {
        p[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApplyError::Invalid(format!("the change has no {name}")))
    };
    if event == "type_proposal" {
        let name = member("name")?;
        let before = p["before"].as_str();
        if w.read_type(name).ok().as_deref() != before {
            return Err(ApplyError::Stale(format!(
                "the type {name} changed since the proposal, so nothing was written"
            )));
        }
        if p["op"] == "delete" {
            w.delete_type_checked(name, |w| authorize_proposal(w, author, event, p))?;
            return Ok(json!({ "op": "type_delete", "type_name": name }));
        }
        let after = member("after")?;
        type_handler_secrets(name, before, after).map_err(ApplyError::Refused)?;
        if before.is_none() {
            w.put_type_checked(name, after, |w| authorize_proposal(w, author, event, p))?;
            return Ok(json!({ "op": "type_create", "type_name": name }));
        }
        let change = TypeChange {
            wcl: after.to_owned(),
            ..TypeChange::default()
        };
        let plan = w.plan_type_change(name, &change)?;
        w.apply_type_change_checked(name, &change, &plan.token, author, |w| {
            authorize_proposal(w, author, event, p)
        })?;
        return Ok(json!({ "op": "type_change", "type_name": name }));
    }
    if event == "taxonomy_proposal" {
        return apply_taxonomy(w, author, p);
    }
    if event == "agent_proposal" {
        return crate::ai_agents::apply_agent(w, p);
    }
    if event == "agent_memory_proposal" {
        return agent_memory::apply(w, author, p);
    }
    if event == "skill_proposal" {
        return crate::ai_skill_changes::apply_skill(w, p);
    }
    if event == "tool_proposal" {
        return crate::ai_scripts::apply_tool(w, p, admin);
    }
    if event == "device_file_proposal" {
        return crate::ai_device_changes::apply(w, p, admin);
    }
    if event == "script_proposal" {
        return crate::ai_script_changes::apply_script(w, p, admin);
    }
    if event == "task_proposal" {
        return crate::ai_task_changes::apply_task(w, p, admin);
    }
    if event != "proposal" {
        return Err(ApplyError::Invalid(format!("{event:?} is not a change")));
    }
    let op = p["op"].as_str().unwrap_or("update");
    if op == "download" && p["source"] == "browser" {
        return crate::ai_browser::apply_save(w, author, p);
    }
    if op == "download" {
        return crate::ai_web::apply_download(w, p);
    }
    let doc = || match &p["after"] {
        v @ Value::Object(_) => Ok(Input::Json(v.clone())),
        _ => Err(ApplyError::Invalid("the change has no after".into())),
    };
    let input = if matches!(op, "update" | "create") {
        Some(doc()?)
    } else {
        None
    };
    let write = match op {
        "update" => writer::CheckedNoteWrite::Update(
            member("note_id")?,
            input.as_ref().expect("update input"),
            Some(member("base_version")?),
        ),
        "create" => {
            writer::CheckedNoteWrite::Create(member("type")?, input.as_ref().expect("create input"))
        }
        "archive" | "unarchive" => writer::CheckedNoteWrite::Archive(
            member("note_id")?,
            op == "archive",
            Some(member("base_version")?),
        ),
        "delete" => {
            writer::CheckedNoteWrite::Delete(member("note_id")?, Some(member("base_version")?))
        }
        other => {
            return Err(ApplyError::Invalid(format!(
                "{other:?} is not a note change"
            )));
        }
    };
    let saved =
        w.commit_checked_note(write, author, |w| authorize_proposal(w, author, event, p))?;
    Ok(json!({ "op": op, "note_id": saved.id, "version": saved.version }))
}

/// A tool call's answer, a proposal not yet applied.
fn propose_or_read(
    w: &Writer,
    author: &Author,
    limits: &Limits,
    agent: &Agent,
    call: &ToolCall,
) -> ToolOut {
    let _acting = writer::acting_as(&Author::Agent(agent.name.clone()));
    w.take_agent_omissions();
    if let Some(e) = &call.error {
        return ToolOut::error(e.clone());
    }
    if let Some(refused) = limits.check(w, call) {
        return refused;
    }
    let input = &call.input;
    let result = match call.name.as_str() {
        "decide" => {
            if !agent.permissions.allows_tool("decide") {
                return ToolOut::error("The agent's permissions.tools does not allow decide");
            }
            serde_json::from_value::<ai::decisions::Input>(input.clone())
                .map_err(|_| ToolOut::error("Invalid decisions input"))
                .and_then(|input| {
                    let (result, line) =
                        ai::decisions::call(w.state(), input, std::time::Duration::from_secs(30));
                    tracing::info!(target: "ai", "{line}");
                    result
                        .map(|reply| ToolOut {
                            content: serde_json::to_string(&reply).expect("decision JSON"),
                            is_error: false,
                            summary: json!({}),
                            proposal: None,
                        })
                        .map_err(|e| ToolOut::error(e.to_string()))
                })
        }
        "search_notes" => str_arg(input, "text").and_then(|text| {
            let limit = input["limit"]
                .as_u64()
                .map_or(10, |n| usize::try_from(n).unwrap_or(SEARCH_LIMIT))
                .clamp(1, SEARCH_LIMIT);
            Vault::search(w, text, limit, script_core::Archived::Exclude)
                .map(|mut rows| {
                    rows.retain(|r| r["id"].as_str().is_some_and(|id| w.agent_may_read(id)));
                    let left_out = limits.keep(&mut rows);
                    rows_out(rows, left_out_of(left_out))
                })
                .map_err(ToolOut::error)
        }),
        "query_notes" => str_arg(input, "query").and_then(|q| {
            Vault::query_page(w, q, QUERY_LIMIT, None)
                .map(|page| {
                    let mut rows = page["rows"].as_array().cloned().unwrap_or_default();
                    rows.retain(|r| r["id"].as_str().is_some_and(|id| w.agent_may_read(id)));
                    let left_out = limits.keep(&mut rows);
                    let mut extra = left_out_of(left_out);
                    extra["total"] = page["total"].clone();
                    extra["more"] = json!(!page["next"].is_null());
                    rows_out(rows, extra)
                })
                .map_err(|e| ToolOut::error(query_error(q, &e)))
        }),
        "get_note" => str_arg(input, "id").and_then(|id| {
            if !w.agent_may_read(id) {
                return Ok(ToolOut::error(format!("Note {id} is held from agents.")));
            }
            Vault::get(w, id)
                .map(|mut note| {
                    cut_strings(&mut note, BODY_LIMIT);
                    note["assets"] = json!(ai_assets::note_assets(w, &note["note"]));
                    let summary = json!({ "notes": [{
                        "id": note["id"], "title": note["note"]["fields"]["title"],
                    }] });
                    ToolOut {
                        content: note.to_string(),
                        is_error: false,
                        summary,
                        proposal: None,
                    }
                })
                .map_err(|e| ToolOut::error(get_note_error(id, &e)))
        }),
        "find_assets" => {
            let found = ai_assets::find_assets(w, input);
            Ok(ToolOut {
                content: found.content,
                is_error: found.is_error,
                summary: found.summary,
                proposal: None,
            })
        }
        "backlinks" => str_arg(input, "id").and_then(|id| {
            Vault::backlinks(w, id)
                .map(|mut rows| {
                    rows.retain(|r| r["id"].as_str().is_some_and(|id| w.agent_may_read(id)));
                    let left_out = limits.keep(&mut rows);
                    rows_out(rows, left_out_of(left_out))
                })
                .map_err(ToolOut::error)
        }),
        "propose_edit" => propose(w, author, &call.id, input),
        "propose_fields" => propose_fields(w, author, &call.id, input),
        "propose_create" => propose_create(w, author, &call.id, input),
        "propose_archive" => propose_archive(w, &call.id, input),
        "propose_delete" => propose_delete(w, author, &call.id, input),
        "list_types" => Ok(list_types(w)),
        "get_type" => get_type(w, input),
        "value_format" => Ok(value_format()),
        "query_format" => Ok(query_format()),
        "type_format" => Ok(type_format()),
        "block_format" => Ok(block_format()),
        "propose_type_change" => propose_type(w, &call.id, input),
        "propose_type_delete" => propose_type_delete(w, &call.id, input),
        "list_taxonomies" => Ok(crate::ai_taxonomies::list(w)),
        "get_taxonomy" => crate::ai_taxonomies::get(w, input),
        "taxonomy_format" => Ok(crate::ai_taxonomies::format()),
        "propose_taxonomy_change" => crate::ai_taxonomies::propose_change(w, &call.id, input),
        "propose_taxonomy_delete" => crate::ai_taxonomies::propose_delete(w, &call.id, input),
        "list_agents" => crate::ai_agents::list(w),
        "get_agent" => crate::ai_agents::get(w, agent, input),
        "propose_agent_change" => crate::ai_agents::propose_change(w, agent, &call.id, input),
        "propose_agent_delete" => crate::ai_agents::propose_delete(w, agent, &call.id, input),
        "list_skills" => crate::ai_skill_changes::list(w, agent),
        "get_skill" => crate::ai_skill_changes::get(w, agent, input),
        "propose_skill_change" => {
            crate::ai_skill_changes::propose_change(w, agent, &call.id, input)
        }
        "propose_skill_file_change" => {
            crate::ai_skill_changes::propose_file(w, agent, &call.id, input)
        }
        "propose_skill_file_delete" => {
            crate::ai_skill_changes::propose_file_delete(w, agent, &call.id, input)
        }
        "propose_skill_delete" => {
            crate::ai_skill_changes::propose_delete(w, agent, &call.id, input)
        }
        "list_tools" => crate::ai_scripts::list(w),
        "get_tool" => crate::ai_scripts::get(w, input),
        "propose_tool_change" => crate::ai_scripts::propose_change(w, agent, &call.id, input),
        "propose_tool_delete" => crate::ai_scripts::propose_delete(w, agent, &call.id, input),
        "get_device_file" => crate::ai_device_changes::get(w, input),
        "propose_device_file_change" => {
            crate::ai_device_changes::propose(w, agent, &call.id, input)
        }
        "get_script" => crate::ai_script_changes::get(w, input),
        "propose_script_change" => {
            crate::ai_script_changes::propose_change(w, agent, &call.id, input)
        }
        "propose_script_delete" => {
            crate::ai_script_changes::propose_delete(w, agent, &call.id, input)
        }
        "get_task" => crate::ai_task_changes::get(w, input),
        "propose_task_change" => crate::ai_task_changes::propose_change(w, agent, &call.id, input),
        "propose_task_delete" => crate::ai_task_changes::propose_delete(w, agent, &call.id, input),
        n if agent_memory::is_cross_tool(n) => {
            agent_memory::run_cross(w, agent, &call.id, n, input)
        }
        other => Err(ToolOut::error(format!("there is no tool {other:?}"))),
    };
    let mut out = result.unwrap_or_else(|e| e);
    if matches!(
        call.name.as_str(),
        "search_notes" | "query_notes" | "backlinks"
    ) {
        let omitted = w.take_agent_omissions();
        if omitted > 0
            && let Ok(Value::Object(mut result)) = serde_json::from_str(&out.content)
        {
            result.insert(
                "notice".into(),
                json!("some notes may be held; the user can review and release them."),
            );
            out.content = Value::Object(result).to_string();
        }
    }
    out
}

/// `propose_edit`: check the edit with a dry-run batch and hand it to the user (ai D14).
fn propose(w: &Writer, author: &Author, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let base = str_arg(input, "base_version")?;
    let note = note_arg(input)?;
    let current = w.read(id).map_err(|e| ToolOut::error(e.to_string()))?;
    let before = doc_to_json(&current.doc);
    let op = Op::Update {
        id: id.to_owned(),
        input: Input::Json(note.clone()),
        base: Some(base.to_owned()),
    };
    let report = w
        .batch(vec![op], true, author)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    if let Some(e) = report.error {
        return Err(ToolOut::error(e.to_string()));
    }
    let item = match report.items.into_iter().next() {
        Some(Ok(item)) => item,
        Some(Err(writer::Error::Conflict {
            current_version, ..
        })) => {
            return Err(conflict(
                id,
                base,
                &current_version,
                "propose against that version",
            ));
        }
        Some(Err(e)) => return Err(item_error(e)),
        None => return Err(ToolOut::error("the edit was not checked")),
    };
    let after = item
        .wcl
        .as_deref()
        .and_then(|t| Input::Wcl(t.to_owned()).parse("note.wcl").ok())
        .map(|d| doc_to_json(&d))
        .unwrap_or(note);
    let ops = diff(&before, &after);
    let title = after["fields"]["title"].clone();
    let summary = json!({ "notes": [{ "id": id, "title": title }] });
    if ops.is_empty() {
        return Ok(ToolOut {
            content: json!({
                "proposed": false,
                "message": "The proposed note is the same as the current one: there is nothing to apply.",
            })
            .to_string(),
            is_error: false,
            summary,
            proposal: None,
        });
    }
    let proposal = json!({
        "id": call_id,
        "op": "update",
        "note_id": id,
        "title": title,
        "base_version": base,
        "before": before,
        "after": after,
        "ops": ops,
    });
    Ok(ToolOut {
        content: json!({
            "proposed": true,
            "note_id": id,
            "changes": ops.len(),
            "message": "The edit is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it.",
        })
        .to_string(),
        is_error: false,
        summary,
        proposal: Some(("proposal", proposal)),
    })
}

/// `propose_fields`: the current note with the given fields set (or removed, for `null`), checked
/// and proposed as [`propose`] does, so every other field and every block stays as it is (TV-34).
fn propose_fields(
    w: &Writer,
    author: &Author,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let base = str_arg(input, "base_version")?;
    let fields = match &input["fields"] {
        Value::String(s) => serde_json::from_str::<Value>(s)
            .map_err(|e| ToolOut::error(format!("fields is not JSON: {e}")))?,
        v => v.clone(),
    };
    let Value::Object(fields) = fields else {
        return Err(ToolOut::error(
            "fields must be a JSON object of field names and their new values",
        ));
    };
    if fields.is_empty() {
        return Err(ToolOut::error("fields names no field to set"));
    }
    let current = w
        .read(id)
        .map_err(|e| ToolOut::error(get_note_error(id, &e.to_string())))?;
    let mut note = doc_to_json(&current.doc);
    for (name, value) in fields {
        if value.is_null() {
            if let Some(map) = note["fields"].as_object_mut() {
                map.remove(&name);
            }
        } else {
            note["fields"][name] = value;
        }
    }
    propose(
        w,
        author,
        call_id,
        &json!({ "id": id, "base_version": base, "note": note }),
    )
}

/// A note that changed since the version the model based its change on.
fn conflict(id: &str, base: &str, current_version: &str, then: &str) -> ToolOut {
    ToolOut::error(format!(
        "conflict: note {id} changed since version {base}; its current version is \
         {current_version}. Nothing was written. Read it again with get_note and {then}."
    ))
}

/// The `note` argument: the whole note as JSON text, or as an object.
fn note_arg(input: &Value) -> Result<Value, ToolOut> {
    match &input["note"] {
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| ToolOut::error(format!("note is not JSON: {e}")))
        }
        v @ Value::Object(_) => Ok(v.clone()),
        _ => Err(ToolOut::error("note must be the whole note as JSON text")),
    }
}

/// A batch item's failure for the model: the message, and the API's error body with the
/// validation issues.
fn item_error(e: writer::Error) -> ToolOut {
    let message = e.to_string();
    let details = ApiError::from(e).body;
    let mut out = ToolOut::error(message.clone());
    out.content = json!({
        "error": message,
        "details": details,
        "message": "Nothing was proposed. Fix the problem and propose it again.",
    })
    .to_string();
    out
}

/// One operation as a dry-run batch: what it would do, or its error for the model.
fn dry_run(w: &Writer, author: &Author, op: Op) -> Result<writer::Item, ToolOut> {
    let report = w
        .batch(vec![op], true, author)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    if let Some(e) = report.error {
        return Err(item_error(e));
    }
    match report.items.into_iter().next() {
        Some(Ok(item)) => Ok(item),
        Some(Err(e)) => Err(item_error(e)),
        None => Err(ToolOut::error("the change was not checked")),
    }
}

/// A note proposal's answer: the `proposal` event, and `content` telling the model it awaits
/// the user.
fn proposed(content: Value, summary: Value, proposal: Value) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary,
        proposal: Some(("proposal", proposal)),
    }
}

/// `propose_create`: check a new note with a dry-run batch create and hand it to the user
/// (ai D20).
fn propose_create(
    w: &Writer,
    author: &Author,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let type_name = str_arg(input, "type")?;
    let note = note_arg(input)?;
    let op = Op::Create {
        type_name: type_name.to_owned(),
        input: Input::Json(note.clone()),
    };
    let item = dry_run(w, author, op)?;
    let after = item
        .wcl
        .as_deref()
        .and_then(|t| Input::Wcl(t.to_owned()).parse("note.wcl").ok())
        .map(|d| doc_to_json(&d))
        .unwrap_or(note);
    let title = after["fields"]["title"].clone();
    let proposal = json!({
        "id": call_id,
        "op": "create",
        "type": type_name,
        "title": title,
        "after": after,
    });
    Ok(proposed(
        json!({
            "proposed": true,
            "op": "create",
            "type": type_name,
            "message": "The new note is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who creates or discards it.",
        }),
        json!({ "notes": [] }),
        proposal,
    ))
}

/// `propose_archive`: check the note exists and is not already in that state, and hand the
/// archive or unarchive to the user (ai D20).
fn propose_archive(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let archived = input["archived"]
        .as_bool()
        .ok_or_else(|| ToolOut::error("archived must be true or false"))?;
    let note = w.read(id).map_err(|e| ToolOut::error(e.to_string()))?;
    if writer::is_archived(&note.doc) == archived {
        return Err(ToolOut::error(if archived {
            format!("note {id} is already archived: there is nothing to apply")
        } else {
            format!("note {id} is not archived: there is nothing to apply")
        }));
    }
    let title = doc_to_json(&note.doc)["fields"]["title"].clone();
    let op = if archived { "archive" } else { "unarchive" };
    let proposal = json!({
        "id": call_id,
        "op": op,
        "note_id": id,
        "title": title,
        "base_version": note.version,
    });
    Ok(proposed(
        json!({
            "proposed": true,
            "op": op,
            "note_id": id,
            "message": format!(
                "The {op} is shown to the user as a proposal. It has not been done: it awaits \
                 the user, who applies or discards it."
            ),
        }),
        json!({ "notes": [{ "id": id, "title": title }] }),
        proposal,
    ))
}

/// `propose_delete`: check the note can be deleted with a dry-run batch delete, and hand the
/// delete to the user (ai D20). Applied, it moves the note to the trash; nothing here purges.
fn propose_delete(
    w: &Writer,
    author: &Author,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let note = w.read(id).map_err(|e| ToolOut::error(e.to_string()))?;
    let op = Op::Delete {
        id: id.to_owned(),
        base: Some(note.version.clone()),
    };
    dry_run(w, author, op)?;
    let title = doc_to_json(&note.doc)["fields"]["title"].clone();
    let proposal = json!({
        "id": call_id,
        "op": "delete",
        "note_id": id,
        "title": title,
        "base_version": note.version,
    });
    Ok(proposed(
        json!({
            "proposed": true,
            "op": "delete",
            "note_id": id,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it. Applied, the note moves to the trash, where the user can restore it.",
        }),
        json!({ "notes": [{ "id": id, "title": title }] }),
        proposal,
    ))
}

/// A type tool's answer: `content` for the model, and the type names for the summary.
fn type_out(content: Value, types: Vec<Value>, type_proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [], "types": types }),
        proposal: type_proposal.map(|p| ("type_proposal", p)),
    }
}

/// `list_types`: every type's name, label, plural, description and whether it is built in.
fn list_types(w: &Writer) -> ToolOut {
    let schema = w.schema();
    let mut types: Vec<&schema::TypeDef> = schema.types().collect();
    types.sort_by(|a, b| a.name.cmp(&b.name));
    let rows: Vec<Value> = types
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "label": t.label,
                "plural": t.plural,
                "description": t.description,
                "builtin": w.is_built_in(&t.name),
            })
        })
        .collect();
    let names = types.iter().map(|t| json!(t.name)).collect();
    type_out(json!({ "types": rows }), names, None)
}

/// `get_type`: the type file as stored, and its fields and templates as the types API lists them.
fn get_type(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = w
        .read_type(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let schema = w.schema();
    let summary = schema
        .type_def(name)
        .map(|t| crate::types::type_json(t, w.is_built_in(name)))
        .unwrap_or(Value::Null);
    Ok(type_out(
        json!({ "name": name, "wcl": wcl, "summary": summary }),
        vec![json!(name)],
        None,
    ))
}

/// The vault format document, embedded at build time.
pub(crate) const VAULT_FORMAT: &str = include_str!("../../../docs/vault-format.md");

/// The text of `docs/vault-format.md` from the heading `from` up to the heading `to`, trimmed.
fn format_section(from: &str, to: &str) -> String {
    let start = VAULT_FORMAT.find(from).unwrap_or(VAULT_FORMAT.len());
    let end = VAULT_FORMAT[start..]
        .find(to)
        .map_or(VAULT_FORMAT.len(), |e| start + e);
    VAULT_FORMAT[start..end].trim().to_owned()
}

/// The sections of `docs/vault-format.md` a type file is written from: type files, field kinds
/// and handlers. Templates and the content blocks their tabs hold are `block_format_text`.
pub fn type_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        format!(
            "{}\n\n{}\n",
            format_section("\n## Type files\n", "\n## Templates\n"),
            format_section("\n## Handlers\n", "\n## Note files\n"),
        )
    })
}

/// The sections of `docs/vault-format.md` a template and a note's content are written from:
/// templates, content blocks and the inline markup of their text.
pub fn block_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        format!(
            "{}\n\n{}\n",
            format_section("\n## Templates\n", "\n## Handlers\n"),
            format_section("\n## Content blocks\n", "\n## Imported HTML\n"),
        )
    })
}

/// How the vault format's WCL maps onto the JSON form the note tools take: the start of
/// `value_format`, before the sections it quotes, which show WCL.
const NOTE_JSON: &str = "## A note as JSON\n\n\
The note tools take and give a note as `{ \"fields\": { ... }, \"blocks\": [ ... ] }`. A plain \
field is a member of `fields` with its value: text, a number, true or false, a list as an array, \
and a record (a date range, a location) as an object with every member, such as \
`{ \"start\": \"2026-11-01\", \"end\": \"2026-11-03\" }`. Leave an optional field with no value \
out. A compound field (content, dict, ink, canvas) is a block under `blocks`: \
`{ \"kind\": \"content\", \"labels\": [\"body\"], \"blocks\": [ ... ] }`, each inner block as \
`{ \"kind\": ..., \"fields\": { ... }, \"blocks\": [ ... ] }` (block_format lists the kinds). \
`created`, `updated` and `archived` are the server's: it ignores them in a write. A field the \
type marks `computed` (get_type's summary shows the flag) is written by its handlers only: \
leave it as it is. The sections below \
show each value in WCL, the vault's file format; the JSON value is the same.\n";

/// The sections of `docs/vault-format.md` a field value is written from: value encodings and
/// note files, after [`NOTE_JSON`].
pub fn value_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        format!(
            "{NOTE_JSON}\n{}\n\n{}\n",
            format_section("\n## Value encodings\n", "\n## Type files\n"),
            format_section("\n## Note files\n", "\n## Event series occurrences\n"),
        )
    })
}

/// `value_format`: how field values are written.
fn value_format() -> ToolOut {
    type_out(json!({ "format": value_format_text() }), Vec::new(), None)
}

/// The sections of `docs/vault-format.md` a query is written from: the query language and query
/// views, with their subsections.
pub fn query_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        format!(
            "{}\n",
            format_section("\n## Query language\n", "\n## Ink\n")
        )
    })
}

/// `query_format`: the query language.
fn query_format() -> ToolOut {
    type_out(json!({ "format": query_format_text() }), Vec::new(), None)
}

/// A `query_notes` error with a hint: the SQL habit the query shows, if any, and where the whole
/// grammar is. Most failed queries in production were SQL written as a query (TV-29).
fn query_error(query: &str, error: &str) -> String {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_ascii_uppercase)
        .collect();
    let has_word = |w: &str| words.iter().any(|x| x == w);
    let mut hints: Vec<&str> = Vec::new();
    if error.contains("unexpected character `=`") {
        hints.push("compare with `==`, not `=`");
    }
    if error.contains("unexpected character `'`") {
        hints.push("text goes in double quotes, as \"Todo\"");
    }
    if has_word("AND") || has_word("OR") || has_word("NOT") {
        hints.push("join conditions with `&&` and `||`, and negate with `!`");
    }
    if has_word("ORDER") {
        hints.push("order with `sort <field> asc|desc`");
    }
    if has_word("LIKE") {
        hints.push("match text with `contains(field, \"text\")`");
    }
    if has_word("NULL") {
        hints.push("test for no value with `empty(field)`");
    }
    if has_word("SELECT") && !query.trim_start().starts_with("from") {
        hints.push("start with `from <type>`; `select` comes near the end");
    }
    if error.contains("there is no type") {
        hints.push("list_types gives every type's name");
    }
    if error.contains("unknown field") {
        hints.push("get_type lists a type's fields");
    }
    let mut out = error.to_owned();
    if !hints.is_empty() {
        out.push_str(". The query language is not SQL: ");
        out.push_str(&hints.join("; "));
    }
    out.push_str(". Read query_format for the whole grammar and examples.");
    out
}

/// A `get_note` error that says how to find a real id when the note is not there: most failed
/// reads in production were ids the model made up or remembered wrongly (TV-29).
fn get_note_error(id: &str, error: &str) -> String {
    let looks_like_id = id.len() == 26 && id.chars().all(|c| c.is_ascii_alphanumeric());
    if !error.contains("does not exist") && looks_like_id {
        return error.to_owned();
    }
    let what = if looks_like_id {
        format!("There is no note {id}")
    } else {
        format!("{id:?} is not a note id, which is 26 letters and digits")
    };
    format!(
        "{what}. Do not guess ids: find the note with query_notes or search_notes and use the id \
         its row gives, exactly as given."
    )
}

/// `type_format`: the type-file vocabulary.
fn type_format() -> ToolOut {
    type_out(json!({ "format": type_format_text() }), Vec::new(), None)
}

/// `block_format`: the template and content block vocabulary.
fn block_format() -> ToolOut {
    type_out(json!({ "format": block_format_text() }), Vec::new(), None)
}

/// Handler bindings are code grants, not ordinary type edits. Recompute from the WCL rather
/// than trusting a flag in the proposal body.
fn tool_binding_changed(p: &Value) -> bool {
    if p["op"] == "delete" {
        return false;
    }
    let name = p["name"].as_str().unwrap_or_default();
    let Ok(after) = schema::ScriptTool::parse(name, p["after"].as_str().unwrap_or_default()) else {
        return true;
    };
    let Some(source) = p["before"].as_str() else {
        return !after.secrets.is_empty() || !after.allow_domains.is_empty();
    };
    let Ok(before) = schema::ScriptTool::parse(name, source) else {
        return true;
    };
    before.script != after.script
        || before.function != after.function
        || before.secrets != after.secrets
        || before.allow_domains != after.allow_domains
}

fn type_handlers_changed(p: &Value) -> bool {
    if p["op"] == "delete" {
        return false;
    }
    let name = p["name"].as_str().unwrap_or_default();
    let Ok(after) = schema::TypeDef::parse(name, p["after"].as_str().unwrap_or_default()) else {
        return true;
    };
    let before = match p["before"].as_str() {
        Some(source) => match schema::TypeDef::parse(name, source) {
            Ok(t) => t.handlers,
            Err(_) => return true,
        },
        None => Vec::new(),
    };
    before != after.handlers
}

fn type_handler_secrets(name: &str, before: Option<&str>, source: &str) -> Result<(), String> {
    let old = before
        .map(|s| schema::TypeDef::parse(name, s))
        .transpose()
        .map_err(|e| e.to_string())?
        .map(|t| t.handlers)
        .unwrap_or_default();
    let ty = schema::TypeDef::parse(name, source).map_err(|e| e.to_string())?;
    for h in ty.handlers {
        if old.contains(&h) {
            continue;
        }
        if let Some(why) = state::proposed_secrets_refused(&h.secrets) {
            return Err(why);
        }
    }
    Ok(())
}

/// `propose_type_change`: check the type file as the type write would, without writing it, and
/// hand it to the user (ai D18). Invalid WCL goes back to the model with the issues.
fn propose_type(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = input["wcl"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolOut::error("wcl must be the whole type file as WCL text"))?;
    let before = w.read_type(name).ok();
    let check = w.check_type(name, wcl).map_err(|e| {
        let message = e.to_string();
        let details = ApiError::from(e).body;
        let mut out = ToolOut::error(message.clone());
        out.content = json!({
            "error": message,
            "issues": details,
            "message": "The type was not proposed. Fix the issues and propose it again.",
        })
        .to_string();
        out
    })?;
    type_handler_secrets(name, before.as_deref(), &check.wcl).map_err(ToolOut::error)?;
    let issues: Vec<Value> = check
        .warnings
        .iter()
        .map(crate::error::issue_json)
        .collect();
    let names = vec![json!(name)];
    if before.as_deref() == Some(check.wcl.as_str()) {
        return Ok(type_out(
            json!({
                "proposed": false,
                "message": "The proposed type file is the same as the current one: there is nothing to apply.",
            }),
            names,
            None,
        ));
    }
    let mut content = json!({
        "proposed": true,
        "name": name,
        "exists": check.exists,
        "warnings": issues,
        "message": if check.exists {
            "The type change is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it."
        } else {
            "The new type is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who applies or discards it."
        },
    });
    if let Some(plan) = &check.plan {
        content["plan"] = json!({
            "added": plan.added,
            "removed": plan.removed,
            "notes_edited": plan.changes.len(),
            "notes_flagged": plan.flagged.len(),
        });
    }
    let proposal = json!({
        "id": call_id,
        "name": name,
        "exists": check.exists,
        "before": before,
        "after": check.wcl,
        "issues": issues,
    });
    Ok(type_out(content, names, Some(proposal)))
}

/// `propose_type_delete`: check the type can be deleted, without deleting it, and hand the
/// delete to the user.
fn propose_type_delete(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let before = w
        .read_type(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    w.check_delete_type(name).map_err(|e| {
        let message = e.to_string();
        let details = ApiError::from(e).body;
        let mut out = ToolOut::error(message.clone());
        out.content = json!({
            "error": message,
            "issues": details,
            "message": "The delete was not proposed.",
        })
        .to_string();
        out
    })?;
    let proposal = json!({
        "id": call_id,
        "kind": "type",
        "op": "delete",
        "name": name,
        "exists": true,
        "before": before,
        "after": null,
        "issues": [],
    });
    Ok(type_out(
        json!({
            "proposed": true,
            "name": name,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.",
        }),
        vec![json!(name)],
        Some(proposal),
    ))
}

/// What a proposal changes: each field whose value differs, then each top-level block (matched
/// by kind and labels) added, removed or changed. Server-set fields are left out.
pub fn diff(before: &Value, after: &Value) -> Vec<Value> {
    let mut ops = Vec::new();
    let empty = serde_json::Map::new();
    let bf = before["fields"].as_object().unwrap_or(&empty);
    let af = after["fields"].as_object().unwrap_or(&empty);
    let mut names: Vec<&String> = bf.keys().chain(af.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        if SERVER_FIELDS.contains(&name.as_str()) {
            continue;
        }
        let (b, a) = (bf.get(name), af.get(name));
        if b != a {
            ops.push(json!({
                "op": "field", "name": name,
                "before": b.cloned().unwrap_or(Value::Null),
                "after": a.cloned().unwrap_or(Value::Null),
            }));
        }
    }
    let key = |b: &Value| {
        let labels: Vec<&str> = b["labels"]
            .as_array()
            .map(|l| l.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let kind = b["kind"].as_str().unwrap_or_default();
        if labels.is_empty() {
            kind.to_owned()
        } else {
            format!("{kind} {}", labels.join(" "))
        }
    };
    let no_blocks = Vec::new();
    let bb = before["blocks"].as_array().unwrap_or(&no_blocks);
    let ab = after["blocks"].as_array().unwrap_or(&no_blocks);
    let mut keys: Vec<String> = Vec::new();
    for b in bb.iter().chain(ab.iter()) {
        let k = key(b);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    for k in keys {
        let b: Vec<&Value> = bb.iter().filter(|x| key(x) == k).collect();
        let a: Vec<&Value> = ab.iter().filter(|x| key(x) == k).collect();
        if b != a {
            ops.push(json!({
                "op": "block", "name": k,
                "before": if b.is_empty() { Value::Null } else if b.len() == 1 { b[0].clone() } else { json!(b) },
                "after": if a.is_empty() { Value::Null } else if a.len() == 1 { a[0].clone() } else { json!(a) },
            }));
        }
    }
    ops
}

/// `GET /api/ai/models` and the proposal diff are tested through the server; the pure helpers
/// here.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_scope_is_capped_by_caller_and_agent() {
        let agent = schema::Agent::parse("a", "label = \"A\"\n").unwrap();
        for scope in [state::Scope::Read, state::Scope::Write, state::Scope::Admin] {
            let direct = super::change_scope(scope, &agent);
            assert!(scope.allows(direct));
            assert!(super::unattended_scope(&agent).allows(direct));
        }
        assert_eq!(
            super::change_scope(state::Scope::Write, &agent),
            state::Scope::Write
        );
        assert_eq!(
            super::change_scope(state::Scope::Read, &agent),
            state::Scope::Read
        );
        let reader =
            Agent::parse("reader", "permissions {\n  tools = [\"search_notes\"]\n}\n").unwrap();
        assert_eq!(unattended_scope(&reader), Scope::Write);
        assert_eq!(change_scope(Scope::Admin, &reader), Scope::Write);
    }

    #[test]
    fn cut_marks_what_it_leaves_out() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abcdef", 4), "abcd\n[truncated: 2 more characters]");
    }

    #[test]
    fn diff_lists_changed_fields_and_blocks_but_not_server_fields() {
        let before = json!({
            "fields": { "title": "A", "updated": "x", "tags": ["a"] },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [] }],
        });
        let after = json!({
            "fields": { "title": "B", "updated": "y", "tags": ["a"] },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [{ "kind": "paragraph" }] }],
        });
        let ops = diff(&before, &after);
        assert_eq!(ops.len(), 2, "{ops:?}");
        assert_eq!(ops[0]["name"], "title");
        assert_eq!(ops[1]["name"], "content body");
    }

    #[test]
    fn type_format_holds_the_type_file_sections() {
        let t = type_format_text();
        assert!(t.starts_with("## Type files"), "{}", &t[..40]);
        assert!(t.contains("## Field kinds"));
        assert!(t.contains("## Handlers"));
        assert!(!t.contains("## Templates"));
        assert!(!t.contains("## Note files"));
        assert!(t.len() < RESULT_LIMIT * 8 / 10, "{} bytes", t.len());
    }

    #[test]
    fn block_format_holds_the_template_and_content_block_sections() {
        let t = block_format_text();
        assert!(t.starts_with("## Templates"), "{}", &t[..40]);
        assert!(t.contains("## Content blocks"));
        assert!(t.contains("## Inline markup"));
        assert!(!t.contains("## Handlers"));
        assert!(!t.contains("## Imported HTML"));
        assert!(t.len() < RESULT_LIMIT * 8 / 10, "{} bytes", t.len());
    }

    #[test]
    fn value_format_holds_the_json_form_value_encodings_and_note_files() {
        let t = value_format_text();
        assert!(t.starts_with("## A note as JSON"), "{}", &t[..40]);
        assert!(t.contains("## Value encodings"));
        assert!(t.contains("| Date range |"));
        assert!(t.contains("## Note files"));
        assert!(t.contains("computed"));
        assert!(!t.contains("## Type files"));
        assert!(!t.contains("## Event series occurrences"));
        assert!(t.len() < RESULT_LIMIT * 8 / 10, "{} bytes", t.len());
    }

    #[test]
    fn only_a_trigger_run_drops_the_changes_that_would_only_be_proposed() {
        // A chat keeps them, whatever its thread's origin.
        for origin in ["user", "mention", "trigger:nightly"] {
            assert_eq!(Held::of(false, origin), Held::Propose, "{origin}");
        }
        // Unattended: a call from a note and a dispatch keep them (TV-77 D2), a trigger's run
        // and any origin not known here drop them.
        assert_eq!(Held::of(true, "mention"), Held::Propose);
        assert_eq!(Held::of(true, "dispatch"), Held::Propose);
        for origin in ["trigger:nightly", "trigger:mention", "subagent:01JB", ""] {
            assert_eq!(Held::of(true, origin), Held::Drop, "{origin}");
        }
    }

    #[test]
    fn an_unattended_run_names_the_switches_that_hold_its_changes_back() {
        let agent = |wcl: &str| Agent::parse("a", wcl).unwrap();
        let note = |a: &Agent| withheld_note(a, Direct::of(confirm_of(a), unattended_scope(a)));
        // Every switch on, every tool: everything is held back, types included.
        let text = note(&agent("label = \"A\"\n")).unwrap();
        assert!(text.contains("## This run is unattended"), "{text}");
        assert!(
            text.contains(
                "create or edit notes: your \"Ask before creating and editing notes\" switch is on"
            ),
            "{text}"
        );
        assert!(text.contains("change or delete types"), "{text}");
        assert!(text.contains("do not say you have no tool"), "{text}");
        // Notes allowed without asking, and a tools list without type tools: neither is listed.
        let text = note(&agent(
            "label = \"A\"\n\npermissions {\n  tools = [\"get_note\", \"propose_fields\", \"propose_archive\"]\n  confirm {\n    notes = false\n  }\n}\n",
        ))
        .unwrap();
        assert!(!text.contains("edit notes"), "{text}");
        assert!(!text.contains("types"), "{text}");
        assert!(text.contains("archive notes"), "{text}");
        // Nothing held back: no section.
        assert!(note(&agent(
            "label = \"A\"\n\npermissions {\n  tools = [\"get_note\", \"propose_edit\"]\n  confirm {\n    notes = false\n  }\n}\n",
        ))
        .is_none());
    }

    #[test]
    fn query_format_holds_the_query_language_and_query_views() {
        let t = query_format_text();
        assert!(t.starts_with("## Query language"), "{}", &t[..40]);
        assert!(t.contains("### It is not SQL"));
        assert!(t.contains("## Query views"));
        assert!(t.contains("### Archived notes in queries"));
        assert!(!t.contains("## Ink"));
        assert!(t.len() < RESULT_LIMIT * 8 / 10, "{} bytes", t.len());
    }

    #[test]
    fn query_format_names_every_function_the_parser_takes() {
        let t = query_format_text();
        for (name, _) in query::FUNCTIONS {
            assert!(
                t.contains(&format!("`{name}(")),
                "{name} is not in the functions table"
            );
        }
    }

    #[test]
    fn every_query_example_in_query_format_parses() {
        let examples = fenced(query_format_text(), "query");
        assert!(examples.len() >= 8, "{} examples", examples.len());
        for q in examples {
            query::parse(&q).unwrap_or_else(|e| panic!("{q}: {e}"));
        }
    }

    /// The bodies of the code blocks fenced as ```` ```<lang> ```` in `text`.
    fn fenced(text: &str, lang: &str) -> Vec<String> {
        let open = format!("```{lang}\n");
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find(&open) {
            let body = &rest[start + open.len()..];
            let end = body.find("```").unwrap_or(body.len());
            out.push(body[..end].trim().to_owned());
            rest = &body[end..];
        }
        out
    }

    #[test]
    fn a_query_error_names_the_sql_habit_and_points_at_query_format() {
        let e = query_error(
            "from task where status = 'Todo' AND due IS NULL ORDER BY due",
            "query syntax error at line 1, column 24: unexpected character `=`",
        );
        assert!(
            e.starts_with("query syntax error at line 1, column 24"),
            "{e}"
        );
        for hint in [
            "`==`",
            "`&&`",
            "`sort <field>",
            "`empty(field)`",
            "query_format",
        ] {
            assert!(e.contains(hint), "{hint} missing: {e}");
        }
        let e = query_error("from tasks", "there is no type \"tasks\"");
        assert!(e.contains("list_types"), "{e}");
        let e = query_error(
            "from task where",
            "query syntax error at line 1, column 16: expected an expression, found the end",
        );
        assert!(!e.contains("not SQL"), "{e}");
        assert!(
            e.ends_with("Read query_format for the whole grammar and examples."),
            "{e}"
        );
    }

    #[test]
    fn a_missing_note_says_to_find_ids_rather_than_guess() {
        let e = get_note_error(
            "01J00000000000000000000000",
            "note 01J00000000000000000000000 does not exist",
        );
        assert!(
            e.starts_with("There is no note 01J00000000000000000000000."),
            "{e}"
        );
        assert!(e.contains("query_notes or search_notes"), "{e}");
        let e = get_note_error("task-12", "bad id");
        assert!(e.starts_with("\"task-12\" is not a note id"), "{e}");
        let other = "you may not read notes of type secret";
        assert_eq!(get_note_error("01J00000000000000000000000", other), other);
    }

    #[test]
    fn type_and_block_format_cover_type_files_to_inline_markup() {
        let (types, blocks) = (type_format_text(), block_format_text());
        for (from, to) in [
            ("\n## Type files\n", "\n## Note files\n"),
            ("\n## Content blocks\n", "\n## Imported HTML\n"),
        ] {
            // Each `## ` section of the old type_format text is whole in one of the two.
            for section in format_section(from, to).split("\n## ") {
                let section = section.trim().trim_start_matches("## ");
                assert!(
                    types.contains(section) || blocks.contains(section),
                    "{} is in neither",
                    section.lines().next().unwrap_or_default()
                );
            }
        }
    }

    #[test]
    fn every_propose_tool_has_a_switch_or_always_proposes() {
        // A propose tool with neither would be offered to a trigger's run, where its proposal
        // waits with no one to see it (TV-77 F11, TV-81 review F1).
        let names: Vec<String> = tool_specs()
            .into_iter()
            .chain(crate::ai_browser::tool_specs())
            .map(|t| t.name)
            .filter(|n| n.starts_with("propose_"))
            .collect();
        assert!(names.len() > 20, "{names:?}");
        for n in &names {
            assert!(
                change_switch(n).is_some() || ALWAYS_PROPOSE.contains(&n.as_str()),
                "{n} has no confirmation switch and is not in ALWAYS_PROPOSE"
            );
        }
        // Each always-propose tool is offered, has no switch to turn it off, and its event is
        // never made at once, whatever the switches say.
        for n in ALWAYS_PROPOSE {
            assert!(names.iter().any(|m| m == n), "{n} is not a tool");
            assert_eq!(change_switch(n), None, "{n}");
        }
        let all_off = Agent::parse(
            "a",
            "label = \"A\"\n\npermissions {\n  confirm {\n    notes = false\n    archive = false\n    delete = false\n    types = false\n    downloads = false\n    taxonomies = false\n    agents = false\n    skills = false\n    tools = false\n  }\n}\n",
        )
        .unwrap();
        let direct = Direct::of(confirm_of(&all_off), Scope::Admin);
        assert!(!direct.covers("tool_proposal", &json!({})));
        assert!(direct.covers("tool_proposal", &json!({"name": "safe", "after": "description = \"Safe\"\nscript = \"safe\"\nfunction = \"run\"\n"})));
        assert!(!direct.covers("tool_proposal", &json!({ "script_after": "fn run() {}" })));
        assert!(!direct.covers("script_proposal", &json!({ "op": "change" })));
        assert!(!direct.covers("device_file_proposal", &json!({})));
        assert!(!direct.covers("script_proposal", &json!({ "op": "delete" })));
        assert!(!direct.covers("task_proposal", &json!({ "op": "create" })));
        assert!(!direct.covers("task_proposal", &json!({ "op": "delete" })));
        // The script and the task tools are the whole list (TV-70, TV-71).
        let mut expected: Vec<&str> = crate::ai_script_changes::PROPOSE_TOOLS
            .iter()
            .chain(crate::ai_task_changes::PROPOSE_TOOLS.iter())
            .copied()
            .collect();
        expected.push("propose_device_file_change");
        expected.sort_unstable();
        let mut listed = ALWAYS_PROPOSE.to_vec();
        listed.sort_unstable();
        assert_eq!(listed, expected);
    }

    #[test]
    fn a_trigger_run_leaves_out_the_task_proposals_and_an_agent_with_them_runs_as_admin() {
        let agent = Agent::parse("a", "label = \"A\"\n").unwrap();
        let direct = Direct::of(confirm_of(&agent), unattended_scope(&agent));
        let names = |drop: Option<Direct>| -> Vec<String> {
            turn_tools(ai::Web::default(), false, None, drop, false, false, false)
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        let chat = names(None);
        let trigger = names(Some(direct));
        assert!(chat.iter().any(|n| n == "propose_device_file_change"));
        assert!(!trigger.iter().any(|n| n == "propose_device_file_change"));
        for n in crate::ai_task_changes::PROPOSE_TOOLS {
            assert!(chat.iter().any(|m| m == n), "{n} in a chat");
            assert!(
                !trigger.iter().any(|m| m == n),
                "{n} not in a trigger's run"
            );
        }
        assert!(trigger.iter().any(|m| m == "get_task"));
        let text = withheld_note(&agent, direct).unwrap();
        assert!(
            text.contains("- propose_task_change: its change always waits for the user's Apply"),
            "{text}"
        );
        let tasks_only = Agent::parse(
            "t",
            "label = \"T\"\n\npermissions {\n  tools = [\"get_task\", \"propose_task_delete\"]\n}\n",
        )
        .unwrap();
        assert_eq!(unattended_scope(&tasks_only), Scope::Admin);
    }

    #[test]
    fn a_trigger_run_leaves_out_the_script_proposals_and_names_them() {
        let agent = Agent::parse("a", "label = \"A\"\n").unwrap();
        let direct = Direct::of(confirm_of(&agent), unattended_scope(&agent));
        let names = |drop: Option<Direct>| -> Vec<String> {
            turn_tools(ai::Web::default(), false, None, drop, false, false, false)
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        let chat = names(None);
        let trigger = names(Some(direct));
        for n in crate::ai_script_changes::PROPOSE_TOOLS {
            assert!(chat.iter().any(|m| m == n), "{n} in a chat");
            assert!(
                !trigger.iter().any(|m| m == n),
                "{n} not in a trigger's run"
            );
        }
        assert!(trigger.iter().any(|m| m == "get_script"));
        let text = withheld_note(&agent, direct).unwrap();
        assert!(
            text.contains("- propose_script_change: its change always waits for the user's Apply"),
            "{text}"
        );
        // An agent that may only propose scripts runs unattended with admin scope (TV-32 R7).
        let scripts_only = Agent::parse(
            "s",
            "label = \"S\"\n\npermissions {\n  tools = [\"get_script\", \"propose_script_change\"]\n}\n",
        )
        .unwrap();
        assert_eq!(unattended_scope(&scripts_only), Scope::Admin);
    }

    #[test]
    fn tools_are_strict_objects() {
        for t in tool_specs() {
            let s = &t.input_schema;
            assert_eq!(s["additionalProperties"], false);
            let props = s["properties"].as_object().unwrap().len();
            assert_eq!(s["required"].as_array().unwrap().len(), props, "{}", t.name);
        }
    }
}
