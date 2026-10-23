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

/// One call of an agent in text (agents G16): `@` and one of the words asked for, at bytes
/// `start..end` of the text, `word` the index of the word it matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    pub start: usize,
    pub end: usize,
    pub word: usize,
}

/// The calls in `text` of any of `words` (agent names and one-word labels), in order. A call is
/// `@` and a word, matched ignoring case, preceded by the start of the text, whitespace, one of
/// `([{"'` or a markup delimiter (`*`, `+`, `~`, `=`, the `}` of a colour), and followed by the
/// end of the text, whitespace or punctuation, so `a@b` and email addresses never match. Where
/// two words match at one place, the longer wins.
///
/// With `markup`, `text` is inline markup (PRD Appendix A): text inside inline code, links,
/// note mentions and placeholder links holds no call, and neither does an escaped character.
/// Without it, `text` is a plain string field's.
pub fn calls(text: &str, words: &[&str], markup: bool) -> Vec<Call> {
    let mut order: Vec<usize> = (0..words.len()).filter(|&w| !words[w].is_empty()).collect();
    order.sort_by_key(|&w| std::cmp::Reverse(words[w].chars().count()));
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        if markup {
            match c {
                '\\' => {
                    let escaped = text[i + 1..].chars().next().filter(|e| is_markup_char(*e));
                    i += 1 + escaped.map_or(0, char::len_utf8);
                    continue;
                }
                '`' => {
                    if let Some(close) = find_unescaped(text, b'`', i + 1) {
                        i = close + 1;
                        continue;
                    }
                }
                '[' => {
                    let end = mention_end(text, i)
                        .or_else(|| placeholder_at(&text[i..]).map(|(_, len)| i + len))
                        .or_else(|| link_end(text, i));
                    if let Some(end) = end {
                        i = end;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if c == '@' && opens_call(text[..i].chars().next_back()) {
            let rest = &text[i + 1..];
            let hit = order.iter().find_map(|&w| {
                prefix_ignoring_case(rest, words[w])
                    .filter(|len| closes_call(rest[*len..].chars().next()))
                    .map(|len| (w, len))
            });
            if let Some((word, len)) = hit {
                out.push(Call {
                    start: i,
                    end: i + 1 + len,
                    word,
                });
                i += 1 + len;
                continue;
            }
        }
        i += c.len_utf8();
    }
    out
}

/// `s` with every markup character escaped, to stand as literal text in inline markup.
pub fn escape_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_markup_char(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn opens_call(before: Option<char>) -> bool {
    before.is_none_or(|c| c.is_whitespace() || "([{\"'\u{201c}\u{2018}*+~=}".contains(c))
}

fn closes_call(after: Option<char>) -> bool {
    after.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// The length in bytes of the start of `s` that is `word`, ignoring case.
fn prefix_ignoring_case(s: &str, word: &str) -> Option<usize> {
    let mut rest = s.char_indices();
    for w in word.chars() {
        let (_, c) = rest.next()?;
        if !c.to_lowercase().eq(w.to_lowercase()) {
            return None;
        }
    }
    Some(rest.next().map_or(s.len(), |(at, _)| at))
}

/// The index of the next unescaped ASCII `ch` at or after byte `from`.
fn find_unescaped(s: &str, ch: u8, from: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut j = from;
    while j < b.len() {
        if b[j] == b'\\' {
            j += 2;
        } else if b[j] == ch {
            return Some(j);
        } else {
            j += 1;
        }
    }
    None
}

/// The end of a note mention, `[[ULID]]` or `[[ULID|label]]`, starting at byte `i`.
fn mention_end(s: &str, i: usize) -> Option<usize> {
    let rest = s.get(i..)?.strip_prefix("[[")?;
    let id = rest.get(..26)?;
    if !crate::encoding::is_ulid(id) {
        return None;
    }
    let after = i + 2 + 26;
    if s[after..].starts_with("]]") {
        return Some(after + 2);
    }
    if !s[after..].starts_with('|') {
        return None;
    }
    let b = s.as_bytes();
    let mut j = after + 1;
    while j < b.len() {
        if b[j] == b'\\' {
            j += 2;
        } else if b[j..].starts_with(b"]]") {
            return Some(j + 2);
        } else {
            j += 1;
        }
    }
    None
}

/// The end of a link, `[label](href)`, starting at byte `i`, read as the web editor reads it: the
/// label may hold code spans and mentions but no other unescaped `[` or `]`.
fn link_end(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'`' => match find_unescaped(s, b'`', j + 1) {
                Some(k) => j = k + 1,
                None => j += 1,
            },
            b'[' => {
                j = mention_end(s, j)
                    .or_else(|| placeholder_at(&s[j..]).map(|(_, len)| j + len))?;
            }
            b']' => {
                if b.get(j + 1) != Some(&b'(') {
                    return None;
                }
                return find_unescaped(s, b')', j + 2).map(|k| k + 1);
            }
            _ => j += 1,
        }
    }
    None
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

    fn found<'a>(text: &'a str, words: &[&str], markup: bool) -> Vec<&'a str> {
        calls(text, words, markup)
            .into_iter()
            .map(|c| &text[c.start..c.end])
            .collect()
    }

    #[test]
    fn calls_need_a_boundary_on_both_sides() {
        let words = ["clerk"];
        assert_eq!(
            found("@clerk tidy this, (@Clerk) and \"@CLERK.\"", &words, true),
            vec!["@clerk", "@Clerk", "@CLERK"]
        );
        for text in [
            "mail me at bob@clerk.com",
            "a@clerk",
            "@clerks",
            "@clerk_2",
            "@cler",
        ] {
            assert!(found(text, &words, true).is_empty(), "{text}");
        }
        assert_eq!(found("**@clerk** {red}@clerk{/}", &words, true).len(), 2);
    }

    #[test]
    fn calls_skip_code_links_mentions_and_escapes() {
        let words = ["clerk"];
        let text = "`@clerk` [ask @clerk](https://x.test/@clerk) [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB|@clerk]] [[+task|@clerk]] \\@clerk";
        assert!(found(text, &words, true).is_empty(), "{text}");
        assert_eq!(found("`@clerk` @clerk", &words, true), vec!["@clerk"]);
        // An unclosed backtick or bracket is text.
        assert_eq!(found("` @clerk [ @clerk", &words, true).len(), 2);
        // A plain string has no markup to skip.
        assert_eq!(found("`@clerk`", &words, false), Vec::<&str>::new());
        assert_eq!(found("see [@clerk](x)", &words, false), vec!["@clerk"]);
    }

    #[test]
    fn the_longer_word_wins() {
        let words = ["tri", "triage"];
        let c = calls("@Triage now", &words, true);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].word, 1);
        assert_eq!(escape_markup("a*b[c]"), "a\\*b\\[c\\]");
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
