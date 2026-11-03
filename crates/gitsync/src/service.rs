//! The connection as the server runs it (git-sync D18, D20): one round at a time, on a thread of
//! its own, with a `running` flag the scheduler and the API share.
//!
//! A round that ends records its time and report in [`STATE`](crate::settings::STATE), and an
//! activity entry `gitsync.round` unless it did nothing. A failed round records its error, and
//! notifies the user only when the error text differs from the one last notified. A successful
//! round clears the error, pins an SSH host key learned under `accept-new` (D19), and removes the
//! branch this instance pushed under a previous sync key (D14).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use state::{NewActivity, NewNotification};
use writer::Writer;

use crate::round::{Paths, Report, remove_branch, run_round};
use crate::settings::{
    KNOWN_HOSTS, SECRETS, Settings, Status, load_secrets, load_settings, load_status, save_status,
};
use crate::{Error, PART_BYTES, Result, clip};

/// What the server sets for every round.
#[derive(Debug, Clone)]
pub struct Options {
    pub git_bin: Option<PathBuf>,
    /// `VAULT_GITSYNC_SSH`, the test seam for SSH.
    pub ssh_bin: Option<PathBuf>,
    /// The largest asset part; tests make it small.
    pub part_bytes: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            git_bin: None,
            ssh_bin: None,
            part_bytes: PART_BYTES,
        }
    }
}

/// The git sync connection of one vault.
pub struct GitSync {
    writer: Arc<Writer>,
    options: Options,
    running: AtomicBool,
}

/// Clears the running flag when the round ends, however it ends.
struct Claim<'a>(&'a AtomicBool);

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl GitSync {
    pub fn new(writer: Arc<Writer>, options: Options) -> Arc<GitSync> {
        Arc::new(GitSync {
            writer,
            options,
            running: AtomicBool::new(false),
        })
    }

    pub fn writer(&self) -> &Arc<Writer> {
        &self.writer
    }

    /// Whether a round is running.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Where the rounds keep their clone, with the server's programs and part size.
    pub fn paths(&self) -> Paths {
        Paths {
            git_bin: self.options.git_bin.clone(),
            ssh_bin: self.options.ssh_bin.clone(),
            part_bytes: self.options.part_bytes,
            ..Paths::of(&self.writer)
        }
    }

    /// The vault folder's name, the instance name when none is set (D28).
    pub fn folder_name(&self) -> String {
        self.writer
            .vault()
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "vault".to_owned())
    }

    /// The saved settings with their defaults filled in, or `None` before any were saved.
    pub fn settings(&self) -> Result<Option<Settings>> {
        Ok(load_settings(self.writer.state())?.map(|s| s.completed(&self.folder_name())))
    }

    /// Whether a round can run: settings with a remote, and a sync key.
    pub fn configured(&self) -> Result<bool> {
        let Some(settings) = self.settings()? else {
            return Ok(false);
        };
        Ok(!settings.remote_url.trim().is_empty()
            && load_secrets(self.writer.state())?.key.is_some())
    }

    fn claim(&self) -> Option<Claim<'_>> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Claim(&self.running))
    }

    /// Start a round on a thread of its own. `false`, starting nothing, while one is running.
    /// The flag is set before this returns, so a status read straight after sees it.
    pub fn start(self: &Arc<Self>) -> bool {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let g = self.clone();
        let spawned = std::thread::Builder::new()
            .name("gitsync".into())
            .spawn(move || {
                let _claim = Claim(&g.running);
                let _ = g.round();
            });
        if spawned.is_err() {
            self.running.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// Run a round on this thread and return its report or error. `None` while one is running.
    pub fn run(&self) -> Option<std::result::Result<Report, String>> {
        let _claim = self.claim()?;
        Some(self.round())
    }

    /// One round, with its outcome recorded.
    fn round(&self) -> std::result::Result<Report, String> {
        let w = &self.writer;
        let state = w.state();
        let paths = self.paths();
        let settings = self.settings();
        let outcome = match &settings {
            Ok(Some(settings)) => load_secrets(state).and_then(|secrets| {
                run_round(w, settings, &secrets, &paths).map(|report| (secrets, report))
            }),
            Ok(None) => Err(Error::Sync("git sync is not set up".into())),
            Err(e) => Err(Error::Sync(e.to_string())),
        };
        let name = match &settings {
            Ok(Some(s)) => s.name.clone(),
            _ => self.folder_name(),
        };
        let mut status = load_status(state).unwrap_or_default();
        status.last_run = Some(rfc3339(w.now_ms()));
        let result = match (outcome, settings) {
            (Ok((secrets, mut report)), Ok(Some(settings))) => {
                if let Some(lines) = report.learned_hosts.take() {
                    match state.set_secret(KNOWN_HOSTS, &lines) {
                        Ok(()) => {
                            status.host_fingerprint = lines
                                .lines()
                                .find_map(|l| crate::sshkey::host_fingerprint(l).ok());
                        }
                        Err(e) => report
                            .warnings
                            .push(format!("the SSH host key could not be pinned: {e}")),
                    }
                }
                if let Some(old) = status.branch.clone().filter(|b| *b != report.branch)
                    && let Err(e) = remove_branch(&settings, &secrets, &paths, &old)
                {
                    report.warnings.push(format!(
                        "the branch written under the previous sync key was not removed: {e}"
                    ));
                }
                status.branch = Some(report.branch.clone());
                let previous = status.last_report.take();
                let news = previous.as_ref().is_none_or(|p| {
                    p.skipped_branches != report.skipped_branches || p.warnings != report.warnings
                });
                if report.changed()
                    || (news && !(report.skipped_branches.is_empty() && report.warnings.is_empty()))
                {
                    state.record_activity(
                        NewActivity::by(
                            "gitsync.round",
                            "sync",
                            "gitsync",
                            &name,
                            report.summary(),
                        )
                        .detail(serde_json::to_value(&report).unwrap_or_default()),
                    );
                }
                status.last_error = None;
                status.notified_error = None;
                status.last_report = Some(report.clone());
                Ok(report)
            }
            (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
            (Ok(_), Ok(None)) => Err("git sync is not set up".to_owned()),
        };
        if let Err(message) = &result {
            let first = message.lines().next().unwrap_or("").trim();
            state.record_activity(
                NewActivity::by(
                    "gitsync.round",
                    "sync",
                    "gitsync",
                    &name,
                    format!("Git sync failed: {first}"),
                )
                .detail(json!({ "error": state::one_line(message, 300) })),
            );
            if status.notified_error.as_deref() != Some(message.as_str()) {
                let n = NewNotification {
                    title: clip(&format!("Git sync failed: {first}"), state::MAX_TITLE),
                    body: Some(clip(message, state::MAX_BODY)).filter(|b| b != first),
                    note_id: None,
                    url: Some("/settings/sync".to_owned()),
                    source: "system".to_owned(),
                };
                if let Err(e) = state.add_notification(&n) {
                    eprintln!("gitsync: recording the failure notification: {e}");
                }
                status.notified_error = Some(message.clone());
            }
            status.last_error = Some(message.clone());
        }
        if let Err(e) = save_status(state, &status) {
            eprintln!("gitsync: saving the round's status: {e}");
        }
        result
    }

    /// Disconnect: remove every `gitsync_*` secret, the clone and what was kept of each synced
    /// note. Notes already synced stay.
    pub fn disconnect(&self) -> Result<()> {
        let state = self.writer.state();
        for name in SECRETS {
            match state.delete_secret(name) {
                Ok(()) | Err(state::Error::UnknownSecret(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        match std::fs::remove_dir_all(self.paths().dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        state.gitsync_clear()?;
        Ok(())
    }

    /// The last rounds' status.
    pub fn status(&self) -> Result<Status> {
        load_status(self.writer.state())
    }
}

/// Milliseconds since the epoch as RFC 3339 in UTC, ending `Z`.
fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
