//! Read-only local measurements. No migrations, checkpoints or retention changes.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// Advisory only, not a retention limit or an automatic maintenance trigger.
pub const DEFAULT_WARNING_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct HistorySize {
    pub note_type: String,
    pub versions: u64,
    /// Retained UTF-8 content bytes, not SQLite allocation or filesystem blocks.
    pub content_bytes: u64,
    /// Encoded payload bytes, excluding SQLite row/index overhead.
    pub stored_content_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct StateSize {
    pub history: Vec<HistorySize>,
    /// File lengths, not allocated filesystem blocks. Sampled separately from SQL.
    pub database_file_bytes: u64,
    pub wal_file_bytes: u64,
    pub warning_threshold_bytes: u64,
    pub warning: Option<&'static str>,
}

/// Measure an existing database without opening the normal migrating State writer.
/// SQL rows form one snapshot; file lengths may change during concurrent writes.
pub fn state_size(path: &Path, warning_threshold_bytes: u64) -> crate::Result<StateSize> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Also works on pre-compression databases, without migrating them.
    let has_sizes: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('versions') WHERE name='content_bytes')",
        [],
        |r| r.get(0),
    )?;
    let bytes = if has_sizes {
        "COALESCE(content_bytes, length(CAST(content AS BLOB)))"
    } else {
        "length(CAST(content AS BLOB))"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT note_type, COUNT(*), SUM({bytes}), SUM(length(CAST(content AS BLOB)))
         FROM versions GROUP BY note_type ORDER BY note_type"
    ))?;
    let history = stmt
        .query_map([], |row| {
            Ok(HistorySize {
                note_type: row.get(0)?,
                versions: row.get(1)?,
                content_bytes: row.get(2)?,
                stored_content_bytes: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let database_file_bytes = std::fs::metadata(path)?.len();
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let wal_file_bytes = match std::fs::metadata(wal_path) {
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(err) => return Err(err.into()),
    };
    let warning = (database_file_bytes.saturating_add(wal_file_bytes) >= warning_threshold_bytes)
        .then_some("State database and WAL meet the warning threshold. Review per-type retained history bytes and available disk space, and verify an off-host backup. No history was removed; retention changes need a separate decision.");
    Ok(StateSize {
        history,
        database_file_bytes,
        wal_file_bytes,
        warning_threshold_bytes,
        warning,
    })
}
