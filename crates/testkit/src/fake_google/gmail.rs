//! The fake Gmail API, under [`super::GMAIL_PATH`]: labels, messages list, message get,
//! attachments and history, for the `users/me` mailbox (google-sync D2, D3, D11, D14).
//!
//! ```no_run
//! use serde_json::json;
//! use testkit::FakeGoogle;
//!
//! let google = FakeGoogle::builder().gmail_page_size(2).start();
//! let work = google.add_label("Work/Clients");
//! let id = google.add_message(json!({
//!     "thread": "t1",
//!     "labels": ["INBOX", work],
//!     "at": 1_790_000_000_000_i64,
//!     "from": "Alex Example <alex@example.com>",
//!     "to": ["me@example.com"],
//!     "subject": "Hello",
//!     "text": "First line\nSecond line",
//!     "html": "<p>First line</p>",
//!     "attachments": [{ "filename": "notes.pdf", "mime": "application/pdf", "size": 1024 }],
//! }));
//! google.set_message_labels(&id, &["INBOX"]); // recorded in history as label changes
//! google.delete_message(&id); // recorded in history as messagesDeleted
//! google.set_history_not_found(true); // history.list answers 404
//! ```
//!
//! Every path needs a bearer token the token endpoint issued ([`super::authorized`]), and
//! answers 401 otherwise. The paths, under `/gmail/v1/users/me`:
//!
//! - `GET /profile`: `{ messagesTotal, historyId }`, with the mailbox's current history id.
//! - `GET /labels`: `{ labels: [{ id, name, type }] }`. The system labels `INBOX`, `SENT`,
//!   `DRAFT`, `UNREAD`, `STARRED`, `IMPORTANT`, `SPAM` and `TRASH` are always there; user labels
//!   come from [`FakeGoogle::add_label`], with ids `Label_<n>`.
//! - `GET /messages`: `{ messages: [{ id, threadId }], resultSizeEstimate, nextPageToken? }`,
//!   newest first, filtered by every `labelIds` given and by `q`, of which only `after:` and
//!   `before:` terms (epoch seconds, or `YYYY/MM/DD` read as UTC) are understood. A page holds
//!   `maxResults` messages (default 100), at most the builder's
//!   [`FakeGoogleBuilder::gmail_page_size`].
//! - `GET /messages/<id>`: the message with `format=full` (default) or `format=metadata` (no
//!   body parts). A deleted message answers 404.
//! - `GET /messages/<id>/attachments/<attachment id>`: `{ attachmentId, size, data }`.
//! - `GET /history?startHistoryId=`: every history record after `startHistoryId`, oldest first,
//!   `{ history, historyId, nextPageToken? }`, paged like messages. `labelId` keeps the records
//!   whose message has that label now. A missing `startHistoryId`, or the
//!   [`FakeGoogle::set_history_not_found`] switch, answers 404.
//!
//! Body data and attachment data are base64url with padding, as Gmail sends them. Each change
//! (a message added, labels changed, a message deleted) takes the next history id.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE;
use serde_json::{Value, json};

use super::{FakeGoogle, FakeGoogleBuilder, GMAIL_PATH, Shared, error, lock};
use crate::standin::{Reply, Request};

/// The system labels every fake mailbox has.
const SYSTEM_LABELS: [&str; 8] = [
    "INBOX",
    "SENT",
    "DRAFT",
    "UNREAD",
    "STARRED",
    "IMPORTANT",
    "SPAM",
    "TRASH",
];

/// How the fake Gmail starts.
#[derive(Debug, Clone, Default)]
pub(super) struct Config {
    page_size: Option<usize>,
    history_not_found: bool,
}

impl FakeGoogleBuilder {
    /// The most messages or history records one Gmail page holds, whatever `maxResults` asks.
    pub fn gmail_page_size(mut self, size: usize) -> Self {
        self.gmail.page_size = Some(size.max(1));
        self
    }

    /// Start with history.list answering 404, as Gmail does for a history id it no longer keeps.
    pub fn history_not_found(mut self, on: bool) -> Self {
        self.gmail.history_not_found = on;
        self
    }
}

#[derive(Debug, Clone)]
struct Label {
    id: String,
    name: String,
    system: bool,
}

#[derive(Debug, Clone)]
struct Attachment {
    id: String,
    filename: String,
    mime: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Message {
    id: String,
    thread: String,
    labels: Vec<String>,
    at: i64,
    history_id: u64,
    headers: Vec<(String, String)>,
    snippet: String,
    text: Option<String>,
    html: Option<String>,
    attachments: Vec<Attachment>,
    deleted: bool,
}

#[derive(Debug, Clone)]
enum Change {
    Added,
    LabelsAdded(Vec<String>),
    LabelsRemoved(Vec<String>),
    Deleted,
}

#[derive(Debug, Clone)]
struct History {
    id: u64,
    message: String,
    change: Change,
}

/// The fake Gmail's mailbox.
#[derive(Debug)]
pub(super) struct State {
    config: Config,
    labels: Vec<Label>,
    messages: Vec<Message>,
    history: Vec<History>,
    history_id: u64,
    issued: usize,
}

impl State {
    pub(super) fn new(config: Config) -> State {
        State {
            config,
            labels: SYSTEM_LABELS
                .iter()
                .map(|l| Label {
                    id: (*l).to_owned(),
                    name: (*l).to_owned(),
                    system: true,
                })
                .collect(),
            messages: Vec::new(),
            history: Vec::new(),
            history_id: 1000,
            issued: 0,
        }
    }

    fn next_history(&mut self) -> u64 {
        self.history_id += 1;
        self.history_id
    }

    fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id && !m.deleted)
    }

    fn page_size(&self, asked: Option<String>) -> usize {
        let asked = asked
            .and_then(|s| s.parse().ok())
            .unwrap_or(100usize)
            .max(1);
        self.config.page_size.map_or(asked, |cap| asked.min(cap))
    }
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

impl FakeGoogle {
    /// Add a user label named `name` (nested labels are named `Parent/Child`), and return its id.
    pub fn add_label(&self, name: &str) -> String {
        let mut gmail = self.gmail();
        gmail.issued += 1;
        let id = format!("Label_{}", gmail.issued);
        gmail.labels.push(Label {
            id: id.clone(),
            name: name.to_owned(),
            system: false,
        });
        id
    }

    /// Add a message and return its id. `spec` holds any of: `id` (default `m<n>`), `thread`
    /// (default the message id), `labels` (label ids), `at` (milliseconds since the epoch, Gmail's
    /// `internalDate`), `from`, `to` and `cc` (a string or a list), `subject`, `snippet`
    /// (default the text's start), `text` (a `text/plain` part), `html` (a `text/html` part),
    /// and `attachments`, each `{ filename, mime, size }` (that many bytes) or
    /// `{ filename, mime, data }` (a string). It is recorded in history as `messagesAdded`.
    pub fn add_message(&self, spec: Value) -> String {
        let mut gmail = self.gmail();
        gmail.issued += 1;
        let n = gmail.issued;
        let id = spec["id"]
            .as_str()
            .map_or_else(|| format!("m{n:04}"), str::to_owned);
        let text = spec["text"].as_str().map(str::to_owned);
        let mut headers = Vec::new();
        for (key, name) in [("from", "From"), ("to", "To"), ("cc", "Cc")] {
            let values = strings(&spec[key]);
            if !values.is_empty() {
                headers.push((name.to_owned(), values.join(", ")));
            }
        }
        if let Some(subject) = spec["subject"].as_str() {
            headers.push(("Subject".to_owned(), subject.to_owned()));
        }
        headers.push((
            "Date".to_owned(),
            "Mon, 5 Jan 2026 10:00:00 +0000".to_owned(),
        ));
        let attachments = spec["attachments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(k, a)| Attachment {
                id: format!("att-{id}-{k}"),
                filename: a["filename"].as_str().unwrap_or("file").to_owned(),
                mime: a["mime"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .to_owned(),
                data: match a["data"].as_str() {
                    Some(d) => d.as_bytes().to_vec(),
                    None => (0..a["size"].as_u64().unwrap_or(0))
                        .map(|b| (b % 251) as u8)
                        .collect(),
                },
            })
            .collect();
        let history_id = gmail.next_history();
        let snippet = spec["snippet"].as_str().map_or_else(
            || {
                text.as_deref()
                    .unwrap_or_default()
                    .chars()
                    .take(100)
                    .collect()
            },
            str::to_owned,
        );
        gmail.messages.push(Message {
            id: id.clone(),
            thread: spec["thread"]
                .as_str()
                .map_or_else(|| id.clone(), str::to_owned),
            labels: strings(&spec["labels"]),
            at: spec["at"].as_i64().unwrap_or(0),
            history_id,
            headers,
            snippet,
            text,
            html: spec["html"].as_str().map(str::to_owned),
            attachments,
            deleted: false,
        });
        gmail.history.push(History {
            id: history_id,
            message: id.clone(),
            change: Change::Added,
        });
        id
    }

    /// Give message `id` exactly `labels`, recording what was added and removed in history.
    ///
    /// # Panics
    ///
    /// When there is no such message.
    pub fn set_message_labels(&self, id: &str, labels: &[&str]) {
        let mut gmail = self.gmail();
        let history_id = gmail.next_history();
        let message = gmail
            .messages
            .iter_mut()
            .find(|m| m.id == id)
            .expect("a fake Gmail message");
        let new: Vec<String> = labels.iter().map(|l| (*l).to_owned()).collect();
        let added: Vec<String> = new
            .iter()
            .filter(|l| !message.labels.contains(l))
            .cloned()
            .collect();
        let removed: Vec<String> = message
            .labels
            .iter()
            .filter(|l| !new.contains(l))
            .cloned()
            .collect();
        message.labels = new;
        message.history_id = history_id;
        if !added.is_empty() {
            gmail.history.push(History {
                id: history_id,
                message: id.to_owned(),
                change: Change::LabelsAdded(added),
            });
        }
        if !removed.is_empty() {
            gmail.history.push(History {
                id: history_id,
                message: id.to_owned(),
                change: Change::LabelsRemoved(removed),
            });
        }
    }

    /// Delete message `id` for good, recording `messagesDeleted` in history.
    pub fn delete_message(&self, id: &str) {
        let mut gmail = self.gmail();
        let history_id = gmail.next_history();
        if let Some(m) = gmail.messages.iter_mut().find(|m| m.id == id) {
            m.deleted = true;
        }
        gmail.history.push(History {
            id: history_id,
            message: id.to_owned(),
            change: Change::Deleted,
        });
    }

    /// Turn the 404 answer to every history.list request on or off.
    pub fn set_history_not_found(&self, on: bool) {
        self.gmail().config.history_not_found = on;
    }

    /// The mailbox's current history id.
    pub fn gmail_history_id(&self) -> u64 {
        self.gmail().history_id
    }
}

/// Answer a Gmail request, or `None` when `request` is not one.
pub(super) fn handle(shared: &Shared, request: &Request) -> Option<Reply> {
    let rest = request.route().strip_prefix(GMAIL_PATH)?;
    if !super::authorized(shared, request) {
        return Some(super::unauthorized());
    }
    let Some(rest) = rest.strip_prefix("/users/me/") else {
        return Some(not_found());
    };
    if request.method != "GET" {
        return Some(error(
            405,
            "METHOD_NOT_ALLOWED",
            "The fake Gmail answers GET only.",
        ));
    }
    let gmail = lock(&shared.gmail);
    let parts: Vec<&str> = rest.split('/').collect();
    Some(match parts.as_slice() {
        ["profile"] => Reply::json(
            200,
            &json!({
                "messagesTotal": gmail.messages.iter().filter(|m| !m.deleted).count(),
                "historyId": gmail.history_id.to_string(),
            }),
        ),
        ["labels"] => labels(&gmail),
        ["messages"] => list(&gmail, request),
        ["messages", id] => get(&gmail, id, request),
        ["messages", id, "attachments", aid] => attachment(&gmail, id, aid),
        ["history"] => history(&gmail, request),
        _ => not_found(),
    })
}

fn not_found() -> Reply {
    error(404, "NOT_FOUND", "Requested entity was not found.")
}

fn labels(gmail: &State) -> Reply {
    let labels: Vec<Value> = gmail
        .labels
        .iter()
        .map(|l| {
            json!({
                "id": l.id,
                "name": l.name,
                "type": if l.system { "system" } else { "user" },
            })
        })
        .collect();
    Reply::json(200, &json!({ "labels": labels }))
}

/// Milliseconds since the epoch of a `q` date: epoch seconds, or `YYYY/MM/DD` as UTC.
fn q_time(v: &str) -> Option<i64> {
    if let Ok(secs) = v.parse::<i64>() {
        return Some(secs * 1000);
    }
    let mut it = v.split('/').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    // Days from the civil date, Howard Hinnant's algorithm.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400_000)
}

/// One page of `items`: the slice from `pageToken` and the token of the next page.
fn page<T: Clone>(items: &[T], request: &Request, size: usize) -> (Vec<T>, Option<String>) {
    let start = request
        .query("pageToken")
        .and_then(|t| t.parse::<usize>().ok())
        .unwrap_or(0)
        .min(items.len());
    let end = (start + size).min(items.len());
    let next = (end < items.len()).then(|| end.to_string());
    (items[start..end].to_vec(), next)
}

fn list(gmail: &State, request: &Request) -> Reply {
    let q = request.query("q").unwrap_or_default();
    let mut after = None;
    let mut before = None;
    for term in q.split_whitespace() {
        if let Some(v) = term.strip_prefix("after:") {
            after = q_time(v);
        } else if let Some(v) = term.strip_prefix("before:") {
            before = q_time(v);
        }
    }
    let wanted: Vec<String> = request
        .path
        .split_once('?')
        .map(|(_, q)| crate::standin::url_pairs(q))
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, _)| k == "labelIds")
        .map(|(_, v)| v)
        .collect();
    let mut found: Vec<&Message> = gmail
        .messages
        .iter()
        .filter(|m| !m.deleted)
        .filter(|m| wanted.iter().all(|l| m.labels.contains(l)))
        .filter(|m| after.is_none_or(|a| m.at >= a))
        .filter(|m| before.is_none_or(|b| m.at < b))
        .collect();
    found.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    let ids: Vec<Value> = found
        .iter()
        .map(|m| json!({ "id": m.id, "threadId": m.thread }))
        .collect();
    let (items, next) = page(&ids, request, gmail.page_size(request.query("maxResults")));
    let mut body = json!({ "resultSizeEstimate": ids.len() });
    if !items.is_empty() {
        body["messages"] = Value::Array(items);
    }
    if let Some(next) = next {
        body["nextPageToken"] = json!(next);
    }
    Reply::json(200, &body)
}

fn data(bytes: &[u8]) -> String {
    URL_SAFE.encode(bytes)
}

fn part(mime: &str, text: &str) -> Value {
    json!({
        "mimeType": mime,
        "filename": "",
        "headers": [{ "name": "Content-Type", "value": format!("{mime}; charset=\"UTF-8\"") }],
        "body": { "size": text.len(), "data": data(text.as_bytes()) },
    })
}

fn get(gmail: &State, id: &str, request: &Request) -> Reply {
    let Some(m) = gmail.message(id) else {
        return not_found();
    };
    let full = request.query("format").is_none_or(|f| f == "full");
    let headers: Vec<Value> = m
        .headers
        .iter()
        .map(|(n, v)| json!({ "name": n, "value": v }))
        .collect();
    let mut texts = Vec::new();
    if let Some(t) = &m.text {
        texts.push(part("text/plain", t));
    }
    if let Some(h) = &m.html {
        texts.push(part("text/html", h));
    }
    let body_part = if texts.len() == 1 {
        texts.remove(0)
    } else {
        json!({ "mimeType": "multipart/alternative", "filename": "", "body": { "size": 0 }, "parts": texts })
    };
    let mut payload = if m.attachments.is_empty() {
        body_part
    } else {
        let mut parts = vec![body_part];
        for a in &m.attachments {
            parts.push(json!({
                "mimeType": a.mime,
                "filename": a.filename,
                "body": { "attachmentId": a.id, "size": a.data.len() },
            }));
        }
        json!({ "mimeType": "multipart/mixed", "filename": "", "body": { "size": 0 }, "parts": parts })
    };
    payload["headers"] = Value::Array(headers);
    if !full {
        let obj = payload.as_object_mut().expect("a payload object");
        obj.remove("parts");
        obj.insert("body".to_owned(), json!({ "size": 0 }));
    }
    let size: usize = m.text.as_ref().map_or(0, String::len)
        + m.html.as_ref().map_or(0, String::len)
        + m.attachments.iter().map(|a| a.data.len()).sum::<usize>();
    Reply::json(
        200,
        &json!({
            "id": m.id,
            "threadId": m.thread,
            "labelIds": m.labels,
            "snippet": m.snippet,
            "historyId": m.history_id.to_string(),
            "internalDate": m.at.to_string(),
            "sizeEstimate": size,
            "payload": payload,
        }),
    )
}

fn attachment(gmail: &State, id: &str, aid: &str) -> Reply {
    let Some(a) = gmail
        .message(id)
        .and_then(|m| m.attachments.iter().find(|a| a.id == aid))
    else {
        return not_found();
    };
    Reply::json(
        200,
        &json!({ "attachmentId": a.id, "size": a.data.len(), "data": data(&a.data) }),
    )
}

fn history(gmail: &State, request: &Request) -> Reply {
    if gmail.config.history_not_found {
        return not_found();
    }
    let Some(start) = request
        .query("startHistoryId")
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return not_found();
    };
    let label = request.query("labelId");
    let records: Vec<Value> = gmail
        .history
        .iter()
        .filter(|h| h.id > start)
        .filter_map(|h| {
            let m = gmail.messages.iter().find(|m| m.id == h.message)?;
            if label.as_ref().is_some_and(|l| !m.labels.contains(l)) {
                return None;
            }
            let message = json!({ "id": m.id, "threadId": m.thread, "labelIds": m.labels });
            let mut record = json!({ "id": h.id.to_string(), "messages": [message] });
            match &h.change {
                Change::Added => record["messagesAdded"] = json!([{ "message": message }]),
                Change::Deleted => {
                    record["messagesDeleted"] =
                        json!([{ "message": { "id": m.id, "threadId": m.thread } }]);
                }
                Change::LabelsAdded(l) => {
                    record["labelsAdded"] = json!([{ "message": message, "labelIds": l }]);
                }
                Change::LabelsRemoved(l) => {
                    record["labelsRemoved"] = json!([{ "message": message, "labelIds": l }]);
                }
            }
            Some(record)
        })
        .collect();
    let (items, next) = page(
        &records,
        request,
        gmail.page_size(request.query("maxResults")),
    );
    let mut body = json!({ "historyId": gmail.history_id.to_string() });
    if !items.is_empty() {
        body["history"] = Value::Array(items);
    }
    if let Some(next) = next {
        body["nextPageToken"] = json!(next);
    }
    Reply::json(200, &body)
}
