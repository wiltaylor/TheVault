//! Agent Skills in the vault (skills S1, S2, S3, S7): `skills/<name>/`, each a folder of files
//! kept exactly as authored, with `SKILL.md` at its root.
//!
//! Reads list every skill with its frontmatter, size and issues ([`Writer::skills`]), read one
//! with its body and files ([`Writer::skill`]), and read a file's bytes ([`Writer::skill_file`]) or
//! text ([`Writer::skill_text`]). A skill whose `SKILL.md` has an error, or that breaks a limit or
//! holds a symlink, is still listed, with `valid()` false: it is offered to no agent.
//!
//! Writes check before they commit: `SKILL.md` must validate (warnings are kept), the skill must
//! stay within [`store::MAX_SKILL_FILES`] files and [`store::MAX_SKILL_BYTES`] bytes, and every
//! path is normalised and confined to the skill's folder. An import validates the whole zip before
//! anything is written, and writes it in one commit. Each write records one `skill.change` entry
//! in the activity log (definitions category).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use schema::{Invalid, Issue, SkillMd, is_skill_name, parse_skill_md, skill_name_problem};
use serde_json::json;
use state::NewActivity;
use store::{
    Change, MAX_SKILL_BYTES, MAX_SKILL_FILES, MAX_SKILL_TEXT_BYTES, SKILL_FILE, SKILLS_DIR,
    SkillTree, is_skill_folder, normalise_skill_path, skill_limits,
};

use crate::{Error, Result, Writer, activity::acting};

/// The largest zip `POST /api/skills/import` takes, in bytes: a skill at its size limit, with room
/// for the zip's own headers.
pub const MAX_SKILL_ZIP: usize = 24 * 1024 * 1024;

/// The vault path of skill `name`'s folder.
pub fn skill_dir(name: &str) -> String {
    format!("{SKILLS_DIR}/{name}")
}

/// One file of a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFileInfo {
    /// The path from the skill's folder, with `/` separators.
    pub path: String,
    pub size: u64,
    /// The media type its name suggests.
    pub media_type: &'static str,
    /// Whether it reads as UTF-8 text with no NUL byte, so it can be shown and edited as text.
    pub text: bool,
}

/// A skill as it is on disk: its folder name, which is its identity, what its `SKILL.md` says,
/// its files, and every problem with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// The folder name.
    pub name: String,
    /// The frontmatter and body of `SKILL.md`; empty when it is missing or unreadable.
    pub md: SkillMd,
    /// Every file, sorted by path.
    pub files: Vec<SkillFileInfo>,
    /// The files' total size in bytes.
    pub size: u64,
    /// The errors and warnings: of `SKILL.md`, of the limits, and of what the folder holds.
    pub issues: Vec<Issue>,
}

impl Skill {
    /// Whether the skill has no errors: only a valid skill is offered to agents.
    pub fn valid(&self) -> bool {
        !self.issues.iter().any(|i| !i.is_warning())
    }

    /// The file at `path`, if the skill has it.
    pub fn file(&self, path: &str) -> Option<&SkillFileInfo> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// The media type a skill file's name suggests.
pub fn skill_media_type(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => "text/markdown",
        "txt" | "text" | "log" | "wcl" | "ws" | "ini" | "cfg" | "conf" => "text/plain",
        "py" => "text/x-python",
        "sh" | "bash" | "zsh" => "text/x-shellscript",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/x-typescript",
        "rb" => "text/x-ruby",
        "rs" => "text/x-rust",
        "go" => "text/x-go",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ if name == "LICENSE" || name == "NOTICE" || name == "README" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Whether `bytes`, the start of a file, look like text: UTF-8 with no NUL. A multi-byte character
/// cut at the end of the sample is allowed.
fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && bytes.len() - e.valid_up_to() < 4,
    }
}

/// How much of a file is read to tell text from binary.
const SNIFF: u64 = 8 * 1024;

fn missing_skill(name: &str) -> Error {
    Error::Missing(format!("there is no skill {name:?}"))
}

/// Names a skill may not take: `import` is the route `POST /api/skills/import`.
pub const RESERVED_SKILL_NAMES: [&str; 1] = ["import"];

/// Whether `name` is a skill name a write may create: valid and not reserved.
fn writable_name(name: &str) -> bool {
    is_skill_name(name) && !RESERVED_SKILL_NAMES.contains(&name)
}

fn bad_name(name: &str) -> Error {
    match skill_name_problem(name) {
        Some(problem) => {
            Error::BadRequest(format!("{name:?} is not a valid skill name: it {problem}"))
        }
        None => Error::BadRequest(format!("{name:?} is reserved and cannot name a skill")),
    }
}

fn invalid(name: &str, issues: Vec<Issue>) -> Error {
    Error::Invalid(Invalid {
        file: format!("{}/{SKILL_FILE}", skill_dir(name)),
        issues,
    })
}

/// What a skill write did, for its activity entry.
enum SkillAct<'a> {
    Create,
    Update,
    WriteFile(&'a str),
    DeleteFile(&'a str),
    Delete,
    Import { replaced: bool, files: usize },
}

impl Writer {
    // ---- reads ----------------------------------------------------------------------------

    /// Every skill under `skills/`, sorted by folder name, valid or not. A folder with no files
    /// at all, such as one a sync left empty, is not a skill.
    pub fn skills(&self) -> Result<Vec<Skill>> {
        let mut out = Vec::new();
        for folder in self.vault.skill_folders()? {
            let tree = self.vault.skill_tree(&folder)?;
            if tree.files.is_empty() && tree.symlinks.is_empty() && tree.other.is_empty() {
                continue;
            }
            out.push(self.skill_from(&folder, &tree)?);
        }
        Ok(out)
    }

    /// The valid skills: those an agent may be offered.
    pub fn valid_skills(&self) -> Result<Vec<Skill>> {
        Ok(self.skills()?.into_iter().filter(Skill::valid).collect())
    }

    /// Skill `name`, with its body and files.
    pub fn skill(&self, name: &str) -> Result<Skill> {
        let tree = self.skill_tree_of(name)?;
        self.skill_from(name, &tree)
    }

    /// The bytes of file `path` of skill `name`. `path` is normalised and confined to the skill.
    pub fn skill_file(&self, name: &str, path: &str) -> Result<Vec<u8>> {
        self.skill_tree_of(name)?;
        match self.vault.read_skill_file(name, path) {
            Err(store::Error::NotFound(_)) => Err(Error::Missing(format!(
                "skill {name:?} has no file {path:?}"
            ))),
            Err(e @ store::Error::InvalidPath { .. }) => Err(Error::BadRequest(e.to_string())),
            other => Ok(other?),
        }
    }

    /// File `path` of skill `name` as text, for a model to read: refused when it is over
    /// [`store::MAX_SKILL_TEXT_BYTES`] or is not UTF-8.
    pub fn skill_text(&self, name: &str, path: &str) -> Result<String> {
        let bytes = self.skill_file(name, path)?;
        if bytes.len() as u64 > MAX_SKILL_TEXT_BYTES {
            return Err(Error::TooLarge(format!(
                "{path} is {} bytes; a skill file read as text may be at most {} MiB",
                bytes.len(),
                MAX_SKILL_TEXT_BYTES / (1024 * 1024)
            )));
        }
        String::from_utf8(bytes)
            .map_err(|_| Error::BadRequest(format!("{path} in skill {name:?} is not text")))
    }

    /// The tree of skill folder `name`; a folder that is missing, or holds nothing, is
    /// [`Error::Missing`].
    fn skill_tree_of(&self, name: &str) -> Result<SkillTree> {
        if !is_skill_folder(name) {
            return Err(missing_skill(name));
        }
        match self.vault.skill_tree(name) {
            Err(store::Error::NotFound(_)) => Err(missing_skill(name)),
            Ok(t) if t.files.is_empty() && t.symlinks.is_empty() && t.other.is_empty() => {
                Err(missing_skill(name))
            }
            other => Ok(other?),
        }
    }

    fn skill_from(&self, name: &str, tree: &SkillTree) -> Result<Skill> {
        let mut issues = Vec::new();
        let mut files = Vec::with_capacity(tree.files.len());
        for f in &tree.files {
            let head = self
                .vault
                .read_range(&format!("{}/{}", skill_dir(name), f.path), 0, SNIFF)
                .unwrap_or_default();
            files.push(SkillFileInfo {
                path: f.path.clone(),
                size: f.size,
                media_type: skill_media_type(&f.path),
                text: looks_like_text(&head),
            });
        }
        let md = match tree.files.iter().any(|f| f.path == SKILL_FILE) {
            false => {
                issues.push(Issue::new(SKILL_FILE, "the skill has no SKILL.md"));
                SkillMd::default()
            }
            true => match self
                .vault
                .read_text(&format!("{}/{SKILL_FILE}", skill_dir(name)))
            {
                Ok(text) => {
                    let (md, found) = parse_skill_md(&text, name);
                    issues.extend(found);
                    md
                }
                Err(_) => {
                    issues.push(Issue::new(SKILL_FILE, "SKILL.md is not UTF-8 text"));
                    SkillMd::default()
                }
            },
        };
        for link in &tree.symlinks {
            issues.push(Issue::new(
                "files",
                format!("{link} is a symlink, which a skill may not hold; it is ignored"),
            ));
        }
        for other in &tree.other {
            issues.push(Issue::new(
                "files",
                format!("{other} is not a plain file or folder with a UTF-8 name"),
            ));
        }
        if let Some(over) = skill_limits(tree.files.iter().map(|f| (f.path.as_str(), f.size))) {
            issues.push(Issue::new("files", over));
        }
        Ok(Skill {
            name: name.to_owned(),
            md,
            size: tree.size(),
            files,
            issues,
        })
    }

    // ---- checks ---------------------------------------------------------------------------

    /// Check `text` as `SKILL.md` of skill `name` as [`Writer::put_skill_md`] does, without
    /// writing it: the name, the specification's constraints and the limits. Returns its
    /// warnings, and whether writing it would create the skill.
    pub fn check_skill_md(&self, name: &str, text: &str) -> Result<(Vec<Issue>, bool)> {
        if !writable_name(name) {
            return Err(bad_name(name));
        }
        let (_, issues) = parse_skill_md(text, name);
        if issues.iter().any(|i| !i.is_warning()) {
            return Err(invalid(name, issues));
        }
        let tree = self.existing_tree(name)?;
        let created = tree.as_ref().is_none_or(|t| t.files.is_empty());
        let files = tree.map(|t| t.files).unwrap_or_default();
        self.check_limits(&files, &[(SKILL_FILE, text.len() as u64)])?;
        Ok((issues, created))
    }

    /// Check a file of `size` bytes at `path` of skill `name` as [`Writer::put_skill_file`]
    /// does, without writing it: the name, the path confined to the skill and clear of its other
    /// files and folders, and the limits. `SKILL.md` is refused: it goes through
    /// [`Writer::check_skill_md`]. Returns whether the skill exists, which the write needs.
    pub fn check_skill_file(&self, name: &str, path: &str, size: u64) -> Result<bool> {
        let rel = normalise_skill_path(path).map_err(|e| Error::BadRequest(e.to_string()))?;
        if rel == SKILL_FILE {
            return Err(Error::BadRequest(
                "SKILL.md is the skill's instructions: write it as SKILL.md, not as a file"
                    .to_owned(),
            ));
        }
        if !writable_name(name) {
            return Err(bad_name(name));
        }
        let files = self
            .existing_tree(name)?
            .filter(|t| !t.files.is_empty())
            .map(|t| t.files);
        let exists = files.is_some();
        let files = files.unwrap_or_default();
        if files.iter().any(|f| f.path.starts_with(&format!("{rel}/"))) {
            return Err(Error::BadRequest(format!(
                "{rel} is a folder in skill {name:?}"
            )));
        }
        if let Some(f) = files
            .iter()
            .find(|f| rel.starts_with(&format!("{}/", f.path)))
        {
            return Err(Error::BadRequest(format!(
                "{} is a file in skill {name:?}, so it cannot hold {rel}",
                f.path
            )));
        }
        self.check_limits(&files, &[(&rel, size)])?;
        Ok(exists)
    }

    // ---- writes ---------------------------------------------------------------------------

    /// Create or replace `SKILL.md` of skill `name` with `text`, which must validate as the
    /// skill's: its `name` equal to `name`, and every other constraint of the specification.
    /// Warnings are kept. Returns the skill as written, and whether it was created.
    pub fn put_skill_md(&self, name: &str, text: &str) -> Result<(Skill, bool)> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let (_, created) = self.check_skill_md(name, text)?;
        let path = self.vault.skill_file_path(name, SKILL_FILE)?;
        let generation = self.commit(vec![Change::write(path, text.as_bytes())])?;
        inner.index.advance(generation)?;
        drop(inner);
        self.record_skill(
            name,
            if created {
                SkillAct::Create
            } else {
                SkillAct::Update
            },
        );
        Ok((self.skill(name)?, created))
    }

    /// Create or replace file `path` of skill `name`, which must exist. `SKILL.md` goes through
    /// [`Writer::put_skill_md`]. Returns the file as written.
    pub fn put_skill_file(&self, name: &str, path: &str, bytes: &[u8]) -> Result<SkillFileInfo> {
        let rel = normalise_skill_path(path).map_err(|e| Error::BadRequest(e.to_string()))?;
        if rel == SKILL_FILE {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::BadRequest("SKILL.md must be UTF-8 text".to_owned()))?;
            let (skill, _) = self.put_skill_md(name, text)?;
            return skill
                .file(SKILL_FILE)
                .cloned()
                .ok_or_else(|| missing_skill(name));
        }
        let _w = self.write_lock();
        let mut inner = self.lock();
        if !self.check_skill_file(name, &rel, bytes.len() as u64)? {
            return Err(missing_skill(name));
        }
        let vault_path = self
            .vault
            .skill_file_path(name, &rel)
            .map_err(|e| Error::BadRequest(e.to_string()))?;
        let generation = self.commit(vec![Change::write(vault_path, bytes)])?;
        inner.index.advance(generation)?;
        drop(inner);
        self.record_skill(name, SkillAct::WriteFile(&rel));
        Ok(SkillFileInfo {
            media_type: skill_media_type(&rel),
            text: looks_like_text(&bytes[..bytes.len().min(SNIFF as usize)]),
            size: bytes.len() as u64,
            path: rel,
        })
    }

    /// Delete file `path` of skill `name`, or every file in folder `path`. `SKILL.md` cannot be
    /// deleted on its own: delete the skill. Returns the paths deleted.
    pub fn delete_skill_file(&self, name: &str, path: &str) -> Result<Vec<String>> {
        let rel = normalise_skill_path(path).map_err(|e| Error::BadRequest(e.to_string()))?;
        if rel == SKILL_FILE {
            return Err(Error::BadRequest(
                "SKILL.md makes the folder a skill; delete the skill instead".to_owned(),
            ));
        }
        let _w = self.write_lock();
        let mut inner = self.lock();
        let tree = self.skill_tree_of(name)?;
        let prefix = format!("{rel}/");
        let gone: Vec<String> = tree
            .files
            .iter()
            .filter(|f| f.path == rel || f.path.starts_with(&prefix))
            .map(|f| f.path.clone())
            .collect();
        let links: Vec<&String> = tree
            .symlinks
            .iter()
            .filter(|l| **l == rel || l.starts_with(&prefix))
            .collect();
        if gone.is_empty() && links.is_empty() {
            return Err(Error::Missing(format!(
                "skill {name:?} has no file {rel:?}"
            )));
        }
        if gone.iter().any(|p| p == SKILL_FILE) {
            return Err(Error::BadRequest(
                "SKILL.md makes the folder a skill; delete the skill instead".to_owned(),
            ));
        }
        if !gone.is_empty() {
            let changes = gone
                .iter()
                .map(|p| Change::delete(format!("{}/{p}", skill_dir(name))))
                .collect();
            let generation = self.commit(changes)?;
            inner.index.advance(generation)?;
        }
        self.remove_links(name, &links)?;
        self.vault.prune_skill_folder(name)?;
        drop(inner);
        self.record_skill(name, SkillAct::DeleteFile(&rel));
        Ok(gone)
    }

    /// Delete skill `name`: its whole folder.
    pub fn delete_skill(&self, name: &str) -> Result<()> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let tree = self.skill_tree_of(name)?;
        if !tree.files.is_empty() {
            let changes = tree
                .files
                .iter()
                .map(|f| Change::delete(format!("{}/{}", skill_dir(name), f.path)))
                .collect();
            let generation = self.commit(changes)?;
            inner.index.advance(generation)?;
        }
        let links: Vec<&String> = tree.symlinks.iter().collect();
        self.remove_links(name, &links)?;
        self.vault.prune_skill_folder(name)?;
        drop(inner);
        self.record_skill(name, SkillAct::Delete);
        Ok(())
    }

    /// Import a skill from `zip`: either one top-level folder holding `SKILL.md`, or `SKILL.md`
    /// at the root. The skill is named by its frontmatter `name`, whatever the folder in the zip
    /// is called. An existing skill of that name is refused unless `replace`, which replaces its
    /// files with the zip's. Everything is checked before anything is written, and the write is
    /// one commit. Returns the skill and whether it replaced one.
    pub fn import_skill_zip(&self, zip: &[u8], replace: bool) -> Result<(Skill, bool)> {
        let files = unzip_skill(zip)?;
        let text = files
            .get(SKILL_FILE)
            .map(|b| std::str::from_utf8(b))
            .ok_or_else(|| Error::BadRequest("the zip holds no SKILL.md".to_owned()))?
            .map_err(|_| Error::BadRequest("SKILL.md must be UTF-8 text".to_owned()))?;
        let name = match parse_skill_md(text, "").0.name {
            Some(n) if is_skill_name(&n) => n,
            _ => {
                let (_, mut issues) = parse_skill_md(text, "");
                issues.retain(|i| i.at != "name" || !i.message.starts_with("must equal"));
                if !issues.iter().any(|i| i.at == "name") {
                    issues.push(Issue::new("name", "is required"));
                }
                return Err(Error::Invalid(Invalid {
                    file: SKILL_FILE.to_owned(),
                    issues,
                }));
            }
        };
        if !writable_name(&name) {
            return Err(bad_name(&name));
        }
        let (_, issues) = parse_skill_md(text, &name);
        if issues.iter().any(|i| !i.is_warning()) {
            return Err(invalid(&name, issues));
        }
        if let Some(over) = skill_limits(files.iter().map(|(p, b)| (p.as_str(), b.len() as u64))) {
            return Err(Error::TooLarge(over));
        }
        let _w = self.write_lock();
        let mut inner = self.lock();
        let existing = self.existing_tree(&name)?;
        let replaced = existing
            .as_ref()
            .is_some_and(|t| !t.files.is_empty() || !t.symlinks.is_empty());
        if replaced && !replace {
            return Err(Error::Exists {
                code: "skill_exists",
                name: name.clone(),
                message: format!(
                    "there is already a skill {name:?}; import with replace=true to replace it"
                ),
            });
        }
        let mut changes = Vec::with_capacity(files.len());
        for (rel, bytes) in &files {
            let path = self
                .vault
                .skill_file_path(&name, rel)
                .map_err(|e| Error::BadRequest(e.to_string()))?;
            changes.push(Change::write(path, bytes.clone()));
        }
        let links: Vec<String> = existing
            .as_ref()
            .map(|t| t.symlinks.clone())
            .unwrap_or_default();
        if let Some(t) = &existing {
            for f in &t.files {
                if !files.contains_key(&f.path) {
                    changes.push(Change::delete(format!("{}/{}", skill_dir(&name), f.path)));
                }
            }
        }
        let generation = self.commit(changes)?;
        inner.index.advance(generation)?;
        self.remove_links(&name, &links.iter().collect::<Vec<_>>())?;
        self.vault.prune_skill_folder(&name)?;
        drop(inner);
        self.record_skill(
            &name,
            SkillAct::Import {
                replaced,
                files: files.len(),
            },
        );
        Ok((self.skill(&name)?, replaced))
    }

    /// Skill `name` as a zip of one folder, `<name>/`, holding its files. Symlinks are left out.
    pub fn export_skill_zip(&self, name: &str) -> Result<Vec<u8>> {
        let tree = self.skill_tree_of(name)?;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let fail = |e: zip::result::ZipError| Error::BadRequest(format!("writing the zip: {e}"));
        for f in &tree.files {
            let bytes = self
                .vault
                .read(&format!("{}/{}", skill_dir(name), f.path))?;
            zip.start_file(format!("{name}/{}", f.path), options)
                .map_err(fail)?;
            zip.write_all(&bytes)
                .map_err(|e| Error::BadRequest(format!("writing the zip: {e}")))?;
        }
        Ok(zip.finish().map_err(fail)?.into_inner())
    }

    /// The tree of skill folder `name` if there is one.
    fn existing_tree(&self, name: &str) -> Result<Option<SkillTree>> {
        match self.vault.skill_tree(name) {
            Ok(t) => Ok(Some(t)),
            Err(store::Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Refuse the change when `files` with `adds` written break a limit.
    fn check_limits(&self, files: &[store::SkillFile], adds: &[(&str, u64)]) -> Result<()> {
        let mut after: BTreeMap<&str, u64> =
            files.iter().map(|f| (f.path.as_str(), f.size)).collect();
        for (p, size) in adds {
            after.insert(p, *size);
        }
        match skill_limits(after) {
            Some(over) => Err(Error::TooLarge(over)),
            None => Ok(()),
        }
    }

    /// Remove symlinks found in skill `name`: a commit never touches a symlink.
    fn remove_links(&self, name: &str, links: &[&String]) -> Result<()> {
        for link in links {
            let full = self.vault.root().join(skill_dir(name)).join(link);
            match std::fs::remove_file(&full) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(Error::Store(store::Error::Io {
                        path: full,
                        source: e,
                    }));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Record one `skill.change` entry (skills S7, activity Y2).
    fn record_skill(&self, name: &str, act: SkillAct<'_>) {
        let (action, summary, path, files) = match act {
            SkillAct::Create => ("create", format!("Created skill {name}"), None, None),
            SkillAct::Update => ("update", format!("Changed skill {name}"), None, None),
            SkillAct::WriteFile(p) => (
                "write_file",
                format!("Changed skill {name}: wrote {p}"),
                Some(p),
                None,
            ),
            SkillAct::DeleteFile(p) => (
                "delete_file",
                format!("Changed skill {name}: deleted {p}"),
                Some(p),
                None,
            ),
            SkillAct::Delete => ("delete", format!("Deleted skill {name}"), None, None),
            SkillAct::Import { replaced, files } => (
                "import",
                if replaced {
                    format!("Replaced skill {name} from a zip")
                } else {
                    format!("Imported skill {name} from a zip")
                },
                None,
                Some(files),
            ),
        };
        let entry = match acting() {
            Some(by) => NewActivity::new("skill.change", "definitions", &by, summary),
            None => NewActivity::server("skill.change", "definitions", summary),
        };
        let mut detail = json!({ "name": name, "action": action, "path": skill_dir(name) });
        if let Some(p) = path {
            detail["file"] = json!(p);
        }
        if let Some(n) = files {
            detail["files"] = json!(n);
        }
        self.state
            .record_activity(entry.subject("skill", name).detail(detail));
    }
}

/// The files of a skill zip, by path from the skill's folder: `SKILL.md` at the root, or one
/// top-level folder holding it. Folders, `__MACOSX/` and `.DS_Store` are skipped; a symlink, an
/// encrypted entry, a path that leaves the folder, or more than the limits allow is refused.
fn unzip_skill(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let bad = |m: String| Error::BadRequest(m);
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| bad(format!("not a zip: {e}")))?;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| bad(format!("reading the zip: {e}")))?;
        let raw = file
            .name()
            .map_err(|e| bad(format!("reading the zip: {e}")))?
            .into_owned();
        if file.is_dir() || raw.ends_with('/') {
            continue;
        }
        let path = normalise_skill_path(&raw).map_err(|e| bad(e.to_string()))?;
        let leaf = path.rsplit('/').next().unwrap_or(&path);
        if path.starts_with("__MACOSX/") || leaf == ".DS_Store" {
            continue;
        }
        if file.is_symlink() {
            return Err(bad(format!(
                "{path} is a symlink, which a skill may not hold"
            )));
        }
        if file.encrypted() {
            return Err(bad(format!("{path} is encrypted")));
        }
        if entries.len() >= MAX_SKILL_FILES {
            return Err(Error::TooLarge(format!(
                "a skill may hold at most {MAX_SKILL_FILES} files; the zip holds more"
            )));
        }
        let room = MAX_SKILL_BYTES - total;
        if file.size() > room {
            return Err(too_big());
        }
        let mut out = Vec::with_capacity(file.size() as usize);
        (&mut file)
            .take(room + 1)
            .read_to_end(&mut out)
            .map_err(|e| bad(format!("reading {path} from the zip: {e}")))?;
        if out.len() as u64 > room {
            return Err(too_big());
        }
        total += out.len() as u64;
        entries.push((path, out));
    }
    let root_skill = entries.iter().any(|(p, _)| p == SKILL_FILE);
    let prefix = if root_skill {
        String::new()
    } else {
        let tops: BTreeSet<&str> = entries
            .iter()
            .map(|(p, _)| p.split_once('/').map_or("", |(top, _)| top))
            .collect();
        match tops.into_iter().collect::<Vec<_>>().as_slice() {
            [top]
                if !top.is_empty()
                    && entries
                        .iter()
                        .any(|(p, _)| *p == format!("{top}/{SKILL_FILE}")) =>
            {
                format!("{top}/")
            }
            _ => {
                return Err(bad(
                    "the zip must hold SKILL.md at its root, or one folder holding SKILL.md"
                        .to_owned(),
                ));
            }
        }
    };
    let mut files = BTreeMap::new();
    for (path, bytes) in entries {
        let rel = path.strip_prefix(&prefix).unwrap_or(&path).to_owned();
        if files.insert(rel.clone(), bytes).is_some() {
            return Err(bad(format!("the zip holds {rel} twice")));
        }
    }
    let paths: Vec<&String> = files.keys().collect();
    for p in &paths {
        if paths.iter().any(|q| q.starts_with(&format!("{p}/"))) {
            return Err(bad(format!("{p} is both a file and a folder in the zip")));
        }
    }
    Ok(files)
}

fn too_big() -> Error {
    Error::TooLarge(format!(
        "a skill's files may add up to at most {} MiB; the zip's add up to more",
        MAX_SKILL_BYTES / (1024 * 1024)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_told_from_binary() {
        assert!(looks_like_text(b"hello\n"));
        assert!(looks_like_text("é".as_bytes()));
        assert!(looks_like_text(&"é".as_bytes()[..1]));
        assert!(!looks_like_text(b"a\0b"));
        assert!(!looks_like_text(&[0xff, 0xfe, 0x41]));
    }

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(skill_media_type("SKILL.md"), "text/markdown");
        assert_eq!(skill_media_type("scripts/run.PY"), "text/x-python");
        assert_eq!(skill_media_type("assets/logo.png"), "image/png");
        assert_eq!(skill_media_type("LICENSE"), "text/plain");
        assert_eq!(skill_media_type("blob"), "application/octet-stream");
    }
}
