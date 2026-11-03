//! The three-way merge of a note a device changed while the server changed it too (D20, D28).
//!
//! The merge takes the base (the server's version the device's change started from), the
//! device's note and the server's note:
//!
//! - **Fields** merge one by one. A field only one side changed takes that side's value; both
//!   sides making the same change is no clash. `created`, `updated` and `archived` are the
//!   server's to write, so the server's are kept.
//! - **Compound fields** (`content`, `dict`, `ink`, `canvas` blocks) merge one by one in the same
//!   way. A `content` block both sides changed merges its top-level blocks with a diff3: each
//!   block is identified by its canonical text, the blocks both sides kept unchanged anchor the
//!   merge, and each stretch between two anchors takes whichever side changed it.
//! - **A clash** is two different changes to one field, or to one stretch of blocks. So is an
//!   alignment the diff3 cannot trust: a block that appears twice in the base, a base block that
//!   a side holds twice, or blocks a side moved. The caller then keeps the server's version and
//!   saves the device's as a conflict copy.
//!
//! Blocks have no ids, so a block is its canonical text: two notes hold the same block when they
//! parse to the same block, which is when their canonical text is the same (D21).
//!
//! Git sync merges the same way but never refuses: [`merge_preferring`] settles each clash to the
//! side the caller names, the note's origin (git-sync D8), and lists what clashed for the
//! notification (git-sync D9).

use std::collections::HashMap;

use schema::{Doc, DocBlock, Value};

/// The top-level fields the server writes on every save. The merge keeps the server's.
const SERVER_FIELDS: [&str; 3] = ["created", "updated", "archived"];

/// The result of a merge.
#[derive(Debug, Clone, PartialEq)]
pub enum Merged {
    /// The note with both sides' changes.
    Clean(Doc),
    /// The two sides' changes overlap. The text says where, for the conflict notification.
    Clash(String),
}

/// One side of a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// A merge whose clashes were settled to one side: the note, and the name of each field or
/// compound field that clashed, in note order.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub doc: Doc,
    pub clashes: Vec<String>,
}

/// How a merge treats a clash: refuse the whole merge, or give the field to one side and note it.
struct Clashes {
    winner: Option<Side>,
    names: Vec<String>,
}

impl Clashes {
    /// The winner's value for clashing item `name`, or `Err(why)` when the merge refuses clashes.
    fn settle<T>(&mut self, name: &str, ours: T, theirs: T, why: String) -> Result<T, String> {
        let pick = match self.winner {
            None => return Err(why),
            Some(Side::Ours) => ours,
            Some(Side::Theirs) => theirs,
        };
        self.names.push(name.to_owned());
        Ok(pick)
    }
}

/// Merge `ours` (the device's note) and `theirs` (the server's) from `base`, the server version
/// the device's change started from. `None` means there is no base: both sides created the note,
/// so anything they hold differently clashes.
pub fn merge(base: Option<&Doc>, ours: &Doc, theirs: &Doc) -> Merged {
    let mut clashes = Clashes {
        winner: None,
        names: Vec::new(),
    };
    match merge_docs(base, ours, theirs, &mut clashes) {
        Ok(doc) => Merged::Clean(doc),
        Err(why) => Merged::Clash(why),
    }
}

/// Merge as [`merge`] does, settling every clash to `winner`'s value of the field, or its whole
/// compound field, rather than refusing. `created`, `updated` and `archived` are `theirs`, as in
/// [`merge`].
pub fn merge_preferring(base: Option<&Doc>, ours: &Doc, theirs: &Doc, winner: Side) -> Resolved {
    let mut clashes = Clashes {
        winner: Some(winner),
        names: Vec::new(),
    };
    let doc = merge_docs(base, ours, theirs, &mut clashes)
        .expect("a merge with a winner settles every clash");
    Resolved {
        doc,
        clashes: clashes.names,
    }
}

fn merge_docs(
    base: Option<&Doc>,
    ours: &Doc,
    theirs: &Doc,
    clashes: &mut Clashes,
) -> Result<Doc, String> {
    let empty = Doc::default();
    let base = base.unwrap_or(&empty);
    let fields = merge_fields(&base.fields, &ours.fields, &theirs.fields, true, clashes)?;
    let blocks = merge_blocks(&base.blocks, &ours.blocks, &theirs.blocks, clashes)?;
    Ok(Doc { fields, blocks })
}

/// The value both sides agree on, or the side that changed it from the base. `Err` when both
/// changed it differently.
fn pick<T: PartialEq + Clone>(
    base: Option<&T>,
    ours: Option<&T>,
    theirs: Option<&T>,
) -> Result<Option<T>, ()> {
    if ours == theirs || base == ours {
        Ok(theirs.cloned())
    } else if base == theirs {
        Ok(ours.cloned())
    } else {
        Err(())
    }
}

/// Merge named items, in the server's order with the device's new ones after.
fn merge_fields(
    base: &[(String, Value)],
    ours: &[(String, Value)],
    theirs: &[(String, Value)],
    top: bool,
    clashes: &mut Clashes,
) -> Result<Vec<(String, Value)>, String> {
    let get = |items: &'_ [(String, Value)], name: &str| {
        items
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let mut names: Vec<&str> = theirs.iter().map(|(n, _)| n.as_str()).collect();
    for (n, _) in ours {
        if !names.contains(&n.as_str()) {
            names.push(n);
        }
    }
    let mut out = Vec::new();
    for name in names {
        if top && SERVER_FIELDS.contains(&name) {
            out.extend(get(theirs, name).map(|v| (name.to_owned(), v)));
            continue;
        }
        let (b, o, t) = (get(base, name), get(ours, name), get(theirs, name));
        let v = match pick(b.as_ref(), o.as_ref(), t.as_ref()) {
            Ok(v) => v,
            Err(()) => clashes.settle(name, o, t, format!("both changed field {name:?}"))?,
        };
        out.extend(v.map(|v| (name.to_owned(), v)));
    }
    Ok(out)
}

/// The identity of a top-level block of a note: its kind and labels. A note holds at most one
/// block per compound field, so this names the field.
fn block_key(b: &DocBlock) -> (&str, &[String]) {
    (b.kind.as_str(), b.labels.as_slice())
}

/// Merge a note's compound field blocks one by one, in the server's order with the device's new
/// ones after.
fn merge_blocks(
    base: &[DocBlock],
    ours: &[DocBlock],
    theirs: &[DocBlock],
    clashes: &mut Clashes,
) -> Result<Vec<DocBlock>, String> {
    let find = |items: &'_ [DocBlock], b: &DocBlock| {
        items.iter().find(|x| block_key(x) == block_key(b)).cloned()
    };
    let mut keys: Vec<&DocBlock> = theirs.iter().collect();
    for b in ours {
        if !keys.iter().any(|k| block_key(k) == block_key(b)) {
            keys.push(b);
        }
    }
    let mut out = Vec::new();
    for key in keys {
        let (b, o, t) = (find(base, key), find(ours, key), find(theirs, key));
        if let Ok(v) = pick(b.as_ref(), o.as_ref(), t.as_ref()) {
            out.extend(v);
            continue;
        }
        let name = key.label().unwrap_or(&key.kind).to_owned();
        let (Some(o), Some(t)) = (o.clone(), t.clone()) else {
            let why = format!("one side changed {} and the other removed it", key.name());
            out.extend(clashes.settle(&name, o, t, why)?);
            continue;
        };
        if o.kind != "content" {
            let why = format!("both changed {}", key.name());
            out.push(clashes.settle(&name, o, t, why)?);
            continue;
        }
        // The content merges strictly; a clash inside it gives the winner the whole field.
        let b = b.map(|b| b.body).unwrap_or_default();
        let mut strict = Clashes {
            winner: None,
            names: Vec::new(),
        };
        let merged = merge_fields(
            &b.fields,
            &o.body.fields,
            &t.body.fields,
            false,
            &mut strict,
        )
        .and_then(|fields| {
            let blocks = diff3(&b.blocks, &o.body.blocks, &t.body.blocks)?;
            Ok(Doc { fields, blocks })
        });
        match merged {
            Ok(body) => out.push(DocBlock {
                kind: o.kind,
                labels: o.labels,
                body,
            }),
            Err(why) => {
                let why = format!("{}: {why}", key.name());
                out.push(clashes.settle(&name, o, t, why)?);
            }
        }
    }
    Ok(out)
}

/// A block's identity in the diff3: its canonical form. The parsed block is compared rather than
/// its printed text, which is the same thing for a canonical note and needs no printer here.
fn identity(b: &DocBlock) -> String {
    format!("{b:?}")
}

/// Where each base block sits in `side`, for the base blocks `side` kept unchanged, as `(base
/// index, side index)` in order. `Err` when the alignment is not one the merge can trust.
fn alignment(
    base_at: &HashMap<String, usize>,
    side: &[String],
) -> Result<Vec<(usize, usize)>, String> {
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for (i, id) in side.iter().enumerate() {
        if let Some(&b) = base_at.get(id) {
            if seen.insert(b, i).is_some() {
                return Err("a block appears twice".to_owned());
            }
            out.push((b, i));
        }
    }
    if out.windows(2).any(|w| w[0].0 > w[1].0) {
        return Err("blocks were moved".to_owned());
    }
    Ok(out)
}

/// The diff3 of three block lists: a list of every change of both sides, or `Err` when changes
/// overlap or the blocks cannot be aligned.
fn diff3(
    base: &[DocBlock],
    ours: &[DocBlock],
    theirs: &[DocBlock],
) -> Result<Vec<DocBlock>, String> {
    if ours == theirs || base == ours {
        return Ok(theirs.to_vec());
    }
    if base == theirs {
        return Ok(ours.to_vec());
    }
    let ids = |list: &[DocBlock]| list.iter().map(identity).collect::<Vec<_>>();
    let (b_ids, o_ids, t_ids) = (ids(base), ids(ours), ids(theirs));
    let mut base_at = HashMap::new();
    for (i, id) in b_ids.iter().enumerate() {
        if base_at.insert(id.clone(), i).is_some() {
            return Err("the base holds the same block twice".to_owned());
        }
    }
    let in_ours: HashMap<usize, usize> = alignment(&base_at, &o_ids)?.into_iter().collect();
    let in_theirs: HashMap<usize, usize> = alignment(&base_at, &t_ids)?.into_iter().collect();

    // Anchors: base blocks both sides kept, with where each side holds them.
    let mut anchors: Vec<(usize, usize, usize)> = (0..base.len())
        .filter_map(|b| Some((b, *in_ours.get(&b)?, *in_theirs.get(&b)?)))
        .collect();
    anchors.push((base.len(), ours.len(), theirs.len()));

    let mut out = Vec::new();
    let (mut b0, mut o0, mut t0) = (0, 0, 0);
    for (b1, o1, t1) in anchors {
        let (b, o, t) = (&base[b0..b1], &ours[o0..o1], &theirs[t0..t1]);
        let chunk = if o == t || b == o {
            t
        } else if b == t {
            o
        } else {
            return Err("both changed the same blocks".to_owned());
        };
        out.extend_from_slice(chunk);
        if b1 < base.len() {
            out.push(base[b1].clone());
        }
        (b0, o0, t0) = (b1 + 1, o1 + 1, t1 + 1);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Doc {
        Doc::parse(text, "notes/memo/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl").expect("parses")
    }

    fn body(paras: &[&str]) -> String {
        let mut s = String::from("content body {\n");
        for p in paras {
            s.push_str(&format!("  paragraph {{\n    text = \"{p}\"\n  }}\n"));
        }
        s.push_str("}\n");
        s
    }

    fn note(fields: &str, paras: &[&str]) -> Doc {
        doc(&format!("title = \"T\"\n{fields}\n{}", body(paras)))
    }

    fn clean(m: Merged) -> Doc {
        match m {
            Merged::Clean(d) => d,
            Merged::Clash(why) => panic!("expected a clean merge: {why}"),
        }
    }

    #[test]
    fn different_fields_and_blocks_merge() {
        let base = note("a = 1\nb = 1", &["one", "two", "three"]);
        let ours = note("a = 2\nb = 1", &["ONE", "two", "three"]);
        let theirs = note("a = 1\nb = 3", &["one", "two", "THREE"]);
        let merged = clean(merge(Some(&base), &ours, &theirs));
        assert_eq!(merged, note("a = 2\nb = 3", &["ONE", "two", "THREE"]));
    }

    #[test]
    fn insertions_on_both_sides_in_different_places_merge() {
        let base = note("", &["one", "two", "three"]);
        let ours = note("", &["zero", "one", "two", "three"]);
        let theirs = note("", &["one", "two", "three", "four"]);
        let merged = clean(merge(Some(&base), &ours, &theirs));
        assert_eq!(merged, note("", &["zero", "one", "two", "three", "four"]));
    }

    #[test]
    fn the_same_change_on_both_sides_is_no_clash() {
        let base = note("a = 1", &["one"]);
        let both = note("a = 2", &["uno"]);
        assert_eq!(clean(merge(Some(&base), &both, &both)), both);
    }

    #[test]
    fn overlapping_changes_clash() {
        let base = note("a = 1", &["one", "two"]);
        assert!(matches!(
            merge(
                Some(&base),
                &note("a = 2", &["one", "two"]),
                &note("a = 3", &["one", "two"])
            ),
            Merged::Clash(_)
        ));
        assert!(matches!(
            merge(
                Some(&base),
                &note("a = 1", &["uno", "two"]),
                &note("a = 1", &["eins", "two"])
            ),
            Merged::Clash(_)
        ));
    }

    #[test]
    fn moved_blocks_clash_when_the_other_side_changed_the_content() {
        let base = note("", &["one", "two", "three"]);
        let ours = note("", &["two", "one", "three"]);
        let theirs = note("", &["one", "two", "THREE"]);
        assert!(matches!(
            merge(Some(&base), &ours, &theirs),
            Merged::Clash(_)
        ));
        // A move alone, with the other side's content as it was, is the mover's.
        let theirs = note("a = 1", &["one", "two", "three"]);
        let merged = clean(merge(Some(&base), &ours, &theirs));
        assert_eq!(merged, note("a = 1", &["two", "one", "three"]));
    }

    #[test]
    fn repeated_blocks_are_ambiguous() {
        let base = note("", &["same", "same", "x"]);
        let ours = note("", &["same", "same", "y"]);
        let theirs = note("", &["new", "same", "same", "x"]);
        assert!(matches!(
            merge(Some(&base), &ours, &theirs),
            Merged::Clash(_)
        ));
    }

    #[test]
    fn server_stamps_win() {
        let base = doc(
            "title = \"T\"\ncreated = \"2026-01-01T00:00:00Z\"\nupdated = \"2026-01-01T00:00:00Z\"\n",
        );
        let ours = doc(
            "title = \"T\"\ncreated = \"2026-01-01T00:00:00Z\"\nupdated = \"2026-01-02T00:00:00Z\"\n",
        );
        let theirs = doc(
            "title = \"T\"\ncreated = \"2026-01-01T00:00:00Z\"\nupdated = \"2026-01-03T00:00:00Z\"\n",
        );
        assert_eq!(clean(merge(Some(&base), &ours, &theirs)), theirs);
    }

    #[test]
    fn a_winner_takes_each_clash_and_both_sides_keep_the_rest() {
        let base = note("a = 1\nb = 1\nc = 1", &["one", "two"]);
        let ours = note("a = 2\nb = 2\nc = 1", &["uno", "two"]);
        let theirs = note("a = 3\nb = 1\nc = 3", &["eins", "two"]);
        let r = merge_preferring(Some(&base), &ours, &theirs, Side::Theirs);
        assert_eq!(r.doc, note("a = 3\nb = 2\nc = 3", &["eins", "two"]));
        assert_eq!(r.clashes, vec!["a", "body"]);
        let r = merge_preferring(Some(&base), &ours, &theirs, Side::Ours);
        assert_eq!(r.doc, note("a = 2\nb = 2\nc = 3", &["uno", "two"]));
        assert_eq!(r.clashes, vec!["a", "body"]);
        // No clash, nothing named.
        let r = merge_preferring(Some(&base), &ours, &base, Side::Theirs);
        assert_eq!(r.doc, ours);
        assert!(r.clashes.is_empty());
    }

    #[test]
    fn without_a_base_only_agreement_merges() {
        let a = note("a = 1", &["one"]);
        let b = note("a = 2", &["one"]);
        assert!(matches!(merge(None, &a, &b), Merged::Clash(_)));
        assert_eq!(clean(merge(None, &a, &a)), a);
    }
}
