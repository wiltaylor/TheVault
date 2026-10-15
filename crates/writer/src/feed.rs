//! The files besides notes that devices sync, and how their changes reach the change feed in
//! `state.db` (D20).
//!
//! Notes reach the feed with their versions. Every other commit goes through [`Writer::commit`],
//! which journals the synced files it wrote or deleted once the commit is in. Opening the vault
//! reconciles the feed with the files on disk, which covers files written before the feed
//! existed, files added outside the server, and a crash between a commit and its journal entry.
//! Assets are not in the feed: a device fetches them on demand (D19).

use std::ops::ControlFlow;

use store::{Change, CommitStep, Vault};

use crate::{Error, Result, Writer};

/// The folders whose files devices sync: folder, file extension, and the kind the feed names.
/// A type file holds its templates, so a template change is a change to its type.
pub const SYNCED_FILES: [(&str, &str, &str); 5] = [
    ("types", ".wcl", "type"),
    ("taxonomies", ".wcl", "taxonomy"),
    ("scripts", ".ws", "script"),
    ("tasks", ".wcl", "task"),
    ("devices", ".wcl", "device"),
];

/// The feed's kind and the name of synced file `path`, such as `("type", "memo")` for
/// `types/memo.wcl`. `None` for any other path.
pub fn synced_file(path: &str) -> Option<(&'static str, &str)> {
    let (dir, file) = path.split_once('/')?;
    SYNCED_FILES.iter().find_map(|(d, ext, kind)| {
        let name = file.strip_suffix(ext).filter(|_| *d == dir)?;
        (!name.is_empty() && !name.contains('/')).then_some((*kind, name))
    })
}

/// `(path, exists)` of each synced file `files` writes or deletes.
fn synced_changes(files: &[Change]) -> Vec<(String, bool)> {
    files
        .iter()
        .map(|c| match c {
            Change::Write { path, .. } => (path.clone(), true),
            Change::Delete { path } => (path.clone(), false),
        })
        .filter(|(path, _)| synced_file(path).is_some())
        .collect()
}

/// Every synced file on disk.
fn synced_on_disk(vault: &Vault) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (dir, _, _) in SYNCED_FILES {
        let root = vault.root().join(dir);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(Error::Store(store::Error::Io { path: root, source })),
        };
        for entry in entries.flatten() {
            let path = format!("{dir}/{}", entry.file_name().to_string_lossy());
            if entry.path().is_file() && synced_file(&path).is_some() {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Bring the feed's file entries up to the files on disk.
pub(crate) fn reconcile(vault: &Vault, state: &state::State) -> Result<()> {
    state.reconcile_files(&synced_on_disk(vault)?)?;
    Ok(())
}

impl Writer {
    /// The text of synced file `path` as it is now, or `None` when it does not exist or `path` is
    /// not a synced file. Read from disk rather than through the store, whose layout does not
    /// cover every synced folder (`devices/`).
    pub fn read_synced(&self, path: &str) -> Result<Option<String>> {
        if synced_file(path).is_none() {
            return Ok(None);
        }
        let full = self.vault.root().join(path);
        match std::fs::read_to_string(&full) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::Store(store::Error::Io { path: full, source })),
        }
    }

    /// Commit `files` and journal the synced ones in the change feed. Returns the generation.
    pub(crate) fn commit(&self, files: Vec<Change>) -> Result<u64> {
        let synced = synced_changes(&files);
        let generation = self.vault.commit(files)?;
        self.state.journal_files(&synced)?;
        Ok(generation)
    }

    /// [`Writer::commit`] calling `hook` at each step.
    pub(crate) fn commit_with_hook(
        &self,
        files: Vec<Change>,
        hook: &mut dyn FnMut(CommitStep) -> ControlFlow<()>,
    ) -> Result<u64> {
        let synced = synced_changes(&files);
        let generation = self.vault.commit_with_hook(files, hook)?;
        self.state.journal_files(&synced)?;
        Ok(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_synced_files_and_nothing_else() {
        assert_eq!(synced_file("types/memo.wcl"), Some(("type", "memo")));
        assert_eq!(
            synced_file("taxonomies/status.wcl"),
            Some(("taxonomy", "status"))
        );
        assert_eq!(synced_file("scripts/hello.ws"), Some(("script", "hello")));
        assert_eq!(synced_file("tasks/nightly.wcl"), Some(("task", "nightly")));
        assert_eq!(
            synced_file("devices/android.wcl"),
            Some(("device", "android"))
        );
        for other in [
            "notes/memo/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl",
            "assets/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA/a.png",
            "types/memo.ws",
            "scripts/hello.wcl",
            "types/.wcl",
            "types/a/b.wcl",
            "types",
        ] {
            assert_eq!(synced_file(other), None, "{other}");
        }
    }
}
