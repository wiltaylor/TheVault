//! The tables of `index.db` and every statement that writes them.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, params};
use schema::{Schema, Term};

use crate::Result;
use crate::project::{Note, Num};
use crate::rebuild::Failure;

/// Bump when the tables change. An index of another version is dropped and recreated empty, and
/// the next rebuild fills it.
const VERSION: i64 = 5;

const TABLES: &str = "
CREATE TABLE notes (
  id      TEXT PRIMARY KEY,
  type    TEXT NOT NULL,
  path    TEXT NOT NULL,
  title   TEXT NOT NULL,
  created TEXT,
  updated TEXT,
  -- created and updated as seconds since the Unix epoch, so they sort in time order.
  created_at REAL,
  updated_at REAL,
  flagged INTEGER NOT NULL,
  -- 1 when the note holds `archived = true`. Queries leave archived notes out unless asked.
  archived INTEGER NOT NULL,
  data    TEXT NOT NULL
);
CREATE INDEX notes_by_type ON notes(type, id);
-- Newest first, ties by id: the order of `sort updated desc`, read without sorting.
CREATE INDEX notes_by_updated ON notes(updated_at DESC, id);
CREATE INDEX notes_by_created ON notes(created_at DESC, id);

CREATE TABLE field_values (
  note  TEXT NOT NULL,
  field TEXT NOT NULL,
  pos   INTEGER NOT NULL,
  part  TEXT NOT NULL,
  kind  TEXT NOT NULL,
  str   TEXT,
  -- No declared type, so an integer and a real each keep their storage class.
  num,
  -- The note's type, so a condition on a field of some types reads only the index.
  type  TEXT NOT NULL,
  PRIMARY KEY (note, field, pos, part)
) WITHOUT ROWID;
CREATE INDEX field_values_by_str ON field_values(field, type, str);
CREATE INDEX field_values_by_num ON field_values(field, type, num);

CREATE TABLE links (
  src   TEXT NOT NULL,
  via   TEXT NOT NULL,
  field TEXT NOT NULL,
  pos   INTEGER NOT NULL,
  dst   TEXT NOT NULL,
  PRIMARY KEY (src, via, field, pos)
);
CREATE INDEX links_by_dst ON links(dst, via, field);

-- `via` is `field` for an asset field, else the kind of the block naming the asset.
CREATE TABLE asset_refs (
  note  TEXT NOT NULL,
  asset TEXT NOT NULL,
  field TEXT NOT NULL,
  via   TEXT NOT NULL,
  PRIMARY KEY (note, asset, field, via)
) WITHOUT ROWID;
CREATE INDEX asset_refs_by_asset ON asset_refs(asset, note);

CREATE TABLE terms (
  taxonomy TEXT NOT NULL,
  path     TEXT NOT NULL,
  name     TEXT NOT NULL,
  parent   TEXT,
  depth    INTEGER NOT NULL,
  pos      INTEGER NOT NULL,
  PRIMARY KEY (taxonomy, path)
);

CREATE TABLE term_ancestors (
  taxonomy TEXT NOT NULL,
  ancestor TEXT NOT NULL,
  path     TEXT NOT NULL,
  PRIMARY KEY (taxonomy, ancestor, path)
);

CREATE TABLE failures (
  path       TEXT PRIMARY KEY,
  diagnostic TEXT NOT NULL
);

-- `generation`: the vault commit generation the index holds every change up to.
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value INTEGER NOT NULL
);
";

/// The tables [`dump`] prints. `failures` is left out: it records a rebuild, not the notes.
const DUMPED: [&str; 6] = [
    "notes",
    "field_values",
    "links",
    "asset_refs",
    "terms",
    "term_ancestors",
];

pub(crate) fn init(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == VERSION {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let existing: Vec<String> = tx
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for t in existing {
        tx.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\""))?;
    }
    tx.execute_batch(TABLES)?;
    tx.pragma_update(None, "user_version", VERSION)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn clear(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM notes; DELETE FROM field_values; DELETE FROM links;
         DELETE FROM asset_refs; DELETE FROM terms; DELETE FROM term_ancestors; DELETE FROM failures;",
    )?;
    Ok(())
}

pub(crate) fn generation(conn: &Connection) -> Result<Option<u64>> {
    let value: Option<i64> = conn
        .query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(value.map(|v| v as u64))
}

pub(crate) fn set_generation(conn: &Connection, generation: u64) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('generation', ?1)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [generation as i64],
    )?;
    Ok(())
}

/// Move the generation from `generation - 1` to `generation`. Any other value is left alone.
pub(crate) fn advance_generation(conn: &Connection, generation: u64) -> Result<()> {
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'generation' AND value = ?1 - 1",
        [generation as i64],
    )?;
    Ok(())
}

pub(crate) fn forget_generation(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM meta WHERE key = 'generation'", [])?;
    Ok(())
}

pub(crate) fn note_path(type_name: &str, id: &str) -> String {
    format!("notes/{type_name}/{id}.wcl")
}

pub(crate) fn insert_note(conn: &Connection, n: &Note) -> Result<()> {
    conn.execute(
        "INSERT INTO notes
           (id, type, path, title, created, updated, created_at, updated_at, flagged, archived,
            data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            n.id,
            n.type_name,
            note_path(&n.type_name, &n.id),
            n.title,
            n.created,
            n.updated,
            n.created_at,
            n.updated_at,
            n.flagged,
            n.archived,
            n.data
        ],
    )?;
    let mut values = conn.prepare_cached(
        "INSERT INTO field_values (note, field, pos, part, kind, str, num, type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for v in &n.values {
        let num = v.num.map(|n| match n {
            Num::Int(i) => rusqlite::types::Value::Integer(i),
            Num::Real(f) => rusqlite::types::Value::Real(f),
        });
        values.execute(params![
            n.id,
            v.field,
            v.pos,
            v.part,
            v.kind,
            v.str,
            num,
            n.type_name
        ])?;
    }
    let mut links = conn.prepare_cached(
        "INSERT INTO links (src, via, field, pos, dst) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for l in &n.links {
        links.execute(params![n.id, l.via, l.field, l.pos, l.dst])?;
    }
    let mut assets = conn.prepare_cached(
        "INSERT OR IGNORE INTO asset_refs (note, asset, field, via) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for a in &n.assets {
        assets.execute(params![n.id, a.asset, a.field, a.via])?;
    }
    Ok(())
}

pub(crate) fn delete_note(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
    conn.execute("DELETE FROM field_values WHERE note = ?1", [id])?;
    conn.execute("DELETE FROM links WHERE src = ?1", [id])?;
    conn.execute("DELETE FROM asset_refs WHERE note = ?1", [id])?;
    Ok(())
}

pub(crate) fn replace_terms(conn: &Connection, schema: &Schema) -> Result<()> {
    conn.execute_batch("DELETE FROM terms; DELETE FROM term_ancestors;")?;
    let mut term = conn.prepare_cached(
        "INSERT INTO terms (taxonomy, path, name, parent, depth, pos) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut ancestor = conn.prepare_cached(
        "INSERT INTO term_ancestors (taxonomy, ancestor, path) VALUES (?1, ?2, ?3)",
    )?;
    for tax in schema.taxonomies() {
        let mut pos = 0i64;
        let mut stack: Vec<(&Term, Vec<String>)> =
            tax.terms.iter().rev().map(|t| (t, Vec::new())).collect();
        while let Some((t, above)) = stack.pop() {
            let mut path = above.clone();
            path.push(t.name.clone());
            let joined = path.join("/");
            let parent = (!above.is_empty()).then(|| above.join("/"));
            term.execute(params![
                tax.name,
                joined,
                t.name,
                parent,
                above.len() as i64,
                pos
            ])?;
            pos += 1;
            for depth in 1..=path.len() {
                ancestor.execute(params![tax.name, path[..depth].join("/"), joined])?;
            }
            for child in t.children.iter().rev() {
                stack.push((child, path.clone()));
            }
        }
    }
    Ok(())
}

pub(crate) fn insert_failure(conn: &Connection, f: &Failure) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO failures (path, diagnostic) VALUES (?1, ?2)",
        params![f.path, f.diagnostic],
    )?;
    Ok(())
}

pub(crate) fn failures(conn: &Connection) -> Result<Vec<Failure>> {
    let mut stmt = conn.prepare("SELECT path, diagnostic FROM failures ORDER BY path")?;
    let rows = stmt.query_map([], |r| {
        Ok(Failure {
            path: r.get(0)?,
            diagnostic: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn dump(conn: &Connection) -> Result<String> {
    let mut out = String::new();
    for table in DUMPED {
        out.push_str(&format!("[{table}]\n"));
        let columns = conn
            .prepare(&format!("SELECT * FROM {table}"))?
            .column_count();
        let order: Vec<String> = (1..=columns).map(|i| i.to_string()).collect();
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM {table} ORDER BY {}",
            order.join(", ")
        ))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let cells: Vec<String> = (0..columns)
                .map(|i| match row.get_ref(i) {
                    Ok(ValueRef::Null) => "NULL".to_owned(),
                    Ok(ValueRef::Integer(n)) => n.to_string(),
                    Ok(ValueRef::Real(f)) => format!("{f:?}"),
                    Ok(ValueRef::Text(t)) => format!("{:?}", String::from_utf8_lossy(t)),
                    Ok(ValueRef::Blob(b)) => format!("blob({})", b.len()),
                    Err(e) => format!("error({e})"),
                })
                .collect();
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
    }
    Ok(out)
}

/// Register the SQL functions `query` uses: `vault_contains(text, part)` and `vault_blank(text)`.
/// A value that is not text is never blank and holds nothing.
pub(crate) fn functions(conn: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("vault_contains", 2, flags, |ctx| {
        let text = match ctx.get_raw(0) {
            ValueRef::Text(t) => t,
            _ => return Ok(false),
        };
        let part = match ctx.get_raw(1) {
            ValueRef::Text(t) => t,
            _ => return Ok(false),
        };
        Ok(contains_folded(
            &String::from_utf8_lossy(text),
            &String::from_utf8_lossy(part),
        ))
    })?;
    conn.create_scalar_function("vault_blank", 1, flags, |ctx| {
        Ok(match ctx.get_raw(0) {
            ValueRef::Text(t) => String::from_utf8_lossy(t).trim().is_empty(),
            _ => false,
        })
    })?;
    Ok(())
}

/// Whether `text` lowercased holds `part`, which is already lowercase: what
/// `text.to_lowercase().contains(part)` says, without lowercasing ASCII text into a new string.
pub(crate) fn contains_folded(text: &str, part: &str) -> bool {
    if part.is_empty() {
        return true;
    }
    if !text.is_ascii() {
        return text.to_lowercase().contains(part);
    }
    // ASCII lowercases to ASCII, so a part with anything else is not in it.
    if !part.is_ascii() {
        return false;
    }
    let (text, part) = (text.as_bytes(), part.as_bytes());
    text.len() >= part.len()
        && text.windows(part.len()).any(|w| {
            w.iter()
                .zip(part)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        })
}
