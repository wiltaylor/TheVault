//! Uploaded files, stored byte for byte at `assets/<ULID>/<filename>` (D22).
//!
//! An asset folder holds exactly one file. An asset cannot be deleted while a type file names its
//! id, because the type would stop validating. Notes that name it refuse the delete too, unless
//! the caller asks to unlink: then every reference to the asset is taken out of those notes in the
//! same commit, as `on_delete = "unlink"` does for a deleted note (D5).

use schema::{Doc, Value};
use state::Author;
use store::Change;

use crate::{Error, Event, NoteChange, Result, Saved, Writer, created_of, note, time};

/// The longest file name an asset may have, in bytes.
pub const MAX_FILENAME: usize = 255;

/// One stored asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    pub filename: String,
    pub size: u64,
}

impl Asset {
    /// The asset's vault path.
    pub fn path(&self) -> String {
        format!("assets/{}/{}", self.id, self.filename)
    }
}

/// Check an uploaded file's name: one path segment of printable characters.
fn check_filename(name: &str) -> Result<()> {
    let bad = |why: &str| {
        Err(Error::BadRequest(format!(
            "{name:?} is not a valid asset file name: {why}"
        )))
    };
    if name.is_empty() || name == "." || name == ".." {
        return bad("it is empty, `.` or `..`");
    }
    if name.len() > MAX_FILENAME {
        return bad("it is longer than 255 bytes");
    }
    if name
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return bad("it contains a slash, a backslash or a control character");
    }
    Ok(())
}

impl Writer {
    /// Store `bytes` as a new asset called `filename`.
    pub fn put_asset(&self, filename: &str, bytes: Vec<u8>) -> Result<Asset> {
        check_filename(filename)?;
        let _w = self.write_lock();
        let mut inner = self.lock();
        let asset = Asset {
            id: ulid::Ulid::new().to_string(),
            filename: filename.to_owned(),
            size: bytes.len() as u64,
        };
        let generation = self
            .vault
            .commit(vec![Change::write(asset.path(), bytes)])?;
        inner.index.advance(generation)?;
        Ok(asset)
    }

    /// Every asset, oldest first.
    pub fn assets(&self) -> Result<Vec<Asset>> {
        let root = self.vault.root().join("assets");
        let entries = std::fs::read_dir(&root).map_err(|source| {
            Error::Store(store::Error::Io {
                path: root.clone(),
                source,
            })
        })?;
        let mut ids: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|id| schema::is_ulid(id))
            .collect();
        ids.sort();
        Ok(ids.iter().filter_map(|id| self.find_asset(id)).collect())
    }

    /// Asset `id` and its bytes.
    pub fn asset(&self, id: &str) -> Result<(Asset, Vec<u8>)> {
        let asset = self.find_asset(id).ok_or_else(|| missing(id))?;
        let bytes = self.vault.read(&asset.path())?;
        Ok((asset, bytes))
    }

    /// Delete asset `id`. A type file naming it refuses the delete. Notes naming it refuse it
    /// too, unless `unlink` is set: then each loses its references to the asset in the same
    /// commit, and the update is theirs, so their handlers run. Returns the asset and the notes
    /// saved.
    pub fn delete_asset(
        &self,
        id: &str,
        unlink: bool,
        author: &Author,
    ) -> Result<(Asset, Vec<Saved>)> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let asset = self.find_asset(id).ok_or_else(|| missing(id))?;
        let by = self.files_naming(id)?;
        let types: Vec<String> = by
            .iter()
            .filter(|p| p.starts_with("types/"))
            .cloned()
            .collect();
        if !types.is_empty() || (!by.is_empty() && !unlink) {
            let by = if types.is_empty() { by } else { types };
            return Err(Error::InUse {
                code: "asset_in_use",
                message: format!("asset {id} cannot be deleted"),
                by,
            });
        }

        let now = time::utc(self.clock.now_ms());
        let mut changes = Vec::new();
        for path in &by {
            let Some((type_name, note_id)) = path
                .strip_prefix("notes/")
                .and_then(|rest| rest.strip_suffix(".wcl"))
                .and_then(|rest| rest.split_once('/'))
            else {
                continue;
            };
            let Some(ty) = inner.schema.type_def(type_name) else {
                continue;
            };
            let mut doc = crate::Input::Wcl(self.vault.read_text(path)?).parse(path)?;
            if !unlink_asset(&mut doc, id) {
                continue;
            }
            let created = created_of(&doc).unwrap_or_else(|| now.clone());
            changes.push(NoteChange {
                id: note_id.to_owned(),
                type_name: type_name.to_owned(),
                event: Event::Update,
                doc: Some(note::arrange(ty, doc, &created, &now)),
            });
        }

        let delete = vec![Change::delete(asset.path())];
        let saved = if changes.is_empty() {
            let generation = self.vault.commit(delete)?;
            inner.index.advance(generation)?;
            Vec::new()
        } else {
            self.apply_with(inner, changes, &|| Ok(()), author, delete, false)?
        };
        let dir = self.vault.root().join("assets").join(id);
        let _ = std::fs::remove_dir(dir);
        Ok((asset, saved))
    }

    /// The asset in `assets/<id>/`, when the folder holds its file.
    pub(crate) fn find_asset(&self, id: &str) -> Option<Asset> {
        if !schema::is_ulid(id) {
            return None;
        }
        let dir = self.vault.root().join("assets").join(id);
        let mut files = std::fs::read_dir(dir).ok()?.flatten().filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| Asset {
                id: id.to_owned(),
                filename: e.file_name().to_string_lossy().into_owned(),
                size: meta.len(),
            })
        });
        files.next()
    }

    /// The vault paths of every note and type file whose text holds `needle`.
    pub(crate) fn files_naming(&self, needle: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let root = self.vault.root();
        let mut files: Vec<String> = Vec::new();
        for (dir, nested) in [("types", false), ("notes", true)] {
            let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !nested {
                    if name.ends_with(".wcl") {
                        files.push(format!("{dir}/{name}"));
                    }
                    continue;
                }
                let Ok(inner) = std::fs::read_dir(e.path()) else {
                    continue;
                };
                for f in inner.flatten() {
                    let file = f.file_name().to_string_lossy().into_owned();
                    if file.ends_with(".wcl") {
                        files.push(format!("{dir}/{name}/{file}"));
                    }
                }
            }
        }
        files.sort();
        for path in files {
            if self.vault.read_text(&path)?.contains(needle) {
                out.push(path);
            }
        }
        Ok(out)
    }
}

/// Take every reference to asset `id` out of a note: the values of its fields, and each block
/// whose `asset` item names it, at any depth. A canvas node that goes takes its edges with it.
/// Returns whether anything changed.
fn unlink_asset(doc: &mut Doc, id: &str) -> bool {
    let is_id = |v: &Value| v.as_str() == Some(id);
    let mut changed = false;
    doc.fields.retain_mut(|(_, value)| match value {
        v if is_id(v) => {
            changed = true;
            false
        }
        Value::List(items) => {
            let before = items.len();
            items.retain(|v| !is_id(v));
            changed |= items.len() != before;
            !items.is_empty()
        }
        _ => true,
    });
    strip_blocks(doc, id) || changed
}

fn strip_blocks(body: &mut Doc, id: &str) -> bool {
    let names = |b: &schema::DocBlock| b.body.field("asset").and_then(Value::as_str) == Some(id);
    let gone: Vec<String> = body
        .blocks
        .iter()
        .filter(|b| b.kind == "node" && names(b))
        .filter_map(|b| b.label().map(str::to_owned))
        .collect();
    let before = body.blocks.len();
    body.blocks.retain(|b| !names(b));
    if !gone.is_empty() {
        body.blocks.retain(|b| {
            b.kind != "edge"
                || !["from", "to"].iter().any(|end| {
                    b.body
                        .field(end)
                        .and_then(Value::as_str)
                        .is_some_and(|n| gone.iter().any(|g| g == n))
                })
        });
    }
    let mut changed = body.blocks.len() != before;
    for b in &mut body.blocks {
        changed |= strip_blocks(&mut b.body, id);
    }
    changed
}

fn missing(id: &str) -> Error {
    Error::Missing(format!("asset {id} does not exist"))
}

#[cfg(test)]
mod tests {
    use super::check_filename;

    #[test]
    fn file_names_are_one_printable_segment() {
        for ok in ["photo.png", "Sample export.zip", "ünïcode.txt", ".hidden"] {
            assert!(check_filename(ok).is_ok(), "{ok}");
        }
        let long = "a".repeat(256);
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb", "a\0b", long.as_str()] {
            assert!(check_filename(bad).is_err(), "{bad:?}");
        }
    }
}
