//! Durable checkpoints, separate from the disconnectable note merge bases.
use serde::{Deserialize, Serialize};
use state::State;

use crate::{Error, Result};

/// Lock before fetching and keep the descriptor through push, including checkpoint writes.
pub(crate) fn lock(dir: &std::path::Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("freshness.lock"))?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        .map_err(std::io::Error::from)?;
    Ok(file)
}

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
    if record.sequence == 0
        || record.hash.len() != 64
        || !record.hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Sync("corrupt git-sync freshness checkpoint".into()));
    }
    Ok(Some(record))
}

fn save(state: &State, name: &str, sequence: u64, hash: &str) -> Result<()> {
    state.set_secret(
        name,
        &serde_json::to_string(&Checkpoint {
            sequence,
            hash: hash.into(),
        })
        .expect("checkpoint serializes"),
    )?;
    Ok(())
}

/// The caller holds the vault's round lock, including across the push.
pub(crate) fn publication(state: &State, peer: &str, hash: &str, remote_sequence: u64) -> Result<u64> {
    let name = format!("gitsync_freshness_publish_{peer}");
    let sequence = match load(state, &name)? {
        Some(old) if old.hash == hash && old.sequence >= remote_sequence => return Ok(old.sequence),
        Some(old) => old
            .sequence
            .max(remote_sequence)
            .checked_add(1)
            .ok_or_else(|| Error::Sync("git-sync sequence exhausted".into()))?,
        None => remote_sequence.checked_add(1)
            .ok_or_else(|| Error::Sync("git-sync sequence exhausted".into()))?,
    };
    // Reserve before push. A failed push may leave a harmless gap, never reuse a number.
    save(state, &name, sequence, hash)?;
    Ok(sequence)
}

pub(crate) fn receive(state: &State, peer: &str, sequence: u64, hash: &str) -> Result<()> {
    if sequence == 0 {
        return Err(Error::Sync(
            "peer has no freshness sequence: upgrade every peer and republish".into(),
        ));
    }
    let name = format!("gitsync_freshness_receive_{peer}");
    if let Some(old) = load(state, &name)? {
        if sequence < old.sequence || (sequence == old.sequence && hash != old.hash) {
            return Err(Error::Sync(
                "git-sync freshness refused: replay or conflicting sequence".into(),
            ));
        }
        if sequence == old.sequence {
            return Ok(());
        }
    }
    save(state, &name, sequence, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_publishers_serialize_durable_reservations() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let key_file = root.join("key");
        let key = state::SecretKey::create_file(&key_file).unwrap();
        let database = root.join("state.db");
        drop(State::open(&database, Some(key)).unwrap());
        let barrier = std::sync::Barrier::new(8);
        let mut sequences = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let barrier = &barrier;
                    let database = &database;
                    let key_file = &key_file;
                    scope.spawn(move || {
                        barrier.wait();
                        let _lock = lock(root).unwrap();
                        let key = state::SecretKey::load_file(key_file).unwrap();
                        let state = State::open(database, Some(key)).unwrap();
                        publication(&state, "publisher", &crate::pack::sha256_hex(&[i]), 0).unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        sequences.sort_unstable();
        assert_eq!(sequences, (1..=8).collect::<Vec<_>>());
        let state = State::open(
            &database,
            Some(state::SecretKey::load_file(&key_file).unwrap()),
        )
        .unwrap();
        let hash = crate::pack::sha256_hex(b"after restart");
        assert_eq!(publication(&state, "publisher", &hash, 0).unwrap(), 9);
        assert_eq!(publication(&state, "publisher", &hash, 9).unwrap(), 9);
        assert_eq!(publication(&state, "publisher", &hash, 40).unwrap(), 41);
        assert_eq!(publication(&state, "restored", &hash, 50).unwrap(), 51);
        assert!(publication(&state, "exhausted", &hash, u64::MAX).is_err());
    }
}
