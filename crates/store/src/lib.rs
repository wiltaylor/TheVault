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
