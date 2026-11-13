//! The encrypted JSON manifest at the top of each instance's folder: what the instance publishes
//! and where each file sits.

use serde::{Deserialize, Serialize};

use crate::crypto::SyncKey;
use crate::layout::Layout;
use crate::{Error, Result};

/// The gitsync format this build writes. A manifest with a larger `format` comes from a newer
/// build, and its branch is skipped (D21).
pub const FORMAT: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub instance: Instance,
    pub notes: Vec<NoteEntry>,
    pub assets: Vec<AssetEntry>,
    pub types: Vec<DefEntry>,
    pub taxonomies: Vec<DefEntry>,
    /// The total size of every blob on the branch, this manifest's own excepted.
    pub bytes: u64,
}

/// The instance that publishes the branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub vault_id: String,
    pub name: String,
}

/// One published note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub note_type: String,
    /// The vault id of the instance that created the note.
    pub origin: String,
    /// The sha256 hex of the note's published plaintext.
    pub hash: String,
    /// The note blob's path from the tree root.
    pub path: String,
    /// The ids of the assets the note references.
    pub assets: Vec<String>,
}

/// One published asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
    /// The paths of its parts from the tree root, in order.
    pub parts: Vec<String>,
}

/// One published type or taxonomy definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefEntry {
    pub name: String,
    pub path: String,
}

#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

impl Manifest {
    /// An empty manifest of this build's [`FORMAT`] for `instance`.
    pub fn new(instance: Instance) -> Self {
        Self {
            format: FORMAT,
            instance,
            notes: Vec::new(),
            assets: Vec::new(),
            types: Vec::new(),
            taxonomies: Vec::new(),
            bytes: 0,
        }
    }

    /// The manifest as a blob.
    pub fn seal(&self, key: &SyncKey) -> Vec<u8> {
        let json = serde_json::to_vec(self).expect("a manifest always serialises");
        key.seal(&Layout::new(key, &self.instance.vault_id).manifest(), &json)
    }

    /// Reads a manifest blob. A blob from another key is [`Error::KeyMismatch`], and a manifest
    /// whose format is newer than [`FORMAT`] is [`Error::NewerFormat`], checked before the rest is
    /// read since a newer format may change its shape.
    pub fn open(key: &SyncKey, layout: &Layout<'_>, blob: &[u8]) -> Result<Self> {
        let json = key.open(&layout.manifest(), blob)?;
        let probe: FormatOnly = serde_json::from_slice(&json)
            .map_err(|e| Error::Malformed(format!("manifest: {e}")))?;
        if probe.format > FORMAT {
            return Err(Error::NewerFormat {
                found: probe.format,
                supported: FORMAT,
            });
        }
        if probe.format != FORMAT {
            return Err(Error::Malformed(
                "legacy manifest refused; upgrade every peer and republish".into(),
            ));
        }
        serde_json::from_slice(&json).map_err(|e| Error::Malformed(format!("manifest: {e}")))
    }
}
