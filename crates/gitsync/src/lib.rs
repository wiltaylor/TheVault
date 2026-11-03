//! Git sync between instances (git-sync D1): each instance publishes chosen notes, encrypted, to
//! a branch of its own in a git repo the owner already has, and imports from the other
//! instances' branches.
//!
//! This crate is blocking, like `crates/sync`, and runs on a thread of its own. Its modules:
//!
//! - [`crypto`]: the shared key, its `tvsync1-` text form, and the deterministic encrypted blobs
//!   and keyed-hash names every file in the repo uses (D4, D13, D14).
//! - [`layout`]: the branch, folder and file paths of an instance's tree (D11, D12, D13).
//! - [`manifest`]: the encrypted JSON index of what an instance publishes.
//! - [`pack`]: notes as blobs, and assets split into encrypted parts (D12).
//! - [`git`]: the `git` CLI runner, with HTTPS token and SSH key auth (D19, D22, D24).
//! - [`sshkey`]: ed25519 deploy keys and host key fingerprints (D19).

pub mod crypto;
pub mod git;
pub mod layout;
pub mod manifest;
pub mod pack;
pub mod sshkey;

pub use crypto::SyncKey;
pub use git::{Auth, GITHUB_KNOWN_HOSTS, Git, TreeEntry};
pub use layout::{Layout, PART_BYTES};
pub use manifest::{AssetEntry, DefEntry, FORMAT, Instance, Manifest, NoteEntry};

/// What can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A blob did not authenticate: it was sealed with another shared key (D14).
    #[error("key mismatch: the data was encrypted with a different sync key")]
    KeyMismatch,
    /// The sync key text is not a `tvsync1-` code.
    #[error("malformed sync key: {0}")]
    BadKey(String),
    /// A blob, manifest or note is not in the shape this build writes.
    #[error("malformed data: {0}")]
    Malformed(String),
    /// A manifest written by a newer build than this one (D21).
    #[error(
        "the branch uses gitsync format {found}, newer than this build supports ({supported}): \
         update this instance"
    )]
    NewerFormat { found: u32, supported: u32 },
    /// A reassembled asset does not match its recorded size or sha256.
    #[error("asset {id} is corrupt: {detail}")]
    Corrupt { id: String, detail: String },
    /// A git command failed. `stderr` has had every credential and key path redacted.
    #[error("git {command} failed ({status}): {stderr}")]
    Git {
        command: String,
        status: String,
        stderr: String,
    },
    /// A git command ran past the runner's timeout and was killed.
    #[error("git {command} timed out after {secs}s")]
    Timeout { command: String, secs: u64 },
    /// An SSH key that cannot be read or made.
    #[error("ssh key: {0}")]
    SshKey(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
