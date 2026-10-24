//! Live events for open clients (D4): what changed, by id, as it changes. The API streams them at
//! `/api/events`; a client fetches the details through the API.
//!
//! Events are published under the lock that makes the change, so subscribers see them in the
//! order the changes happened. Each carries a sequence number, counting from 1 since the server
//! started. A subscriber that falls more than [`CAPACITY`] events behind loses the oldest, and
//! learns so from [`Subscription::recv`].

use std::sync::Mutex;

use tokio::sync::{broadcast, watch};

/// How many events a subscriber may fall behind before it loses some.
pub const CAPACITY: usize = 1024;

/// One change an open client may want to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// A notification was added.
    NotificationCreated { id: String, unread: u64 },
    /// An unread notification's title or body changed, such as an agent's count of drafted
    /// Outbox items going up within one turn.
    NotificationUpdated { id: String, unread: u64 },
    /// One notification was marked read, or every one when `id` is `None`.
    NotificationRead { id: Option<String>, unread: u64 },
    /// One notification was removed.
    NotificationDismissed { id: String, unread: u64 },
    /// The read notifications were removed.
    NotificationCleared { cleared: usize, unread: u64 },
    /// A note was written or deleted. `version` is its new version, or for a delete the version
    /// it went to the trash with. `token` names the API token that made the change, when one
    /// did, so the push dispatcher does not tell a device about its own writes.
    NoteChanged {
        id: String,
        type_name: String,
        version: String,
        deleted: bool,
        token: Option<String>,
    },
    /// A command was queued for a device.
    DeviceCommand { device: String, id: String },
    /// A device's settings changed in Settings > Devices, so the device should take them again.
    DeviceSettings { device: String },
    /// The app builds the server hands out (`/api/clients`) changed: a build was published,
    /// replaced or removed. `clients` is every build listed now.
    ClientsUpdated { clients: Vec<ClientBuild> },
    /// A chat turn on an agent's thread started or ended (agents G17). `turn` is the turn's id
    /// in the turns form, `None` for a turn with no id (an agent trigger's run).
    AiTurn {
        agent: String,
        thread: String,
        turn: Option<String>,
        running: bool,
    },
    /// An entry was added to the activity log, or a coalesced `note.update` entry counted another
    /// edit, keeping its id (activity Y3).
    Activity { item: crate::Activity },
}

/// One app build in a [`LiveEvent::ClientsUpdated`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBuild {
    /// `apk`, `appimage`, `deb` or `rpm`.
    pub kind: String,
    pub arch: String,
    pub version: String,
    /// `release` or `debug`.
    pub build: String,
}

impl LiveEvent {
    /// The event's name on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            LiveEvent::NotificationCreated { .. } => "notification.created",
            LiveEvent::NotificationUpdated { .. } => "notification.updated",
            LiveEvent::NotificationRead { .. } => "notification.read",
            LiveEvent::NotificationDismissed { .. } => "notification.dismissed",
            LiveEvent::NotificationCleared { .. } => "notification.cleared",
            LiveEvent::NoteChanged { .. } => "note.changed",
            LiveEvent::DeviceCommand { .. } => "device.command",
            LiveEvent::DeviceSettings { .. } => "device.settings",
            LiveEvent::ClientsUpdated { .. } => "clients.updated",
            LiveEvent::AiTurn { .. } => "ai_turn",
            LiveEvent::Activity { .. } => "activity",
        }
    }
}

/// What went wrong receiving an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missed {
    /// The subscriber fell behind and this many events were lost.
    Lagged(u64),
    /// The publisher has gone.
    Closed,
}

/// The publishing side, one per [`crate::State`].
pub struct Events {
    sender: broadcast::Sender<(u64, LiveEvent)>,
    /// The last sequence number sent. Held while sending, so numbers go out in order.
    seq: Mutex<u64>,
    /// Whether [`Events::close`] was called.
    closed: watch::Sender<bool>,
}

impl Default for Events {
    fn default() -> Self {
        Events {
            sender: broadcast::channel(CAPACITY).0,
            seq: Mutex::new(0),
            closed: watch::Sender::new(false),
        }
    }
}

impl Events {
    /// Send `event` to every subscriber. With none, it goes nowhere.
    pub fn publish(&self, event: LiveEvent) {
        let mut seq = self.seq.lock().unwrap_or_else(|e| e.into_inner());
        *seq += 1;
        let _ = self.sender.send((*seq, event));
    }

    /// Receive every event published from now on.
    pub fn subscribe(&self) -> Subscription {
        Subscription {
            events: self.sender.subscribe(),
            closed: self.closed.subscribe(),
        }
    }

    /// End every subscription, as the server shuts down: each one's next receive is
    /// [`Missed::Closed`]. Events published afterwards still reach new subscribers.
    pub fn close(&self) {
        self.closed.send_replace(true);
    }
}

/// One subscriber's view of the events.
pub struct Subscription {
    events: broadcast::Receiver<(u64, LiveEvent)>,
    closed: watch::Receiver<bool>,
}

impl Subscription {
    /// The next event and its sequence number.
    pub async fn recv(&mut self) -> Result<(u64, LiveEvent), Missed> {
        tokio::select! {
            biased;
            _ = self.closed.wait_for(|c| *c) => Err(Missed::Closed),
            r = self.events.recv() => r.map_err(|e| match e {
                broadcast::error::RecvError::Lagged(n) => Missed::Lagged(n),
                broadcast::error::RecvError::Closed => Missed::Closed,
            }),
        }
    }

    /// The next event if one is waiting, without blocking.
    pub fn try_recv(&mut self) -> Option<Result<(u64, LiveEvent), Missed>> {
        if *self.closed.borrow() {
            return Some(Err(Missed::Closed));
        }
        match self.events.try_recv() {
            Ok(e) => Some(Ok(e)),
            Err(broadcast::error::TryRecvError::Empty) => None,
            Err(broadcast::error::TryRecvError::Lagged(n)) => Some(Err(Missed::Lagged(n))),
            Err(broadcast::error::TryRecvError::Closed) => Some(Err(Missed::Closed)),
        }
    }
}
