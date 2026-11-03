//! Where an instance's files sit in the repo (git-sync D11, D12, D13).
//!
//! Each instance owns one branch, `tv-<name("instance", vault_id)>`, whose tree holds a single
//! folder of the same name. Inside it:
//!
//! - `manifest`, an encrypted blob.
//! - `n/<2 hex>/<name("note", id)>`, one encrypted note.
//! - `a/<name("asset", id)>/<k>`, the encrypted parts of an asset, each at most [`PART_BYTES`].
//! - `d/<name("type"|"taxonomy", name)>`, encrypted definitions.
//!
//! Every path here is from the root of the branch's tree, folder included.

use crate::crypto::SyncKey;

/// The most bytes an asset part blob holds: 40 MB, under GitHub's 50 MB warning. Tests pass a
/// smaller size to [`crate::pack::split`].
pub const PART_BYTES: usize = 40 * 1024 * 1024;

/// The prefix of every instance branch.
pub const BRANCH_PREFIX: &str = "tv-";

/// The paths of one instance's tree under one shared key.
#[derive(Debug, Clone)]
pub struct Layout<'k> {
    key: &'k SyncKey,
    folder: String,
}

impl<'k> Layout<'k> {
    /// The layout of the instance with vault id `vault_id`.
    pub fn new(key: &'k SyncKey, vault_id: &str) -> Self {
        Self {
            key,
            folder: key.name("instance", vault_id),
        }
    }

    /// The layout of the instance whose branch is `branch`, when it is a `tv-` branch.
    pub fn for_branch(key: &'k SyncKey, branch: &str) -> Option<Self> {
        let folder = branch.strip_prefix(BRANCH_PREFIX)?;
        Some(Self {
            key,
            folder: folder.to_owned(),
        })
    }

    /// The branch name, `tv-<folder>`.
    pub fn branch(&self) -> String {
        format!("{BRANCH_PREFIX}{}", self.folder)
    }

    /// The single folder at the root of the branch's tree.
    pub fn folder(&self) -> &str {
        &self.folder
    }

    /// The encrypted manifest.
    pub fn manifest(&self) -> String {
        format!("{}/manifest", self.folder)
    }

    /// The blob of note `id`, fanned out by the first two hex characters of its name.
    pub fn note(&self, id: &str) -> String {
        let name = self.key.name("note", id);
        format!("{}/n/{}/{name}", self.folder, &name[..2])
    }

    /// The folder holding asset `id`'s parts.
    pub fn asset_dir(&self, id: &str) -> String {
        format!("{}/a/{}", self.folder, self.key.name("asset", id))
    }

    /// Part `k` (from 0) of asset `id`.
    pub fn asset_part(&self, id: &str, k: usize) -> String {
        format!("{}/{k}", self.asset_dir(id))
    }

    /// The definition of note type `name`.
    pub fn type_def(&self, name: &str) -> String {
        format!("{}/d/{}", self.folder, self.key.name("type", name))
    }

    /// The definition of taxonomy `name`.
    pub fn taxonomy(&self, name: &str) -> String {
        format!("{}/d/{}", self.folder, self.key.name("taxonomy", name))
    }
}

/// The branch of the instance with vault id `vault_id`.
pub fn branch(key: &SyncKey, vault_id: &str) -> String {
    Layout::new(key, vault_id).branch()
}
