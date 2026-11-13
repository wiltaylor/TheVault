//! The connection's settings, credentials and last round, all kept as secrets in `state.db`, so
//! none of them is in a vault file or the repo (git-sync D18).
//!
//! | Secret | Holds |
//! |---|---|
//! | [`SETTINGS`] | [`Settings`] as JSON |
//! | [`TOKEN`] | the HTTPS token (D24) |
//! | [`SSH_KEY`] | the OpenSSH private key (D19) |
//! | [`KNOWN_HOSTS`] | the pinned SSH host lines (D19) |
//! | [`KEY`] | the shared key in its `tvsync1-` form (D14) |
//! | [`STATE`] | [`Status`] as JSON: the last run, error and report, and the host fingerprint |

use serde::{Deserialize, Serialize};
use state::State;

use crate::round::Report;
use crate::{Error, Result, SyncKey};

pub const SETTINGS: &str = state::GITSYNC_SETTINGS;
pub const TOKEN: &str = state::GITSYNC_TOKEN;
pub const SSH_KEY: &str = state::GITSYNC_SSH_KEY;
pub const KNOWN_HOSTS: &str = state::GITSYNC_KNOWN_HOSTS;
pub const KEY: &str = state::GITSYNC_KEY;
pub const STATE: &str = state::GITSYNC_STATE;

/// Every secret the connection keeps, which a disconnect removes.
pub const SECRETS: [&str; 6] = [SETTINGS, TOKEN, SSH_KEY, KNOWN_HOSTS, KEY, STATE];

/// The schedule when none is set: every 15 minutes (D20).
pub const DEFAULT_SCHEDULE: &str = "*/15 * * * *";

/// The published size past which a round warns: 1 GiB (D25).
pub const DEFAULT_WARN_BYTES: u64 = 1 << 30;

/// The published size past which the push is refused: 4 GiB (D25).
pub const DEFAULT_MAX_BYTES: u64 = 4 << 30;

/// How the round signs in to the remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// A token over HTTPS (D24).
    #[default]
    Https,
    /// A deploy key over SSH (D19).
    Ssh,
}

/// The connection's settings. A missing key reads as its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub remote_url: String,
    pub auth: AuthKind,
    /// The HTTPS username, `x-access-token` when empty.
    pub https_username: String,
    /// Which local notes this instance publishes (D23).
    pub export_query: String,
    /// Which notes it takes from the other instances' published sets (D23).
    pub import_query: String,
    /// A five-field cron expression.
    pub schedule: String,
    /// The instance's display name, the vault folder's name when empty (D28).
    pub name: String,
    pub warn_bytes: u64,
    pub max_bytes: u64,
    pub paused: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            remote_url: String::new(),
            auth: AuthKind::Https,
            https_username: crate::git::DEFAULT_HTTPS_USERNAME.to_owned(),
            export_query: String::new(),
            import_query: String::new(),
            schedule: DEFAULT_SCHEDULE.to_owned(),
            name: String::new(),
            warn_bytes: DEFAULT_WARN_BYTES,
            max_bytes: DEFAULT_MAX_BYTES,
            paused: false,
        }
    }
}

impl Settings {
    /// Fill what was left empty: the name from `folder_name`, the username and the schedule from
    /// their defaults.
    pub fn completed(mut self, folder_name: &str) -> Settings {
        if self.name.trim().is_empty() {
            self.name = folder_name.to_owned();
        }
        if self.https_username.trim().is_empty() {
            self.https_username = crate::git::DEFAULT_HTTPS_USERNAME.to_owned();
        }
        if self.schedule.trim().is_empty() {
            self.schedule = DEFAULT_SCHEDULE.to_owned();
        }
        self
    }
}

/// The credentials a round uses.
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    pub token: Option<String>,
    pub ssh_key: Option<String>,
    /// The pinned host lines, empty while none is pinned.
    pub known_hosts: String,
    pub key: Option<SyncKey>,
}

/// What the last rounds left: shown in Settings > Sync, never a secret value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// When the last round ended, RFC 3339 in UTC.
    pub last_run: Option<String>,
    /// Why the last round failed; `None` once one succeeds.
    pub last_error: Option<String>,
    pub last_report: Option<Report>,
    /// The SHA256 fingerprint of the pinned SSH host key.
    pub host_fingerprint: Option<String>,
    /// The error text last notified, so a repeated error notifies once (D20).
    pub notified_error: Option<String>,
    /// The branch this instance last pushed, so a key rotation can remove the old one.
    pub branch: Option<String>,
}

fn read(state: &State, name: &str) -> Result<Option<String>> {
    if !state.has_secret_key() {
        return Ok(None);
    }
    Ok(state.get_secret(name)?)
}

fn json<T: for<'de> Deserialize<'de>>(state: &State, name: &str) -> Result<Option<T>> {
    match read(state, name)? {
        None => Ok(None),
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Error::Sync(format!("secret {name} is not valid: {e}"))),
    }
}

fn write_json<T: Serialize>(state: &State, name: &str, value: &T) -> Result<()> {
    let text = serde_json::to_string(value).map_err(|e| Error::Sync(e.to_string()))?;
    Ok(state.set_secret(name, &text)?)
}

/// The saved settings, or `None` before any were saved.
pub fn load_settings(state: &State) -> Result<Option<Settings>> {
    json(state, SETTINGS)
}

pub fn save_settings(state: &State, settings: &Settings) -> Result<()> {
    write_json(state, SETTINGS, settings)
}

/// The credentials, each `None` when not set. A sync key that does not parse reads as none.
pub fn load_secrets(state: &State) -> Result<Secrets> {
    Ok(Secrets {
        token: read(state, TOKEN)?.filter(|t| !t.is_empty()),
        ssh_key: read(state, SSH_KEY)?.filter(|k| !k.trim().is_empty()),
        known_hosts: read(state, KNOWN_HOSTS)?.unwrap_or_default(),
        key: read(state, KEY)?.and_then(|k| SyncKey::parse(&k).ok()),
    })
}

/// The last rounds' status, empty before the first.
pub fn load_status(state: &State) -> Result<Status> {
    Ok(json(state, STATE)?.unwrap_or_default())
}

pub fn save_status(state: &State, status: &Status) -> Result<()> {
    write_json(state, STATE, status)
}
