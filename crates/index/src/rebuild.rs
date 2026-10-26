//! A full rebuild from the vault folder (D28): every definition and note file is read and
//! validated, the ones that pass are indexed, and the ones that fail are reported.

use std::collections::HashMap;

use schema::{Doc, Invalid, NoteLookup, Schema, SchemaError, Taxonomy, TypeDef};

use crate::{Index, Result, asset_text, io_err, project, sql};

/// A vault file that a rebuild could not index, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The file's path in the vault, such as `notes/task/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl`.
    pub path: String,
    pub diagnostic: String,
}

/// What a rebuild did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildReport {
    /// Notes indexed.
    pub indexed: usize,
    /// Every file that failed, sorted by path.
    pub failures: Vec<Failure>,
}

pub(crate) fn rebuild(index: &mut Index, vault: &store::Vault) -> Result<RebuildReport> {
    // Read first: a commit that lands while the files are read leaves the index behind, never
    // marked current without it.
    let generation = vault.generation()?;
    let mut failures = Vec::new();
    let schema = load_schema(vault, &mut failures)?;

    let files = note_files(vault, &mut failures)?;
    let lookup = Lookup {
        types: files
            .iter()
            .map(|f| (f.id.clone(), f.type_name.clone()))
            .collect(),
        vault,
    };

    extract_missing(index, vault)?;

    let mut notes = Vec::new();
    for f in &files {
        let checked = vault
            .read_text(&f.path)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                let doc = Doc::parse(&text, &f.path).map_err(|e| diagnostic(&e))?;
                let report =
                    schema
                        .validate_note(&f.type_name, &doc, &lookup)
                        .map_err(|issues| {
                            diagnostic(&Invalid {
                                file: f.path.clone(),
                                issues,
                            })
                        })?;
                Ok((doc, report))
            });
        match checked {
            Ok((doc, report)) => {
                // validate_note refused any note whose type is missing, so this always holds.
                if let Some(ty) = schema.type_def(&f.type_name) {
                    let mut note = project::project(ty, &f.id, &doc, &report);
                    crate::append_text(&mut note.search.body, &index.assets.of_doc(&doc));
                    notes.push(note);
                }
            }
            Err(diagnostic) => failures.push(Failure {
                path: f.path.clone(),
                diagnostic,
            }),
        }
    }
    failures.sort_by(|a, b| a.path.cmp(&b.path));

    let tx = index.conn.transaction()?;
    sql::clear(&tx)?;
    sql::replace_terms(&tx, &schema)?;
    for n in &notes {
        sql::insert_note(&tx, n)?;
    }
    for f in &failures {
        sql::insert_failure(&tx, f)?;
    }
    sql::set_generation(&tx, generation)?;
    let indexed = notes.len();
    index
        .search
        .replace_all(notes.into_iter().map(|n| n.search).collect())?;
    tx.commit()?;
    Ok(RebuildReport { indexed, failures })
}

/// Extract the text of every asset of a [`asset_text::TextKind`] that has none kept yet: PDFs,
/// Word documents, spreadsheets and CSV files. One that cannot be read keeps no text, so it is
/// not tried again.
fn extract_missing(index: &mut Index, vault: &store::Vault) -> Result<()> {
    let root = vault.root().join("assets");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(());
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|id| schema::is_ulid(id) && !index.assets.has(id))
        .collect();
    ids.sort();
    for id in ids {
        let Ok(files) = std::fs::read_dir(root.join(&id)) else {
            continue;
        };
        let Some(file) = files.flatten().find(|e| e.path().is_file()) else {
            continue;
        };
        let Some(kind) = asset_text::TextKind::of(&file.file_name().to_string_lossy()) else {
            continue;
        };
        let too_big = file
            .metadata()
            .is_ok_and(|m| m.len() > asset_text::MAX_EXTRACT_BYTES as u64);
        let extracted = if too_big {
            asset_text::Extracted::Failed(format!(
                "larger than {} bytes",
                asset_text::MAX_EXTRACT_BYTES
            ))
        } else {
            match std::fs::read(file.path()) {
                Ok(bytes) => asset_text::extract_text(kind, bytes),
                Err(_) => continue,
            }
        };
        index.set_asset_text(&id, &extracted)?;
    }
    Ok(())
}

/// The issues of an invalid file, without the file name the failure already carries.
fn diagnostic(e: &Invalid) -> String {
    e.issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Load every type and taxonomy that is valid on its own and consistent with the rest. A type
/// that fails is reported and dropped, and the rest are checked again without it, because a type
/// that named it may now fail too.
fn load_schema(vault: &store::Vault, failures: &mut Vec<Failure>) -> Result<Schema> {
    let mut types = Vec::new();
    for (name, path) in definition_files(vault, "types", failures)? {
        match vault
            .read_text(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| TypeDef::parse(&name, &text).map_err(|e| diagnostic(&e)))
        {
            Ok(t) => types.push(t),
            Err(diagnostic) => failures.push(Failure { path, diagnostic }),
        }
    }
    let mut taxonomies = Vec::new();
    for (name, path) in definition_files(vault, "taxonomies", failures)? {
        match vault
            .read_text(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| Taxonomy::parse(&name, &text).map_err(|e| diagnostic(&e)))
        {
            Ok(t) => taxonomies.push(t),
            Err(diagnostic) => failures.push(Failure { path, diagnostic }),
        }
    }
    loop {
        match Schema::new(types.clone(), taxonomies.clone()) {
            Ok(schema) => return Ok(schema),
            Err(SchemaError(failed)) => {
                let before = types.len();
                for f in failed {
                    let name = f
                        .file
                        .strip_prefix("types/")
                        .and_then(|s| s.strip_suffix(".wcl"))
                        .unwrap_or_default()
                        .to_owned();
                    types.retain(|t| t.name != name);
                    failures.push(Failure {
                        diagnostic: diagnostic(&f),
                        path: f.file,
                    });
                }
                if types.len() == before {
                    return Ok(Schema::default());
                }
            }
        }
    }
}

/// `(stem, path)` of every `.wcl` file in a definition folder, sorted. Anything else in the
/// folder is reported.
fn definition_files(
    vault: &store::Vault,
    dir: &str,
    failures: &mut Vec<Failure>,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for name in list(vault, dir)? {
        let path = format!("{dir}/{name}");
        match name.strip_suffix(".wcl") {
            Some(stem) if schema::is_name(stem) => out.push((stem.to_owned(), path)),
            _ => failures.push(Failure {
                path,
                diagnostic: format!("is not a definition file: expected {dir}/<name>.wcl"),
            }),
        }
    }
    Ok(out)
}

struct NoteFile {
    id: String,
    type_name: String,
    path: String,
}

/// Every `notes/<type>/<ULID>.wcl`, sorted by path. Anything else under `notes/` is reported.
fn note_files(vault: &store::Vault, failures: &mut Vec<Failure>) -> Result<Vec<NoteFile>> {
    let mut out = Vec::new();
    for type_name in list(vault, "notes")? {
        let dir = format!("notes/{type_name}");
        if !vault.root().join(&dir).is_dir() {
            failures.push(Failure {
                path: dir,
                diagnostic: "is not a folder: expected notes/<type>/<ULID>.wcl".to_owned(),
            });
            continue;
        }
        for name in list(vault, &dir)? {
            let path = format!("{dir}/{name}");
            match name.strip_suffix(".wcl") {
                Some(id) if schema::is_ulid(id) && schema::is_name(&type_name) => {
                    out.push(NoteFile {
                        id: id.to_owned(),
                        type_name: type_name.clone(),
                        path,
                    })
                }
                _ => failures.push(Failure {
                    path,
                    diagnostic: "is not a note file: expected notes/<type>/<ULID>.wcl".to_owned(),
                }),
            }
        }
    }
    Ok(out)
}

/// The entry names of a vault folder, sorted. Hidden entries are skipped.
fn list(vault: &store::Vault, dir: &str) -> Result<Vec<String>> {
    let full = vault.root().join(dir);
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&full).map_err(io_err(&full))? {
        let name = entry
            .map_err(io_err(&full))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Note existence from the files on disk, including notes that fail validation themselves, and
/// asset existence from `assets/<id>/`.
struct Lookup<'a> {
    types: HashMap<String, String>,
    vault: &'a store::Vault,
}

impl NoteLookup for Lookup<'_> {
    fn note_type(&self, id: &str) -> Option<String> {
        self.types.get(id).cloned()
    }

    fn asset_exists(&self, id: &str) -> bool {
        schema::is_ulid(id) && self.vault.root().join("assets").join(id).is_dir()
    }

    fn asset_filename(&self, id: &str) -> Option<String> {
        if !schema::is_ulid(id) {
            return None;
        }
        let dir = std::fs::read_dir(self.vault.root().join("assets").join(id)).ok()?;
        dir.flatten()
            .find(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
    }
}
