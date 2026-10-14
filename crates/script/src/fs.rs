//! The `fs` host module: files in the directories a scheduled task names, and nowhere else.
//!
//! A task's `read_paths` may be read and its `write_paths` read and written. Every call resolves
//! its path before touching it:
//!
//! - the path must be absolute and hold no `..`;
//! - symbolic links are followed, and it is the real path that must lie under an allowed
//!   directory, itself resolved the same way, so a link cannot lead out of one;
//! - a path inside the vault is refused for reading and writing alike: scripts change notes
//!   through the `vault` module, never by editing vault files, and the vault's `.server` folder
//!   holds the password hash and tokens;
//! - a file is opened without following a link in its last component, so a link swapped in
//!   after the check is refused rather than followed.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use wscript::{Fault, HostError, Module, Script};

use crate::{Vault, run};

/// One directory entry from `fs::list`.
#[derive(Script, Debug, Clone)]
#[script(name = "Entry")]
pub struct Entry {
    pub name: String,
    /// The entry's full path, ready to pass to another `fs` call.
    pub path: String,
    pub is_dir: bool,
    /// Bytes, for a file. `0` for a directory.
    pub size: i64,
    /// Last modified, in milliseconds since the Unix epoch.
    pub modified: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Where `path` really is, if a run with these directories may reach it for `access`.
pub fn resolve(
    path: &str,
    access: Access,
    read_paths: &[String],
    write_paths: &[String],
    protected: &[PathBuf],
) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!(
            "{path:?} is not an absolute path; build one with fs::join"
        ));
    }
    if p.components().any(|c| c == Component::ParentDir) {
        return Err(format!("{path:?} uses \"..\"; give the path without it"));
    }
    let real = real_path(p)?;
    let roots: Vec<PathBuf> = match access {
        Access::Read => read_paths.iter().chain(write_paths).collect::<Vec<_>>(),
        Access::Write => write_paths.iter().collect(),
    }
    .into_iter()
    .filter_map(|r| fs::canonicalize(r).ok())
    .collect();
    for v in protected {
        let v = fs::canonicalize(v).unwrap_or_else(|_| v.clone());
        if real.starts_with(&v) {
            return Err(format!(
                "{path:?} is inside the vault; change notes with the vault module"
            ));
        }
    }
    if !roots.iter().any(|r| real.starts_with(r)) {
        let verb = match access {
            Access::Read => "read",
            Access::Write => "write",
        };
        return Err(format!(
            "{path:?} is outside the directories this task may {verb}"
        ));
    }
    Ok(real)
}

/// `p` with every symbolic link resolved. The part of `p` that does not exist yet is appended to
/// the real path of the part that does.
fn real_path(p: &Path) -> Result<PathBuf, String> {
    let mut missing = Vec::new();
    let mut at = p;
    loop {
        match fs::canonicalize(at) {
            Ok(mut real) => {
                for name in missing.iter().rev() {
                    real.push(name);
                }
                return Ok(real);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A dangling link is there but cannot be resolved: following it could lead
                // anywhere.
                if at.symlink_metadata().is_ok() {
                    return Err(format!("{} is a symbolic link to nothing", at.display()));
                }
                let Some(name) = at.file_name() else {
                    return Err(format!("{} does not exist", p.display()));
                };
                missing.push(name.to_owned());
                at = at
                    .parent()
                    .ok_or_else(|| format!("{} does not exist", p.display()))?;
            }
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
}

/// Resolve `path` for this run, within the deadline.
fn checked(
    path: &str,
    access: Access,
    protected: &[PathBuf],
) -> Result<Result<PathBuf, String>, HostError> {
    let (read, write) = run::with(|r| {
        r.remaining()?;
        Ok((r.job.read_paths.clone(), r.job.write_paths.clone()))
    })?;
    Ok(resolve(path, access, &read, &write, protected))
}

fn max_file() -> Result<usize, HostError> {
    run::with(|r| Ok(r.limits.max_file))
}

/// Open a resolved path without following a link in its last component.
fn open(path: &Path, opts: &mut OpenOptions) -> Result<File, String> {
    opts.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The bytes of the file at a resolved path, up to the size limit.
pub fn read_file(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let file = open(path, OpenOptions::new().read(true))?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() > max {
        return Err(format!(
            "{} is larger than the {max} bytes a script may read",
            path.display()
        ));
    }
    Ok(bytes)
}

/// Read a file a task may read, for `vault::upload_asset_file`.
pub fn read_allowed(
    path: &str,
    protected: &[PathBuf],
) -> Result<Result<Vec<u8>, String>, HostError> {
    let max = max_file()?;
    Ok(checked(path, Access::Read, protected)?.and_then(|p| read_file(&p, max)))
}

fn write_file(path: &Path, bytes: &[u8], append: bool) -> Result<(), String> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    open(path, &mut opts)?
        .write_all(bytes)
        .map_err(|e| format!("{}: {e}", path.display()))
}

type Reply<T> = Fault<Result<T, String>>;

pub fn module(vault: Arc<dyn Vault>) -> Module {
    // Asked at every call: a handler host's vault is bound only after the host is made.
    let protected = vault;
    let mut m = Module::new("fs");
    m.doc("Files in the directories this task names in read_paths and write_paths");

    let p = protected.clone();
    m.fn_named(
        "read_text",
        ["path"],
        move |path: String| -> Reply<String> {
            Fault((|| {
                let max = max_file()?;
                Ok(
                    checked(&path, Access::Read, &p.protected())?.and_then(|real| {
                        String::from_utf8(read_file(&real, max)?)
                            .map_err(|_| format!("{path:?} is not UTF-8 text; use fs::read_bytes"))
                    }),
                )
            })())
        },
    );
    let p = protected.clone();
    m.fn_named(
        "read_bytes",
        ["path"],
        move |path: String| -> Reply<String> {
            Fault((|| {
                let max = max_file()?;
                Ok(checked(&path, Access::Read, &p.protected())?
                    .and_then(|real| read_file(&real, max))
                    .map(|b| BASE64.encode(b)))
            })())
        },
    );
    let p = protected.clone();
    m.fn_named(
        "write_text",
        ["path", "text"],
        move |path: String, text: String| -> Reply<()> {
            Fault(
                checked(&path, Access::Write, &p.protected())
                    .map(|r| r.and_then(|real| write_file(&real, text.as_bytes(), false))),
            )
        },
    );
    let p = protected.clone();
    m.fn_named(
        "append_text",
        ["path", "text"],
        move |path: String, text: String| -> Reply<()> {
            Fault(
                checked(&path, Access::Write, &p.protected())
                    .map(|r| r.and_then(|real| write_file(&real, text.as_bytes(), true))),
            )
        },
    );
    let p = protected.clone();
    m.fn_named(
        "write_bytes",
        ["path", "base64"],
        move |path: String, data: String| -> Reply<()> {
            Fault(checked(&path, Access::Write, &p.protected()).map(|r| {
                r.and_then(|real| {
                    let bytes = BASE64
                        .decode(data.trim())
                        .map_err(|e| format!("the data is not base64: {e}"))?;
                    write_file(&real, &bytes, false)
                })
            }))
        },
    );
    let p = protected.clone();
    m.fn_named("list", ["path"], move |path: String| -> Reply<Vec<Entry>> {
        Fault(checked(&path, Access::Read, &p.protected()).map(|r| r.and_then(|real| list(&real))))
    });
    let p = protected.clone();
    m.fn_named("exists", ["path"], move |path: String| -> Reply<bool> {
        Fault(checked(&path, Access::Read, &p.protected()).map(|r| r.map(|real| real.exists())))
    });
    let p = protected.clone();
    m.fn_named("mkdir", ["path"], move |path: String| -> Reply<()> {
        Fault(checked(&path, Access::Write, &p.protected()).map(|r| {
            r.and_then(|real| {
                fs::create_dir_all(&real).map_err(|e| format!("{}: {e}", real.display()))
            })
        }))
    });
    let p = protected;
    m.fn_named("remove", ["path"], move |path: String| -> Reply<()> {
        Fault(checked(&path, Access::Write, &p.protected()).map(|r| {
            r.and_then(|real| {
                if real.is_dir() {
                    return Err(format!("{path:?} is a directory; fs::remove removes files"));
                }
                fs::remove_file(&real).map_err(|e| format!("{}: {e}", real.display()))
            })
        }))
    });
    m.fn_named(
        "join",
        ["base", "name"],
        |base: String, name: String| -> String {
            Path::new(&base).join(name).to_string_lossy().into_owned()
        },
    );
    m
}

fn list(dir: &Path) -> Result<Vec<Entry>, String> {
    let read = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let meta = entry
            .metadata()
            .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        out.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() {
                0
            } else {
                i64::try_from(meta.len()).unwrap_or(i64::MAX)
            },
            modified,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dirs {
        _tmp: tempfile::TempDir,
        read: PathBuf,
        write: PathBuf,
        outside: PathBuf,
        vault: PathBuf,
    }

    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let d = Dirs {
            read: base.join("read"),
            write: base.join("write"),
            outside: base.join("outside"),
            vault: base.join("write/vault"),
            _tmp: tmp,
        };
        for p in [&d.read, &d.write, &d.outside, &d.vault] {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(d.outside.join("secret.txt"), "secret").unwrap();
        fs::write(d.read.join("in.txt"), "in").unwrap();
        d
    }

    fn check(d: &Dirs, path: &Path, access: Access) -> Result<PathBuf, String> {
        resolve(
            path.to_str().unwrap(),
            access,
            &[d.read.to_string_lossy().into_owned()],
            &[d.write.to_string_lossy().into_owned()],
            std::slice::from_ref(&d.vault),
        )
    }

    #[test]
    fn paths_inside_the_allowed_directories_resolve() {
        let d = dirs();
        assert_eq!(
            check(&d, &d.read.join("in.txt"), Access::Read),
            Ok(d.read.join("in.txt"))
        );
        // A file that does not exist yet, in a directory that does not either.
        let new = d.write.join("new/deeper/out.txt");
        assert_eq!(check(&d, &new, Access::Write), Ok(new));
        // A write directory may be read.
        assert!(check(&d, &d.write, Access::Read).is_ok());
    }

    #[test]
    fn a_read_only_directory_refuses_writes() {
        let d = dirs();
        let err = check(&d, &d.read.join("in.txt"), Access::Write).unwrap_err();
        assert!(
            err.contains("outside the directories this task may write"),
            "{err}"
        );
    }

    #[test]
    fn paths_outside_relative_paths_and_dot_dot_are_refused() {
        let d = dirs();
        let err = check(&d, &d.outside.join("secret.txt"), Access::Read).unwrap_err();
        assert!(err.contains("outside the directories"), "{err}");
        let sneaky = d.read.join("../outside/secret.txt");
        let err = check(&d, &sneaky, Access::Read).unwrap_err();
        assert!(err.contains("\"..\""), "{err}");
        let err = resolve("read/in.txt", Access::Read, &[], &[], &[]).unwrap_err();
        assert!(err.contains("not an absolute path"), "{err}");
    }

    #[test]
    fn a_symbolic_link_out_of_an_allowed_directory_is_refused() {
        let d = dirs();
        std::os::unix::fs::symlink(&d.outside, d.read.join("escape")).unwrap();
        let err = check(&d, &d.read.join("escape/secret.txt"), Access::Read).unwrap_err();
        assert!(err.contains("outside the directories"), "{err}");
        std::os::unix::fs::symlink(d.outside.join("secret.txt"), d.write.join("link.txt")).unwrap();
        let err = check(&d, &d.write.join("link.txt"), Access::Write).unwrap_err();
        assert!(err.contains("outside the directories"), "{err}");
        // A dangling link could be created anywhere by writing through it.
        std::os::unix::fs::symlink(d.outside.join("new.txt"), d.write.join("dangling")).unwrap();
        let err = check(&d, &d.write.join("dangling"), Access::Write).unwrap_err();
        assert!(err.contains("symbolic link to nothing"), "{err}");
    }

    #[test]
    fn the_vault_is_refused_even_inside_an_allowed_directory() {
        let d = dirs();
        for access in [Access::Read, Access::Write] {
            let err = check(&d, &d.vault.join("notes/x.wcl"), access).unwrap_err();
            assert!(err.contains("inside the vault"), "{err}");
        }
        // Through a link, too.
        std::os::unix::fs::symlink(&d.vault, d.write.join("v")).unwrap();
        let err = check(&d, &d.write.join("v/types"), Access::Write).unwrap_err();
        assert!(err.contains("inside the vault"), "{err}");
    }

    #[test]
    fn opening_does_not_follow_a_link_swapped_in_after_the_check() {
        let d = dirs();
        let target = d.write.join("swap.txt");
        let real = check(&d, &target, Access::Write).unwrap();
        std::os::unix::fs::symlink(d.outside.join("secret.txt"), &target).unwrap();
        assert!(write_file(&real, b"overwritten", false).is_err());
        assert_eq!(
            fs::read_to_string(d.outside.join("secret.txt")).unwrap(),
            "secret"
        );
    }
}
