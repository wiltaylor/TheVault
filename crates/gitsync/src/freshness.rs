//! Durable checkpoints, separate from the disconnectable note merge bases.
use serde::{Deserialize, Serialize};
use state::State;

use crate::{Error, Result};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    sequence: u64,
    hash: String,
}

fn load(state: &State, name: &str) -> Result<Option<Checkpoint>> {
    let Some(text) = state.get_secret(name)? else {
        return Ok(None);
    };
    let record: Checkpoint = serde_json::from_str(&text)
        .map_err(|_| Error::Sync("corrupt git-sync freshness checkpoint".into()))?;
    if record.sequence == 0 || record.hash.len() != 64 || !record.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Sync("corrupt git-sync freshness checkpoint".into()));
    }
    Ok(Some(record))
}

fn save(state: &State, name: &str, sequence: u64, hash: &str) -> Result<()> {
    state.set_secret(name, &serde_json::to_string(&Checkpoint { sequence, hash: hash.into() }).expect("checkpoint serializes"))?;
    Ok(())
}

/// The caller holds the vault's round lock, including across the push.
pub(crate) fn publication(state: &State, peer: &str, hash: &str) -> Result<u64> {
    let name = format!("gitsync_freshness_publish_{peer}");
    let sequence = match load(state, &name)? {
        Some(old) if old.hash == hash => return Ok(old.sequence),
        Some(old) => old.sequence.checked_add(1).ok_or_else(|| Error::Sync("git-sync sequence exhausted".into()))?,
        None => 1,
    };
    // Reserve before push. A failed push may leave a harmless gap, never reuse a number.
    save(state, &name, sequence, hash)?;
    Ok(sequence)
}

pub(crate) fn receive(state: &State, peer: &str, sequence: u64, hash: &str) -> Result<()> {
    if sequence == 0 {
        return Err(Error::Sync("peer has no freshness sequence: upgrade every peer and republish".into()));
    }
    let name = format!("gitsync_freshness_receive_{peer}");
    if let Some(old) = load(state, &name)? {
        if sequence < old.sequence || (sequence == old.sequence && hash != old.hash) {
            return Err(Error::Sync("git-sync freshness refused: replay or conflicting sequence".into()));
        }
        if sequence == old.sequence {
            return Ok(());
        }
    }
    save(state, &name, sequence, hash)
}
