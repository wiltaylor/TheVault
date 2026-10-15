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

/// Merge `ours` (the device's note) and `theirs` (the server's) from `base`, the server version
/// the device's change started from. `None` means there is no base: both sides created the note,
/// so anything they hold differently clashes.
pub fn merge(base: Option<&Doc>, ours: &Doc, theirs: &Doc) -> Merged {
    let empty = Doc::default();
    let base = base.unwrap_or(&empty);
    let fields = match merge_fields(&base.fields, &ours.fields, &theirs.fields, true) {
        Ok(f) => f,
        Err(why) => return Merged::Clash(why),
    };
    let blocks = match merge_blocks(&base.blocks, &ours.blocks, &theirs.blocks) {
        Ok(b) => b,
        Err(why) => return Merged::Clash(why),
    };
    Merged::Clean(Doc { fields, blocks })
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
        match pick(b.as_ref(), o.as_ref(), t.as_ref()) {
            Ok(v) => out.extend(v.map(|v| (name.to_owned(), v))),
            Err(()) => return Err(format!("both changed field {name:?}")),
        }
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
        let (Some(o), Some(t)) = (o, t) else {
            return Err(format!(
                "one side changed {} and the other removed it",
                key.name()
            ));
        };
        if o.kind != "content" {
            return Err(format!("both changed {}", key.name()));
        }
        let b = b.map(|b| b.body).unwrap_or_default();
        let fields = merge_fields(&b.fields, &o.body.fields, &t.body.fields, false)
            .map_err(|why| format!("{}: {why}", key.name()))?;
        let blocks = diff3(&b.blocks, &o.body.blocks, &t.body.blocks)
            .map_err(|why| format!("{}: {why}", key.name()))?;
        out.push(DocBlock {
            kind: o.kind,
            labels: o.labels,
            body: Doc { fields, blocks },
        });
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
    fn without_a_base_only_agreement_merges() {
        let a = note("a = 1", &["one"]);
        let b = note("a = 2", &["one"]);
        assert!(matches!(merge(None, &a, &b), Merged::Clash(_)));
        assert_eq!(clean(merge(None, &a, &a)), a);
    }
}
