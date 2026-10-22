//! `vault backup` and `vault restore-backup` (D25).
//!
//! A backup is one SQLite Archive (`sqlar`) file: every file under the vault's layout folders, a
//! consistent copy of `.vault/state.db`, and, only with `--include-key`, the secret key as
//! `key/<vault-id>.key`. `sqlite3 -A` lists and extracts it as well. Entries are stored
//! uncompressed. `index.db` and the full-text index are left out because the server rebuilds them
//! from the files when it starts.

use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};

use crate::Fail;
use crate::local::{key_path, open_state, state_db};

const STATE_ENTRY: &str = ".vault/state.db";
const KEY_DIR: &str = "key";

fn sqlite(e: rusqlite::Error) -> Fail {
    Fail::new("archive", e.to_string())
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Fail + '_ {
    move |e| Fail::new("io", format!("{}: {e}", path.display()))
}

/// `vault backup`: write `vault`'s files and `state.db` into a new archive at `out`.
pub fn backup(vault: &Path, out: &Path, include_key: bool) -> Result<Value, Fail> {
    let state = open_state(vault)?;
    let vault_id = state.vault_id().map_err(Fail::from_display("state"))?;
    if out.exists() {
        return Err(Fail::new(
            "exists",
            format!(
                "{} already exists; backup never overwrites a file",
                out.display()
            ),
        ));
    }
    let key_file = key_path(&vault_id)?;
    if include_key && !key_file.is_file() {
        return Err(Fail::new(
            "secret_key",
            format!(
                "--include-key: there is no key file at {}",
                key_file.display()
            ),
        ));
    }

    // The archive's folder may not exist yet, as /data/backups on a fresh container (ticket 133).
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }

    let scratch = tempfile::tempdir().map_err(Fail::from_display("io"))?;
    let state_copy = scratch.path().join("state.db");
    state
        .backup_to(&state_copy)
        .map_err(Fail::from_display("state"))?;
    drop(state);

    // Build the archive beside `out` and move it into place, so a failed backup leaves nothing.
    let partial = out.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let result = write_archive(
        vault,
        &partial,
        &state_copy,
        include_key.then_some(&key_file),
        &vault_id,
    );
    let files = match result {
        Ok(files) => files,
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
    };
    std::fs::rename(&partial, out).map_err(io(out))?;

    let key = if include_key {
        json!({ "included": true, "path": key_file })
    } else {
        let message = format!(
            "the secret key is not in this backup; keep {} safe, or pass --include-key",
            key_file.display()
        );
        eprintln!("vault backup: {message}");
        json!({ "included": false, "path": key_file, "message": message })
    };
    Ok(json!({
        "archive": out,
        "vault_id": vault_id,
        "files": files,
        "state_db": true,
        "key": key,
    }))
}

/// Write the archive and return how many vault files it holds, not counting `state.db` or the
/// key.
fn write_archive(
    vault: &Path,
    out: &Path,
    state_copy: &Path,
    key: Option<&PathBuf>,
    vault_id: &str,
) -> Result<usize, Fail> {
    let mut conn = Connection::open(out).map_err(sqlite)?;
    conn.execute_batch(
        "CREATE TABLE sqlar (
            name  TEXT PRIMARY KEY,
            mode  INT,
            mtime INT,
            sz    INT,
            data  BLOB
        );",
    )
    .map_err(sqlite)?;
    let tx = conn.transaction().map_err(sqlite)?;
    let mut files = Vec::new();
    for dir in store::LAYOUT_DIRS {
        walk(&vault.join(dir), dir, &mut files)?;
    }
    let count = files.len();
    files.push((STATE_ENTRY.to_owned(), state_copy.to_path_buf()));
    if let Some(key) = key {
        files.push((format!("{KEY_DIR}/{vault_id}.key"), key.clone()));
    }
    {
        let mut insert = tx
            .prepare("INSERT INTO sqlar (name, mode, mtime, sz, data) VALUES (?1, ?2, ?3, ?4, ?5)")
            .map_err(sqlite)?;
        for (name, path) in &files {
            let data = std::fs::read(path).map_err(io(path))?;
            let meta = std::fs::metadata(path).map_err(io(path))?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            insert
                .execute(params![name, mode(&meta), mtime, data.len() as i64, data])
                .map_err(sqlite)?;
        }
    }
    tx.commit().map_err(sqlite)?;
    Ok(count)
}

#[cfg(unix)]
fn mode(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::PermissionsExt;
    i64::from(meta.permissions().mode())
}

#[cfg(not(unix))]
fn mode(_: &std::fs::Metadata) -> i64 {
    0o100644
}

/// Every regular file under `dir`, named by its `/`-separated path from the vault folder.
fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) -> Result<(), Fail> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(dir)(e)),
    };
    let mut entries: Vec<_> = entries.collect::<Result<_, _>>().map_err(io(dir))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let Some(name) = e.file_name().to_str().map(str::to_owned) else {
            return Err(Fail::new(
                "io",
                format!("{} has a name that is not UTF-8", path.display()),
            ));
        };
        let child = format!("{rel}/{name}");
        let kind = e.file_type().map_err(io(&path))?;
        if kind.is_dir() {
            walk(&path, &child, out)?;
        } else if kind.is_file() {
            out.push((child, path));
        }
    }
    Ok(())
}

/// `vault restore-backup`: unpack an archive into `dir`, which must be missing or empty. A key in
/// the archive goes to the vault's key path unless a key file is already there.
pub fn restore(archive: &Path, dir: &Path) -> Result<Value, Fail> {
    if !archive.is_file() {
        return Err(Fail::new(
            "not_found",
            format!("{} does not exist", archive.display()),
        ));
    }
    if dir.exists() && std::fs::read_dir(dir).map_err(io(dir))?.next().is_some() {
        return Err(Fail::new(
            "exists",
            format!("{} is not empty; restore into a new folder", dir.display()),
        ));
    }
    let conn =
        Connection::open_with_flags(archive, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(sqlite)?;
    let mut rows = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT name, sz, data FROM sqlar ORDER BY name")
            .map_err(|e| Fail::new("archive", format!("not a vault backup: {e}")))?;
        let mut q = stmt.query([]).map_err(sqlite)?;
        while let Some(row) = q.next().map_err(sqlite)? {
            let name: String = row.get(0).map_err(sqlite)?;
            let sz: i64 = row.get(1).map_err(sqlite)?;
            let data: Option<Vec<u8>> = row.get(2).map_err(sqlite)?;
            rows.push((name, sz, data.unwrap_or_default()));
        }
    }
    if !rows.iter().any(|(name, _, _)| name == STATE_ENTRY) {
        return Err(Fail::new(
            "archive",
            "not a vault backup: it holds no state.db",
        ));
    }

    let mut files = 0;
    let mut key = None;
    for (name, sz, data) in rows {
        if sz != data.len() as i64 {
            return Err(Fail::new(
                "archive",
                format!("{name} is compressed; vault backups store entries uncompressed"),
            ));
        }
        if let Some(file) = name.strip_prefix(&format!("{KEY_DIR}/")) {
            key = Some((file.trim_end_matches(".key").to_owned(), data));
            continue;
        }
        let target = entry_path(dir, &name)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&target, &data).map_err(io(&target))?;
        if name != STATE_ENTRY {
            files += 1;
        }
    }
    let vault = store::Vault::open(dir).map_err(Fail::from_display("store"))?;
    let state = open_state(vault.root())?;
    let vault_id = state.vault_id().map_err(Fail::from_display("state"))?;
    let key = match key {
        Some((id, bytes)) => {
            if id != vault_id {
                return Err(Fail::new(
                    "archive",
                    format!("the archive's key is for vault {id}, not {vault_id}"),
                ));
            }
            restore_key(&key_path(&vault_id)?, &bytes)?
        }
        None => json!({ "restored": false, "in_archive": false, "path": key_path(&vault_id)? }),
    };
    Ok(json!({
        "vault": vault.root(),
        "vault_id": vault_id,
        "files": files,
        "state_db": state_db(vault.root()),
        "key": key,
    }))
}

/// Where an archive entry goes under `dir`. Only layout folders and `.vault/state.db` are
/// accepted, so an archive cannot write anywhere else.
fn entry_path(dir: &Path, name: &str) -> Result<PathBuf, Fail> {
    let refuse = || Fail::new("archive", format!("refusing archive entry {name:?}"));
    if name == STATE_ENTRY {
        return Ok(dir.join(".vault").join("state.db"));
    }
    let path = Path::new(name);
    let top = name.split('/').next().unwrap_or_default();
    if !store::LAYOUT_DIRS.contains(&top)
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(refuse());
    }
    Ok(dir.join(path))
}

fn restore_key(path: &Path, bytes: &[u8]) -> Result<Value, Fail> {
    if path.exists() {
        let same = std::fs::read(path).map_err(io(path))? == bytes;
        return Ok(json!({
            "restored": false,
            "in_archive": true,
            "path": path,
            "message": if same {
                "the key file is already in place"
            } else {
                "a different key file is already in place and was left alone"
            },
        }));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(path).map_err(io(path))?;
    file.write_all(bytes).map_err(io(path))?;
    Ok(json!({ "restored": true, "in_archive": true, "path": path }))
}
