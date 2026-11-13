//! Vault folder layout, WCL read, canonical WCL write, data-only check and atomic multi-file commits.
//!
//! Every vault file is plain data (D20). [`parse_data_only`] rejects anything else and names the
//! offending item, [`canonical`] prints the one canonical form every write emits (D21), and
//! [`Vault`] owns the D22 folder: it reads through a loader that cannot leave the vault and
//! commits a set of writes and deletes all at once or not at all. `skills/` is the one folder of
//! files that are not WCL: each skill's files are kept exactly as authored (skills S1).

mod canonical;
mod commit;
mod data_only;
mod skills;
mod vault;

use std::path::PathBuf;

pub use canonical::{canonical, to_canonical};
pub use commit::Change;
#[doc(hidden)]
pub use commit::CommitStep;
pub use data_only::{DataOnlyError, parse_data_only};
pub use skills::{
    MAX_SKILL_BYTES, MAX_SKILL_FILES, MAX_SKILL_TEXT_BYTES, SKILL_FILE, SKILLS_DIR, SkillFile,
    SkillTree, is_skill_folder, is_skill_path, normalise_skill_path, skill_limits,
};
pub use vault::{LAYOUT_DIRS, Vault};

/// Lifetime ownership for the server and offline history repair. Never unlink this file.
/// Closing the returned handle (including process exit) releases the lock.
pub fn lock_owner(root: &std::path::Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".vault/owner.lock"))?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(owner_lock_error)?;
    Ok(file)
}

fn owner_lock_error(e: rustix::io::Errno) -> std::io::Error {
    if e == rustix::io::Errno::WOULDBLOCK {
        std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            format!("vault is in use by a server or history repair: {e}"),
        )
    } else {
        e.into()
    }
}

#[test]
fn owner_lock_preserves_non_contention_error() {
    let error = owner_lock_error(rustix::io::Errno::OPNOTSUPP);
    assert_eq!(
        error.raw_os_error(),
        Some(rustix::io::Errno::OPNOTSUPP.raw_os_error())
    );
    assert!(!error.to_string().contains("in use"));
    assert_eq!(
        owner_lock_error(rustix::io::Errno::WOULDBLOCK).kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// Errors from the vault store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}: {message}")]
    Parse { file: String, message: String },
    #[error(transparent)]
    DataOnly(#[from] DataOnlyError),
    #[error("{path:?} is not a valid vault path: {reason}")]
    InvalidPath { path: String, reason: String },
    #[error("{} is outside the vault", .0.display())]
    OutsideVault(PathBuf),
    #[error("{} is not a vault: it has no .vault folder", .0.display())]
    NotAVault(PathBuf),
    #[error("{0} is not valid UTF-8")]
    NotUtf8(String),
    #[error("{0} does not exist")]
    NotFound(String),
    #[error("{0} appears more than once in one commit")]
    DuplicatePath(String),
    #[error("the commit was interrupted")]
    Interrupted,
    #[error("the commit journal is corrupt: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}
