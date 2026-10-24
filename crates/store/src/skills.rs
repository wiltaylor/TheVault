//! The `skills/` folder (skills S1): one folder per Agent Skill, `skills/<name>/`, holding the
//! skill's files exactly as authored. Nothing under it is WCL: the data-only and canonical rules
//! do not apply, and a `.wcl` file inside a skill is stored byte for byte like any other.
//!
//! This module owns what the store can say about the tree without reading `SKILL.md`: which
//! folders there are, the files of one with their sizes, the limits on a skill, and paths inside a
//! skill, normalised and confined to its folder. Symlinks are never followed: a listing reports
//! them, and a read or write through one is refused.

use std::path::{Path, PathBuf};

use crate::{Error, Result, Vault, io_err};

/// The folder skills live in.
pub const SKILLS_DIR: &str = "skills";

/// The file that makes a folder a skill: YAML frontmatter, then Markdown instructions.
pub const SKILL_FILE: &str = "SKILL.md";

/// The most files one skill may hold.
pub const MAX_SKILL_FILES: usize = 200;

/// The most bytes one skill's files may add up to: 20 MiB.
pub const MAX_SKILL_BYTES: u64 = 20 * 1024 * 1024;

/// The largest text file read into a model whole: 2 MiB.
pub const MAX_SKILL_TEXT_BYTES: u64 = 2 * 1024 * 1024;

/// One file of a skill: its path from the skill's folder, with `/` separators, and its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    pub path: String,
    pub size: u64,
}

/// What is inside one skill folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillTree {
    /// Every regular file, sorted by path.
    pub files: Vec<SkillFile>,
    /// Every symlink found, by path from the skill's folder. They are not followed.
    pub symlinks: Vec<String>,
    /// Entries whose names are not UTF-8, or are neither files, folders nor symlinks.
    pub other: Vec<String>,
}

impl SkillTree {
    /// The total size of the files.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Whether `name` can be a skill folder's name on disk: one plain segment that is not hidden.
/// Whether it is also a valid skill name is the schema's question.
pub fn is_skill_folder(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.len() <= 255
}

/// Whether vault path `path` is a file inside a skill's folder.
pub fn is_skill_path(path: &str) -> bool {
    path.strip_prefix("skills/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(folder, file)| !folder.is_empty() && !file.is_empty())
}

/// `rel`, a path inside a skill, normalised: `\` read as `/`, `.` segments, a leading `./` and
/// repeated slashes dropped. Refused when it is empty, absolute, holds a `..` segment or a NUL, or
/// is longer than 1024 bytes.
pub fn normalise_skill_path(rel: &str) -> Result<String> {
    let fail = |reason: &str| {
        Err(Error::InvalidPath {
            path: rel.to_owned(),
            reason: reason.to_owned(),
        })
    };
    if rel.contains('\0') {
        return fail("it contains a NUL");
    }
    let unified = rel.replace('\\', "/");
    let drive = unified.as_bytes().get(1) == Some(&b':');
    if unified.starts_with('/') || Path::new(&unified).is_absolute() || drive {
        return fail("it is absolute; a skill's paths are relative to its folder");
    }
    let mut segments = Vec::new();
    for seg in unified.split('/') {
        match seg {
            "" | "." => {}
            ".." => return fail("it leaves the skill's folder with `..`"),
            s => segments.push(s),
        }
    }
    if segments.is_empty() {
        return fail("it names no file");
    }
    let out = segments.join("/");
    if out.len() > 1024 {
        return fail("it is longer than 1024 bytes");
    }
    Ok(out)
}

/// Why a skill with these files is over the limits, or `None` when it is within them.
pub fn skill_limits<'a>(files: impl IntoIterator<Item = (&'a str, u64)>) -> Option<String> {
    let (mut count, mut total) = (0usize, 0u64);
    for (_, size) in files {
        count += 1;
        total += size;
    }
    if count > MAX_SKILL_FILES {
        return Some(format!(
            "a skill may hold at most {MAX_SKILL_FILES} files; this one would hold {count}"
        ));
    }
    if total > MAX_SKILL_BYTES {
        return Some(format!(
            "a skill's files may add up to at most {} MiB; these add up to {total} bytes",
            MAX_SKILL_BYTES / (1024 * 1024)
        ));
    }
    None
}

impl Vault {
    /// Every folder under `skills/` whose name could be a skill's, sorted. Symlinks, files and
    /// hidden entries are left out.
    pub fn skill_folders(&self) -> Result<Vec<String>> {
        let dir = self.root().join(SKILLS_DIR);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(&dir)(e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_err(&dir))?;
            let kind = entry.file_type().map_err(io_err(entry.path()))?;
            if let Some(name) = entry.file_name().to_str()
                && kind.is_dir()
                && is_skill_folder(name)
            {
                out.push(name.to_owned());
            }
        }
        out.sort();
        Ok(out)
    }

    /// The files of skill folder `folder`, walked without following symlinks. A folder that does
    /// not exist is [`Error::NotFound`].
    pub fn skill_tree(&self, folder: &str) -> Result<SkillTree> {
        let root = self.skill_root(folder)?;
        match std::fs::symlink_metadata(&root) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(Error::NotFound(format!("{SKILLS_DIR}/{folder}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound(format!("{SKILLS_DIR}/{folder}")));
            }
            Err(e) => return Err(io_err(&root)(e)),
        }
        let mut tree = SkillTree::default();
        walk(&root, "", &mut tree)?;
        tree.files.sort_by(|a, b| a.path.cmp(&b.path));
        tree.symlinks.sort();
        tree.other.sort();
        Ok(tree)
    }

    /// The vault path of `rel` inside skill folder `folder`, normalised, after checking that no
    /// part of it that exists is a symlink. The file need not exist.
    pub fn skill_file_path(&self, folder: &str, rel: &str) -> Result<String> {
        let rel = normalise_skill_path(rel)?;
        let root = self.skill_root(folder)?;
        let mut at = root.clone();
        for part in std::iter::once(None).chain(rel.split('/').map(Some)) {
            if let Some(part) = part {
                at = at.join(part);
            }
            match std::fs::symlink_metadata(&at) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(Error::InvalidPath {
                        path: format!("{SKILLS_DIR}/{folder}/{rel}"),
                        reason: "it goes through a symlink, which a skill may not hold".to_owned(),
                    });
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(io_err(&at)(e)),
            }
        }
        let path = format!("{SKILLS_DIR}/{folder}/{rel}");
        self.contain(&self.resolve(&path)?)?;
        Ok(path)
    }

    /// The bytes of `rel` inside skill folder `folder`.
    pub fn read_skill_file(&self, folder: &str, rel: &str) -> Result<Vec<u8>> {
        let path = self.skill_file_path(folder, rel)?;
        let full = self.root().join(&path);
        if full.is_dir() {
            return Err(Error::NotFound(path));
        }
        self.read(&path)
    }

    /// Remove the empty folders inside skill folder `folder`, and the folder itself once it is
    /// empty: a commit removes files, never folders.
    pub fn prune_skill_folder(&self, folder: &str) -> Result<()> {
        let root = self.skill_root(folder)?;
        if std::fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir()) {
            prune(&root)?;
        }
        Ok(())
    }

    fn skill_root(&self, folder: &str) -> Result<PathBuf> {
        if !is_skill_folder(folder) {
            return Err(Error::InvalidPath {
                path: format!("{SKILLS_DIR}/{folder}"),
                reason: "a skill's folder is one plain name that does not start with `.`"
                    .to_owned(),
            });
        }
        Ok(self.root().join(SKILLS_DIR).join(folder))
    }
}

fn walk(dir: &Path, rel: &str, tree: &mut SkillTree) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(io_err(dir))?;
    for entry in entries {
        let entry = entry.map_err(io_err(dir))?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            tree.other
                .push(format!("{rel}{}", entry.file_name().to_string_lossy()));
            continue;
        };
        let child = format!("{rel}{name}");
        let kind = entry.file_type().map_err(io_err(&path))?;
        if kind.is_symlink() {
            tree.symlinks.push(child);
        } else if kind.is_dir() {
            walk(&path, &format!("{child}/"), tree)?;
        } else if kind.is_file() {
            let size = entry.metadata().map_err(io_err(&path))?.len();
            tree.files.push(SkillFile { path: child, size });
        } else {
            tree.other.push(child);
        }
    }
    Ok(())
}

/// Remove every empty folder under `dir`, then `dir` if it is empty. Returns whether `dir` went.
fn prune(dir: &Path) -> Result<bool> {
    let mut empty = true;
    for entry in std::fs::read_dir(dir).map_err(io_err(dir))? {
        let entry = entry.map_err(io_err(dir))?;
        let kind = entry.file_type().map_err(io_err(entry.path()))?;
        if !(kind.is_dir() && prune(&entry.path())?) {
            empty = false;
        }
    }
    if empty {
        std::fs::remove_dir(dir).map_err(io_err(dir))?;
    }
    Ok(empty)
}
