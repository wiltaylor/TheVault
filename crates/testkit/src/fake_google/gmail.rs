//! The fake Gmail API, under [`super::GMAIL_PATH`]: labels, messages list, message get,
//! attachments, history and send, for the `users/me` mailbox (google-sync D2, D3, D11, D14;
//! outbox X4), and the media upload of a message to send, under [`super::GMAIL_UPLOAD_PATH`].
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
//! let sent = google.sent_messages(); // what messages.send received, parsed
//! google.set_send_error(Some((500, "Backend Error"))); // messages.send answers that
//! google.set_history_not_found(true); // history.list answers 404
//! google.set_gmail_delay(Some(std::time::Duration::from_secs(2))); // each request waits 2 s
//! google.set_gmail_delay(None); // and any request still waiting answers now
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
//! - `POST /messages/send`: `{ raw, threadId? }`, `raw` the RFC 822 message in base64url. The
//!   message joins the mailbox labelled `SENT`, in thread `threadId` or a thread of its own, with
//!   its `From`, `To`, `Cc` and `Subject` (encoded words decoded), a `Message-ID`, its first
//!   `text/plain` and `text/html` parts and its attachments; `Bcc` is not kept, as Gmail keeps it
//!   from the recipients. Answers `{ id, threadId, labelIds }`, or the
//!   [`FakeGoogle::set_send_error`] error.
//! - `POST /upload/gmail/v1/users/me/messages/send?uploadType=media` with `Content-Type:
//!   message/rfc822`: the same, with the message as the body.
//!
//! Every message has a `Message-ID` header, `<id@mail.fake.example>` unless `add_message` was
//! given `message_id`, and a `References` header when it was given `references`.
//!
//! Body data and attachment data are base64url with padding, as Gmail sends them. Each change
//! (a message added, labels changed, a message deleted) takes the next history id.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use super::{FakeGoogle, FakeGoogleBuilder, GMAIL_PATH, GMAIL_UPLOAD_PATH, Shared, error, lock};
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
    delay: Option<Duration>,
    send_error: Option<(u16, String)>,
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

    /// Wait `delay` before answering each Gmail request, so a sync run is slow.
    pub fn gmail_delay(mut self, delay: Duration) -> Self {
        self.gmail.delay = Some(delay);
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

/// How a message reached `messages.send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRoute {
    /// `POST /gmail/v1/users/me/messages/send` with `{ raw }`.
    Raw,
    /// `POST /upload/gmail/v1/users/me/messages/send?uploadType=media`.
    Upload,
}

/// One message `messages.send` received, as it came and as the fake read it.
#[derive(Debug, Clone)]
pub struct SentMessage {
    /// The id the fake gave it, the answer's `id`.
    pub id: String,
    pub route: SendRoute,
    /// The `threadId` the request named, if any.
    pub thread_id: Option<String>,
    /// The RFC 822 message, decoded from base64url for [`SendRoute::Raw`].
    pub raw: String,
    /// The top-level headers, unfolded, in order, values as written.
    pub headers: Vec<(String, String)>,
    /// The first `text/plain` part, decoded.
    pub text: Option<String>,
    /// The first `text/html` part, decoded.
    pub html: Option<String>,
    /// Each attachment as `(filename, MIME type, bytes)`.
    pub attachments: Vec<(String, String, Vec<u8>)>,
}

impl SentMessage {
    /// The value of top-level header `name`, ignoring case, as written.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
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
    sent: Vec<SentMessage>,
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
            sent: Vec::new(),
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
            "Message-ID".to_owned(),
            spec["message_id"]
                .as_str()
                .map_or_else(|| format!("<{id}@mail.fake.example>"), str::to_owned),
        ));
        if let Some(references) = spec["references"].as_str() {
            headers.push(("References".to_owned(), references.to_owned()));
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

    /// Wait `delay` before answering each Gmail request, or answer at once with `None`. A request
    /// already waiting is held to the new delay, so `None` releases it.
    pub fn set_gmail_delay(&self, delay: Option<Duration>) {
        self.gmail().config.delay = delay;
    }

    /// The mailbox's current history id.
    pub fn gmail_history_id(&self) -> u64 {
        self.gmail().history_id
    }

    /// Every message `messages.send` took, oldest first.
    pub fn sent_messages(&self) -> Vec<SentMessage> {
        self.gmail().sent.clone()
    }

    /// Answer every `messages.send` with Google's error body of `status` and `message`, or send
    /// again with `None`.
    pub fn set_send_error(&self, error: Option<(u16, &str)>) {
        self.gmail().config.send_error = error.map(|(s, m)| (s, m.to_owned()));
    }
}

/// Answer a Gmail request, or `None` when `request` is not one.
pub(super) fn handle(shared: &Shared, request: &Request) -> Option<Reply> {
    let route = request.route();
    let (upload, rest) = match route.strip_prefix(GMAIL_UPLOAD_PATH) {
        Some(rest) => (true, rest),
        None => (false, route.strip_prefix(GMAIL_PATH)?),
    };
    if !super::authorized(shared, request) {
        return Some(super::unauthorized());
    }
    let Some(rest) = rest.strip_prefix("/users/me/") else {
        return Some(not_found());
    };
    if rest == "messages/send" {
        if request.method != "POST" {
            return Some(error(
                405,
                "METHOD_NOT_ALLOWED",
                "messages.send takes POST.",
            ));
        }
        return Some(send(&mut lock(&shared.gmail), request, upload));
    }
    if upload {
        return Some(not_found());
    }
    if request.method != "GET" {
        return Some(error(
            405,
            "METHOD_NOT_ALLOWED",
            "The fake Gmail answers GET only.",
        ));
    }
    // Wait without holding the mailbox, so the test can still change it.
    super::hold(|| lock(&shared.gmail).config.delay);
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

/// `messages.send`, by `raw` or by media upload.
fn send(gmail: &mut State, request: &Request, upload: bool) -> Reply {
    if let Some((status, message)) = &gmail.config.send_error {
        let code = match status {
            400 => "INVALID_ARGUMENT",
            403 => "PERMISSION_DENIED",
            _ => "INTERNAL",
        };
        return error(*status, code, message);
    }
    let (raw, thread_id, route) = if upload {
        if request.query("uploadType").as_deref() != Some("media") {
            return error(400, "INVALID_ARGUMENT", "uploadType must be media.");
        }
        if !request
            .header("content-type")
            .is_some_and(|c| c.starts_with("message/rfc822"))
        {
            return error(400, "INVALID_ARGUMENT", "The media must be message/rfc822.");
        }
        (
            String::from_utf8_lossy(&request.body).into_owned(),
            None,
            SendRoute::Upload,
        )
    } else {
        let body = request.json();
        let Some(raw) = body["raw"].as_str() else {
            return error(
                400,
                "INVALID_ARGUMENT",
                "'raw' RFC822 payload message string or uploading message via /upload/* URL required",
            );
        };
        let bytes = match URL_SAFE_NO_PAD.decode(raw.trim_end_matches('=')) {
            Ok(b) => b,
            Err(_) => return error(400, "INVALID_ARGUMENT", "Invalid value for ByteString"),
        };
        (
            String::from_utf8_lossy(&bytes).into_owned(),
            body["threadId"].as_str().map(str::to_owned),
            SendRoute::Raw,
        )
    };
    let (headers, body) = split_message(&raw);
    let mut parsed = Parsed::default();
    walk_mime(&headers, body, &mut parsed);
    let header = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| decode_words(v))
    };
    if header("to").is_none() && header("cc").is_none() && header("bcc").is_none() {
        return error(400, "INVALID_ARGUMENT", "Invalid To header");
    }
    gmail.issued += 1;
    let id = format!("s{:04}", gmail.issued);
    let thread = thread_id.clone().unwrap_or_else(|| id.clone());
    let mut kept = Vec::new();
    for name in ["From", "To", "Cc", "Subject", "In-Reply-To", "References"] {
        if let Some(v) = header(name) {
            kept.push((name.to_owned(), v));
        }
    }
    kept.push(("Message-ID".to_owned(), format!("<{id}@mail.fake.example>")));
    let history_id = gmail.next_history();
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let attachments = parsed
        .attachments
        .iter()
        .enumerate()
        .map(|(k, (filename, mime, data))| Attachment {
            id: format!("att-{id}-{k}"),
            filename: filename.clone(),
            mime: mime.clone(),
            data: data.clone(),
        })
        .collect();
    gmail.messages.push(Message {
        id: id.clone(),
        thread: thread.clone(),
        labels: vec!["SENT".to_owned()],
        at,
        history_id,
        headers: kept,
        snippet: parsed
            .text
            .as_deref()
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect(),
        text: parsed.text.clone(),
        html: parsed.html.clone(),
        attachments,
        deleted: false,
    });
    gmail.history.push(History {
        id: history_id,
        message: id.clone(),
        change: Change::Added,
    });
    gmail.sent.push(SentMessage {
        id: id.clone(),
        route,
        thread_id,
        raw: raw.clone(),
        headers: headers.clone(),
        text: parsed.text,
        html: parsed.html,
        attachments: parsed.attachments,
    });
    Reply::json(
        200,
        &json!({ "id": id, "threadId": thread, "labelIds": ["SENT"] }),
    )
}

/// What [`walk_mime`] found in a message.
#[derive(Default)]
struct Parsed {
    text: Option<String>,
    html: Option<String>,
    attachments: Vec<(String, String, Vec<u8>)>,
}

/// The headers of a MIME entity, unfolded, and its body.
fn split_message(text: &str) -> (Vec<(String, String)>, &str) {
    let (head, body) = match text.find("\r\n\r\n") {
        Some(at) => (&text[..at], &text[at + 4..]),
        None => match text.find("\n\n") {
            Some(at) => (&text[..at], &text[at + 2..]),
            None => (text, ""),
        },
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = headers.last_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if let Some((n, v)) = line.split_once(':') {
            headers.push((n.trim().to_owned(), v.trim().to_owned()));
        }
    }
    (headers, body)
}

/// The value of parameter `name` of a header such as `Content-Type`, without quotes.
fn param(value: &str, name: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().trim_matches('"').to_owned())
    })
}

/// RFC 2047 encoded words of `=?UTF-8?B?...?=` form decoded, the rest as it is.
fn decode_words(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("=?") {
        let Some(len) = rest[start + 2..].find("?=") else {
            break;
        };
        let word = &rest[start + 2..start + 2 + len];
        let mut parts = word.splitn(3, '?');
        let (charset, enc, data) = (parts.next(), parts.next(), parts.next());
        out.push_str(&rest[..start]);
        match (charset, enc, data) {
            (Some(c), Some(e), Some(d))
                if c.eq_ignore_ascii_case("utf-8") && e.eq_ignore_ascii_case("b") =>
            {
                let bytes = STANDARD.decode(d).unwrap_or_default();
                out.push_str(&String::from_utf8_lossy(&bytes));
            }
            _ => out.push_str(&rest[start..start + 4 + len]),
        }
        rest = &rest[start + 4 + len..];
    }
    out.push_str(rest);
    out
}

/// The bytes of a part's body, by its `Content-Transfer-Encoding`.
fn part_bytes(headers: &[(String, String)], body: &str) -> Vec<u8> {
    let encoding = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-transfer-encoding"))
        .map(|(_, v)| v.to_ascii_lowercase());
    if encoding.as_deref() == Some("base64") {
        let clean: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        STANDARD.decode(clean).unwrap_or_default()
    } else {
        body.as_bytes().to_vec()
    }
}

fn walk_mime(headers: &[(String, String)], body: &str, found: &mut Parsed) {
    let get = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    let content_type = get("content-type").unwrap_or_else(|| "text/plain".to_owned());
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if mime.starts_with("multipart/") {
        let Some(boundary) = param(&content_type, "boundary") else {
            return;
        };
        let open = format!("--{boundary}");
        for chunk in body.split(open.as_str()).skip(1) {
            if chunk.starts_with("--") {
                break;
            }
            let chunk = chunk.strip_prefix("\r\n").unwrap_or(chunk);
            let chunk = chunk.strip_suffix("\r\n").unwrap_or(chunk);
            let (h, b) = split_message(chunk);
            walk_mime(&h, b, found);
        }
        return;
    }
    let disposition = get("content-disposition").unwrap_or_default();
    let filename = param(&disposition, "filename").or_else(|| param(&content_type, "name"));
    let bytes = part_bytes(headers, body);
    if let Some(name) = filename {
        found.attachments.push((decode_words(&name), mime, bytes));
    } else if mime == "text/plain" && found.text.is_none() {
        found.text = Some(String::from_utf8_lossy(&bytes).into_owned());
    } else if mime == "text/html" && found.html.is_none() {
        found.html = Some(String::from_utf8_lossy(&bytes).into_owned());
    }
}
