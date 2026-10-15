//! The live link to the home server (D9, D12): the unread count for the tray badge, and server
//! notifications mirrored as desktop notifications.
//!
//! One task holds `GET /api/events` open on the home server with the device token, and
//! reconnects with a growing pause when the stream fails. On connecting, and on every
//! `notification.*` event, it reads the unread notifications and reconciles them with what the
//! desktop shows ([`Mirror`]):
//!
//! - a notification that is unread and that the app has not seen yet is raised on the desktop;
//! - a raised notification that is no longer unread, because it was read, dismissed or cleared
//!   anywhere, is closed;
//! - the notifications already unread when the app first connects only count towards the badge,
//!   so starting the app does not raise a pile of old ones.
//!
//! Closing a raised notification on the desktop marks it read on the server, as a swipe does on
//! Android; clicking it opens what it points at in the window, and marks it read too.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use crate::notifier::{Notifier, Signal};

/// How long the stream may stay silent before it counts as dead. The server sends a heartbeat
/// every 15 seconds.
const SILENCE: Duration = Duration::from_secs(45);

/// The first pause before reconnecting, doubled after each failure up to [`MAX_BACKOFF`].
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// How many unread notifications the reconcile reads.
const PAGE: usize = 100;

/// The home server and the device token, once the device is paired and the token is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub url: String,
    pub token: String,
}

/// What the app's shell does for the live link: the tray badge and the window.
pub trait Shell: Send + Sync + 'static {
    /// The home server's unread notification count changed.
    fn badge(&self, unread: u64);
    /// Show the window at the web client's `path`.
    fn open(&self, path: &str);
}

/// Where the live link stands, for `/_desktop/status`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    /// Whether the event stream is open.
    pub connected: bool,
    /// The home server's unread count when it was last read.
    pub unread: Option<u64>,
    /// Why the stream is not open.
    pub message: Option<String>,
    /// How many server notifications the desktop shows now.
    pub shown: usize,
}

/// A server notification, as far as the desktop shows it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl Item {
    /// The web client's path the notification opens, when it opens one.
    pub fn path(&self) -> Option<String> {
        match (&self.note, &self.url) {
            (Some(note), _) => Some(format!("/notes/{note}")),
            (None, Some(url)) if url.starts_with('/') && !url.starts_with("//") => {
                Some(url.clone())
            }
            _ => None,
        }
    }
}

/// What the desktop should do to match the server's unread notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show(Item),
    Close(String),
}

/// Which server notifications the desktop has seen and which it shows.
#[derive(Debug, Default)]
pub struct Mirror {
    /// Whether the first reconcile has run.
    primed: bool,
    /// Every notification id seen unread since the app started.
    seen: HashSet<String>,
    /// The ids the desktop shows now.
    shown: HashSet<String>,
}

impl Mirror {
    /// The actions that bring the desktop in line with `unread`, newest first as the server lists
    /// them. Shows come oldest first, so the newest ends on top.
    pub fn reconcile(&mut self, unread: &[Item]) -> Vec<Action> {
        let ids: HashSet<&str> = unread.iter().map(|n| n.id.as_str()).collect();
        let mut gone: Vec<String> = self
            .shown
            .iter()
            .filter(|id| !ids.contains(id.as_str()))
            .cloned()
            .collect();
        gone.sort();
        let mut actions: Vec<Action> = gone
            .into_iter()
            .map(|id| {
                self.shown.remove(&id);
                Action::Close(id)
            })
            .collect();
        let primed = std::mem::replace(&mut self.primed, true);
        for n in unread.iter().rev() {
            if self.seen.insert(n.id.clone()) && primed {
                self.shown.insert(n.id.clone());
                actions.push(Action::Show(n.clone()));
            }
        }
        actions
    }

    /// The desktop no longer shows `id`: the user or the notification daemon closed it.
    pub fn closed(&mut self, id: &str) {
        self.shown.remove(id);
    }

    pub fn shown(&self) -> usize {
        self.shown.len()
    }
}

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

/// The live link's handle.
pub struct Live {
    status: Arc<Mutex<Status>>,
}

impl Live {
    /// Starts the live task. It waits until `remote` names a home server, and follows it when it
    /// changes.
    pub fn start(shell: Arc<dyn Shell>, remote: watch::Receiver<Option<Remote>>) -> Live {
        let status = Arc::new(Mutex::new(Status::default()));
        let task = Task {
            shell,
            status: status.clone(),
            mirror: Mirror::default(),
            items: HashMap::new(),
            notifier: None,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("an HTTP client builds"),
        };
        tauri::async_runtime::spawn(task.run(remote));
        Live { status }
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Why one connection ended.
enum End {
    /// The server refused the token: wait for a new one.
    Refused(String),
    /// The stream failed or went quiet: reconnect after a pause.
    Failed(String),
    /// The home server or the token changed: connect again at once.
    Changed,
}

struct Task {
    shell: Arc<dyn Shell>,
    status: Arc<Mutex<Status>>,
    mirror: Mirror,
    /// The notifications the desktop shows, by id, for a click to open.
    items: HashMap<String, Item>,
    notifier: Option<Notifier>,
    client: reqwest::Client,
}

impl Task {
    async fn run(mut self, mut remote: watch::Receiver<Option<Remote>>) {
        self.notifier = match Notifier::connect().await {
            Ok(n) => Some(n),
            Err(e) => {
                eprintln!("thevault: desktop notifications are off: {e}");
                None
            }
        };
        let mut backoff = MIN_BACKOFF;
        loop {
            let Some(current) = remote.borrow_and_update().clone() else {
                if remote.changed().await.is_err() {
                    return;
                }
                continue;
            };
            let end = self.connection(&current, &mut remote, &mut backoff).await;
            {
                let mut s = lock(&self.status);
                s.connected = false;
                s.message = match &end {
                    End::Refused(m) | End::Failed(m) => Some(m.clone()),
                    End::Changed => None,
                };
            }
            match end {
                End::Changed => backoff = MIN_BACKOFF,
                End::Refused(_) => {
                    if remote.changed().await.is_err() {
                        return;
                    }
                }
                End::Failed(_) => {
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        changed = remote.changed() => if changed.is_err() { return },
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }

    /// One connection to the event stream, until it ends.
    async fn connection(
        &mut self,
        remote: &Remote,
        changes: &mut watch::Receiver<Option<Remote>>,
        backoff: &mut Duration,
    ) -> End {
        let res = self
            .client
            .get(format!("{}/api/events", remote.url))
            .bearer_auth(&remote.token)
            .header("accept", "text/event-stream")
            .send()
            .await;
        let res = match res {
            Ok(res) => res,
            Err(e) => return End::Failed(format!("cannot reach the server: {e}")),
        };
        let code = res.status();
        if code == reqwest::StatusCode::UNAUTHORIZED || code == reqwest::StatusCode::FORBIDDEN {
            return End::Refused(format!("the server refused the device token ({code})"));
        }
        if !code.is_success() {
            return End::Failed(format!("the event stream answered {code}"));
        }
        {
            let mut s = lock(&self.status);
            s.connected = true;
            s.message = None;
        }
        *backoff = MIN_BACKOFF;
        if let Err(e) = self.refresh(remote).await {
            return End::Failed(e);
        }
        let mut stream = res.bytes_stream();
        let mut parser = SseParser::default();
        loop {
            tokio::select! {
                chunk = tokio::time::timeout(SILENCE, stream.next()) => {
                    let chunk = match chunk {
                        Err(_) => return End::Failed("the event stream went quiet".to_owned()),
                        Ok(None) => return End::Failed("the server closed the event stream".to_owned()),
                        Ok(Some(Err(e))) => return End::Failed(format!("the event stream failed: {e}")),
                        Ok(Some(Ok(chunk))) => chunk,
                    };
                    let events = parser.push(&String::from_utf8_lossy(&chunk));
                    if events.iter().any(|e| e.name.starts_with("notification."))
                        && let Err(e) = self.refresh(remote).await
                    {
                        return End::Failed(e);
                    }
                }
                signal = next_signal(&mut self.notifier) => self.on_signal(remote, signal).await,
                changed = changes.changed() => {
                    if changed.is_err() || changes.borrow().as_ref() != Some(remote) {
                        return End::Changed;
                    }
                }
            }
        }
    }

    /// Reads the unread notifications and brings the badge and the desktop in line.
    async fn refresh(&mut self, remote: &Remote) -> Result<(), String> {
        let res = self
            .client
            .get(format!(
                "{}/api/notifications?unread=true&limit={PAGE}",
                remote.url
            ))
            .bearer_auth(&remote.token)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("reading the notifications: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("reading the notifications: {}", res.status()));
        }
        let page: Value = res
            .json()
            .await
            .map_err(|e| format!("reading the notifications: {e}"))?;
        let unread = page.get("unread").and_then(Value::as_u64).unwrap_or(0);
        let items: Vec<Item> = page
            .get("items")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("reading the notifications: {e}"))?
            .unwrap_or_default();
        for action in self.mirror.reconcile(&items) {
            match action {
                Action::Show(item) => {
                    if let Some(notifier) = &mut self.notifier {
                        if let Err(e) = notifier.show(&item).await {
                            eprintln!("thevault: showing notification {}: {e}", item.id);
                            self.mirror.closed(&item.id);
                            continue;
                        }
                    } else {
                        self.mirror.closed(&item.id);
                        continue;
                    }
                    self.items.insert(item.id.clone(), item);
                }
                Action::Close(id) => {
                    self.items.remove(&id);
                    if let Some(notifier) = &mut self.notifier {
                        notifier.close(&id).await;
                    }
                }
            }
        }
        self.shell.badge(unread);
        let mut s = lock(&self.status);
        s.unread = Some(unread);
        s.shown = self.mirror.shown();
        Ok(())
    }

    /// The user closed or clicked a notification the app raised.
    async fn on_signal(&mut self, remote: &Remote, signal: Signal) {
        let (id, clicked) = match signal {
            Signal::Dismissed(id) => (id, false),
            Signal::Clicked(id) => (id, true),
            Signal::Expired(id) => {
                self.mirror.closed(&id);
                self.items.remove(&id);
                lock(&self.status).shown = self.mirror.shown();
                return;
            }
        };
        self.mirror.closed(&id);
        let item = self.items.remove(&id);
        lock(&self.status).shown = self.mirror.shown();
        if clicked {
            let path = item
                .and_then(|i| i.path())
                .unwrap_or_else(|| "/".to_owned());
            self.shell.open(&path);
        }
        let marked = self
            .client
            .post(format!("{}/api/notifications/{id}/read", remote.url))
            .bearer_auth(&remote.token)
            .timeout(Duration::from_secs(20))
            .send()
            .await;
        if let Err(e) = marked {
            eprintln!("thevault: marking notification {id} read: {e}");
        }
    }
}

/// The next signal from the notification daemon, or never when there is no daemon.
async fn next_signal(notifier: &mut Option<Notifier>) -> Signal {
    match notifier {
        Some(n) => n.next_signal().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> Item {
        Item {
            id: id.to_owned(),
            title: format!("title {id}"),
            body: None,
            note: None,
            url: None,
        }
    }

    fn shows(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Show(n) => Some(n.id.as_str()),
                Action::Close(_) => None,
            })
            .collect()
    }

    fn closes(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Close(id) => Some(id.as_str()),
                Action::Show(_) => None,
            })
            .collect()
    }

    #[test]
    fn notifications_unread_at_start_only_count() {
        let mut mirror = Mirror::default();
        assert!(mirror.reconcile(&[item("b"), item("a")]).is_empty());
        assert_eq!(mirror.shown(), 0);
    }

    #[test]
    fn a_new_notification_shows_once_and_closes_when_no_longer_unread() {
        let mut mirror = Mirror::default();
        mirror.reconcile(&[item("a")]);
        let actions = mirror.reconcile(&[item("c"), item("b"), item("a")]);
        assert_eq!(shows(&actions), ["b", "c"], "oldest first");
        assert!(closes(&actions).is_empty());
        assert!(
            mirror
                .reconcile(&[item("c"), item("b"), item("a")])
                .is_empty()
        );
        let actions = mirror.reconcile(&[item("c"), item("a")]);
        assert_eq!(closes(&actions), ["b"]);
        assert!(shows(&actions).is_empty());
        assert_eq!(closes(&mirror.reconcile(&[])), ["c"]);
        assert_eq!(mirror.shown(), 0);
    }

    #[test]
    fn a_notification_closed_on_the_desktop_does_not_come_back() {
        let mut mirror = Mirror::default();
        mirror.reconcile(&[]);
        assert_eq!(shows(&mirror.reconcile(&[item("a")])), ["a"]);
        mirror.closed("a");
        assert!(mirror.reconcile(&[item("a")]).is_empty());
        assert!(mirror.reconcile(&[]).is_empty(), "nothing left to close");
    }

    #[test]
    fn a_notification_from_while_disconnected_shows_on_reconnect() {
        let mut mirror = Mirror::default();
        mirror.reconcile(&[]);
        // The stream dropped; on reconnect the reconcile sees what arrived meanwhile.
        assert_eq!(shows(&mirror.reconcile(&[item("a")])), ["a"]);
    }

    #[test]
    fn a_notification_opens_its_note_or_an_app_path() {
        let mut n = item("a");
        assert_eq!(n.path(), None);
        n.url = Some("https://example.net".into());
        assert_eq!(n.path(), None);
        n.url = Some("//example.net/x".into());
        assert_eq!(n.path(), None);
        n.url = Some("/tasks".into());
        assert_eq!(n.path().as_deref(), Some("/tasks"));
        n.note = Some("01J9ZQ4V6W3K8M2N5P7R9T1XYZ".into());
        assert_eq!(
            n.path().as_deref(),
            Some("/notes/01J9ZQ4V6W3K8M2N5P7R9T1XYZ")
        );
    }

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
}
