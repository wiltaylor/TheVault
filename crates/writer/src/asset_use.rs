//! Which notes use each asset, cleaning up the assets nothing uses, and the asset trash.
//!
//! An asset is used by every indexed note naming it (`index::Index::asset_users`), by every note
//! in the trash whose last content names it, and by every type file naming it. A deleted asset
//! is not gone at once: its file moves to `.vault/asset-trash/<id>/<filename>`, with the time
//! of the delete in `.vault/asset-trash/<id>.deleted`, and can be restored for
//! [`ASSET_TRASH_DAYS`] days. Older entries are purged whenever the asset trash is read or
//! written.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use index::AssetUse;
use store::Change;

use crate::assets::{check_filename, missing};
use crate::{Asset, Error, Result, Writer};

/// How long a deleted asset can be restored.
pub const ASSET_TRASH_DAYS: i64 = 30;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A note in the trash naming an asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedUse {
    pub id: String,
    pub type_name: String,
    pub title: String,
}

/// Everything that uses one asset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetUsage {
    /// Live notes, archived ones among them.
    pub notes: Vec<AssetUse>,
    /// Notes in the trash. Restoring one would bring the reference back.
    pub trashed: Vec<TrashedUse>,
    /// Type files naming the asset, as type names: a template image or an image map.
    pub types: Vec<String>,
}

impl AssetUsage {
    pub fn is_unused(&self) -> bool {
        self.notes.is_empty() && self.trashed.is_empty() && self.types.is_empty()
    }
}

/// An asset with how many things use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSummary {
    pub asset: Asset,
    pub notes: usize,
    pub trashed: usize,
    pub types: usize,
}

impl AssetSummary {
    pub fn used_by(&self) -> usize {
        self.notes + self.trashed + self.types
    }
}

/// A deleted asset that can still be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedAsset {
    pub asset: Asset,
    pub deleted_ms: i64,
    /// When it is purged for good.
    pub purge_ms: i64,
}

/// The ULIDs among the quoted strings of `text`.
pub(crate) fn ulids_in(text: &str) -> HashSet<&str> {
    text.split('"').filter(|s| schema::is_ulid(s)).collect()
}

fn title_of(content: &str, path: &str) -> String {
    crate::Input::Wcl(content.to_owned())
        .parse(path)
        .ok()
        .and_then(|d| {
            d.field("title")
                .and_then(schema::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

impl Writer {
    fn asset_trash_dir(&self) -> PathBuf {
        self.vault.server_dir().join("asset-trash")
    }

    /// Everything that uses asset `id`, whether or not the asset exists.
    pub fn asset_usage(&self, id: &str) -> Result<AssetUsage> {
        let notes = self.read_index(|index, _| index.asset_users(id))?;
        let trashed = self.trashed_naming(id)?;
        let types = self
            .files_naming(&format!("\"{id}\""))?
            .into_iter()
            .filter_map(|p| {
                p.strip_prefix("types/")
                    .and_then(|p| p.strip_suffix(".wcl"))
                    .map(str::to_owned)
            })
            .collect();
        Ok(AssetUsage {
            notes,
            trashed,
            types,
        })
    }

    /// The notes in the trash whose last content names asset `id`.
    pub(crate) fn trashed_naming(&self, id: &str) -> Result<Vec<TrashedUse>> {
        let mut out = Vec::new();
        for e in self.state.list_trash()? {
            if ulids_in(&e.content).contains(id) {
                let path = crate::note_path(&e.note_type, &e.note_id);
                out.push(TrashedUse {
                    title: title_of(&e.content, &path),
                    id: e.note_id,
                    type_name: e.note_type,
                });
            }
        }
        Ok(out)
    }

    /// The title of note `id` as it went to the trash, if it is there.
    pub fn trashed_title(&self, id: &str) -> Option<String> {
        let e = self
            .state
            .list_trash()
            .ok()?
            .into_iter()
            .find(|e| e.note_id == id)?;
        Some(title_of(
            &e.content,
            &crate::note_path(&e.note_type, &e.note_id),
        ))
    }

    /// Every asset, oldest first, with how many notes, trashed notes and types use it.
    pub fn asset_summaries(&self) -> Result<Vec<AssetSummary>> {
        let assets = self.assets()?;
        let notes = self.read_index(|index, _| index.asset_use_counts())?;
        let mut trashed: HashMap<String, usize> = HashMap::new();
        for e in self.state.list_trash()? {
            for id in ulids_in(&e.content) {
                *trashed.entry(id.to_owned()).or_default() += 1;
            }
        }
        let mut types: HashMap<String, usize> = HashMap::new();
        for path in self.definition_and_note_files() {
            if !path.starts_with("types/") {
                continue;
            }
            let text = self.vault.read_text(&path)?;
            for id in ulids_in(&text) {
                *types.entry(id.to_owned()).or_default() += 1;
            }
        }
        Ok(assets
            .into_iter()
            .map(|asset| AssetSummary {
                notes: notes.get(&asset.id).copied().unwrap_or(0),
                trashed: trashed.get(&asset.id).copied().unwrap_or(0),
                types: types.get(&asset.id).copied().unwrap_or(0),
                asset,
            })
            .collect())
    }

    /// Whether the text extracted from asset `id` holds `part`, ignoring case.
    pub fn asset_text_holds(&self, id: &str, part: &str) -> bool {
        let part = part.to_lowercase();
        self.read_index(|index, _| index.asset_text_matches(id, &part))
    }

    /// The text the index extracted from asset `id`, a PDF, or why it has none.
    pub fn asset_text(&self, id: &str) -> index::StoredText {
        self.read_index(|index, _| index.asset_text(id))
    }

    /// Delete every asset nothing uses, or with `only`, those of them it names, all in one
    /// commit, into the asset trash. A note or type file whose text names an asset keeps it,
    /// even one the index does not hold. With `dry_run` nothing changes. Returns the assets
    /// deleted, or that would be.
    pub fn cleanup_assets(&self, only: Option<&[String]>, dry_run: bool) -> Result<Vec<Asset>> {
        let _w = self.write_lock();
        self.cleanup_assets_locked(only, dry_run)
    }

    /// [`Writer::cleanup_assets`] for a caller already holding the write lock.
    pub(crate) fn cleanup_assets_locked(
        &self,
        only: Option<&[String]>,
        dry_run: bool,
    ) -> Result<Vec<Asset>> {
        let mut named: HashSet<String> = HashSet::new();
        for path in self.definition_and_note_files() {
            let text = self.vault.read_text(&path)?;
            named.extend(ulids_in(&text).into_iter().map(str::to_owned));
        }
        let unused: Vec<Asset> = self
            .asset_summaries()?
            .into_iter()
            .filter(|s| s.used_by() == 0 && !named.contains(&s.asset.id))
            .filter(|s| only.is_none_or(|ids| ids.contains(&s.asset.id)))
            .map(|s| s.asset)
            .collect();
        if dry_run || unused.is_empty() {
            return Ok(unused);
        }
        let mut inner = self.lock();
        for a in &unused {
            self.keep_in_trash(a)?;
        }
        let changes = unused.iter().map(|a| Change::delete(a.path())).collect();
        match self.commit(changes) {
            Ok(generation) => inner.index.advance(generation)?,
            Err(e) => {
                for a in &unused {
                    self.drop_from_trash(&a.id);
                }
                return Err(e);
            }
        }
        for a in &unused {
            inner.index.forget_asset_text(&a.id);
            let _ = std::fs::remove_dir(self.vault.root().join("assets").join(&a.id));
        }
        drop(inner);
        self.purge_asset_trash();
        Ok(unused)
    }

    /// Keep a copy of `asset`'s file in the asset trash, before its delete commits.
    pub(crate) fn keep_in_trash(&self, asset: &Asset) -> Result<()> {
        let io = |path: PathBuf| move |source| Error::Store(store::Error::Io { path, source });
        let dir = self.asset_trash_dir().join(&asset.id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(io(dir.clone()))?;
        let from = self.vault.root().join(asset.path());
        let to = dir.join(&asset.filename);
        if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).map_err(io(to.clone()))?;
        }
        let stamp = self.asset_trash_dir().join(format!("{}.deleted", asset.id));
        std::fs::write(&stamp, self.clock.now_ms().to_string()).map_err(io(stamp.clone()))?;
        Ok(())
    }

    /// Forget asset `id`'s entry in the asset trash.
    pub(crate) fn drop_from_trash(&self, id: &str) {
        let root = self.asset_trash_dir();
        let _ = std::fs::remove_dir_all(root.join(id));
        let _ = std::fs::remove_file(root.join(format!("{id}.deleted")));
    }

    /// Purge the asset trash of entries older than [`ASSET_TRASH_DAYS`].
    fn purge_asset_trash(&self) {
        let now = self.clock.now_ms();
        for t in self.read_asset_trash() {
            if t.purge_ms <= now {
                self.drop_from_trash(&t.asset.id);
            }
        }
    }

    fn read_asset_trash(&self) -> Vec<TrashedAsset> {
        let root = self.asset_trash_dir();
        let Ok(entries) = std::fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if !schema::is_ulid(&id) || !e.path().is_dir() {
                continue;
            }
            let Some(file) = std::fs::read_dir(e.path())
                .ok()
                .and_then(|mut d| d.find_map(|f| f.ok().filter(|f| f.path().is_file())))
            else {
                continue;
            };
            let deleted_ms = std::fs::read_to_string(root.join(format!("{id}.deleted")))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            out.push(TrashedAsset {
                asset: Asset {
                    filename: file.file_name().to_string_lossy().into_owned(),
                    size: file.metadata().map(|m| m.len()).unwrap_or(0),
                    id,
                },
                deleted_ms,
                purge_ms: deleted_ms + ASSET_TRASH_DAYS * DAY_MS,
            });
        }
        out.sort_by(|a, b| {
            b.deleted_ms
                .cmp(&a.deleted_ms)
                .then(a.asset.id.cmp(&b.asset.id))
        });
        out
    }

    /// Every deleted asset that can still be restored, most recently deleted first.
    pub fn asset_trash(&self) -> Vec<TrashedAsset> {
        self.purge_asset_trash();
        self.read_asset_trash()
    }

    /// Bring asset `id` back from the asset trash, under its id and file name. Notes a delete
    /// took it out of do not get it back: their history has the versions that named it.
    pub fn restore_asset(&self, id: &str) -> Result<Asset> {
        let trashed = self
            .asset_trash()
            .into_iter()
            .find(|t| t.asset.id == id)
            .ok_or_else(|| Error::Missing(format!("asset {id} is not in the asset trash")))?;
        if self.find_asset(id).is_some() {
            return Err(Error::IdTaken(id.to_owned()));
        }
        let from = self
            .asset_trash_dir()
            .join(id)
            .join(&trashed.asset.filename);
        let bytes = std::fs::read(&from).map_err(|source| {
            Error::Store(store::Error::Io {
                path: from.clone(),
                source,
            })
        })?;
        let asset = self.put_asset_with_id(id, &trashed.asset.filename, bytes)?;
        self.drop_from_trash(id);
        Ok(asset)
    }

    /// Give asset `id` the file name `filename`. Notes name assets by id, so none changes.
    pub fn rename_asset(&self, id: &str, filename: &str) -> Result<Asset> {
        check_filename(filename)?;
        let _w = self.write_lock();
        let mut inner = self.lock();
        let asset = self.find_asset(id).ok_or_else(|| missing(id))?;
        if asset.filename == filename {
            return Ok(asset);
        }
        let bytes = self.vault.read(&asset.path())?;
        let renamed = Asset {
            filename: filename.to_owned(),
            ..asset.clone()
        };
        let generation = self.commit(vec![
            Change::delete(asset.path()),
            Change::write(renamed.path(), bytes),
        ])?;
        inner.index.advance(generation)?;
        Ok(renamed)
    }
}
