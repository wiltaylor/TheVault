//! The chat's Outbox tools (outbox X9): `draft_email`, `draft_sms` and `draft_event`.
//!
//! Each creates one Outbox item, written as the agent whose turn it is (`Author::Agent`, the
//! built-in Assistant included), so the writer's X2 rules decide what it may be: `review`, or
//! `approved` when the agent's `permissions.outbox.send_without_review` is on. Drafting is not a
//! change the chat's Apply confirms, since the Outbox is the review, so the tools are offered to
//! unattended runs too. They need a caller with `write`. A person note id given for an address
//! resolves to the person's first email or phone number when drafted.

use schema::Agent;
use serde_json::{Value, json};
use state::{Author, Scope};
use writer::{Input, OUTBOX_TYPE, Writer, doc_to_json};

use ai::ToolSpec;

use crate::ai_chat::{Limits, ToolOut, tool};

/// The Outbox tools' names.
pub const DRAFT_TOOLS: [&str; 3] = ["draft_email", "draft_sms", "draft_event"];

/// The Outbox tools' specs.
pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    let list =
        |what: &str| json!({ "type": "array", "items": { "type": "string" }, "description": what });
    vec![
        tool(
            "draft_email",
            "Draft an email into the Outbox. It is not sent: it waits in the Outbox for the user to review, approve and send, unless this agent may send without review, when it is sent at once. Answers the Outbox item's id and status.",
            json!({
                "to": list("Each recipient: an email address, or a person note's id, which becomes that person's first email address."),
                "cc": list("Cc recipients, as for to; empty for none."),
                "bcc": list("Bcc recipients, as for to; empty for none."),
                "subject": { "type": "string", "description": "The subject line." },
                "body": { "type": "string", "description": "The message as plain text; a blank line starts a new paragraph." },
                "attachments": list("Asset ids of files to attach (find_assets finds them); empty for none."),
                "in_reply_to": { "type": "string", "description": "The id of the email note this answers, so the reply joins its thread; empty for a new email." },
            }),
        ),
        tool(
            "draft_sms",
            "Draft an SMS into the Outbox, for the user's phone to send. It is not sent: it waits in the Outbox for the user to review and approve, unless this agent may send without review. Answers the Outbox item's id and status.",
            json!({
                "to": { "type": "string", "description": "A phone number, or a person note's id, which becomes that person's first phone number." },
                "body": { "type": "string", "description": "The text of the message." },
                "device": { "type": "string", "description": "The id of the phone that sends it; empty for the vault's one Android device with SMS sending on." },
            }),
        ),
        tool(
            "draft_event",
            "Draft a calendar invite into the Outbox: a new Google Calendar event, or a change to an existing event note, which emails its attendees once sent. It is not sent: it waits in the Outbox for the user to review and approve, unless this agent may send without review. For a change, an empty field leaves that part of the event as it is. Answers the Outbox item's id and status.",
            json!({
                "event": { "type": "string", "description": "The id of the event note to change; empty for a new event." },
                "title": { "type": "string", "description": "The event's title; empty to keep a changed event's title." },
                "when": {
                    "type": "object",
                    "description": "When it starts and ends: dates (2026-03-02) for an all-day event, or RFC 3339 date-times with an offset (2026-03-02T09:30:00+11:00). Both empty to keep a changed event's time.",
                    "properties": {
                        "start": { "type": "string" },
                        "end": { "type": "string" },
                    },
                    "required": ["start", "end"],
                    "additionalProperties": false,
                },
                "location": { "type": "string", "description": "Where; empty for none or to keep it." },
                "attendees": list("Each attendee: an email address, or a person note's id, which becomes that person's first email address. Empty for none or to keep them."),
                "description": { "type": "string", "description": "The event's description as plain text; empty for none or to keep it." },
                "calendar": { "type": "string", "description": "The Google calendar id; empty for the default calendar or to keep it." },
            }),
        ),
    ]
}

/// Whether `name` is an Outbox tool.
pub(crate) fn is_draft_tool(name: &str) -> bool {
    DRAFT_TOOLS.contains(&name)
}

fn text<'a>(input: &'a Value, name: &str) -> &'a str {
    input[name].as_str().map_or("", str::trim)
}

fn texts(input: &Value, name: &str) -> Vec<String> {
    input[name]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether `s` reads as a note id: 26 Crockford base-32 characters.
fn looks_like_id(s: &str) -> bool {
    s.len() == 26 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The person note `id`, its title and fields, when it is one the agent may read.
fn person(w: &Writer, limits: &Limits, id: &str) -> Result<Option<(String, Value)>, ToolOut> {
    let Ok(n) = w.read(id) else {
        return Ok(None);
    };
    if n.type_name != "person" {
        return Err(ToolOut::error(format!(
            "{id} is a {} note, not a person",
            n.type_name
        )));
    }
    if !limits.allows("person") {
        return Err(ToolOut::error("This agent may not read person notes."));
    }
    let fields = doc_to_json(&n.doc)["fields"].clone();
    let title = fields["title"].as_str().unwrap_or(id).to_owned();
    Ok(Some((title, fields)))
}

/// `to`, an address or a person note's id, as an address from the person's `field` list, and
/// the name to show for it.
fn resolve(
    w: &Writer,
    limits: &Limits,
    to: &str,
    field: &str,
    what: &str,
) -> Result<(String, String), ToolOut> {
    if looks_like_id(to)
        && let Some((name, fields)) = person(w, limits, to)?
    {
        let first = fields[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .find(|s| !s.is_empty());
        return match first {
            Some(a) => Ok((a.to_owned(), name)),
            None => Err(ToolOut::error(format!("{name} has no {what}"))),
        };
    }
    Ok((to.to_owned(), to.to_owned()))
}

/// Each address in `list` resolved, and the names to show.
fn resolve_all(
    w: &Writer,
    limits: &Limits,
    list: &[String],
) -> Result<(Vec<String>, Vec<String>), ToolOut> {
    let mut addresses = Vec::new();
    let mut names = Vec::new();
    for to in list {
        let (a, n) = resolve(w, limits, to, "emails", "email address")?;
        if !a.contains('@') {
            return Err(ToolOut::error(format!(
                "{to:?} is not an email address or a person note's id"
            )));
        }
        addresses.push(a);
        names.push(n);
    }
    Ok((addresses, names))
}

/// `text` as a content block called `label`: one paragraph per blank-line-separated part, its
/// markup characters escaped. `None` for empty text.
fn content(label: &str, text: &str) -> Option<Value> {
    let blocks: Vec<Value> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| json!({ "kind": "paragraph", "fields": { "text": crate::agent_threads::plain(p) } }))
        .collect();
    (!blocks.is_empty()).then(|| json!({ "kind": "content", "labels": [label], "blocks": blocks }))
}

/// A reference argument naming a note of type `type_name`, or empty.
fn note_of(w: &Writer, id: &str, type_name: &str, what: &str) -> Result<Option<Value>, ToolOut> {
    if id.is_empty() {
        return Ok(None);
    }
    let n = w
        .read(id)
        .map_err(|_| ToolOut::error(format!("{what} {id} is not a note")))?;
    if n.type_name != type_name {
        return Err(ToolOut::error(format!(
            "{what} {id} is a {} note, not an {type_name}",
            n.type_name
        )));
    }
    Ok(Some(doc_to_json(&n.doc)["fields"].clone()))
}

/// Run Outbox tool `name` for `agent` with the caller's `scope`.
pub(crate) fn draft(
    w: &Writer,
    scope: Scope,
    agent: &Agent,
    limits: &Limits,
    name: &str,
    input: &Value,
) -> ToolOut {
    if !scope.allows(Scope::Write) {
        return ToolOut::error(
            "Drafting into the Outbox needs a token with write access: this one may only read.",
        );
    }
    let built = match name {
        "draft_email" => email(w, limits, input),
        "draft_sms" => sms(w, limits, input),
        "draft_event" => event(w, limits, input),
        other => Err(ToolOut::error(format!("there is no tool {other:?}"))),
    };
    match built {
        Ok(d) => create(w, agent, d),
        Err(e) => e,
    }
}

/// An item to create: its fields and blocks, and what the chat's card says of it.
struct Drafted {
    kind: &'static str,
    title: String,
    /// Who it goes to, as the card names them.
    to: Vec<String>,
    fields: Value,
    blocks: Vec<Value>,
}

fn email(w: &Writer, limits: &Limits, input: &Value) -> Result<Drafted, ToolOut> {
    let (to, names) = resolve_all(w, limits, &texts(input, "to"))?;
    if to.is_empty() {
        return Err(ToolOut::error("to must name at least one recipient"));
    }
    let (cc, _) = resolve_all(w, limits, &texts(input, "cc"))?;
    let (bcc, _) = resolve_all(w, limits, &texts(input, "bcc"))?;
    let attachments = texts(input, "attachments");
    for a in &attachments {
        if w.asset_info(a).is_err() {
            return Err(ToolOut::error(format!(
                "there is no asset {a}; find_assets finds the files to attach"
            )));
        }
    }
    let in_reply_to = text(input, "in_reply_to");
    let replied = note_of(w, in_reply_to, "email", "in_reply_to")?;
    let mut subject = text(input, "subject").to_owned();
    if subject.is_empty()
        && let Some(r) = &replied
    {
        let s = r["subject"]
            .as_str()
            .or_else(|| r["title"].as_str())
            .unwrap_or("");
        subject = if s.to_ascii_lowercase().starts_with("re:") {
            s.to_owned()
        } else {
            format!("Re: {s}")
        };
    }
    let title = if subject.is_empty() {
        format!("Email to {}", names.join(", "))
    } else {
        subject.clone()
    };
    let mut fields = json!({
        "title": title,
        "kind": "email",
        "to": to,
        "subject": subject,
    });
    if !cc.is_empty() {
        fields["cc"] = json!(cc);
    }
    if !bcc.is_empty() {
        fields["bcc"] = json!(bcc);
    }
    if !attachments.is_empty() {
        fields["attachments"] = json!(attachments);
    }
    if replied.is_some() {
        fields["in_reply_to"] = json!(in_reply_to);
    }
    Ok(Drafted {
        kind: "email",
        title,
        to: names,
        fields,
        blocks: content("body", input["body"].as_str().unwrap_or(""))
            .into_iter()
            .collect(),
    })
}

fn sms(w: &Writer, limits: &Limits, input: &Value) -> Result<Drafted, ToolOut> {
    let to = text(input, "to");
    if to.is_empty() {
        return Err(ToolOut::error(
            "to must be a phone number or a person note's id",
        ));
    }
    let (phone, name) = resolve(w, limits, to, "phones", "phone number")?;
    if !phone.chars().any(|c| c.is_ascii_digit()) {
        return Err(ToolOut::error(format!(
            "{to:?} is not a phone number or a person note's id"
        )));
    }
    let body = text(input, "body");
    if body.is_empty() {
        return Err(ToolOut::error("body must be the text of the message"));
    }
    let title = format!("SMS to {name}");
    let mut fields = json!({ "title": title, "kind": "sms", "phone": phone });
    let device = text(input, "device");
    if !device.is_empty() {
        fields["device"] = json!(device);
    }
    Ok(Drafted {
        kind: "sms",
        title,
        to: vec![name],
        fields,
        blocks: content("body", body).into_iter().collect(),
    })
}

fn event(w: &Writer, limits: &Limits, input: &Value) -> Result<Drafted, ToolOut> {
    let event_id = text(input, "event");
    let current = note_of(w, event_id, "event", "event")?;
    let mut title = text(input, "title").to_owned();
    let start = input["when"]["start"].as_str().map_or("", str::trim);
    let end = input["when"]["end"].as_str().map_or("", str::trim);
    if current.is_none() {
        if title.is_empty() {
            return Err(ToolOut::error("a new event needs a title"));
        }
        if start.is_empty() {
            return Err(ToolOut::error("a new event needs when.start"));
        }
    }
    if title.is_empty()
        && let Some(c) = &current
    {
        title = c["title"].as_str().unwrap_or("Event").to_owned();
    }
    let (attendees, names) = resolve_all(w, limits, &texts(input, "attendees"))?;
    let mut fields = json!({ "title": title, "kind": "event" });
    if current.is_some() {
        fields["event"] = json!(event_id);
    }
    if !start.is_empty() || !end.is_empty() {
        let start = if start.is_empty() { end } else { start };
        let end = if end.is_empty() { start } else { end };
        fields["when"] = json!({ "start": start, "end": end });
    }
    for name in ["location", "calendar"] {
        let v = text(input, name);
        if !v.is_empty() {
            fields[name] = json!(v);
        }
    }
    if !attendees.is_empty() {
        fields["attendees_raw"] = json!(attendees);
    }
    Ok(Drafted {
        kind: "event",
        title: title.clone(),
        to: names,
        fields,
        blocks: content("description", text(input, "description"))
            .into_iter()
            .collect(),
    })
}

/// Create the item as `agent`: `review`, or `approved` when it may send without review.
fn create(w: &Writer, agent: &Agent, d: Drafted) -> ToolOut {
    let bypass = agent.permissions.outbox.send_without_review;
    let status = if bypass { "approved" } else { "review" };
    let mut fields = d.fields;
    fields["status"] = json!(status);
    fields["drafted_by"] = json!(agent.name);
    let note = json!({ "fields": fields, "blocks": d.blocks });
    let author = Author::Agent(agent.name.clone());
    let saved = match w.create(OUTBOX_TYPE, &Input::Json(note), &author) {
        Ok(s) => s,
        Err(e) => {
            let message = e.to_string();
            let details = crate::ApiError::from(e).body;
            let mut out = ToolOut::error(message.clone());
            out.content = json!({
                "error": message,
                "details": details,
                "message": "Nothing was drafted. Fix the problem and draft it again.",
            })
            .to_string();
            return out;
        }
    };
    let what = match d.kind {
        "email" => "email",
        "sms" => "SMS",
        _ => "calendar invite",
    };
    let message = if bypass {
        format!(
            "The {what} is in the Outbox as approved: this agent may send without review, so it is \
             being sent now. Item id {}.",
            saved.id
        )
    } else {
        format!(
            "The {what} is in the Outbox for the user to review (item id {}). It has not been \
             sent: it is sent only when the user approves it there.",
            saved.id
        )
    };
    ToolOut {
        content: json!({
            "drafted": true,
            "id": saved.id,
            "kind": d.kind,
            "status": status,
            "message": message,
        })
        .to_string(),
        is_error: false,
        summary: json!({
            "notes": [{ "id": saved.id, "title": d.title }],
            "draft": {
                "note_id": saved.id,
                "kind": d.kind,
                "status": status,
                "title": d.title,
                "to": d.to,
            },
        }),
        proposal: None,
    }
}
