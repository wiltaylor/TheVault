//! The vault folder (D22) and every read from it.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use wcl_lang::ast::Source;
use wcl_lang::{Document, Environment, FileLoader};

use crate::{Error, Result, commit, io_err, parse_data_only};

/// The folders a vault holds files in, in the order `init` creates them. `.vault/` is the server's
/// own folder and is not listed.
pub const LAYOUT_DIRS: [&str; 6] = ["notes", "types", "taxonomies", "scripts", "tasks", "assets"];

/// The server's own folder: `index.db`, `state.db` and the commit journal.
pub(crate) const SERVER_DIR: &str = ".vault";

/// An open vault folder.
pub struct Vault {
    root: PathBuf,
    /// Serialises commits made through this handle.
    pub(crate) commit_lock: Mutex<()>,
}

impl Vault {
    /// Create the D22 layout under `root`, creating `root` itself if needed, and open it. Running it
    /// on an existing vault changes nothing.
    pub fn init(root: &Path) -> Result<Vault> {
        std::fs::create_dir_all(root.join(SERVER_DIR)).map_err(io_err(root))?;
        Vault::open(root)
    }

    /// Open the vault at `root`, recreate any missing layout folder, and finish or discard a
    /// commit a crash interrupted.
    pub fn open(root: &Path) -> Result<Vault> {
        let root = root.canonicalize().map_err(io_err(root))?;
        if !root.join(SERVER_DIR).is_dir() {
            return Err(Error::NotAVault(root));
        }
        for dir in LAYOUT_DIRS {
            let path = root.join(dir);
            std::fs::create_dir_all(&path).map_err(io_err(path))?;
        }
        let vault = Vault {
            root,
            commit_lock: Mutex::new(()),
        };
        commit::recover(&vault)?;
        Ok(vault)
    }

    /// The vault folder, canonicalised.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `.vault/` folder.
    pub fn server_dir(&self) -> PathBuf {
        self.root.join(SERVER_DIR)
    }

    /// Read a vault file's bytes. `path` is relative to the vault, with `/` separators.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let full = self.resolve(path)?;
        let real = self.contain(&full)?;
        std::fs::read(&real).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(path.to_owned()),
            _ => Error::Io {
                path: full,
                source: e,
            },
        })
    }

    /// Read at most `len` bytes of a vault file, from byte `start`.
    pub fn read_range(&self, path: &str, start: u64, len: u64) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let full = self.resolve(path)?;
        let real = self.contain(&full)?;
        let fail = |e: std::io::Error| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(path.to_owned()),
            _ => Error::Io {
                path: full.clone(),
                source: e,
            },
        };
        let mut file = std::fs::File::open(&real).map_err(fail)?;
        file.seek(SeekFrom::Start(start)).map_err(fail)?;
        let mut out = Vec::new();
        file.take(len).read_to_end(&mut out).map_err(fail)?;
        Ok(out)
    }

    /// Read a vault file as text.
    pub fn read_text(&self, path: &str) -> Result<String> {
        String::from_utf8(self.read(path)?).map_err(|_| Error::NotUtf8(path.to_owned()))
    }

    /// Read and parse a WCL vault file, rejecting it unless it is data-only (D20).
    pub fn read_wcl(&self, path: &str) -> Result<Source> {
        parse_data_only(&self.read_text(path)?, path)
    }

    /// Open a WCL vault file as a `wcl_lang` document for evaluation and schema checks. The file
    /// must be data-only, and the document reads any further file through [`Vault::loader`].
    pub fn open_document(&self, path: &str, env: &Environment) -> Result<Document> {
        let text = self.read_text(path)?;
        parse_data_only(&text, path)?;
        let full = self.resolve(path)?;
        let dir = full.parent().map(Path::to_path_buf);
        Document::open_at_with_loader(&text, path, dir, env, self.loader()).map_err(|e| {
            Error::Parse {
                file: path.to_owned(),
                message: e.to_string(),
            }
        })
    }

    /// A `wcl_lang` file loader that reads only files inside the vault, never `.vault/`, and never
    /// through a symlink that leads out. WCL's own loaders fall through to the whole disk.
    pub fn loader(&self) -> FileLoader {
        let root = self.root.clone();
        Arc::new(move |p: &Path| {
            let denied = || {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("{} is outside the vault", p.display()),
                )
            };
            let real = p.canonicalize()?;
            let rel = real.strip_prefix(&root).map_err(|_| denied())?;
            if rel.starts_with(SERVER_DIR) {
                return Err(denied());
            }
            std::fs::read_to_string(&real)
        })
    }

    /// The absolute path of a checked vault-relative path. The file need not exist.
    pub(crate) fn resolve(&self, path: &str) -> Result<PathBuf> {
        check_path(path)?;
        Ok(self.root.join(path))
    }

    /// Fail unless `full`, with every symlink followed, is still inside the vault and outside
    /// `.vault/`. Returns the real path. A path that does not exist yet is checked through its
    /// nearest existing ancestor.
    pub(crate) fn contain(&self, full: &Path) -> Result<PathBuf> {
        let mut existing = full;
        let mut rest = Vec::new();
        let real = loop {
            match existing.canonicalize() {
                Ok(real) => break real,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let Some(parent) = existing.parent() else {
                        return Err(Error::OutsideVault(full.to_path_buf()));
                    };
                    rest.push(existing.file_name().unwrap_or_default().to_owned());
                    existing = parent;
                }
                Err(e) => return Err(io_err(full)(e)),
            }
        };
        let inside = real
            .strip_prefix(&self.root)
            .is_ok_and(|rel| !rel.starts_with(SERVER_DIR) && rel.components().next().is_some());
        if !inside {
            return Err(Error::OutsideVault(full.to_path_buf()));
        }
        Ok(rest.into_iter().rev().fold(real, |p, name| p.join(name)))
    }
}

/// Check a vault-relative path: `/`-separated, under a layout folder, no `.`, `..` or empty
/// segment, and the right extension for its folder.
pub(crate) fn check_path(path: &str) -> Result<()> {
    let fail = |reason: &str| {
        Err(Error::InvalidPath {
            path: path.to_owned(),
            reason: reason.to_owned(),
        })
    };
    if path.contains('\\') || path.contains('\0') {
        return fail("it contains a backslash or NUL");
    }
    if path.starts_with('/') || Path::new(path).is_absolute() {
        return fail("it is absolute; vault paths are relative to the vault folder");
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 {
        return fail("it does not name a file inside a vault folder");
    }
    for seg in &segments {
        if seg.is_empty() || *seg == "." || *seg == ".." {
            return fail("it has an empty, `.` or `..` segment");
        }
    }
    if !Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return fail("it has a segment that is not a plain name");
    }
    let top = segments[0];
    if !LAYOUT_DIRS.contains(&top) {
        return fail("it is not under notes/, types/, taxonomies/, scripts/, tasks/ or assets/");
    }
    let wants = match top {
        "notes" | "types" | "taxonomies" | "tasks" => Some(".wcl"),
        "scripts" => Some(".ws"),
        _ => None,
    };
    if let Some(ext) = wants
        && !path.ends_with(ext)
    {
        return fail(&format!("files under {top}/ end in {ext}"));
    }
    Ok(())
}
