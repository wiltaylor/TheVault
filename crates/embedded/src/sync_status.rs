//! What the web client shows about sync in the apps: `GET /api/app/sync` and `POST /api/app/sync`
//! (D19, D20, D27).
//!
//! Each app runs its own sync loop (the desktop's `sync_loop`, Android's core) and tells the
//! embedded server where it stands through a [`SyncSource`]. The embedded server adds what it
//! knows itself, the number of local writes waiting in the queue, and folds the two into one of
//! three states the sidebar's sync icon shows: `synced`, `syncing` or `error`.

use serde::Serialize;
use serde_json::{Value, json};

/// Where an app's sync loop stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    /// No home server yet.
    Unpaired,
    /// Paired, before the first round has finished.
    Starting,
    /// A round is running.
    Syncing,
    /// The last round finished.
    Synced,
    /// The server does not answer; changes wait in the queue.
    Offline,
    /// The server is newer than this app: sync stops until the app is updated (D27).
    UpdateRequired,
    /// The server refused the device token: the device was revoked.
    Revoked,
    /// The last round failed for another reason.
    Error,
}

impl SyncPhase {
    /// What a failed round means for the user, and the words that say so.
    pub fn of_error(e: &sync::Error) -> (SyncPhase, String) {
        match e {
            sync::Error::ServerNewer { .. } => (
                SyncPhase::UpdateRequired,
                format!("Update the app to keep syncing. {e}"),
            ),
            sync::Error::Http(_) => (SyncPhase::Offline, e.to_string()),
            sync::Error::Server { status, .. } if *status == 401 || *status == 403 => (
                SyncPhase::Revoked,
                "The server refused this device's token: it was revoked. Your changes wait \
                 here; pair the device again to sync them."
                    .to_owned(),
            ),
            other => (SyncPhase::Error, other.to_string()),
        }
    }
}

/// The three states the sync icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Green: the last round succeeded and nothing waits.
    Synced,
    /// Yellow: a round runs, or local writes wait for the next one.
    Syncing,
    /// Red: the last round failed, the server cannot be reached, or the device is not paired.
    Error,
}

impl SyncState {
    /// The state of `phase` with `queued` local writes waiting.
    pub fn of(phase: SyncPhase, queued: usize) -> SyncState {
        match phase {
            SyncPhase::Starting | SyncPhase::Syncing => SyncState::Syncing,
            SyncPhase::Synced if queued > 0 => SyncState::Syncing,
            SyncPhase::Synced => SyncState::Synced,
            SyncPhase::Unpaired
            | SyncPhase::Offline
            | SyncPhase::UpdateRequired
            | SyncPhase::Revoked
            | SyncPhase::Error => SyncState::Error,
        }
    }
}

/// What an app's sync loop says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub phase: SyncPhase,
    /// What went wrong, for the phases that need words.
    pub message: Option<String>,
    /// When the last round succeeded, in milliseconds since the epoch.
    pub last_ok_ms: Option<u64>,
    /// The home server's address, once paired.
    pub server: Option<String>,
}

impl SyncReport {
    /// The report of an app with no sync loop, or none yet.
    pub fn unpaired() -> SyncReport {
        SyncReport {
            phase: SyncPhase::Unpaired,
            message: Some("This device is not paired with a server.".to_owned()),
            last_ok_ms: None,
            server: None,
        }
    }

    /// The `GET /api/app/sync` answer with `queued` local writes waiting.
    pub fn to_json(&self, queued: usize) -> Value {
        json!({
            "state": SyncState::of(self.phase, queued),
            "phase": self.phase,
            "queued": queued,
            "last_ok_ms": self.last_ok_ms,
            "last_error": self.message,
            "server": self.server,
        })
    }
}

/// An app's sync loop, as the embedded server sees it.
pub trait SyncSource: Send + Sync {
    /// Where the loop stands now.
    fn report(&self) -> SyncReport;
    /// Run a round as soon as possible: the "Sync now" button.
    fn sync_now(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_is_green_only_when_the_last_round_succeeded_and_nothing_waits() {
        assert_eq!(SyncState::of(SyncPhase::Synced, 0), SyncState::Synced);
        assert_eq!(SyncState::of(SyncPhase::Synced, 2), SyncState::Syncing);
        assert_eq!(SyncState::of(SyncPhase::Syncing, 0), SyncState::Syncing);
        assert_eq!(SyncState::of(SyncPhase::Starting, 0), SyncState::Syncing);
        for phase in [
            SyncPhase::Unpaired,
            SyncPhase::Offline,
            SyncPhase::UpdateRequired,
            SyncPhase::Revoked,
            SyncPhase::Error,
        ] {
            assert_eq!(SyncState::of(phase, 0), SyncState::Error, "{phase:?}");
        }
    }

    #[test]
    fn errors_say_what_the_user_can_do() {
        let (phase, message) = SyncPhase::of_error(&sync::Error::ServerNewer {
            server_protocol: 2,
            server_format: 3,
            protocol: 1,
            format: 2,
        });
        assert_eq!(phase, SyncPhase::UpdateRequired);
        assert!(message.starts_with("Update the app"), "{message}");
        let (phase, message) = SyncPhase::of_error(&sync::Error::Http("refused".into()));
        assert_eq!(phase, SyncPhase::Offline);
        assert!(message.contains("cannot reach the server"), "{message}");
        let (phase, _) = SyncPhase::of_error(&sync::Error::Server {
            what: "pushing".into(),
            status: 401,
            body: String::new(),
        });
        assert_eq!(phase, SyncPhase::Revoked);
        let (phase, _) = SyncPhase::of_error(&sync::Error::Protocol("odd".into()));
        assert_eq!(phase, SyncPhase::Error);
    }
}
