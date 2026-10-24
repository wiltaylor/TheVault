//! What an AI chat turn records in the activity log (activity Y2): `ai.turn.start` and
//! `ai.turn.end` (agent, provider, model, thread, origin; at the end the tokens, time and
//! outcome), `ai.tool` for each tool call with a short summary of its arguments, `ai.browser` for
//! each browser command with its exit code, and `ai.proposal` and `ai.applied` for the changes
//! the turn proposed or made. Model message text and the notes a tool writes are never copied:
//! only names, ids, titles, query text, hosts and the first characters of a command.
//!
//! Every entry of one turn carries `detail.turn`, so the Log page can nest a turn's tool calls
//! under it, and `detail.agent` and `detail.thread` when there are any.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::{Map, Value, json};
use state::{Author, NewActivity, one_line};
use writer::Writer;

/// The longest argument text an entry repeats, in characters.
const ARG_CHARS: usize = 120;

/// The most notes an entry lists from a tool's summary.
const NOTES_LISTED: usize = 5;

/// The tool arguments that are safe to show: names, ids, query text and the like. Anything else,
/// such as a note's fields or a memory's text, is left out.
const SHOWN_ARGS: [&str; 14] = [
    "query", "q", "id", "type", "name", "agent", "title", "url", "command", "path", "filename",
    "archived", "op", "limit",
];

/// One running turn's entries.
pub(crate) struct TurnActivity {
    writer: Arc<Writer>,
    author: Author,
    agent: String,
    thread: Option<String>,
    origin: String,
    turn: String,
    began: Instant,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    provider: Option<String>,
    model: Option<String>,
    input: u64,
    output: u64,
    calls: u64,
    tools: u64,
    /// The tools called and not yet answered, by call id: their name and argument summary.
    pending: HashMap<String, (String, Value)>,
    ended: bool,
}

/// `url` without its query and fragment, which may carry keys.
fn bare_url(url: &str) -> String {
    let cut = url.split(['?', '#']).next().unwrap_or_default();
    one_line(cut, ARG_CHARS)
}

/// Whether tool `name`'s `text` argument is search text, safe to show, rather than something it
/// writes, such as a memory's text.
fn text_is_search(name: &str) -> bool {
    (name.starts_with("search_") || name.starts_with("list_") || name == "web_search")
        && !name.contains("memor")
}

/// The arguments of tool `name`'s call `input` that are safe to show.
fn shown_args(name: &str, input: &Value) -> Value {
    let mut out = Map::new();
    let Some(obj) = input.as_object() else {
        return Value::Object(out);
    };
    if text_is_search(name)
        && let Some(Value::String(t)) = obj.get("text")
    {
        out.insert("text".into(), Value::String(one_line(t, ARG_CHARS)));
    }
    for key in SHOWN_ARGS {
        let Some(v) = obj.get(key) else { continue };
        let v = match (key, v) {
            ("url", Value::String(s)) => Value::String(bare_url(s)),
            (_, Value::String(s)) => Value::String(one_line(s, ARG_CHARS)),
            (_, Value::Number(_) | Value::Bool(_)) => v.clone(),
            _ => continue,
        };
        out.insert(key.to_owned(), v);
    }
    if let Some(ids) = obj.get("ids").and_then(Value::as_array) {
        out.insert("ids".into(), json!(ids.len()));
    }
    Value::Object(out)
}

/// The arguments as one short line, such as `"plans" type=task`.
fn args_line(args: &Value) -> String {
    let Some(obj) = args.as_object() else {
        return String::new();
    };
    let mut parts = Vec::new();
    // Search text first, then the rest in the order SHOWN_ARGS gives them.
    let order = ["text"].into_iter().chain(SHOWN_ARGS).chain(["ids"]);
    for (k, v) in order.filter_map(|k| obj.get_key_value(k)) {
        let text = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        parts.push(match k.as_str() {
            "query" | "q" | "text" => format!("{text:?}"),
            _ => format!("{k}={text}"),
        });
    }
    one_line(&parts.join(" "), 200)
}

/// The notes a tool's summary names, `{id, title}` each.
fn summary_notes(summary: &Value) -> Vec<Value> {
    summary["notes"]
        .as_array()
        .map(|notes| {
            notes
                .iter()
                .take(NOTES_LISTED)
                .map(|n| {
                    json!({
                        "id": n["id"],
                        "title": n["title"].as_str().map(|t| one_line(t, ARG_CHARS)),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a proposal or a change the turn made is about, as one line.
fn change_line(event: &str, p: &Value) -> String {
    let op = p["op"].as_str().unwrap_or("update");
    let title = p["title"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .map(|t| format!(" \"{}\"", one_line(t, ARG_CHARS)));
    let name = p["name"].as_str().map(|n| format!(" {n}"));
    let what = match event {
        "type_proposal" => format!("type{}", name.unwrap_or_default()),
        "taxonomy_proposal" => format!("taxonomy{}", name.unwrap_or_default()),
        "agent_proposal" => format!("agent{}", name.unwrap_or_default()),
        "agent_memory_proposal" => format!("memory{}", title.unwrap_or_default()),
        _ => match p["type"].as_str() {
            Some(t) => format!("{t}{}", title.unwrap_or_default()),
            None => format!("note{}", title.unwrap_or_default()),
        },
    };
    format!("{op} {what}")
}

impl TurnActivity {
    /// A turn of `agent` by `author`, on `thread` when it has one, started from `origin`: `user`,
    /// `trigger:<name>` or `mention`.
    pub(crate) fn new(
        writer: Arc<Writer>,
        author: Author,
        agent: &str,
        thread: Option<String>,
        origin: &str,
    ) -> TurnActivity {
        TurnActivity {
            writer,
            author,
            agent: agent.to_owned(),
            thread,
            origin: origin.to_owned(),
            turn: state::random_hex(8),
            began: Instant::now(),
            inner: Mutex::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// An entry of this turn, linked to its thread, with the turn's ids in its detail.
    fn entry(&self, kind: &str, summary: String, mut detail: Value) -> NewActivity {
        detail["turn"] = json!(self.turn);
        detail["agent"] = json!(self.agent);
        detail["origin"] = json!(self.origin);
        if let Some(t) = &self.thread {
            detail["thread"] = json!(t);
        }
        let mut e = NewActivity::new(kind, "ai", &self.author, summary).detail(detail);
        if let Some(t) = &self.thread {
            e = e.subject("thread", t);
        }
        e
    }

    fn record(&self, e: NewActivity) {
        self.writer.state().record_activity(e);
    }

    /// The turn began.
    pub(crate) fn start(&self) {
        let who = if self.origin == "user" {
            String::new()
        } else {
            format!(" ({})", self.origin)
        };
        self.record(self.entry(
            "ai.turn.start",
            format!("{} started a turn{who}", self.agent),
            json!({}),
        ));
    }

    /// One model call ended, with its provider, model and tokens.
    pub(crate) fn called(&self, provider: &str, model: &str, input: u64, output: u64) {
        let mut i = self.lock();
        i.provider = Some(provider.to_owned());
        i.model = Some(model.to_owned());
        i.input += input;
        i.output += output;
        i.calls += 1;
    }

    /// The model called tool `name` with `input`; recorded once it is answered.
    pub(crate) fn tool_called(&self, id: &str, name: &str, input: &Value) {
        self.lock()
            .pending
            .insert(id.to_owned(), (name.to_owned(), shown_args(name, input)));
    }

    /// Tool call `id` was answered: an `ai.tool` entry, or `ai.browser` for a browser command.
    pub(crate) fn tool_answered(&self, id: &str, summary: &Value, is_error: bool) {
        let Some((name, args)) = ({
            let mut i = self.lock();
            i.tools += 1;
            i.pending.remove(id)
        }) else {
            return;
        };
        let mut detail = json!({ "tool": name, "args": args, "is_error": is_error });
        let notes = summary_notes(summary);
        if !notes.is_empty() {
            detail["notes"] = json!(notes);
        }
        if is_error && let Some(m) = summary["message"].as_str() {
            detail["error"] = json!(one_line(m, 200));
        }
        let mut line = format!("{name} {}", args_line(&args));
        let kind = if let Some(b) = summary.get("browser").filter(|b| b.is_object()) {
            detail["exit"] = b["exit"].clone();
            detail["ms"] = b["ms"].clone();
            detail["timed_out"] = b["timed_out"].clone();
            if let Some(exit) = b["exit"].as_i64() {
                line.push_str(&format!(" exit={exit}"));
            }
            "ai.browser"
        } else if crate::ai_browser::is_browser_tool(&name) {
            "ai.browser"
        } else {
            "ai.tool"
        };
        if is_error {
            line.push_str(" (failed)");
        }
        self.record(self.entry(kind, line.trim().to_owned(), detail));
    }

    /// The turn sent `event`: a proposal for the user, or a change it made itself.
    pub(crate) fn change(&self, event: &str, p: &Value) {
        let applied = event == "applied";
        let what = change_line(if applied { "proposal" } else { event }, p);
        let (kind, verb) = if applied {
            ("ai.applied", "Made")
        } else {
            ("ai.proposal", "Proposed")
        };
        let detail = json!({
            "event": event,
            "proposal": p["id"],
            "op": p["op"],
            "note": p["note_id"],
            "type": p["type"],
            "name": p["name"],
        });
        self.record(self.entry(kind, format!("{verb}: {what}"), detail));
    }

    /// The turn ended with `outcome` (`done`, `refused`, `error` or `stopped`), and `why` for a
    /// refusal's category or an error's kind.
    pub(crate) fn end(&self, outcome: &str, why: Option<&str>) {
        let (provider, model, input, output, calls, tools) = {
            let mut i = self.lock();
            if i.ended {
                return;
            }
            i.ended = true;
            (
                i.provider.clone(),
                i.model.clone(),
                i.input,
                i.output,
                i.calls,
                i.tools,
            )
        };
        let ms = self.began.elapsed().as_millis() as u64;
        let mut line = format!(
            "{} ended a turn: {outcome}, {input} in / {output} out tokens, {tools} tool calls",
            self.agent
        );
        if let Some(w) = why {
            line.push_str(&format!(" ({})", one_line(w, 80)));
        }
        self.record(self.entry(
            "ai.turn.end",
            line,
            json!({
                "outcome": outcome,
                "why": why.map(|w| one_line(w, 200)),
                "provider": provider,
                "model": model,
                "input_tokens": input,
                "output_tokens": output,
                "calls": calls,
                "tools": tools,
                "ms": ms,
            }),
        ));
    }
}

/// Record that `author` applied, or failed to apply, the proposal `change` of kind `event`
/// through `POST /api/ai/apply`.
pub(crate) fn applied(
    w: &Writer,
    author: &Author,
    event: &str,
    change: &Value,
    error: Option<&str>,
) {
    let what = change_line(event, change);
    let (kind, line) = match error {
        None => ("ai.applied", format!("Applied: {what}")),
        Some(_) => ("ai.applied", format!("Could not apply: {what}")),
    };
    let mut detail = json!({
        "event": event,
        "proposal": change["id"],
        "op": change["op"],
        "note": change["note_id"],
        "type": change["type"],
        "name": change["name"],
        "agent": change["agent"],
        "ok": error.is_none(),
    });
    if let Some(e) = error {
        detail["error"] = json!(one_line(e, 200));
    }
    let mut e = NewActivity::new(kind, "ai", author, line).detail(detail);
    if let Some(t) = change["thread"].as_str() {
        e = e.subject("thread", t);
    }
    w.state().record_activity(e);
}

/// Record that `author` discarded proposal `id` of `thread`.
pub(crate) fn discarded(w: &Writer, author: &Author, thread: &str, id: &str, agent: &str) {
    w.state().record_activity(
        NewActivity::new("ai.discarded", "ai", author, "Discarded a proposal")
            .subject("thread", thread)
            .detail(json!({ "proposal": id, "thread": thread, "agent": agent })),
    );
}
