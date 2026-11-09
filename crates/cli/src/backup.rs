//! `vault backup` and `vault restore-backup` (D25).
//!
//! A backup is one SQLite Archive (`sqlar`) file: every file under the vault's layout folders, a
//! consistent copy of `.vault/state.db`, and, only with `--include-key`, the secret key as
//! `key/<vault-id>.key`. `sqlite3 -A` lists and extracts it as well. Entries are stored
//! uncompressed. `index.db` and the full-text index are left out because the server rebuilds them
//! from the files when it starts.

use std::path::{Component, Path, PathBuf};
use std::io::{Read, Write};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

use crate::Fail;
use crate::local::{key_path, open_state, state_db};

/// Open `state.db` of the vault at `vault` read-only, without migrating it, so the backup holds
/// the schema version the vault was at and the build that wrote it can restore it (TV-41).
fn open_state_read_only(vault: &Path) -> Result<Connection, Fail> {
    let db = state_db(vault);
    if !db.is_file() {
        return Err(Fail::new(
            "not_a_vault",
            format!(
                "{} is not a vault: it has no .vault/state.db",
                vault.display()
            ),
        ));
    }
    Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| Fail::new("state", format!("{}: {e}", db.display())))
}

fn state_err(e: rusqlite::Error) -> Fail {
    Fail::new("state", e.to_string())
}

const STATE_ENTRY: &str = ".vault/state.db";
const KEY_DIR: &str = "key";
const MANIFEST: &str = ".vault-backup/manifest.json";
const CHUNK_SIZE: usize = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct ChunkedFile {
    name: String,
    size: u64,
    parts: usize,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    files: Vec<ChunkedFile>,
}

fn part_name(file: usize, part: usize) -> String {
    format!(".vault-backup/{file}.part-{part:08}")
}

fn sqlite(e: rusqlite::Error) -> Fail {
    Fail::new("archive", e.to_string())
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Fail + '_ {
    move |e| Fail::new("io", format!("{}: {e}", path.display()))
}

/// `vault backup`: write `vault`'s files and `state.db` into a new archive at `out`.
pub fn backup(vault: &Path, out: &Path, include_key: bool) -> Result<Value, Fail> {
    let state = open_state_read_only(vault)?;
    let stored: Option<String> = state
        .query_row("SELECT value FROM meta WHERE key = 'vault_id'", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(state_err)?;
    // A vault that never ran git sync and was never backed up has no id yet. The archive gets a
    // new one in its copy of state.db, as `State::vault_id` would have made, and the live
    // state.db is left as it is (TV-41). Each such backup gets a different id, and the live
    // vault gets its own id at its first server start or git sync. Writing the id to the live
    // state.db here would change a file that the pre-upgrade backup must leave as it is, so the
    // behaviour is documented in docs/install.md instead (TV-52).
    let made_id = stored.is_none();
    // With no stored id, the key file is named after an id that does not exist yet, so there is
    // no key to include. Fail before anything is written (TV-73).
    if made_id && include_key {
        return Err(Fail::new(
            "no_vault_id",
            format!(
                "--include-key: {} has no vault id yet, so its key file cannot be found; start the server once (or run git sync), then back up again",
                vault.display()
            ),
        ));
    }
    let vault_id = stored.unwrap_or_else(|| state::random_hex(16));
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
    let target = state_copy
        .to_str()
        .ok_or_else(|| Fail::new("io", format!("{} is not UTF-8", state_copy.display())))?;
    // VACUUM INTO writes a consistent copy, also while the server has state.db open, and keeps
    // its user_version.
    state
        .execute("VACUUM INTO ?1", [target])
        .map_err(state_err)?;
    drop(state);
    if made_id {
        Connection::open(&state_copy)
            .and_then(|copy| {
                copy.execute(
                    "INSERT INTO meta (key, value) VALUES ('vault_id', ?1)",
                    [&vault_id],
                )
            })
            .map_err(state_err)?;
    }

    // Build the archive beside `out` and move it into place, so a failed backup leaves nothing.
    let partial = out.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let result = write_archive(
        vault,
        &partial,
        &state_copy,
        include_key.then_some(&key_file),
        &vault_id,
        CHUNK_SIZE,
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
    chunk_size: usize,
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
        let mut manifest = Manifest { version: 1, files: Vec::new() };
        for (name, path) in &files {
            let meta = std::fs::metadata(path).map_err(io(path))?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            let mut input = std::fs::File::open(path).map_err(io(path))?;
            let mut hash = Sha256::new();
            let mut size = 0;
            let mut parts = 0;
            loop {
                let mut data = Vec::new();
                input.by_ref().take(chunk_size as u64).read_to_end(&mut data).map_err(io(path))?;
                if data.is_empty() && parts > 0 { break; }
                hash.update(&data);
                size += data.len() as u64;
                let entry = if meta.len() > chunk_size as u64 {
                    part_name(manifest.files.len(), parts)
                } else { name.clone() };
                insert.execute(params![entry, mode(&meta), mtime, data.len() as i64, data]).map_err(sqlite)?;
                parts += 1;
                if meta.len() <= chunk_size as u64 { break; }
            }
            if size != meta.len() { return Err(Fail::new("io", format!("{} changed size during backup", path.display()))); }
            if meta.len() > chunk_size as u64 {
                manifest.files.push(ChunkedFile { name: name.clone(), size, parts, sha256: format!("{:x}", hash.finalize()) });
            }
        }
        if !manifest.files.is_empty() {
            let data = serde_json::to_vec(&manifest).map_err(Fail::from_display("archive"))?;
            insert.execute(params![MANIFEST, 0o100600, 0, data.len() as i64, data]).map_err(sqlite)?;
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
    // Keep a read transaction across validation and extraction, including concurrent writers.
    conn.execute_batch("BEGIN").map_err(sqlite)?;
    let scratch = tempfile::tempdir().map_err(Fail::from_display("io"))?;
    let rows = stage_archive(&conn, scratch.path())?;
    let mut files = 0;
    let mut key = None;
    for (name, staged) in rows {
        if let Some(file) = name.strip_prefix(&format!("{KEY_DIR}/")) {
            key = Some((file.trim_end_matches(".key").to_owned(), std::fs::read(&staged).map_err(io(&staged))?));
            continue;
        }
        let target = entry_path(dir, &name)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::copy(&staged, &target).map_err(io(&target))?;
        if name != STATE_ENTRY { files += 1; }
    }
    let vault = store::Vault::open(dir).map_err(Fail::from_display("store"))?;
    let state = open_state(vault.root())?;
    let vault_id = state.vault_id().map_err(Fail::from_display("state"))?;
    let key = match key {
        Some((id, bytes)) => {
            if id != vault_id {
                return Err(Fail::new("archive", format!("the archive's key is for vault {id}, not {vault_id}")));
            }
            restore_key(&key_path(&vault_id)?, &bytes)?
        }
        None => json!({ "restored": false, "in_archive": false, "path": key_path(&vault_id)? }),
    };
    Ok(json!({ "vault": vault.root(), "vault_id": vault_id, "files": files,
        "state_db": state_db(vault.root()), "key": key }))
}

// Validate the whole archive before touching the destination. Only paths and metadata remain
// in memory between rows; file contents live in temporary staging files.
fn stage_archive(conn: &Connection, dir: &Path) -> Result<Vec<(String, PathBuf)>, Fail> {
    let mut rows = std::collections::BTreeMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT name, sz, data FROM sqlar ORDER BY name")
            .map_err(|e| Fail::new("archive", format!("not a vault backup: {e}")))?;
        let mut q = stmt.query([]).map_err(sqlite)?;
        while let Some(row) = q.next().map_err(sqlite)? {
            let name: String = row.get(0).map_err(sqlite)?;
            let sz: i64 = row.get(1).map_err(sqlite)?;
            let data: Option<Vec<u8>> = row.get(2).map_err(sqlite)?;
            let data = data.unwrap_or_default();
            if sz != data.len() as i64 {
                return Err(Fail::new("archive", format!("{name} is compressed; vault backups store entries uncompressed")));
            }
            let path = dir.join(rows.len().to_string());
            std::fs::write(&path, data).map_err(io(&path))?;
            if rows.insert(name, path).is_some() {
                return Err(Fail::new("archive", "duplicate archive entry"));
            }
        }
    }
    if let Some(path) = rows.remove(MANIFEST) {
        let manifest: Manifest = serde_json::from_reader(std::fs::File::open(&path).map_err(io(&path))?)
            .map_err(Fail::from_display("archive"))?;
        if manifest.version != 1 { return Err(Fail::new("archive", "unsupported chunk manifest version")); }
        for (index, file) in manifest.files.into_iter().enumerate() {
            validate_name(dir, &file.name)?;
            if rows.contains_key(&file.name) || file.parts == 0 || file.parts > rows.len() {
                return Err(Fail::new("archive", "invalid chunk manifest or missing part"));
            }
            let output = dir.join(format!("joined-{index}"));
            let mut out = std::fs::File::create(&output).map_err(io(&output))?;
            let mut hash = Sha256::new();
            let mut size = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            for part in 0..file.parts {
                let path = rows.remove(&part_name(index, part)).ok_or_else(|| Fail::new("archive", "missing chunk part"))?;
                let mut input = std::fs::File::open(&path).map_err(io(&path))?;
                loop {
                    let n = input.read(&mut buffer).map_err(io(&path))?;
                    if n == 0 { break; }
                    hash.update(&buffer[..n]);
                    size += n as u64;
                    out.write_all(&buffer[..n]).map_err(io(&output))?;
                }
            }
            if size != file.size || format!("{:x}", hash.finalize()) != file.sha256 {
                return Err(Fail::new("archive", format!("{}: chunk size or SHA-256 mismatch", file.name)));
            }
            rows.insert(file.name, output);
        }
    }
    for name in rows.keys() { validate_name(dir, name)?; }
    if !rows.contains_key(STATE_ENTRY) {
        return Err(Fail::new(
            "archive",
            "not a vault backup: it holds no state.db",
        ));
    }

    Ok(rows.into_iter().collect())
}

fn validate_name(dir: &Path, name: &str) -> Result<(), Fail> {
    if let Some(key) = name.strip_prefix("key/") {
        if key.ends_with(".key") && Path::new(key).components().count() == 1
            && Path::new(key).components().all(|c| matches!(c, Component::Normal(_))) {
            return Ok(());
        }
    }
    entry_path(dir, name).map(|_| ())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(chunk: usize) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(vault.join("assets")).unwrap();
        std::fs::write(vault.join("assets/large.bin"), (0..251).collect::<Vec<u8>>()).unwrap();
        std::fs::write(vault.join("assets/large.bin.part-0001"), b"ordinary file").unwrap();
        let state = tmp.path().join("state.db");
        std::fs::write(&state, b"snapshot bytes").unwrap();
        let archive = tmp.path().join("backup.sqlar");
        write_archive(&vault, &archive, &state, None, "test", chunk).unwrap();
        (tmp, archive)
    }

    #[test]
    fn chunked_archive_round_trips_with_bounded_chunks() {
        let (tmp, archive) = fixture(32);
        let conn = Connection::open(archive).unwrap();
        let max: i64 = conn.query_row("SELECT max(sz) FROM sqlar WHERE name != ?1", [MANIFEST], |r| r.get(0)).unwrap();
        assert!(max <= 32);
        let staging = tmp.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let rows = stage_archive(&conn, &staging).unwrap();
        for (name, path) in rows {
            let original = if name == STATE_ENTRY { tmp.path().join("state.db") } else { tmp.path().join("vault").join(name) };
            assert_eq!(std::fs::read(path).unwrap(), std::fs::read(original).unwrap());
        }
    }

    #[test]
    fn missing_chunk_refused_before_destination_writes() {
        let (tmp, archive) = fixture(32);
        Connection::open(&archive).unwrap().execute("DELETE FROM sqlar WHERE name = ?1", [part_name(0, 1)]).unwrap();
        let dest = tmp.path().join("destination");
        assert!(restore(&archive, &dest).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn corrupt_chunk_refused_before_destination_writes() {
        let (tmp, archive) = fixture(32);
        Connection::open(&archive).unwrap().execute("UPDATE sqlar SET data = zeroblob(sz) WHERE name = ?1", [part_name(0, 1)]).unwrap();
        let dest = tmp.path().join("destination");
        std::fs::create_dir(&dest).unwrap();
        assert!(restore(&archive, &dest).is_err());
        assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
    }

    #[test]
    fn old_single_entry_archive_still_stages() {
        let (tmp, archive) = fixture(1024);
        let conn = Connection::open(archive).unwrap();
        let staging = tmp.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let rows = stage_archive(&conn, &staging).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|(name, _)| name == STATE_ENTRY));
    }
}
