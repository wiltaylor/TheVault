//! Explicit offline history maintenance. Callers must hold vault ownership during repair.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;

use crate::history::{MAX_SNAPSHOT_BYTES, encode_snapshot, snapshot_column};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct CorruptRow {
    pub note_id: String,
    pub note_type: String,
    pub version: String,
    pub head: bool,
}

fn scan(conn: &Connection) -> Result<Vec<CorruptRow>> {
    let mut stmt = conn.prepare(
        "SELECT v.note_id, v.note_type, v.token, v.content,
         EXISTS(SELECT 1 FROM heads h WHERE h.note_id=v.note_id AND h.token=v.token)
         FROM versions v ORDER BY v.note_id, v.seq",
    )?;
    let mut rows = stmt.query([])?;
    let mut corrupt = Vec::new();
    while let Some(row) = rows.next()? {
        if snapshot_column(row, 3).is_err() {
            corrupt.push(CorruptRow {
                note_id: row.get(0)?,
                note_type: row.get(1)?,
                version: row.get(2)?,
                head: row.get(4)?,
            });
        }
    }
    Ok(corrupt)
}

/// No migrations, writes, retention or normal State open.
pub fn check(path: &Path) -> Result<Vec<CorruptRow>> {
    scan(&Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

#[derive(Debug, Serialize)]
pub struct Action {
    pub note_id: String,
    pub version: String,
    pub action: &'static str,
    pub replacement: Option<String>,
}

/// Atomically repair the selected notes and audit every action. `contents` names the
/// confirmed note IDs, and supplies current file text for every corrupt live head.
/// Healthy versions are never rewritten. No retention policy is applied.
pub fn repair(path: &Path, contents: &BTreeMap<String, Option<String>>) -> Result<Vec<Action>> {
    let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let schema: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if schema != crate::SCHEMA_VERSION {
        return Err(Error::Corrupt("history repair requires the current state schema".into()));
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let corrupt = scan(&tx)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis() as i64;
    let mut actions = Vec::new();
    for row in corrupt.into_iter().filter(|r| contents.contains_key(&r.note_id)) {
        let replacement = if row.head {
            let content = contents[&row.note_id].as_ref().ok_or_else(|| Error::Corrupt("corrupt head needs current note file content".into()))?;
            let encoded = if content.len() > MAX_SNAPSHOT_BYTES {
                rusqlite::types::Value::Text(content.clone())
            } else {
                rusqlite::types::Value::Blob(encode_snapshot(content.as_bytes())?)
            };
            let mut raw = [0u8; 16];
            crate::random_bytes(&mut raw);
            let token = crate::to_hex(&raw);
            tx.execute("INSERT INTO versions (note_id,note_type,token,author_kind,author_name,at_ms,content,started_ms,content_bytes) VALUES (?1,?2,?3,'token','history-repair',?4,?5,?4,?6)", params![row.note_id,row.note_type,token,now,encoded,content.len() as i64])?;
            tx.execute("UPDATE heads SET token=?2 WHERE note_id=?1", params![row.note_id,token])?;
            crate::changes::journal(&tx, &crate::changes::ChangeKey::Note(row.note_id.clone()), true, now)?;
            Some(token)
        } else { None };
        tx.execute("DELETE FROM versions WHERE token=?1", [&row.version])?;
        let action = Action { note_id: row.note_id, version: row.version, action: if row.head { "replace_corrupt_head" } else { "drop_corrupt_version" }, replacement };
        let id = crate::activity::next_id(&tx, now)?;
        let detail = serde_json::to_string(&action).map_err(|e| Error::Corrupt(e.to_string()))?;
        tx.execute("INSERT INTO activity (id,at_ms,kind,category,actor_kind,actor,summary,subject_kind,subject,detail) VALUES (?1,?2,'history.repair','notes','token','history-repair','Offline history repair','note',?3,?4)", params![id,now,action.note_id,detail])?;
        actions.push(action);
    }
    tx.commit()?;
    Ok(actions)
}
