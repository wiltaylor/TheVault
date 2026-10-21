//! The local vault an app keeps, and how joining another vault starts it fresh (tickets 143 and
//! 145; D20, D29). The Android core and the desktop app share it.
//!
//! The local vault records the id of the home server's vault it is a copy of
//! ([`sync::Queue::server_vault`]). When a new pairing names a different vault
//! ([`LocalVault::joins_another_vault`]), or a round's handshake does ([`sync::Error::OtherVault`]),
//! the app calls [`LocalVault::start_fresh`]: the local vault folder moves aside, to
//! `vault.prev-<milliseconds>` beside it, with its queue, cursors and device events, and an empty
//! vault opens in its place, which the next round fills from the server whole. Nothing queued for
//! the other vault reaches this one. The fresh vault records the move
//! ([`sync::Queue::fresh_start`]), which `GET /api/app` answers as `fresh_start` and Settings >
//! Devices shows. A folder moved aside is removed once it is 7 days old ([`prune_previous`]), when
//! the app opens the vault ([`LocalVault::open`]) and after each round ([`LocalVault::prune`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::{Config, Embedded};

/// The name of a local vault folder moved aside by a fresh start, before its time in
/// milliseconds: `vault.prev-<ms>`.
pub const PREVIOUS: &str = "vault.prev-";

/// How long a local vault moved aside is kept.
pub const KEEP_PREVIOUS: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Where a fresh start moves the local vault folder `vault` at `at_ms`: `vault.prev-<at_ms>`
/// beside it.
pub fn previous_dir(vault: &Path, at_ms: i64) -> PathBuf {
    vault.with_file_name(format!("{PREVIOUS}{at_ms}"))
}

/// What [`prune_previous`] did.
#[derive(Debug, Default)]
pub struct Pruned {
    /// The folders removed.
    pub removed: Vec<PathBuf>,
    /// The folders old enough to go that could not be removed, with why.
    pub kept: Vec<(PathBuf, std::io::Error)>,
}

/// Remove the local vaults a fresh start moved aside beside `vault` that are 7 days old or more
/// at `now`.
pub fn prune_previous(vault: &Path, now: SystemTime) -> Pruned {
    let mut pruned = Pruned::default();
    let Some(parent) = vault.parent() else {
        return pruned;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return pruned;
    };
    let now_ms = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(at_ms) = name
            .to_str()
            .and_then(|n| n.strip_prefix(PREVIOUS))
            .and_then(|ms| ms.parse::<i64>().ok())
        else {
            continue;
        };
        if now_ms - at_ms < KEEP_PREVIOUS.as_millis() as i64 {
            continue;
        }
        let path = entry.path();
        match std::fs::remove_dir_all(&path) {
            Ok(()) => pruned.removed.push(path),
            Err(e) => pruned.kept.push((path, e)),
        }
    }
    pruned
}

/// A fresh start that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshStart {
    /// When the local vault moved, in milliseconds since the epoch.
    pub at_ms: i64,
    /// Where the old local vault went.
    pub previous: PathBuf,
    /// The vault the old local vault was a copy of, when it recorded one.
    pub from_vault: Option<String>,
    /// The vault the fresh local vault is a copy of.
    pub to_vault: String,
}

/// An app's local vault: its folder and the embedded server open on it, which a fresh start
/// replaces. Take the server again for each use ([`LocalVault::server`]) rather than keeping it.
pub struct LocalVault {
    dir: PathBuf,
    server: RwLock<Arc<Embedded>>,
}

/// The embedded server of the local vault folder `dir`, as both apps open it.
fn open_embedded(dir: &Path) -> anyhow::Result<Embedded> {
    Embedded::open(Config {
        vault: dir.to_owned(),
        web: None,
        secret_key: None,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl LocalVault {
    /// Open the local vault folder `dir`, creating it when it is missing, after removing the
    /// folders fresh starts moved aside 7 days ago or more. Answers what that removed, too.
    pub fn open(dir: &Path) -> anyhow::Result<(LocalVault, Pruned)> {
        std::fs::create_dir_all(dir)?;
        let pruned = prune_previous(dir, SystemTime::now());
        let server = Arc::new(open_embedded(dir)?);
        Ok((
            LocalVault {
                dir: dir.to_owned(),
                server: RwLock::new(server),
            },
            pruned,
        ))
    }

    /// The local vault folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The embedded server of the local vault as it is now.
    pub fn server(&self) -> Arc<Embedded> {
        self.server
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether a pairing with the home server's vault `vault_id` needs a fresh local vault: the
    /// local vault is a copy of another vault, or it synced before vault ids were recorded. A
    /// local vault that never synced, such as one with only a share made before pairing, takes
    /// `vault_id` as its own instead and answers `false`.
    pub fn joins_another_vault(&self, vault_id: &str) -> anyhow::Result<bool> {
        let server = self.server();
        let queue = server.queue();
        match queue.server_vault()? {
            Some(recorded) => Ok(recorded != vault_id),
            None if queue.has_synced()? => Ok(true),
            None => {
                queue.set_server_vault(vault_id)?;
                Ok(false)
            }
        }
    }

    /// Start a fresh local vault for the home server's vault `vault_id`: move the folder aside to
    /// `vault.prev-<ms>` and open an empty one in its place, paired as the old one was and in the
    /// same app build, recording `vault_id` and the move. `ready` runs on the fresh server before
    /// it replaces the old one, for what the app hands every server, such as its sync source. The
    /// old server closes once the last caller holding it lets go. Call it while no round runs.
    pub fn start_fresh(
        &self,
        vault_id: &str,
        ready: impl FnOnce(&Arc<Embedded>) -> anyhow::Result<()>,
    ) -> anyhow::Result<FreshStart> {
        let old = self.server();
        let from_vault = old.queue().server_vault().ok().flatten();
        let at_ms = now_ms();
        let previous = previous_dir(&self.dir, at_ms);
        std::fs::rename(&self.dir, &previous).map_err(|e| {
            anyhow::anyhow!(
                "moving the local vault {} aside to {}: {e}",
                self.dir.display(),
                previous.display()
            )
        })?;
        std::fs::create_dir_all(&self.dir)?;
        let fresh = Arc::new(open_embedded(&self.dir)?);
        fresh.set_remote(old.remote());
        if let Some(build) = old.app_build() {
            fresh.set_app(build);
        }
        let queue = fresh.queue();
        queue.set_server_vault(vault_id)?;
        queue.set_fresh_start(&json!({
            "at_ms": at_ms,
            "previous": previous.display().to_string(),
            "from_vault": from_vault,
            "to_vault": vault_id,
        }))?;
        ready(&fresh)?;
        *self.server.write().unwrap_or_else(PoisonError::into_inner) = fresh;
        drop(old);
        Ok(FreshStart {
            at_ms,
            previous,
            from_vault,
            to_vault: vault_id.to_owned(),
        })
    }

    /// Remove the folders fresh starts moved aside 7 days ago or more.
    pub fn prune(&self) -> Pruned {
        prune_previous(&self.dir, SystemTime::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_moved_aside_go_after_seven_days_and_others_stay() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let day: i64 = 24 * 60 * 60 * 1000;
        let now_ms: i64 = 100 * day;
        let old = previous_dir(&vault, now_ms - 8 * day);
        let recent = previous_dir(&vault, now_ms - day);
        let other = dir.path().join("vault.prev-notanumber");
        std::fs::create_dir_all(old.join(".vault")).unwrap();
        std::fs::create_dir_all(&recent).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let now = UNIX_EPOCH + Duration::from_millis(now_ms as u64);
        let pruned = prune_previous(&vault, now);
        assert_eq!(pruned.removed, vec![old.clone()]);
        assert!(pruned.kept.is_empty());
        assert!(!old.exists());
        assert!(recent.exists() && other.exists() && vault.exists());
    }

    #[test]
    fn a_fresh_start_moves_the_vault_aside_and_records_it() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let (local, _) = LocalVault::open(&vault).unwrap();
        // Never synced: the first vault id is taken as the local vault's own.
        assert!(!local.joins_another_vault("a").unwrap());
        assert!(!local.joins_another_vault("a").unwrap());
        assert!(local.joins_another_vault("b").unwrap());
        std::fs::write(vault.join("kept.txt"), "old").unwrap();
        let mut readied = false;
        let fresh = local
            .start_fresh("b", |_| {
                readied = true;
                Ok(())
            })
            .unwrap();
        assert!(readied);
        assert_eq!(fresh.from_vault.as_deref(), Some("a"));
        assert_eq!(fresh.to_vault, "b");
        assert_eq!(fresh.previous, previous_dir(&vault, fresh.at_ms));
        assert!(fresh.previous.join("kept.txt").is_file());
        assert!(!vault.join("kept.txt").exists());
        let queue = local.server().queue().clone();
        assert_eq!(queue.server_vault().unwrap().as_deref(), Some("b"));
        let record = queue.fresh_start().unwrap().unwrap();
        assert_eq!(record["from_vault"], "a");
        assert_eq!(record["to_vault"], "b");
        assert_eq!(record["previous"], fresh.previous.display().to_string());
        assert!(!local.joins_another_vault("b").unwrap());
    }
}
