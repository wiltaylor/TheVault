//! What the web client shows about sync in the apps: `GET /api/app/sync` and `POST /api/app/sync`
//! (D19, D20, D27).
//!
//! Each app runs its own sync loop (the desktop's `sync_loop`, Android's core) and tells the
//! embedded server where it stands through a [`SyncSource`]. The embedded server adds what it
//! knows itself, the local writes waiting in the queue and the assets that failed to reach the
//! server and the notes held back for them ([`Waiting`], ticket 141), and the notes the server
//! refused, each with its reason (ticket 143), and folds the two into one of three states the
//! sidebar's sync icon shows: `synced`, `syncing` or `error`.

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
    /// The server is newer than this app and publishes a build of it: sync waits while the app
    /// updates itself (the version gate, `sync::gate`, D27).
    Updating,
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
            sync::Error::Updating { message, .. } => (SyncPhase::Updating, message.clone()),
            sync::Error::Http(_) | sync::Error::Transfer(_) => (SyncPhase::Offline, e.to_string()),
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
    /// The state of `phase` with `queued` local writes or stuck assets waiting.
    pub fn of(phase: SyncPhase, queued: usize) -> SyncState {
        match phase {
            SyncPhase::Starting | SyncPhase::Syncing | SyncPhase::Updating => SyncState::Syncing,
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
    /// What went wrong, for the phases that need words, and what a round that finished could not
    /// do, such as an asset that would not upload.
    pub message: Option<String>,
    /// When the last round succeeded, in milliseconds since the epoch.
    pub last_ok_ms: Option<u64>,
    /// When the last round ended, succeeded or not.
    pub last_attempt_ms: Option<u64>,
    /// The home server's address, once paired.
    pub server: Option<String>,
    /// What the version gate warns about while sync goes on: the server is newer than the app
    /// and the app cannot update to it (`sync::gate`, D27).
    pub warning: Option<String>,
}

impl SyncReport {
    /// The report of an app with no sync loop, or none yet.
    pub fn unpaired() -> SyncReport {
        SyncReport {
            phase: SyncPhase::Unpaired,
            message: Some("This device is not paired with a server.".to_owned()),
            last_ok_ms: None,
            last_attempt_ms: None,
            server: None,
            warning: None,
        }
    }

    /// The `GET /api/app/sync` answer with `waiting` waiting.
    pub fn to_json(&self, waiting: &Waiting) -> Value {
        let stuck: Vec<Value> = waiting
            .stuck
            .iter()
            .map(|f| {
                json!({
                    "id": f.id,
                    "filename": f.filename,
                    "size": f.size,
                    "error": f.error,
                    "attempts": f.attempts,
                    "last_ms": f.last_ms,
                    "next_ms": f.next_ms,
                })
            })
            .collect();
        let refused: Vec<Value> = waiting
            .refused
            .iter()
            .map(|h| json!({ "note_id": h.note_id, "reason": h.reason }))
            .collect();
        json!({
            "state": SyncState::of(self.phase, waiting.queued + waiting.stuck.len()),
            "phase": self.phase,
            "queued": waiting.queued,
            "held": waiting.held,
            "refused_notes": refused,
            "assets_waiting": waiting.stuck.len(),
            "stuck_assets": stuck,
            "last_ok_ms": self.last_ok_ms,
            "last_attempt_ms": self.last_attempt_ms,
            "last_error": self.message,
            "server": self.server,
            "warning": self.warning,
        })
    }
}

/// What waits for the home server, from the sync tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waiting {
    /// Local writes in the queue.
    pub queued: usize,
    /// Of those, the notes held back for an asset that has not reached the server.
    pub held: usize,
    /// Of those, the notes the server refused in the last round, each with the reason.
    pub refused: Vec<sync::Held>,
    /// The assets that failed to reach the server, to be tried again.
    pub stuck: Vec<sync::AssetFailure>,
}

impl Waiting {
    /// What waits in `queue`.
    pub fn of(queue: &sync::Queue) -> Waiting {
        let (refused, held): (Vec<sync::Held>, Vec<sync::Held>) = queue
            .held()
            .unwrap_or_default()
            .into_iter()
            .partition(|h| h.asset_id.is_none());
        Waiting {
            queued: queue.pending().map_or(0, |p| p.len()),
            held: held.len(),
            refused,
            stuck: queue.asset_failures().unwrap_or_default(),
        }
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
        assert_eq!(SyncState::of(SyncPhase::Updating, 0), SyncState::Syncing);
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
    fn a_stuck_asset_keeps_the_state_yellow_and_is_named_with_its_error() {
        let report = SyncReport {
            phase: SyncPhase::Synced,
            message: Some("1 asset could not be sent".to_owned()),
            last_ok_ms: None,
            last_attempt_ms: Some(1_000),
            server: Some("https://home".to_owned()),
            warning: None,
        };
        let waiting = Waiting {
            queued: 0,
            held: 0,
            refused: Vec::new(),
            stuck: vec![sync::AssetFailure {
                id: "01J00000000000000000000000".to_owned(),
                filename: "video.mp4".to_owned(),
                size: 72 * 1024 * 1024,
                error: "stalled".to_owned(),
                attempts: 3,
                last_ms: 900,
                next_ms: 2_000,
            }],
        };
        let out = report.to_json(&waiting);
        assert_eq!(out["state"], "syncing", "{out}");
        assert_eq!(out["assets_waiting"], 1, "{out}");
        assert_eq!(out["stuck_assets"][0]["filename"], "video.mp4", "{out}");
        assert_eq!(out["stuck_assets"][0]["error"], "stalled", "{out}");
        assert_eq!(out["stuck_assets"][0]["attempts"], 3, "{out}");
        assert_eq!(out["last_error"], "1 asset could not be sent", "{out}");
        assert_eq!(out["last_attempt_ms"], 1_000, "{out}");
        assert_eq!(out["last_ok_ms"], Value::Null, "{out}");
    }

    #[test]
    fn a_note_the_server_refused_is_named_with_its_reason() {
        let report = SyncReport {
            phase: SyncPhase::Synced,
            message: Some("1 note could not be sent to the server: note 01J".to_owned()),
            last_ok_ms: Some(1_000),
            last_attempt_ms: Some(1_000),
            server: Some("https://home".to_owned()),
            warning: None,
        };
        let waiting = Waiting {
            queued: 1,
            held: 0,
            refused: vec![sync::Held {
                note_id: "01J00000000000000000000001".to_owned(),
                asset_id: None,
                reason: "the server answered 400".to_owned(),
            }],
            stuck: Vec::new(),
        };
        let out = report.to_json(&waiting);
        assert_eq!(out["state"], "syncing", "{out}");
        assert_eq!(out["held"], 0, "{out}");
        assert_eq!(
            out["refused_notes"],
            json!([{ "note_id": "01J00000000000000000000001", "reason": "the server answered 400" }])
        );
        assert!(out["last_error"].as_str().unwrap().contains("01J"), "{out}");
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
        let (phase, _) = SyncPhase::of_error(&sync::Error::Transfer("stalled".into()));
        assert_eq!(phase, SyncPhase::Offline);
        let (phase, _) = SyncPhase::of_error(&sync::Error::Protocol("odd".into()));
        assert_eq!(phase, SyncPhase::Error);
    }
}
