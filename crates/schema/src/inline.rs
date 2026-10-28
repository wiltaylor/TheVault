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

/// Every text of inline markup in block `b` itself, not the blocks inside it, with the item that
/// holds it: the item [`markup_item`] names, or each cell of a table's `row`, as `cells[0]`.
pub fn markup_texts(b: &DocBlock) -> Vec<(String, &str)> {
    if let Some(item) = markup_item(&b.kind) {
        return b
            .body
            .field(item)
            .and_then(|v| v.as_str())
            .map(|t| vec![(item.to_owned(), t)])
            .unwrap_or_default();
    }
    match (b.kind.as_str(), b.body.field("cells")) {
        ("row", Some(crate::doc::Value::List(cells))) => cells
            .iter()
            .enumerate()
            .filter_map(|(n, c)| c.as_str().map(|t| (format!("cells[{n}]"), t)))
            .collect(),
        _ => Vec::new(),
    }
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
        for (item, text) in markup_texts(b) {
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

/// One call of an agent in inline markup (agents G19): a link
/// `[@<Label> <request>](agent:<name>?call=<id>)` at bytes `start..end` of the text. `label` is
/// the link's text with its escapes read, `agent` the agent's name and `call` the call's ULID,
/// made by the client that committed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCall {
    pub start: usize,
    pub end: usize,
    pub agent: String,
    pub call: String,
    pub label: String,
}

/// The URL scheme of a link that calls an agent.
pub const AGENT_CALL_SCHEME: &str = "agent:";

/// The agent and call id an `agent:<name>?call=<ULID>` URL names.
pub fn parse_agent_call(href: &str) -> Option<(String, String)> {
    let rest = href.strip_prefix(AGENT_CALL_SCHEME)?;
    let (name, call) = rest.split_once("?call=")?;
    (is_name(name) && crate::encoding::is_ulid(call)).then(|| (name.to_owned(), call.to_owned()))
}

/// The calls of agents in inline markup (PRD Appendix A), in order: every link whose URL is
/// `agent:<name>?call=<ULID>`. Text inside inline code, note mentions and placeholder links holds
/// none, and neither does an escaped character; other links are not calls.
pub fn agent_calls(text: &str) -> Vec<AgentCall> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
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
                if let Some(end) = mention_end(text, i)
                    .or_else(|| placeholder_at(&text[i..]).map(|(_, len)| i + len))
                {
                    i = end;
                    continue;
                }
                if let Some((close, end)) = link_at(text, i) {
                    let href = unescape_markup(&text[close + 2..end - 1]);
                    if let Some((agent, call)) = parse_agent_call(&href) {
                        out.push(AgentCall {
                            start: i,
                            end,
                            agent,
                            call,
                            label: unescape_markup(&text[i + 1..close]),
                        });
                    }
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    out
}

/// `s` with each backslash escape of a markup character read as the character: the inverse of
/// [`escape_markup`].
pub fn unescape_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&e) = chars.peek()
            && is_markup_char(e)
        {
            out.push(e);
            chars.next();
        } else {
            out.push(c);
        }
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

/// A link, `[label](href)`, starting at byte `i`, read as the web editor reads it: the index of
/// the `]` that ends its label and the index after its `)`. The label may hold code spans,
/// mentions and placeholder links but no other unescaped `[` or `]`.
fn link_at(s: &str, i: usize) -> Option<(usize, usize)> {
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
                return find_unescaped(s, b')', j + 2).map(|k| (j, k + 1));
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

    const CALL: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

    #[test]
    fn agent_calls_are_agent_links() {
        let text = format!(
            "Hi [@Clerk tidy \\*this\\*](agent:clerk?call={CALL}) and [@Owl](agent:owl?call={CALL}) done"
        );
        let got = agent_calls(&text);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].agent, "clerk");
        assert_eq!(got[0].call, CALL);
        assert_eq!(got[0].label, "@Clerk tidy *this*");
        assert_eq!(
            &text[got[0].start..got[0].end],
            format!("[@Clerk tidy \\*this\\*](agent:clerk?call={CALL})")
        );
        assert_eq!(got[1].agent, "owl");
    }

    #[test]
    fn agent_calls_skip_code_escapes_other_links_and_plain_text() {
        for text in [
            "@clerk tidy this".to_owned(),
            format!("`[@Clerk x](agent:clerk?call={CALL})`"),
            format!("\\[@Clerk x](agent:clerk?call={CALL})"),
            format!("[@Clerk](/agents/clerk/chat?thread={CALL})"),
            "[@Clerk x](agent:clerk)".to_owned(),
            format!("[@Clerk x](agent:Clerk?call={CALL})"),
            "[@Clerk x](agent:clerk?call=nope)".to_owned(),
            format!("[[{CALL}|[@Clerk x](agent:clerk?call={CALL})]]"),
        ] {
            assert!(agent_calls(&text).is_empty(), "{text}");
        }
        // A link's label may hold code and a mention.
        let text = format!("[@Clerk see `x` [[{CALL}]]](agent:clerk?call={CALL})");
        assert_eq!(agent_calls(&text).len(), 1);
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
