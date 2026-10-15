//! Atomic multi-file commits (D4).
//!
//! A commit is a redo journal in `.vault/txn/`:
//!
//! 1. Every new file is written to `.vault/txn/<n>` and synced.
//! 2. `manifest.json` lists every write and delete. It is written to a temporary name, synced and
//!    renamed into place. That rename is the commit point.
//! 3. Each staged file is renamed over its target and each delete is applied.
//! 4. `.vault/generation` is set to the commit's generation, written into the manifest.
//! 5. The journal is removed.
//!
//! The generation counts commits: each one is one more than the last. The index records the
//! generation it has applied, so a server can tell on start whether the index is current.
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
const GENERATION_FILE: &str = "generation";

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
    /// The generation this commit makes. A journal from before generations has none, and takes
    /// the next one when it is rolled forward.
    #[serde(default)]
    generation: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Op {
    Write { path: String, staged: String },
    Delete { path: String },
}

impl Vault {
    /// Apply every change or none of them, even across a crash. Returns the generation the
    /// commit made.
    pub fn commit(&self, changes: Vec<Change>) -> Result<u64> {
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
    ) -> Result<u64> {
        let changes = self.prepare(changes)?;
        let _guard = self.commit_lock.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = self.lock_journal()?;
        recover_locked(self)?;
        let generation = read_generation(self)? + 1;

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
            let manifest = serde_json::to_vec(&Manifest {
                ops,
                generation: Some(generation),
            })
            .expect("a manifest always serialises");
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
        roll_forward(self, hook)?;
        Ok(generation)
    }

    /// The generation of the last commit, 0 before the first. Every commit adds one, and a crash
    /// part way through a commit leaves it where it was until recovery finishes the commit.
    pub fn generation(&self) -> Result<u64> {
        read_generation(self)
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
        lock_exclusive(&file).map_err(io_err(&path))?;
        Ok(file)
    }
}

/// Block until this process holds `file`'s exclusive advisory lock, released when the file is
/// closed.
///
/// This is flock(2) on every target rather than `std::fs::File::lock`, which std leaves
/// `Unsupported` on Android, where the app's local vault would then fail to open.
fn lock_exclusive(file: &File) -> std::io::Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::LockExclusive).map_err(Into::into)
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
    let generation = match manifest.generation {
        Some(g) => g,
        None => read_generation(vault)? + 1,
    };
    write_generation(vault, generation)?;
    // The manifest goes first: a crash after this leaves a journal with nothing to redo.
    std::fs::remove_file(&manifest_path).map_err(io_err(&manifest_path))?;
    sync_dir(&txn)?;
    std::fs::remove_dir_all(&txn).map_err(io_err(&txn))
}

fn read_generation(vault: &Vault) -> Result<u64> {
    let path = vault.server_dir().join(GENERATION_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => text.trim().parse().map_err(|_| {
            Error::Corrupt(format!("{} holds {:?}, not a number", path.display(), text))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(io_err(&path)(e)),
    }
}

/// Replace `.vault/generation` atomically: a crash leaves the old number or the new one.
fn write_generation(vault: &Vault, generation: u64) -> Result<()> {
    let dir = vault.server_dir();
    let tmp = dir.join("generation.tmp");
    write_synced(&tmp, generation.to_string().as_bytes())?;
    let path = dir.join(GENERATION_FILE);
    std::fs::rename(&tmp, &path).map_err(io_err(&path))?;
    sync_dir(&dir)
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::fs::{FlockOperation, flock};

    #[test]
    fn the_journal_lock_excludes_other_opens_until_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let open = || {
            File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .unwrap()
        };
        let held = open();
        lock_exclusive(&held).unwrap();
        let other = open();
        let busy = flock(&other, FlockOperation::NonBlockingLockShared).unwrap_err();
        assert_eq!(busy, rustix::io::Errno::WOULDBLOCK);
        drop(held);
        flock(&other, FlockOperation::NonBlockingLockExclusive).unwrap();
    }
}
