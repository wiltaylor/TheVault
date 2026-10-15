//! The push dispatcher (D4, D9): it sends each paired device a WebPush message (RFC 8291) at its
//! UnifiedPush endpoint when a notification is created, read, dismissed or cleared, and when a
//! command is queued for it. A message names the event and the ids it concerns, nothing more;
//! the device fetches the rest through the API.
//!
//! The dispatcher follows [`state::Events`], the same stream `/api/events` serves. Each delivery
//! runs on its own task: a failed attempt (no answer, 429 or 5xx) is retried with doubling
//! waits, and an endpoint the push service answers 404 or 410 for is gone, so it is removed from
//! the device. Devices without an endpoint are not sent anything.

pub mod webpush;

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{Value, json};
use state::{Device, LiveEvent, Missed, PushTarget, State};
use tokio::task::JoinHandle;

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
        LiveEvent::NoteChanged { .. } => return None,
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

    /// Follow `owner`'s events on the current Tokio runtime until they close. It subscribes
    /// before returning, so every event published after this call is pushed.
    pub fn spawn<S: HasState>(self, owner: Arc<S>) -> JoinHandle<()> {
        let mut events = owner.state().events().subscribe();
        let this = Arc::new(self);
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok((_, event)) => {
                        if let Some(message) = message(&event) {
                            this.send(&owner, message).await;
                        }
                    }
                    Err(Missed::Lagged(n)) => {
                        eprintln!("push: fell behind and did not send {n} events")
                    }
                    Err(Missed::Closed) => return,
                }
            }
        })
    }

    /// Start delivering `message` to every device it is for, each on its own task.
    async fn send<S: HasState>(self: &Arc<Self>, owner: &Arc<S>, message: Message) {
        let o = owner.clone();
        let devices = match tokio::task::spawn_blocking(move || o.state().list_devices()).await {
            Ok(Ok(devices)) => devices,
            Ok(Err(e)) => {
                eprintln!("push: listing the devices failed: {e}");
                return;
            }
            Err(e) => {
                eprintln!("push: listing the devices failed: {e}");
                return;
            }
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
