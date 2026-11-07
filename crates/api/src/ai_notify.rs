//! The chat's `notify_user` tool (TV-32 S1, decision N1): an agent raises a notification under
//! the bell at once, with no proposal, so it also works in unattended runs, where no one could
//! apply one.
//!
//! The notification's source is `agent:<name>`, so the bell names the agent. It may open one
//! note, which must be a live note of a type the agent may read; an agent gives no other link.
//! The title and body are checked as every notification is (`state::MAX_TITLE`,
//! `state::MAX_BODY`).
//!
//! Each notification is also pushed to the paired phones, so an agent may send at most
//! [`MAX_PER_TURN`] in one turn, [`MAX_PER_HOUR`] in an hour and [`MAX_PER_DAY`] in a day. All
//! agents together may send at most [`MAX_PER_DAY_ALL_AGENTS`] in a day (TV-75). The turn count
//! is shared by the turn and the sub agent turns it starts ([`NotifyCount`]). The hour and day
//! counts are checked in the same transaction that stores the notification
//! (`State::add_notification_within`), so concurrent calls cannot pass them together. A refused
//! call tells the model which limit it reached and writes nothing. The tool needs a caller with
//! `write`, as `POST /api/notifications` does.

use std::sync::{Mutex, MutexGuard};

use ai::ToolSpec;
use schema::Agent;
use serde_json::{Value, json};
use state::{NewNotification, Scope, SourceLimit, SourceMatch};
use writer::Writer;

use crate::ai_chat::{Limits, ToolOut, tool};

/// The tool's name.
pub const TOOL: &str = "notify_user";

/// The most notifications an agent sends in one turn.
pub const MAX_PER_TURN: usize = 3;

/// The most notifications an agent sends in an hour.
pub const MAX_PER_HOUR: usize = 10;

/// The most notifications an agent sends in a day.
pub const MAX_PER_DAY: usize = 30;

/// The most notifications all agents together send in a day, on top of [`MAX_PER_DAY`] for each.
pub const MAX_PER_DAY_ALL_AGENTS: usize = 60;

/// The source prefix of every agent's notifications (`agent:<name>`).
const AGENT_SOURCES: &str = "agent:";

const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// The hour and day limits of one agent, counted over the notifications stored with the agent's
/// source, then the day limit of all agents together.
const LIMITS: [SourceLimit; 3] = [
    SourceLimit {
        window_ms: HOUR_MS,
        max: MAX_PER_HOUR,
        sources: SourceMatch::Own,
    },
    SourceLimit {
        window_ms: DAY_MS,
        max: MAX_PER_DAY,
        sources: SourceMatch::Own,
    },
    SourceLimit {
        window_ms: DAY_MS,
        max: MAX_PER_DAY_ALL_AGENTS,
        sources: SourceMatch::Prefix(AGENT_SOURCES),
    },
];

/// The notifications one turn has sent. A turn and each sub agent turn it starts share one, so
/// the turn and its sub agents together send at most [`MAX_PER_TURN`] (TV-74).
#[derive(Debug, Default)]
pub(crate) struct NotifyCount(Mutex<usize>);

impl NotifyCount {
    fn count(&self) -> MutexGuard<'_, usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Take one of the turn's places, or `false` when it has none left.
    fn take(&self) -> bool {
        let mut sent = self.count();
        if *sent >= MAX_PER_TURN {
            return false;
        }
        *sent += 1;
        true
    }

    /// Give back a place taken for a call that stored nothing.
    fn give_back(&self) {
        *self.count() -= 1;
    }
}

/// The tool's spec.
pub(crate) fn tool_spec() -> ToolSpec {
    tool(
        TOOL,
        &format!(
            "Send the user a notification now: it shows under the bell in the app, from this \
             agent, and is pushed to the user's paired phones. Use it for something the user \
             should know soon, such as the result of an unattended run that needs them; not for \
             an answer in a chat the user is reading. It is sent at once, without asking. At \
             most {MAX_PER_TURN} per turn, {MAX_PER_HOUR} per hour and {MAX_PER_DAY} per day, \
             and all agents together {MAX_PER_DAY_ALL_AGENTS} per day; a call over a limit is \
             refused and sends nothing."
        ),
        json!({
            "title": { "type": "string", "description": format!("A short title, 1 to {} characters.", state::MAX_TITLE) },
            "body": { "type": "string", "description": format!("More detail as plain text, at most {} characters; empty for none.", state::MAX_BODY) },
            "note_id": { "type": "string", "description": "The id of a note the notification opens; empty for none. Copy it from a row or a result, never make one up." },
        }),
    )
}

/// The source of `agent`'s notifications.
pub(crate) fn source(agent: &Agent) -> String {
    format!("agent:{}", agent.name)
}

fn text<'a>(input: &'a Value, name: &str) -> &'a str {
    input[name].as_str().map_or("", str::trim)
}

/// Run `notify_user` for `agent`, with the caller's `scope`, counting its sends in `sent`.
pub(crate) fn notify(
    w: &Writer,
    scope: Scope,
    agent: &Agent,
    limits: &Limits,
    sent: &NotifyCount,
    input: &Value,
) -> ToolOut {
    if !scope.allows(Scope::Write) {
        return ToolOut::error(
            "Sending a notification needs a token with write access: this one may only read.",
        );
    }
    let source = source(agent);
    let note_id = text(input, "note_id");
    let mut note_title = None;
    if !note_id.is_empty() {
        let note = match w.read(note_id) {
            Ok(n) => n,
            Err(_) => {
                return ToolOut::error(format!(
                    "there is no note {note_id}: give the id of an existing note, or none"
                ));
            }
        };
        if !limits.allows(&note.type_name) {
            return ToolOut::error(format!("This agent may not read {} notes.", note.type_name));
        }
        note_title = writer::doc_to_json(&note.doc)["fields"]["title"]
            .as_str()
            .map(str::to_owned);
    }
    if !sent.take() {
        return refused(&format!(
            "this turn already sent {MAX_PER_TURN} notifications, the most one turn may send"
        ));
    }
    let body = text(input, "body");
    let n = NewNotification {
        title: text(input, "title").to_owned(),
        body: (!body.is_empty()).then(|| body.to_owned()),
        note_id: (!note_id.is_empty()).then(|| note_id.to_owned()),
        url: None,
        source,
    };
    let stored = match w.notify_within(&n, &LIMITS) {
        Ok(s) => s,
        Err(writer::Error::State(state::Error::NotificationLimit {
            by,
            window_ms,
            count,
            max,
        })) => {
            sent.give_back();
            return refused(&limit_reached(&by, window_ms, count, max));
        }
        Err(e) => {
            sent.give_back();
            let message = e.to_string();
            let mut out = ToolOut::error(message.clone());
            out.content = json!({
                "error": message,
                "message": "Nothing was sent. Fix the problem and try again.",
            })
            .to_string();
            return out;
        }
    };
    let notes: Vec<Value> = stored
        .note_id
        .iter()
        .map(|id| json!({ "id": id, "title": note_title.clone().unwrap_or_else(|| id.clone()) }))
        .collect();
    ToolOut {
        content: json!({
            "sent": true,
            "id": stored.id,
            "message": "The notification is under the user's bell and was pushed to their paired phones.",
        })
        .to_string(),
        is_error: false,
        summary: json!({
            "notes": notes,
            "notification": { "id": stored.id, "title": stored.title },
        }),
        proposal: None,
    }
}

/// Why a limit refused the call: `by` is the agent's source, or [`AGENT_SOURCES`] for the cap of
/// all agents together.
fn limit_reached(by: &str, window_ms: i64, count: usize, max: usize) -> String {
    if by == AGENT_SOURCES {
        format!(
            "all agents together sent {count} notifications in the last 24 hours, and the limit \
             is {max} a day"
        )
    } else if window_ms == HOUR_MS {
        format!(
            "this agent sent {count} notifications in the last hour, and the limit is {max} an hour"
        )
    } else {
        format!(
            "this agent sent {count} notifications in the last 24 hours, and the limit is {max} a day"
        )
    }
}

/// The answer to a call over a limit: why, and that nothing was sent.
fn refused(why: &str) -> ToolOut {
    let message = format!(
        "Not sent: {why}. Do not try again now; say in your answer what you would have told the user."
    );
    let mut out = ToolOut::error(message.clone());
    out.content = json!({ "error": message, "limited": true }).to_string();
    out
}
