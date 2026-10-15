//! The sidebar's pinned tree. An entry is a pinned note, named by the note's id, or a folder with
//! a made-up id and a name. Every entry has a parent (`None` for the root) and a position among
//! its siblings; notes and folders can both have children. A pin whose note is gone stays until
//! it is removed, so restoring the note from the trash brings its pin, and its children, back.

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Error, Result, State, random_hex};

/// What a pinned entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinKind {
    /// A pinned note. The entry's id is the note's id.
    Note,
    /// A folder, with its name.
    Folder { name: String },
}

/// One row of the pinned tree, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinEntry {
    pub id: String,
    pub kind: PinKind,
    pub parent: Option<String>,
    pub position: i64,
}

/// One entry of the pinned tree with its children, in order, as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinNode {
    pub id: String,
    pub kind: PinKind,
    pub children: Vec<PinNode>,
}

fn entry_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PinEntry> {
    let kind: String = r.get(1)?;
    let name: Option<String> = r.get(2)?;
    Ok(PinEntry {
        id: r.get(0)?,
        kind: if kind == "folder" {
            PinKind::Folder {
                name: name.unwrap_or_default(),
            }
        } else {
            PinKind::Note
        },
        parent: r.get(3)?,
        position: r.get(4)?,
    })
}

const ENTRY_COLUMNS: &str = "id, kind, name, parent, position";

fn entry(conn: &Connection, id: &str) -> Result<Option<PinEntry>> {
    Ok(conn
        .query_row(
            &format!("SELECT {ENTRY_COLUMNS} FROM pin_entries WHERE id = ?1"),
            [id],
            entry_from_row,
        )
        .optional()?)
}

/// The ids of `parent`'s children, in order.
fn children(conn: &Connection, parent: Option<&str>) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM pin_entries WHERE parent IS ?1 ORDER BY position, created_ms, id",
    )?;
    let ids = stmt
        .query_map([parent], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Make `ids` the children of `parent`, in that order.
fn set_children(conn: &Connection, parent: Option<&str>, ids: &[String]) -> Result<()> {
    let mut stmt =
        conn.prepare("UPDATE pin_entries SET parent = ?1, position = ?2 WHERE id = ?3")?;
    for (i, id) in ids.iter().enumerate() {
        stmt.execute(params![parent, i as i64, id])?;
    }
    Ok(())
}

/// Remove entry `e`, putting its children in its place among its siblings.
fn remove_lifting_children(conn: &Connection, e: &PinEntry) -> Result<()> {
    let parent = e.parent.as_deref();
    let lifted = children(conn, Some(&e.id))?;
    let mut siblings = children(conn, parent)?;
    let at = match siblings.iter().position(|x| *x == e.id) {
        Some(at) => {
            siblings.remove(at);
            at
        }
        None => siblings.len(),
    };
    conn.execute("DELETE FROM pin_entries WHERE id = ?1", [&e.id])?;
    siblings.splice(at..at, lifted);
    set_children(conn, parent, &siblings)
}

impl State {
    /// The pinned note ids, in tree order: each entry before its children.
    pub fn pins(&self) -> Result<Vec<String>> {
        fn walk(nodes: &[PinNode], out: &mut Vec<String>) {
            for n in nodes {
                if n.kind == PinKind::Note {
                    out.push(n.id.clone());
                }
                walk(&n.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.pin_tree(|_| Ok(true))?, &mut out);
        Ok(out)
    }

    /// Every stored entry, unordered.
    pub fn pin_entries(&self) -> Result<Vec<PinEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM pin_entries ORDER BY parent, position"
        ))?;
        let rows = stmt
            .query_map([], entry_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The pinned tree. A note for which `shown` is false, such as one in the trash, is left out
    /// and its children take its place among its siblings.
    pub fn pin_tree(&self, mut shown: impl FnMut(&str) -> Result<bool>) -> Result<Vec<PinNode>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM pin_entries ORDER BY position, created_ms, id"
        ))?;
        let rows = stmt
            .query_map([], entry_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        drop(conn);
        let mut by_parent: std::collections::HashMap<Option<String>, Vec<PinEntry>> =
            Default::default();
        let ids: std::collections::HashSet<String> = rows.iter().map(|e| e.id.clone()).collect();
        for e in rows {
            // An entry whose parent is missing would be unreachable; show it at the root.
            let parent = e.parent.clone().filter(|p| ids.contains(p));
            by_parent.entry(parent).or_default().push(e);
        }
        fn build(
            parent: Option<String>,
            by_parent: &mut std::collections::HashMap<Option<String>, Vec<PinEntry>>,
            shown: &mut dyn FnMut(&str) -> Result<bool>,
            out: &mut Vec<PinNode>,
        ) -> Result<()> {
            let Some(entries) = by_parent.remove(&parent) else {
                return Ok(());
            };
            for e in entries {
                let mut kids = Vec::new();
                build(Some(e.id.clone()), by_parent, shown, &mut kids)?;
                if e.kind == PinKind::Note && !shown(&e.id)? {
                    out.extend(kids);
                } else {
                    out.push(PinNode {
                        id: e.id,
                        kind: e.kind,
                        children: kids,
                    });
                }
            }
            Ok(())
        }
        let mut out = Vec::new();
        build(None, &mut by_parent, &mut shown, &mut out)?;
        Ok(out)
    }

    /// One stored entry.
    pub fn pin_entry(&self, id: &str) -> Result<Option<PinEntry>> {
        entry(&self.conn(), id)
    }

    /// Pin `note_id` at the end of the root. Pinning a pinned note changes nothing, wherever it
    /// is in the tree. Returns whether the note was newly pinned.
    pub fn pin(&self, note_id: &str) -> Result<bool> {
        let now = self.now();
        let added = self.conn().execute(
            "INSERT OR IGNORE INTO pin_entries (id, kind, name, parent, position, created_ms)
             VALUES (?1, 'note', NULL, NULL,
                     (SELECT COALESCE(MAX(position), -1) + 1 FROM pin_entries WHERE parent IS NULL),
                     ?2)",
            params![note_id, now],
        )?;
        Ok(added == 1)
    }

    /// Unpin `note_id`; its children move up into its place. Returns whether it was pinned.
    pub fn unpin(&self, note_id: &str) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let Some(e) = entry(&tx, note_id)?.filter(|e| e.kind == PinKind::Note) else {
            return Ok(false);
        };
        remove_lifting_children(&tx, &e)?;
        tx.commit()?;
        Ok(true)
    }

    /// Create a folder called `name` at the end of `parent`'s children, or of the root.
    pub fn create_pin_folder(&self, name: &str, parent: Option<&str>) -> Result<PinEntry> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::EmptyFolderName);
        }
        let now = self.now();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if let Some(p) = parent
            && entry(&tx, p)?.is_none()
        {
            return Err(Error::UnknownPin(p.to_owned()));
        }
        let id = format!("f{}", random_hex(8));
        let position = children(&tx, parent)?.len() as i64;
        tx.execute(
            "INSERT INTO pin_entries (id, kind, name, parent, position, created_ms)
             VALUES (?1, 'folder', ?2, ?3, ?4, ?5)",
            params![id, name, parent, position, now],
        )?;
        let e = entry(&tx, &id)?.ok_or_else(|| Error::UnknownPin(id.clone()))?;
        tx.commit()?;
        Ok(e)
    }

    /// Rename folder `id`.
    pub fn rename_pin_folder(&self, id: &str, name: &str) -> Result<PinEntry> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::EmptyFolderName);
        }
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE pin_entries SET name = ?2 WHERE id = ?1 AND kind = 'folder'",
            params![id, name],
        )?;
        if changed == 0 {
            return Err(Error::UnknownPin(id.to_owned()));
        }
        entry(&conn, id)?.ok_or_else(|| Error::UnknownPin(id.to_owned()))
    }

    /// Delete folder `id`; its children move up into its place.
    pub fn delete_pin_folder(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let Some(e) = entry(&tx, id)?.filter(|e| matches!(e.kind, PinKind::Folder { .. })) else {
            return Err(Error::UnknownPin(id.to_owned()));
        };
        remove_lifting_children(&tx, &e)?;
        tx.commit()?;
        Ok(())
    }

    /// Move entry `id` to `parent` (`None` for the root) so it ends up at `index` among that
    /// parent's children. `index` is clamped to the end. Moving an entry into itself or into one
    /// of its own descendants is refused with [`Error::PinCycle`].
    pub fn move_pin(&self, id: &str, parent: Option<&str>, index: usize) -> Result<PinEntry> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let Some(e) = entry(&tx, id)? else {
            return Err(Error::UnknownPin(id.to_owned()));
        };
        if let Some(target) = parent {
            let mut at = Some(target.to_owned());
            while let Some(cur) = at {
                if cur == id {
                    return Err(Error::PinCycle {
                        id: id.to_owned(),
                        target: target.to_owned(),
                    });
                }
                let Some(up) = entry(&tx, &cur)? else {
                    return Err(Error::UnknownPin(cur));
                };
                at = up.parent;
            }
        }
        let mut old: Vec<String> = children(&tx, e.parent.as_deref())?;
        old.retain(|x| x != id);
        set_children(&tx, e.parent.as_deref(), &old)?;
        let mut new: Vec<String> = children(&tx, parent)?;
        new.retain(|x| x != id);
        new.insert(index.min(new.len()), id.to_owned());
        set_children(&tx, parent, &new)?;
        let moved = entry(&tx, id)?.ok_or_else(|| Error::UnknownPin(id.to_owned()))?;
        tx.commit()?;
        Ok(moved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;

    fn open() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(&dir.path().join("state.db"), None).unwrap();
        (dir, state)
    }

    /// The tree as `id(child child)` text, for compact assertions.
    fn shape(s: &State, hidden: &[&str]) -> String {
        fn render(nodes: &[PinNode]) -> String {
            nodes
                .iter()
                .map(|n| {
                    let label = match &n.kind {
                        PinKind::Note => n.id.clone(),
                        PinKind::Folder { name } => format!("[{name}]"),
                    };
                    if n.children.is_empty() {
                        label
                    } else {
                        format!("{label}({})", render(&n.children))
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        render(&s.pin_tree(|id| Ok(!hidden.contains(&id))).unwrap())
    }

    #[test]
    fn pins_keep_the_order_they_were_added_in() {
        let (_dir, s) = open();
        assert!(s.pin("b").unwrap());
        assert!(s.pin("a").unwrap());
        assert!(s.pin("c").unwrap());
        assert_eq!(s.pins().unwrap(), ["b", "a", "c"]);
    }

    #[test]
    fn pinning_twice_keeps_one_pin_in_its_first_place() {
        let (_dir, s) = open();
        s.pin("a").unwrap();
        s.pin("b").unwrap();
        assert!(!s.pin("a").unwrap());
        assert_eq!(s.pins().unwrap(), ["a", "b"]);
    }

    #[test]
    fn unpinning_removes_only_that_pin() {
        let (_dir, s) = open();
        s.pin("a").unwrap();
        s.pin("b").unwrap();
        assert!(s.unpin("a").unwrap());
        assert!(!s.unpin("a").unwrap());
        assert_eq!(s.pins().unwrap(), ["b"]);
        s.pin("a").unwrap();
        assert_eq!(s.pins().unwrap(), ["b", "a"]);
    }

    #[test]
    fn pins_survive_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        State::open(&path, None).unwrap().pin("a").unwrap();
        assert_eq!(State::open(&path, None).unwrap().pins().unwrap(), ["a"]);
    }

    #[test]
    fn the_tree_keeps_folders_and_nested_notes_in_order() {
        let (_dir, s) = open();
        s.pin("a").unwrap();
        let work = s.create_pin_folder("Work", None).unwrap();
        s.pin("b").unwrap();
        s.pin("c").unwrap();
        s.move_pin("b", Some(&work.id), 0).unwrap();
        s.move_pin("c", Some(&work.id), 0).unwrap();
        s.move_pin("a", Some("c"), 5).unwrap();
        let inner = s.create_pin_folder("Inner", Some(&work.id)).unwrap();
        assert_eq!(shape(&s, &[]), "[Work](c(a) b [Inner])");
        s.move_pin(&inner.id, None, 0).unwrap();
        assert_eq!(shape(&s, &[]), "[Inner] [Work](c(a) b)");
        // Moving within the same parent: the index is the final position.
        s.move_pin("c", Some(&work.id), 1).unwrap();
        assert_eq!(shape(&s, &[]), "[Inner] [Work](b c(a))");
        assert_eq!(s.pins().unwrap(), ["b", "c", "a"]);
        let positions: Vec<_> = s
            .pin_entries()
            .unwrap()
            .into_iter()
            .filter(|e| e.parent.is_none())
            .map(|e| e.position)
            .collect();
        assert_eq!(positions, [0, 1]);
    }

    #[test]
    fn moving_into_itself_or_a_descendant_is_refused() {
        let (_dir, s) = open();
        let f = s.create_pin_folder("F", None).unwrap();
        let g = s.create_pin_folder("G", Some(&f.id)).unwrap();
        s.pin("n").unwrap();
        s.move_pin("n", Some(&g.id), 0).unwrap();
        for target in [&f.id, &g.id, &"n".to_owned()] {
            assert!(matches!(
                s.move_pin(&f.id, Some(target), 0),
                Err(Error::PinCycle { .. })
            ));
        }
        assert!(matches!(
            s.move_pin("n", Some("n"), 0),
            Err(Error::PinCycle { .. })
        ));
        assert!(matches!(
            s.move_pin("n", Some("nope"), 0),
            Err(Error::UnknownPin(_))
        ));
        assert!(matches!(
            s.move_pin("nope", None, 0),
            Err(Error::UnknownPin(_))
        ));
        assert_eq!(shape(&s, &[]), "[F]([G](n))");
    }

    #[test]
    fn deleting_a_folder_moves_its_children_into_its_place() {
        let (_dir, s) = open();
        s.pin("a").unwrap();
        let f = s.create_pin_folder("F", None).unwrap();
        s.pin("d").unwrap();
        for (i, id) in ["b", "c"].iter().enumerate() {
            s.pin(id).unwrap();
            s.move_pin(id, Some(&f.id), i).unwrap();
        }
        assert_eq!(shape(&s, &[]), "a [F](b c) d");
        s.delete_pin_folder(&f.id).unwrap();
        assert_eq!(shape(&s, &[]), "a b c d");
        assert!(matches!(
            s.delete_pin_folder(&f.id),
            Err(Error::UnknownPin(_))
        ));
        assert!(matches!(
            s.delete_pin_folder("a"),
            Err(Error::UnknownPin(_))
        ));
    }

    #[test]
    fn unpinning_a_note_moves_its_children_into_its_place() {
        let (_dir, s) = open();
        let f = s.create_pin_folder("F", None).unwrap();
        for id in ["x", "p", "y", "c1", "c2"] {
            s.pin(id).unwrap();
        }
        s.move_pin("x", Some(&f.id), 0).unwrap();
        s.move_pin("p", Some(&f.id), 1).unwrap();
        s.move_pin("y", Some(&f.id), 2).unwrap();
        s.move_pin("c1", Some("p"), 0).unwrap();
        s.move_pin("c2", Some("p"), 1).unwrap();
        assert_eq!(shape(&s, &[]), "[F](x p(c1 c2) y)");
        assert!(s.unpin("p").unwrap());
        assert_eq!(shape(&s, &[]), "[F](x c1 c2 y)");
        assert!(!s.unpin(&f.id).unwrap(), "unpin only removes notes");
    }

    #[test]
    fn a_hidden_note_is_left_out_and_its_children_shown_in_its_place() {
        let (_dir, s) = open();
        for id in ["a", "t", "b", "k1", "k2"] {
            s.pin(id).unwrap();
        }
        s.move_pin("k1", Some("t"), 0).unwrap();
        s.move_pin("k2", Some("t"), 1).unwrap();
        assert_eq!(shape(&s, &[]), "a t(k1 k2) b");
        assert_eq!(shape(&s, &["t"]), "a k1 k2 b");
        // The stored tree is untouched, so showing the note again brings back its children.
        assert_eq!(shape(&s, &[]), "a t(k1 k2) b");
    }

    #[test]
    fn folder_names_are_trimmed_and_cannot_be_empty() {
        let (_dir, s) = open();
        assert!(matches!(
            s.create_pin_folder("  ", None),
            Err(Error::EmptyFolderName)
        ));
        let f = s.create_pin_folder(" Reading ", None).unwrap();
        assert_eq!(
            f.kind,
            PinKind::Folder {
                name: "Reading".into()
            }
        );
        let r = s.rename_pin_folder(&f.id, "Later").unwrap();
        assert_eq!(
            r.kind,
            PinKind::Folder {
                name: "Later".into()
            }
        );
        assert!(matches!(
            s.rename_pin_folder("nope", "x"),
            Err(Error::UnknownPin(_))
        ));
        assert!(matches!(
            s.create_pin_folder("x", Some("nope")),
            Err(Error::UnknownPin(_))
        ));
    }

    #[test]
    fn version_5_flat_pins_migrate_to_the_root_in_their_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        State::open(&path, None).unwrap();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE pin_entries;
                 CREATE TABLE pins (note_id TEXT PRIMARY KEY, position INTEGER NOT NULL,
                                    pinned_ms INTEGER NOT NULL);
                 INSERT INTO pins VALUES ('b', 0, 10), ('a', 3, 5), ('c', 3, 1), ('d', 7, 0);
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        }
        let s = State::open(&path, None).unwrap();
        assert_eq!(shape(&s, &[]), "b c a d");
        let positions: Vec<_> = s
            .pin_entries()
            .unwrap()
            .iter()
            .map(|e| e.position)
            .collect();
        assert_eq!(positions, [0, 1, 2, 3]);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, crate::SCHEMA_VERSION);
        let old: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'pins')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!old);
    }
}
