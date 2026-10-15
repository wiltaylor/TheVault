//! The link neighbourhood of one note: the notes it links to and the notes that link to it, with
//! every link among them. The web client draws it as the note's graph.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension};

use crate::Result;

/// A note in a [`Graph`].
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub type_name: String,
    pub title: String,
    pub updated: Option<String>,
}

/// A link in a [`Graph`]: `via` is `"field"` for a reference field or `"mention"` for a mention
/// in a content or canvas field, and `field` names that field. A note that names the same note
/// several times in one field gives one edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub via: String,
    pub field: String,
}

/// A note, its neighbours and the links among them.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// The note the graph is around.
    pub center: GraphNode,
    /// The notes it links to or that link to it, most recently updated first. A note in the trash
    /// is not indexed, so it is never one.
    pub neighbours: Vec<GraphNode>,
    /// Every link between two notes of the graph, the centre included, sorted.
    pub edges: Vec<GraphEdge>,
    /// How many neighbours were left out to keep to the limit.
    pub truncated: usize,
}

fn node(conn: &Connection, id: &str) -> Result<Option<GraphNode>> {
    Ok(conn
        .query_row(
            "SELECT id, type, title, updated FROM notes WHERE id = ?1",
            [id],
            |r| {
                Ok(GraphNode {
                    id: r.get(0)?,
                    type_name: r.get(1)?,
                    title: r.get(2)?,
                    updated: r.get(3)?,
                })
            },
        )
        .optional()?)
}

/// The graph around note `id`, keeping at most `limit` neighbours. `None` when the index does not
/// hold the note.
pub(crate) fn graph(conn: &Connection, id: &str, limit: usize) -> Result<Option<Graph>> {
    let Some(center) = node(conn, id)? else {
        return Ok(None);
    };
    // Every note linked either way, newest first. A link to a note that is not indexed (deleted,
    // or never created) has no row in `notes` and drops out of the join.
    let mut stmt = conn.prepare(
        "SELECT n.id, n.type, n.title, n.updated FROM notes n
         WHERE n.id != ?1 AND n.id IN (
           SELECT dst FROM links WHERE src = ?1
           UNION SELECT src FROM links WHERE dst = ?1)
         ORDER BY n.updated_at IS NULL, n.updated_at DESC, n.id",
    )?;
    let all = stmt
        .query_map([id], |r| {
            Ok(GraphNode {
                id: r.get(0)?,
                type_name: r.get(1)?,
                title: r.get(2)?,
                updated: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = all.len().saturating_sub(limit);
    let neighbours: Vec<GraphNode> = all.into_iter().take(limit).collect();

    let members: BTreeSet<&str> = std::iter::once(center.id.as_str())
        .chain(neighbours.iter().map(|n| n.id.as_str()))
        .collect();
    // The links out of every member, kept when they land on a member too.
    let mut out = conn.prepare("SELECT src, dst, via, field FROM links WHERE src = ?1")?;
    let mut edges = BTreeSet::new();
    for src in &members {
        let rows = out.query_map([src], |r| {
            Ok(GraphEdge {
                from: r.get(0)?,
                to: r.get(1)?,
                via: r.get(2)?,
                field: r.get(3)?,
            })
        })?;
        for e in rows {
            let e = e?;
            if e.from != e.to && members.contains(e.to.as_str()) {
                edges.insert(e);
            }
        }
    }
    Ok(Some(Graph {
        center,
        neighbours,
        edges: edges.into_iter().collect(),
        truncated,
    }))
}
