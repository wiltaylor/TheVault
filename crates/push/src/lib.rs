//! The push dispatcher (D4, D9): it sends each paired device a WebPush message (RFC 8291) at its
//! UnifiedPush endpoint when a notification is created, read, dismissed or cleared, and when a
//! command is queued for it. A message names the event and the ids it concerns, nothing more;
//! the device fetches the rest through the API.
//!
//! The dispatcher follows [`state::Events`], the same stream `/api/events` serves. Each delivery
//! runs on its own task: a failed attempt (no answer, 429 or 5xx) is retried with doubling
//! waits, and an endpoint the push service answers 404 or 410 for is gone, so it is removed from
//! the device. Devices without an endpoint are not sent anything.
//!
//! Note changes are coalesced, so a burst of edits sends one push ([`Changes`]): the first change
//! after a quiet spell goes out [`Dispatcher::with_note_window`]'s `delay` later, and no more
//! than one `note.changed` goes out per window. Each device is told the ids changed since the
//! last one, `{"event": "note.changed", "ids": [...]}`, leaving out the changes its own token
//! made; a device whose own writes are all there is gets nothing. Too many ids for one message
//! send the first [`MAX_IDS`] and `"more": true`. The device runs a sync round on it.
//!
//! A device's settings changing in Settings > Devices is coalesced the same way, in a batch of
//! its own: each device whose settings changed is sent `{"event": "device.settings"}`, and only
//! that device. The device takes its settings again in the sync round it runs on it.
//!
//! The app builds the server hands out changing (`clients.updated`, a publish) is coalesced the
//! same way too: every device with an endpoint is sent `{"event": "clients.updated"}` once per
//! window, and checks the server for a newer app.

pub mod webpush;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{Value, json};
use state::{Device, LiveEvent, Missed, PushTarget, State};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// Whatever owns the vault's [`State`]: the server's writer, or a bare `State` in tests.
pub trait HasState: Send + Sync + 'static {
    fn state(&self) -> &State;
}

impl HasState for State {
    fn state(&self) -> &State {
        self
    }
}

impl HasState for writer::Writer {
    fn state(&self) -> &State {
        writer::Writer::state(self)
    }
}

/// One push message, before encryption.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// The only device it goes to, or `None` for every device with an endpoint.
    pub device: Option<String>,
    /// The JSON the device decrypts.
    pub body: Value,
    /// The `Urgency` header (RFC 8030): `high` for what the user should see now.
    pub urgency: &'static str,
}

/// The push message for `event`, or `None` when devices are not told about it.
pub fn message(event: &LiveEvent) -> Option<Message> {
    let name = event.name();
    let (device, body, urgency) = match event {
        LiveEvent::NotificationCreated { id, .. } => {
            (None, json!({ "event": name, "id": id }), "high")
        }
        LiveEvent::NotificationRead { id, .. } => {
            (None, json!({ "event": name, "id": id }), "normal")
        }
        LiveEvent::NotificationDismissed { id, .. } => {
            (None, json!({ "event": name, "id": id }), "normal")
        }
        LiveEvent::NotificationCleared { .. } => (None, json!({ "event": name }), "normal"),
        LiveEvent::DeviceCommand { device, id } => (
            Some(device.clone()),
            json!({ "event": name, "id": id }),
            "high",
        ),
        // Coalesced by the dispatcher instead: see [`Changes`].
        LiveEvent::NoteChanged { .. }
        | LiveEvent::DeviceSettings { .. }
        | LiveEvent::ClientsUpdated { .. } => return None,
    };
    Some(Message {
        device,
        body,
        urgency,
    })
}

/// How one delivery ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// The push service accepted it.
    Delivered,
    /// The push service said the endpoint is gone (404 or 410), so it was removed.
    Gone,
    /// The push service refused it with this status, which retrying would not change.
    Refused(u16),
    /// Every attempt failed.
    GaveUp,
    /// The device's keys could not encrypt it.
    BadKeys,
}

/// Sends push messages. [`Dispatcher::default`] is what the server runs.
#[derive(Clone)]
pub struct Dispatcher {
    client: reqwest::Client,
    attempts: u32,
    first_wait: Duration,
    ttl: Duration,
    note_delay: Duration,
    note_window: Duration,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Dispatcher {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("an HTTP client with a timeout"),
            attempts: 5,
            first_wait: Duration::from_secs(2),
            ttl: Duration::from_secs(24 * 60 * 60),
            note_delay: Duration::from_secs(1),
            note_window: Duration::from_secs(10),
        }
    }
}

impl Dispatcher {
    /// Try each delivery `attempts` times, waiting `first_wait` after the first failure and
    /// twice as long after each one after.
    pub fn with_retries(mut self, attempts: u32, first_wait: Duration) -> Self {
        self.attempts = attempts.max(1);
        self.first_wait = first_wait;
        self
    }

    /// Push note changes `delay` after the first one, and at most one `note.changed` every
    /// `window`. The defaults are 1 and 10 seconds.
    pub fn with_note_window(mut self, delay: Duration, window: Duration) -> Self {
        self.note_delay = delay;
        self.note_window = window;
        self
    }

    /// Follow `owner`'s events on the current Tokio runtime until they close. It subscribes
    /// before returning, so every event published after this call is pushed.
    pub fn spawn<S: HasState>(self, owner: Arc<S>) -> JoinHandle<()> {
        let mut events = owner.state().events().subscribe();
        let this = Arc::new(self);
        tokio::spawn(async move {
            let mut changes = Changes::new(this.note_delay, this.note_window);
            // The devices whose settings changed, as changes with no token.
            let mut settings = Changes::new(this.note_delay, this.note_window);
            // Publishes of app builds, as one change with no token.
            let mut clients = Changes::new(this.note_delay, this.note_window);
            loop {
                let due = changes.due();
                let settings_due = settings.due();
                let clients_due = clients.due();
                tokio::select! {
                    got = events.recv() => match got {
                        Ok((_, LiveEvent::NoteChanged { id, token, .. })) => {
                            changes.add(id, token, Instant::now());
                        }
                        Ok((_, LiveEvent::DeviceSettings { device })) => {
                            settings.add(device, None, Instant::now());
                        }
                        Ok((_, LiveEvent::ClientsUpdated { .. })) => {
                            clients.add(CLIENTS_UPDATED.to_owned(), None, Instant::now());
                        }
                        Ok((_, event)) => {
                            if let Some(message) = message(&event) {
                                this.send(&owner, message).await;
                            }
                        }
                        Err(Missed::Lagged(n)) => {
                            eprintln!("push: fell behind and did not send {n} events")
                        }
                        Err(Missed::Closed) => {
                            let batch = changes.take(Instant::now());
                            this.send_changes(&owner, batch).await;
                            let batch = settings.take(Instant::now());
                            this.send_settings(&owner, batch).await;
                            let batch = clients.take(Instant::now());
                            this.send_clients(&owner, batch).await;
                            return;
                        }
                    },
                    _ = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                        let batch = changes.take(Instant::now());
                        this.send_changes(&owner, batch).await;
                    }
                    _ = tokio::time::sleep_until(settings_due.unwrap_or_else(Instant::now)), if settings_due.is_some() => {
                        let batch = settings.take(Instant::now());
                        this.send_settings(&owner, batch).await;
                    }
                    _ = tokio::time::sleep_until(clients_due.unwrap_or_else(Instant::now)), if clients_due.is_some() => {
                        let batch = clients.take(Instant::now());
                        this.send_clients(&owner, batch).await;
                    }
                }
            }
        })
    }

    /// Every device, or `None` when listing them failed.
    async fn devices<S: HasState>(owner: &Arc<S>) -> Option<Vec<Device>> {
        let o = owner.clone();
        match tokio::task::spawn_blocking(move || o.state().list_devices()).await {
            Ok(Ok(devices)) => Some(devices),
            Ok(Err(e)) => {
                eprintln!("push: listing the devices failed: {e}");
                None
            }
            Err(e) => {
                eprintln!("push: listing the devices failed: {e}");
                None
            }
        }
    }

    /// Start telling every device with an endpoint which notes in `batch` changed, leaving out
    /// the changes its own token made.
    async fn send_changes<S: HasState>(self: &Arc<Self>, owner: &Arc<S>, batch: Vec<Change>) {
        if batch.is_empty() {
            return;
        }
        let Some(devices) = Self::devices(owner).await else {
            return;
        };
        for device in devices {
            let Some(target) = device.push.clone() else {
                continue;
            };
            let Some(body) = changes_body(&batch, &device.token_id) else {
                continue;
            };
            let this = self.clone();
            let owner = owner.clone();
            let body = body.to_string().into_bytes();
            tokio::spawn(async move {
                this.deliver(&owner, &device.id, &target, &body, "normal")
                    .await;
            });
        }
    }

    /// Start telling each device in `batch`, a device id each, that its settings changed.
    async fn send_settings<S: HasState>(self: &Arc<Self>, owner: &Arc<S>, batch: Vec<Change>) {
        if batch.is_empty() {
            return;
        }
        let Some(devices) = Self::devices(owner).await else {
            return;
        };
        let body = Arc::new(
            json!({ "event": "device.settings" })
                .to_string()
                .into_bytes(),
        );
        for (device, target) in devices
            .into_iter()
            .filter(|d| batch.iter().any(|c| c.id == d.id))
            .filter_map(|d| d.push.map(|p| (d.id, p)))
        {
            let this = self.clone();
            let owner = owner.clone();
            let body = body.clone();
            tokio::spawn(async move {
                this.deliver(&owner, &device, &target, &body, "normal")
                    .await;
            });
        }
    }

    /// Start telling every device with an endpoint that the app builds changed, when `batch`
    /// holds a publish.
    async fn send_clients<S: HasState>(self: &Arc<Self>, owner: &Arc<S>, batch: Vec<Change>) {
        if batch.is_empty() {
            return;
        }
        let message = Message {
            device: None,
            body: json!({ "event": CLIENTS_UPDATED }),
            urgency: "normal",
        };
        self.send(owner, message).await;
    }

    /// Start delivering `message` to every device it is for, each on its own task.
    async fn send<S: HasState>(self: &Arc<Self>, owner: &Arc<S>, message: Message) {
        let Some(devices) = Self::devices(owner).await else {
            return;
        };
        let body = Arc::new(message.body.to_string().into_bytes());
        for (device, target) in targets(devices, message.device.as_deref()) {
            let this = self.clone();
            let owner = owner.clone();
            let body = body.clone();
            tokio::spawn(async move {
                this.deliver(&owner, &device, &target, &body, message.urgency)
                    .await;
            });
        }
    }

    /// Encrypt `plaintext` to `target` and send it, retrying as the module docs say. A 404 or
    /// 410 removes the endpoint from device `device`, unless it registered another meanwhile.
    pub async fn deliver<S: HasState>(
        &self,
        owner: &Arc<S>,
        device: &str,
        target: &PushTarget,
        plaintext: &[u8],
        urgency: &str,
    ) -> Delivery {
        let body = match webpush::encrypt(&target.p256dh, &target.auth, plaintext) {
            Ok(body) => body,
            Err(e) => {
                eprintln!("push: device {device}: {e}");
                return Delivery::BadKeys;
            }
        };
        let mut wait = self.first_wait;
        for attempt in 1..=self.attempts {
            let sent = self
                .client
                .post(&target.endpoint)
                .header("content-encoding", "aes128gcm")
                .header("content-type", "application/octet-stream")
                .header("ttl", self.ttl.as_secs().to_string())
                .header("urgency", urgency)
                .body(body.clone())
                .send()
                .await;
            match sent.map(|r| r.status()) {
                Ok(s) if s.is_success() => return Delivery::Delivered,
                Ok(StatusCode::NOT_FOUND | StatusCode::GONE) => {
                    let (o, id, endpoint) =
                        (owner.clone(), device.to_owned(), target.endpoint.clone());
                    let dropped = tokio::task::spawn_blocking(move || {
                        o.state().drop_device_push(&id, &endpoint)
                    })
                    .await;
                    if let Ok(Err(e)) = dropped {
                        eprintln!("push: device {device}: removing its endpoint failed: {e}");
                    }
                    return Delivery::Gone;
                }
                Ok(s) if s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() => {
                    eprintln!("push: device {device}: attempt {attempt} got {s}");
                }
                Ok(s) => {
                    eprintln!("push: device {device}: the push service refused it with {s}");
                    return Delivery::Refused(s.as_u16());
                }
                Err(e) => eprintln!("push: device {device}: attempt {attempt} failed: {e}"),
            }
            if attempt < self.attempts {
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
        }
        Delivery::GaveUp
    }
}

/// The devices among `devices` a message goes to, with where: those with an endpoint, and only
/// `only` when it is given.
pub fn targets(devices: Vec<Device>, only: Option<&str>) -> Vec<(String, PushTarget)> {
    devices
        .into_iter()
        .filter(|d| only.is_none_or(|id| d.id == id))
        .filter_map(|d| d.push.map(|p| (d.id, p)))
        .collect()
}

/// The event, and push message, that says the app builds changed.
pub const CLIENTS_UPDATED: &str = "clients.updated";

/// The most note ids one `note.changed` push names. A WebPush message holds about 4 KB.
pub const MAX_IDS: usize = 64;

/// One note change waiting to be pushed: the note, and the token that made it, when one did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub token: Option<String>,
}

/// The note changes waiting for the next `note.changed` push, and when it may go out.
#[derive(Debug)]
pub struct Changes {
    delay: Duration,
    window: Duration,
    waiting: Vec<Change>,
    /// When the first change waiting arrived.
    first: Option<Instant>,
    /// When the last push went out.
    last: Option<Instant>,
}

impl Changes {
    pub fn new(delay: Duration, window: Duration) -> Changes {
        Changes {
            delay,
            window,
            waiting: Vec::new(),
            first: None,
            last: None,
        }
    }

    /// Note `id` changed at `now`, by `token`.
    pub fn add(&mut self, id: String, token: Option<String>, now: Instant) {
        let change = Change { id, token };
        if !self.waiting.contains(&change) {
            self.waiting.push(change);
        }
        self.first.get_or_insert(now);
    }

    /// When the waiting changes go out: `delay` after the first of them, and not before a
    /// `window` has passed since the last push. `None` when nothing waits.
    pub fn due(&self) -> Option<Instant> {
        let first = self.first? + self.delay;
        Some(match self.last {
            Some(last) => first.max(last + self.window),
            None => first,
        })
    }

    /// Take the waiting changes to push them at `now`.
    pub fn take(&mut self, now: Instant) -> Vec<Change> {
        self.first = None;
        if !self.waiting.is_empty() {
            self.last = Some(now);
        }
        std::mem::take(&mut self.waiting)
    }
}

/// The `note.changed` message for the device whose token is `token`: the ids in `batch` its own
/// token did not change, or `None` when there are none.
pub fn changes_body(batch: &[Change], token: &str) -> Option<Value> {
    let mut seen = HashSet::new();
    let ids: Vec<&str> = batch
        .iter()
        .filter(|c| c.token.as_deref() != Some(token))
        .map(|c| c.id.as_str())
        .filter(|id| seen.insert(*id))
        .collect();
    if ids.is_empty() {
        return None;
    }
    let mut body = json!({ "event": "note.changed", "ids": &ids[..ids.len().min(MAX_IDS)] });
    if ids.len() > MAX_IDS {
        body["more"] = json!(true);
    }
    Some(body)
}
