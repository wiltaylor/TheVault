//! The sync loop: when the desktop app syncs with its home server, and what it shows about it
//! (D19, D21, D27).
//!
//! One thread runs [`sync::Engine`] rounds, since the engine blocks. A round runs when the app
//! starts, on a timer, when the server answers again after being unreachable, a moment after
//! each local write, a moment after the live link ([`crate::live`]) sees a note change on the
//! server that the local vault does not hold yet, and when the user asks. The desktop keeps every asset
//! ([`sync::AssetPolicy::All`]).
//!
//! Before each round the loop checks that the server answers at all, with a short timeout. While
//! it does not, the app is offline: local writes wait in the queue and the check repeats every
//! few seconds, so the round after the network or the server comes back runs within seconds.
//!
//! Once the loop has the device token it hands the server and the token to the live link
//! ([`crate::live`]), which reads the token from nowhere else.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use embedded::Embedded;
use serde::Serialize;

use crate::device::{Device, TokenStore};
use crate::live::Remote;

/// How often a paired device syncs while nothing else asks it to.
const INTERVAL: Duration = Duration::from_secs(60);

/// How often an offline device checks whether the server answers again.
const OFFLINE_RETRY: Duration = Duration::from_secs(3);

/// How long the reachability check waits for the server.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long after a local write the round runs, so a burst of saves syncs once.
const AFTER_WRITE: Duration = Duration::from_millis(750);

/// How long after a change on the server the round runs, so a burst of changes pulls once.
const AFTER_REMOTE: Duration = Duration::from_secs(1);

/// Far enough away to mean "not until asked".
const NEVER: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// Why the loop should look again.
pub enum Trigger {
    /// A write reached the local vault.
    LocalWrite,
    /// The user asked to sync now.
    Now,
    /// A note changed on the server, or the live link reconnected (`None`).
    ServerChanged(Option<sync::live::NoteChanged>),
    /// Setup paired the device.
    Paired(Device, String),
}

/// Where sync stands: the phases the embedded server reports to the web client's sync icon.
pub use embedded::SyncPhase as Phase;

/// What the app shows about sync.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub paired: bool,
    pub server_url: Option<String>,
    pub device_name: Option<String>,
    pub phase: Phase,
    /// What went wrong, for the phases that need words.
    pub message: Option<String>,
    /// Local changes waiting for the server.
    pub pending: usize,
    /// When the last round finished, in milliseconds since the epoch.
    pub last_sync_ms: Option<u64>,
    /// The device name the setup page suggests.
    pub default_name: String,
}

/// The loop's handle: send it triggers and read its status.
pub struct SyncLoop {
    tx: Mutex<Sender<Trigger>>,
    status: Arc<Mutex<Status>>,
    embedded: Arc<Embedded>,
}

impl SyncLoop {
    /// Starts the loop for the local vault behind `embedded`, paired as `device` when it is. The
    /// first round runs at once.
    pub fn start(
        embedded: Arc<Embedded>,
        device: Option<Device>,
        tokens: Box<dyn TokenStore>,
        remote: tokio::sync::watch::Sender<Option<Remote>>,
    ) -> SyncLoop {
        let status = Arc::new(Mutex::new(Status {
            paired: device.is_some(),
            server_url: device.as_ref().map(|d| d.server_url.clone()),
            device_name: device.as_ref().map(|d| d.device_name.clone()),
            phase: if device.is_some() {
                Phase::Starting
            } else {
                Phase::Unpaired
            },
            message: None,
            pending: 0,
            last_sync_ms: None,
            default_name: crate::device::default_name(),
        }));
        let (tx, rx) = channel();
        let worker = Worker {
            embedded: embedded.clone(),
            status: status.clone(),
            tokens,
            device,
            engine: None,
            remote,
        };
        std::thread::Builder::new()
            .name("sync".to_owned())
            .spawn(move || worker.run(rx))
            .expect("the sync thread starts");
        SyncLoop {
            tx: Mutex::new(tx),
            status,
            embedded,
        }
    }

    /// The loop as the embedded server's sync source, for the web client's sync icon.
    pub fn source(&self) -> Source {
        Source {
            tx: Mutex::new(lock(&self.tx).clone()),
            status: self.status.clone(),
        }
    }

    pub fn trigger(&self, trigger: Trigger) {
        let _ = lock(&self.tx).send(trigger);
    }

    /// The status, with the queue counted now.
    pub fn status(&self) -> Status {
        let mut status = lock(&self.status).clone();
        status.pending = self
            .embedded
            .queue()
            .pending()
            .map_or(status.pending, |p| p.len());
        status
    }
}

/// What `GET /api/app/sync` reads of the loop, and how `POST /api/app/sync` asks it for a round.
pub struct Source {
    tx: Mutex<Sender<Trigger>>,
    status: Arc<Mutex<Status>>,
}

impl embedded::SyncSource for Source {
    fn report(&self) -> embedded::SyncReport {
        let s = lock(&self.status);
        embedded::SyncReport {
            phase: s.phase,
            message: s.message.clone(),
            last_ok_ms: s.last_sync_ms,
            server: s.server_url.clone(),
        }
    }

    fn sync_now(&self) {
        let _ = lock(&self.tx).send(Trigger::Now);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// How one round ended, and so what the app shows and when the next one runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub phase: Phase,
    pub message: Option<String>,
    pub next: Duration,
}

impl Outcome {
    fn new(phase: Phase, message: Option<String>, next: Duration) -> Outcome {
        Outcome {
            phase,
            message,
            next,
        }
    }

    /// What a failed round means for the user, and when the next one runs.
    pub fn of_error(e: &sync::Error) -> Outcome {
        let (phase, message) = Phase::of_error(e);
        let next = match phase {
            Phase::Offline => OFFLINE_RETRY,
            Phase::Revoked => NEVER,
            _ => INTERVAL,
        };
        Outcome::new(phase, Some(message), next)
    }
}

struct Worker {
    embedded: Arc<Embedded>,
    status: Arc<Mutex<Status>>,
    tokens: Box<dyn TokenStore>,
    device: Option<Device>,
    engine: Option<sync::Engine>,
    remote: tokio::sync::watch::Sender<Option<Remote>>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Trigger>) {
        let probe = reqwest::blocking::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .expect("an HTTP client builds");
        let mut due = Instant::now();
        loop {
            let wait = due.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(Trigger::LocalWrite) => {
                    due = due.min(Instant::now() + AFTER_WRITE);
                    continue;
                }
                Ok(Trigger::Now) => due = Instant::now(),
                Ok(Trigger::ServerChanged(change)) => {
                    let news = change.is_none_or(|c| {
                        self.device.is_some() && sync::live::is_news(self.embedded.queue(), &c)
                    });
                    if news {
                        due = due.min(Instant::now() + AFTER_REMOTE);
                    }
                    continue;
                }
                Ok(Trigger::Paired(device, token)) => {
                    self.engine = None;
                    self.build_engine(&device, token);
                    {
                        let mut s = lock(&self.status);
                        s.paired = true;
                        s.server_url = Some(device.server_url.clone());
                        s.device_name = Some(device.device_name.clone());
                        s.phase = Phase::Starting;
                        s.message = None;
                        s.last_sync_ms = None;
                    }
                    self.device = Some(device);
                    due = Instant::now();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() < due {
                continue;
            }
            let outcome = self.round(&probe);
            due = Instant::now() + outcome.next;
            let mut s = lock(&self.status);
            s.phase = outcome.phase;
            s.message = outcome.message;
            if outcome.phase == Phase::Synced {
                s.last_sync_ms = Some(now_ms());
            }
        }
    }

    fn build_engine(&mut self, device: &Device, token: String) -> Option<Outcome> {
        let remote = Remote {
            url: device.server_url.clone(),
            token: token.clone(),
        };
        let paired = device.remote(token);
        match self
            .embedded
            .sync_engine(paired.clone(), sync::AssetPolicy::desktop())
        {
            Ok(engine) => {
                self.engine = Some(engine);
                // The embedded server sends the home server's own routes, such as Settings >
                // Devices and the notifications, to the same server.
                self.embedded.set_remote(Some(paired));
                self.remote.send_replace(Some(remote));
                None
            }
            Err(e) => Some(Outcome::of_error(&e)),
        }
    }

    fn round(&mut self, probe: &reqwest::blocking::Client) -> Outcome {
        let Some(device) = self.device.clone() else {
            return Outcome::new(Phase::Unpaired, None, NEVER);
        };
        if self.engine.is_none() {
            let token = match self.tokens.get(&device.device_id) {
                Ok(token) => token,
                Err(e) => {
                    return Outcome::new(
                        Phase::Error,
                        Some(format!("Cannot read the device token: {e}")),
                        INTERVAL,
                    );
                }
            };
            if let Some(failed) = self.build_engine(&device, token) {
                return failed;
            }
        }
        let info = format!("{}/api/sync/info", device.server_url);
        if let Err(e) = probe.get(&info).send() {
            return Outcome::new(
                Phase::Offline,
                Some(format!("cannot reach the server: {e}")),
                OFFLINE_RETRY,
            );
        }
        lock(&self.status).phase = Phase::Syncing;
        let engine = self.engine.as_ref().expect("the engine was just built");
        match engine.sync() {
            Ok(_) => Outcome::new(Phase::Synced, None, INTERVAL),
            Err(e) => Outcome::of_error(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_server_asks_for_an_update_and_keeps_the_changes() {
        let outcome = Outcome::of_error(&sync::Error::ServerNewer {
            server_protocol: 2,
            server_format: 3,
            protocol: 1,
            format: 2,
        });
        assert_eq!(outcome.phase, Phase::UpdateRequired);
        let message = outcome.message.unwrap();
        assert!(message.starts_with("Update the app"), "{message}");
        assert!(message.contains("your changes wait"), "{message}");
    }

    #[test]
    fn an_unreachable_server_is_offline_and_retried_soon() {
        let outcome = Outcome::of_error(&sync::Error::Http("connection refused".into()));
        assert_eq!(outcome.phase, Phase::Offline);
        assert_eq!(outcome.next, OFFLINE_RETRY);
    }

    #[test]
    fn a_refused_token_means_revoked_and_stops_retrying() {
        let outcome = Outcome::of_error(&sync::Error::Server {
            what: "reading the server's sync info".into(),
            status: 401,
            body: "{}".into(),
        });
        assert_eq!(outcome.phase, Phase::Revoked);
        assert_eq!(outcome.next, NEVER);
    }
}
