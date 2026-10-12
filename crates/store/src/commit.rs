//! Atomic multi-file commits (D4).
//!
//! A commit is a redo journal in `.vault/txn/`:
//!
//! 1. Every new file is written to `.vault/txn/<n>` and synced.
//! 2. `manifest.json` lists every write and delete. It is written to a temporary name, synced and
//!    renamed into place. That rename is the commit point.
//! 3. Each staged file is renamed over its target and each delete is applied.
//! 4. The journal is removed.
//!
//! Recovery runs when a vault is opened and before every commit. A journal with a manifest is
//! rolled forward: applying an operation twice is harmless, because a staged file that is gone
//! was already renamed into place. A journal without one is discarded, since no target has been
//! touched yet. Either way a crash leaves every file of a commit changed or none of them.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::ops::ControlFlow;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result, Vault, canonical, io_err};

const TXN_DIR: &str = "txn";
const MANIFEST: &str = "manifest.json";
const LOCK_FILE: &str = "commit.lock";

/// One file change in a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Create or replace a file. A `.wcl` file must be data-only and is stored in canonical form.
    Write { path: String, contents: Vec<u8> },
    /// Remove a file. It must exist.
    Delete { path: String },
}

impl Change {
    pub fn write(path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Change {
        Change::Write {
            path: path.into(),
            contents: contents.into(),
        }
    }

    pub fn delete(path: impl Into<String>) -> Change {
        Change::Delete { path: path.into() }
    }

    fn path(&self) -> &str {
        match self {
            Change::Write { path, .. } | Change::Delete { path } => path,
        }
    }
}

/// A point inside a commit, reported to the hook of [`Vault::commit_with_hook`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStep {
    /// The new contents of change `n` are staged in the journal.
    Staged(usize),
    /// The manifest is in place: the commit will now happen even across a crash.
    Committed,
    /// Change `n` has been applied to its target.
    Applied(usize),
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    ops: Vec<Op>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Op {
    Write { path: String, staged: String },
    Delete { path: String },
}

impl Vault {
    /// Apply every change or none of them, even across a crash.
    pub fn commit(&self, changes: Vec<Change>) -> Result<()> {
        self.commit_with_hook(changes, &mut |_| ControlFlow::Continue(()))
    }

    /// [`Vault::commit`], calling `hook` at each [`CommitStep`]. A hook that breaks stops the
    /// commit where it stands, without cleaning up, the way a crash would, and the commit returns
    /// [`Error::Interrupted`]. For fault-injection tests.
    #[doc(hidden)]
    pub fn commit_with_hook(
        &self,
        changes: Vec<Change>,
        hook: &mut dyn FnMut(CommitStep) -> ControlFlow<()>,
    ) -> Result<()> {
        let changes = self.prepare(changes)?;
        let _guard = self.commit_lock.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = self.lock_journal()?;
        recover_locked(self)?;

        let txn = self.server_dir().join(TXN_DIR);
        std::fs::create_dir_all(&txn).map_err(io_err(&txn))?;
        let staged = (|| {
            let mut ops = Vec::with_capacity(changes.len());
            for (n, change) in changes.into_iter().enumerate() {
                match change {
                    Change::Write { path, contents } => {
                        let name = n.to_string();
                        write_synced(&txn.join(&name), &contents)?;
                        ops.push(Op::Write { path, staged: name });
                    }
                    Change::Delete { path } => ops.push(Op::Delete { path }),
                }
                if hook(CommitStep::Staged(n)).is_break() {
                    return Ok(None);
                }
            }
            let manifest =
                serde_json::to_vec(&Manifest { ops }).expect("a manifest always serialises");
            let tmp = txn.join("manifest.tmp");
            write_synced(&tmp, &manifest)?;
            let path = txn.join(MANIFEST);
            std::fs::rename(&tmp, &path).map_err(io_err(&path))?;
            sync_dir(&txn)?;
            Ok(Some(()))
        })();
        match staged {
            Ok(Some(())) => {}
            Ok(None) => return Err(Error::Interrupted),
            Err(e) => {
                // Nothing outside the journal was touched, so dropping it undoes the commit.
                let _ = std::fs::remove_dir_all(&txn);
                return Err(e);
            }
        }
        if hook(CommitStep::Committed).is_break() {
            return Err(Error::Interrupted);
        }
        roll_forward(self, hook)
    }

    /// Check every change and put each `.wcl` write into canonical form before anything touches
    /// the disk.
    fn prepare(&self, changes: Vec<Change>) -> Result<Vec<Change>> {
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(changes.len());
        for change in changes {
            let path = change.path().to_owned();
            if !seen.insert(path.clone()) {
                return Err(Error::DuplicatePath(path));
            }
            let real = self.contain(&self.resolve(&path)?)?;
            out.push(match change {
                Change::Write { path, contents } if path.ends_with(".wcl") => {
                    let text =
                        String::from_utf8(contents).map_err(|_| Error::NotUtf8(path.clone()))?;
                    let contents = canonical(&text, &path)?.into_bytes();
                    Change::Write { path, contents }
                }
                Change::Delete { path } if !real.is_file() => return Err(Error::NotFound(path)),
                other => other,
            });
        }
        Ok(out)
    }

    fn lock_journal(&self) -> Result<File> {
        let path = self.server_dir().join(LOCK_FILE);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io_err(&path))?;
        file.lock().map_err(io_err(&path))?;
        Ok(file)
    }
}

/// Finish or discard a commit an earlier process left behind.
pub(crate) fn recover(vault: &Vault) -> Result<()> {
    let _guard = vault.commit_lock.lock().unwrap_or_else(|e| e.into_inner());
    let _lock = vault.lock_journal()?;
    recover_locked(vault)
}

fn recover_locked(vault: &Vault) -> Result<()> {
    let txn = vault.server_dir().join(TXN_DIR);
    if !txn.exists() {
        return Ok(());
    }
    if txn.join(MANIFEST).is_file() {
        roll_forward(vault, &mut |_| ControlFlow::Continue(()))
    } else {
        std::fs::remove_dir_all(&txn).map_err(io_err(&txn))
    }
}

/// Apply the manifest in the journal, then remove the journal.
fn roll_forward(vault: &Vault, hook: &mut dyn FnMut(CommitStep) -> ControlFlow<()>) -> Result<()> {
    let txn = vault.server_dir().join(TXN_DIR);
    let manifest_path = txn.join(MANIFEST);
    let bytes = std::fs::read(&manifest_path).map_err(io_err(&manifest_path))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| Error::Corrupt(e.to_string()))?;
    for (n, op) in manifest.ops.iter().enumerate() {
        match op {
            Op::Write { path, staged } => {
                let target = vault.resolve(path)?;
                let from = txn.join(staged);
                if from.exists() {
                    let dir = target.parent().expect("a vault path has a parent");
                    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
                    std::fs::rename(&from, &target).map_err(io_err(&target))?;
                    sync_dir(dir)?;
                }
            }
            Op::Delete { path } => {
                let target = vault.resolve(path)?;
                match std::fs::remove_file(&target) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(io_err(&target)(e));
                    }
                    _ => {}
                }
                if let Some(dir) = target.parent().filter(|d| d.exists()) {
                    sync_dir(dir)?;
                }
            }
        }
        if hook(CommitStep::Applied(n)).is_break() {
            return Err(Error::Interrupted);
        }
    }
    // The manifest goes first: a crash after this leaves a journal with nothing to redo.
    std::fs::remove_file(&manifest_path).map_err(io_err(&manifest_path))?;
    sync_dir(&txn)?;
    std::fs::remove_dir_all(&txn).map_err(io_err(&txn))
}

fn write_synced(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = File::create(path).map_err(io_err(path))?;
    file.write_all(contents).map_err(io_err(path))?;
    file.sync_all().map_err(io_err(path))
}

fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(io_err(dir))
}
