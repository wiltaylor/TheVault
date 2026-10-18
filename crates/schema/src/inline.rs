//! Placeholder links in inline markup: `[[+type|title]]`, a link to a note of `type` titled
//! `title` that does not exist yet. Clicking one in the app creates the note and rewrites the
//! placeholder as a mention of it. A placeholder has no target, so the index records no link.

use crate::doc::{Doc, DocBlock};
use crate::encoding::is_name;

/// The content blocks with an item of inline markup, and that item: `text` for most, an
/// admonition's `title`.
pub const MARKUP_ITEMS: [(&str, &str); 6] = [
    ("heading", "text"),
    ("paragraph", "text"),
    ("item", "text"),
    ("quote", "text"),
    ("toggle", "text"),
    ("admonition", "title"),
];

/// The item of a block of `kind` that holds inline markup, if it has one.
pub fn markup_item(kind: &str) -> Option<&'static str> {
    MARKUP_ITEMS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, item)| *item)
}

/// One `[[+type|title]]` in inline markup. `title` is unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub type_name: String,
    pub title: String,
}

/// The placeholder starting at the beginning of `s`, with the number of bytes it spans. The
/// type is a type name; the title runs to the first unescaped `]]`, where a backslash escapes
/// any character, and is not blank.
pub fn placeholder_at(s: &str) -> Option<(Placeholder, usize)> {
    let rest = s.strip_prefix("[[+")?;
    let bar = rest.find('|')?;
    let type_name = &rest[..bar];
    if !is_name(type_name) {
        return None;
    }
    let body = &rest[bar + 1..];
    let mut title = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some((_, e)) if is_markup_char(e) => title.push(e),
                Some((_, e)) => {
                    title.push('\\');
                    title.push(e);
                }
                None => return None,
            },
            ']' if body[i..].starts_with("]]") => {
                if title.trim().is_empty() {
                    return None;
                }
                let len = 3 + bar + 1 + i + 2;
                return Some((
                    Placeholder {
                        type_name: type_name.to_owned(),
                        title,
                    },
                    len,
                ));
            }
            c => title.push(c),
        }
    }
    None
}

/// Characters a backslash escapes in inline markup (PRD Appendix A).
fn is_markup_char(c: char) -> bool {
    "\\*+~`={}[]()|".contains(c)
}

/// The placeholders in inline markup, in order. Escaped brackets and code spans hold none.
pub fn placeholders(text: &str) -> Vec<Placeholder> {
    let mut out = Vec::new();
    let mut rest = text;
    let mut in_code = false;
    while let Some(c) = rest.chars().next() {
        match c {
            '\\' => {
                let mut it = rest.chars();
                it.next();
                it.next();
                rest = it.as_str();
                continue;
            }
            '`' => in_code = !in_code,
            '[' if !in_code => {
                if let Some((p, len)) = placeholder_at(rest) {
                    out.push(p);
                    rest = &rest[len..];
                    continue;
                }
            }
            _ => {}
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Every placeholder in the markup text of `blocks`, at any depth, with the path of the text
/// item that holds it, as `paragraph#2.text` under `at`, numbered as content checks number blocks.
pub(crate) fn placeholders_in(blocks: &[DocBlock], at: &str, out: &mut Vec<(String, Placeholder)>) {
    for (n, b) in blocks.iter().enumerate() {
        let path = format!("{at} > {}#{}", b.kind, n + 1);
        if let Some(item) = markup_item(&b.kind)
            && let Some(text) = b.body.field(item).and_then(|v| v.as_str())
        {
            for p in placeholders(text) {
                out.push((format!("{path}.{item}"), p));
            }
        }
        placeholders_in(&b.body.blocks, &path, out);
    }
}

/// [`placeholders_in`] over a whole body.
pub(crate) fn placeholders_in_doc(body: &Doc, at: &str) -> Vec<(String, Placeholder)> {
    let mut out = Vec::new();
    placeholders_in(&body.blocks, at, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str, title: &str) -> Placeholder {
        Placeholder {
            type_name: t.into(),
            title: title.into(),
        }
    }

    #[test]
    fn reads_type_and_title() {
        let s = "[[+task|Fix pump]] after";
        assert_eq!(placeholder_at(s), Some((p("task", "Fix pump"), 18)));
    }

    #[test]
    fn title_escapes_bars_and_brackets() {
        let s = r"[[+task|a \| b \]\] c\\]]";
        let (got, len) = placeholder_at(s).unwrap();
        assert_eq!(got, p("task", r"a | b ]] c\"));
        assert_eq!(len, s.len());
    }

    #[test]
    fn needs_a_type_name_and_a_title() {
        for s in [
            "[[+Task|x]]",
            "[[+task]]",
            "[[+task|]]",
            "[[+task|  ]]",
            "[[+|x]]",
            "[[+task|x",
            "[[task|x]]",
            "[[+9task|x]]",
        ] {
            assert_eq!(placeholder_at(s), None, "{s}");
        }
    }

    #[test]
    fn finds_placeholders_outside_code_and_escapes() {
        let text = r"a [[+task|One]] \[[+task|Two]] `[[+task|Three]]` [[+project_x|Four]]";
        assert_eq!(
            placeholders(text),
            vec![p("task", "One"), p("project_x", "Four")]
        );
    }
}
