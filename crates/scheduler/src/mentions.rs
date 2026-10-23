//! Calls of agents written in notes (agents G16): where in a note a call may stand, what a call
//! asks, and the note with its calls replaced.
//!
//! A call stands in a note's content blocks, in the inline markup of each paragraph, heading,
//! list item, quote, toggle and admonition title at any depth, or in one of its string fields.
//! [`schema::calls`] finds the calls in one text; inline code, links and note mentions hold none.

use schema::{Call, Doc, DocBlock, Value};

/// One text of a note that holds calls: a markup item of a content block, or a string field's
/// value (or one value of a list of strings).
pub(crate) struct Place {
    pub text: String,
    pub calls: Vec<Call>,
}

/// Each text of `doc` that may hold a call, mutably, with whether it is inline markup: the
/// top-level `fields` named (plain strings) and the markup items of every `content` block.
fn each_text(doc: &mut Doc, fields: &[String], f: &mut impl FnMut(&mut String, bool)) {
    for (name, value) in &mut doc.fields {
        if !fields.contains(name) {
            continue;
        }
        match value {
            Value::Str(s) => f(s, false),
            Value::List(items) => {
                for v in items {
                    if let Value::Str(s) = v {
                        f(s, false);
                    }
                }
            }
            _ => {}
        }
    }
    for b in &mut doc.blocks {
        if b.kind == "content" {
            each_markup(&mut b.body.blocks, f);
        }
    }
}

fn each_markup(blocks: &mut [DocBlock], f: &mut impl FnMut(&mut String, bool)) {
    for b in blocks {
        if let Some(item) = schema::markup_item(&b.kind) {
            for (name, value) in &mut b.body.fields {
                if name == item
                    && let Value::Str(s) = value
                {
                    f(s, true);
                }
            }
        }
        each_markup(&mut b.body.blocks, f);
    }
}

/// The texts of `doc` that hold a call of any of `words`, in note order.
pub(crate) fn places(doc: &Doc, fields: &[String], words: &[&str]) -> Vec<Place> {
    let mut doc = doc.clone();
    let mut out = Vec::new();
    each_text(&mut doc, fields, &mut |s, markup| {
        let calls = schema::calls(s, words, markup);
        if !calls.is_empty() {
            out.push(Place {
                text: s.clone(),
                calls,
            });
        }
    });
    out
}

/// `text` with each call that `pick` names replaced by what `with` makes of it.
fn replaced(text: &str, calls: &[Call], pick: impl Fn(&Call) -> bool, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for c in calls.iter().filter(|c| pick(c)) {
        out.push_str(&text[at..c.start]);
        out.push_str(with);
        at = c.end;
    }
    out.push_str(&text[at..]);
    out
}

/// What the agent whose words `mine` picks is asked: each text that calls it, with its calls
/// replaced by "you", one text to a paragraph.
pub(crate) fn instruction(places: &[Place], mine: impl Fn(usize) -> bool) -> String {
    places
        .iter()
        .filter(|p| p.calls.iter().any(|c| mine(c.word)))
        .map(|p| replaced(&p.text, &p.calls, |c| mine(c.word), "you"))
        .map(|t| t.trim().to_owned())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Replace, in `doc`, every call of a word `mine` picks: in markup by `link`, in a plain string
/// field by `plain`. Answers how many were replaced.
pub(crate) fn rewrite(
    doc: &mut Doc,
    fields: &[String],
    words: &[&str],
    mine: impl Fn(usize) -> bool,
    link: &str,
    plain: &str,
) -> usize {
    let mut n = 0;
    each_text(doc, fields, &mut |s, markup| {
        let calls = schema::calls(s, words, markup);
        let count = calls.iter().filter(|c| mine(c.word)).count();
        if count > 0 {
            n += count;
            *s = replaced(
                s,
                &calls,
                |c| mine(c.word),
                if markup { link } else { plain },
            );
        }
    });
    n
}

/// The first `n` characters of `s`, with its runs of whitespace as single spaces.
pub(crate) fn first_chars(s: &str, n: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(n).collect();
    if flat.chars().count() > n {
        out = out.trim_end().to_owned();
        out.push('\u{2026}');
    }
    out
}
