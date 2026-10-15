//! Following the home server's live events (D4), so an open app pulls a change made elsewhere
//! within seconds rather than at its next timed round.
//!
//! The server's `GET /api/events` stream sends `note.changed` with `{id, type, version,
//! deleted}` for every committed note change. [`SseParser`] splits the stream into events,
//! [`NoteChanged::of`] reads a `note.changed`, and [`is_news`] tells a change the device does not
//! hold yet from the echo of its own push, so the device's own writes do not start another round.
//! [`follow`] holds one connection open with the blocking client the engine uses; the caller
//! reconnects. The desktop app follows the stream on its async runtime and uses only the parser
//! and [`is_news`].

use std::io::Read;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde_json::Value;

use crate::{Error, Queue, Remote, Result};

/// How long the stream may stay silent before it counts as dead. The server sends a heartbeat
/// every 15 seconds.
pub const SILENCE: Duration = Duration::from_secs(45);

/// One server-sent event: its name and its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub data: String,
}

/// Splits a server-sent event stream into events, whatever chunks it arrives in.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
    name: String,
    data: Vec<String>,
}

impl SseParser {
    /// The events `chunk` completes.
    pub fn push(&mut self, chunk: &str) -> Vec<Event> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=end).collect();
            let line = line.trim_end_matches('\n').trim_end_matches('\r');
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(Event {
                        name: if self.name.is_empty() {
                            "message".to_owned()
                        } else {
                            std::mem::take(&mut self.name)
                        },
                        data: self.data.join("\n"),
                    });
                }
                self.name.clear();
                self.data.clear();
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.name = value.to_owned(),
                "data" => self.data.push(value.to_owned()),
                _ => {}
            }
        }
        events
    }
}

/// A `note.changed` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteChanged {
    pub id: String,
    pub version: String,
    pub deleted: bool,
}

impl NoteChanged {
    /// The change `event` reports, when it is a `note.changed` with the data it should carry.
    pub fn of(event: &Event) -> Option<NoteChanged> {
        if event.name != "note.changed" {
            return None;
        }
        let data: Value = serde_json::from_str(&event.data).ok()?;
        Some(NoteChanged {
            id: data.get("id")?.as_str()?.to_owned(),
            version: data.get("version")?.as_str()?.to_owned(),
            deleted: data
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// Whether the device lacks `change`: it does unless its copy of the note already matches that
/// version, as it does once its own push of the change has finished.
pub fn is_news(queue: &Queue, change: &NoteChanged) -> bool {
    match queue.synced(&change.id) {
        Ok(Some(s)) => s.server_version != change.version || s.deleted != change.deleted,
        _ => true,
    }
}

/// What [`follow`] saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The stream is open: what changed while it was closed is only in the change feed.
    Opened,
    /// A note changed on the server.
    NoteChanged(NoteChanged),
}

/// Hold `GET /api/events` open on `remote`'s server and hand each [`Seen`] to `on`, until the
/// stream ends, goes quiet for longer than `silence`, or `keep` answers false; `keep` is asked
/// after every read, so at least every heartbeat. Answers `Ok` when `keep` stopped it and an
/// error otherwise: [`Error::Server`] with status 401 or 403 when the server refused the token.
/// It blocks.
pub fn follow(
    remote: &Remote,
    silence: Duration,
    keep: &dyn Fn() -> bool,
    on: &mut dyn FnMut(Seen),
) -> Result<()> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(silence)
        .build()
        .map_err(|e| Error::Http(e.to_string()))?;
    let url = format!("{}/api/events", remote.url.trim_end_matches('/'));
    let mut res = client
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {}", remote.token))
        .header(ACCEPT, "text/event-stream")
        .send()
        .map_err(|e| Error::Http(e.to_string()))?;
    let status = res.status();
    if status != StatusCode::OK {
        return Err(Error::Server {
            what: "opening the event stream".to_owned(),
            status: status.as_u16(),
            body: res.text().unwrap_or_default(),
        });
    }
    on(Seen::Opened);
    let mut parser = SseParser::default();
    let mut buf = [0u8; 4096];
    // Bytes of a character split across reads.
    let mut partial = Vec::new();
    loop {
        if !keep() {
            return Ok(());
        }
        let n = res.read(&mut buf).map_err(|e| Error::Http(e.to_string()))?;
        if n == 0 {
            return Err(Error::Http("the server closed the event stream".to_owned()));
        }
        partial.extend_from_slice(&buf[..n]);
        let valid = match std::str::from_utf8(&partial) {
            Ok(_) => partial.len(),
            Err(e) => e.valid_up_to(),
        };
        let text = String::from_utf8_lossy(&partial[..valid]).into_owned();
        partial.drain(..valid);
        for event in parser.push(&text) {
            if let Some(change) = NoteChanged::of(&event) {
                on(Seen::NoteChanged(change));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_sent_events_parse_across_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(": open\n\n").is_empty(), "a comment is no event");
        assert!(p.push("event: notification.created\nid: 1\nda").is_empty());
        let events = p.push("ta: {\"id\":\"a\",\"unread\":1}\n\n: heartbeat\n\n");
        assert_eq!(
            events,
            [Event {
                name: "notification.created".into(),
                data: "{\"id\":\"a\",\"unread\":1}".into()
            }]
        );
        let events = p.push("event: note.changed\r\ndata: x\r\ndata: y\r\n\r\ndata:z\n\n");
        assert_eq!(
            events,
            [
                Event {
                    name: "note.changed".into(),
                    data: "x\ny".into()
                },
                Event {
                    name: "message".into(),
                    data: "z".into()
                }
            ]
        );
    }

    #[test]
    fn a_note_change_reads_its_id_and_version() {
        let event = Event {
            name: "note.changed".into(),
            data: r#"{"id":"n","type":"memo","version":"v2","deleted":true}"#.into(),
        };
        assert_eq!(
            NoteChanged::of(&event),
            Some(NoteChanged {
                id: "n".into(),
                version: "v2".into(),
                deleted: true
            })
        );
        let other = Event {
            name: "notification.created".into(),
            data: event.data.clone(),
        };
        assert_eq!(NoteChanged::of(&other), None);
    }
}
