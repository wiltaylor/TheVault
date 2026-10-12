//! Uploaded files, stored byte for byte at `assets/<ULID>/<filename>` (D22).
//!
//! An asset folder holds exactly one file. An asset cannot be deleted while a note or a type file
//! names its id, because those files would stop validating.

use store::Change;

use crate::{Error, Result, Writer};

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

    /// Delete asset `id`, unless a note or type file names it.
    pub fn delete_asset(&self, id: &str) -> Result<Asset> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let asset = self.find_asset(id).ok_or_else(|| missing(id))?;
        let by = self.files_naming(id)?;
        if !by.is_empty() {
            return Err(Error::InUse {
                code: "asset_in_use",
                message: format!("asset {id} cannot be deleted"),
                by,
            });
        }
        let generation = self.vault.commit(vec![Change::delete(asset.path())])?;
        inner.index.advance(generation)?;
        let dir = self.vault.root().join("assets").join(id);
        let _ = std::fs::remove_dir(dir);
        Ok(asset)
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
