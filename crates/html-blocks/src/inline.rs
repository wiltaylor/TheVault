//! Inline markup (PRD Appendix A, D41) as runs of styled text: written from the runs an HTML
//! walk collects, and read back into runs for the HTML Google takes. The writer and the reader
//! follow `web/src/editor/markup.ts`: marks are toggles, not a stack, so between two runs the
//! writer closes every mark that ends and opens every mark that starts, in the order link,
//! colour, bold, italic, underline, strike, highlight, code. Code is a literal span: it closes
//! whenever any other mark changes and opens again after.

use schema::PALETTE;

/// The marks of a run of text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    pub link: Option<String>,
    pub colour: Option<&'static str>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub highlight: bool,
    pub code: bool,
}

/// One piece of a line of inline content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String, Style),
    /// A line break inside the block.
    Break,
}

/// Every character a backslash escapes in markup.
const MARKUP_CHARS: &str = "\\*+~`={}[]()|";

/// Plain text as markup, escaped as the web client escapes it: `\`, `*`, `` ` ``, `{`, `[` and
/// `]` always, and `+`, `~` and `=` where one touches another of its kind or the edge of the
/// run, where a delimiter could sit beside it.
pub fn escape_text(t: &str) -> String {
    let chars: Vec<char> = t.chars().collect();
    let mut out = String::with_capacity(t.len());
    for (k, &ch) in chars.iter().enumerate() {
        let paired = matches!(ch, '+' | '~' | '=')
            && (k == 0
                || k + 1 == chars.len()
                || chars[k - 1] == ch
                || chars.get(k + 1) == Some(&ch));
        if matches!(ch, '\\' | '*' | '`' | '{' | '[' | ']') || paired {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn escape_code(t: &str) -> String {
    t.replace('\\', "\\\\").replace('`', "\\`")
}

/// The opening delimiters of the marks of `s` that `keep` lacks, outermost first.
fn opens(s: &Style, keep: &Style, out: &mut String) {
    if s.link.is_some() && keep.link.is_none() {
        out.push('[');
    }
    if let (Some(c), None) = (s.colour, keep.colour) {
        out.push('{');
        out.push_str(c);
        out.push('}');
    }
    for (on, kept, d) in [
        (s.bold, keep.bold, "**"),
        (s.italic, keep.italic, "*"),
        (s.underline, keep.underline, "++"),
        (s.strike, keep.strike, "~~"),
        (s.highlight, keep.highlight, "=="),
        (s.code, keep.code, "`"),
    ] {
        if on && !kept {
            out.push_str(d);
        }
    }
}

/// The closing delimiters of the marks of `s` that `keep` lacks, innermost first.
fn closes(s: &Style, keep: &Style, out: &mut String) {
    for (on, kept, d) in [
        (s.code, keep.code, "`"),
        (s.highlight, keep.highlight, "=="),
        (s.strike, keep.strike, "~~"),
        (s.underline, keep.underline, "++"),
        (s.italic, keep.italic, "*"),
        (s.bold, keep.bold, "**"),
    ] {
        if on && !kept {
            out.push_str(d);
        }
    }
    if s.colour.is_some() && keep.colour.is_none() {
        out.push_str("{/}");
    }
    if let (Some(href), None) = (&s.link, &keep.link) {
        out.push_str("](");
        out.push_str(&href.replace('\\', "\\\\").replace(')', "\\)"));
        out.push(')');
    }
}

/// The marks `current` and `want` share, so stay open between two runs. A link or colour that
/// changes is a different mark; code never stays open across a change.
fn kept(current: &Style, want: &Style) -> Style {
    let changed = current != want;
    Style {
        link: (current.link == want.link)
            .then(|| want.link.clone())
            .flatten(),
        colour: (current.colour == want.colour)
            .then_some(want.colour)
            .flatten(),
        bold: current.bold && want.bold,
        italic: current.italic && want.italic,
        underline: current.underline && want.underline,
        strike: current.strike && want.strike,
        highlight: current.highlight && want.highlight,
        code: current.code && want.code && !changed,
    }
}

/// Write `pieces` as markup.
pub fn serialize(pieces: &[Piece]) -> String {
    let mut out = String::new();
    let mut current = Style::default();
    let step = |out: &mut String, current: &mut Style, want: &Style| {
        let keep = kept(current, want);
        closes(current, &keep, out);
        opens(want, &keep, out);
        *current = want.clone();
    };
    for p in pieces {
        match p {
            Piece::Text(t, s) => {
                step(&mut out, &mut current, s);
                out.push_str(&if s.code {
                    escape_code(t)
                } else {
                    escape_text(t)
                });
            }
            Piece::Break => {
                // A break carries no marks, as HTML's `<br>` does: the marks around it close
                // before it and open again after.
                step(&mut out, &mut current, &Style::default());
                out.push('\n');
            }
        }
    }
    step(&mut out, &mut current, &Style::default());
    out
}

/// Whitespace collapsed, zero-width characters dropped, the text trimmed at each end and around
/// each break, adjacent runs of one style merged, and whitespace-only runs left with only their
/// link: the inline content of one block, as HTML would show it.
pub fn normalise(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut space = true;
    for p in pieces {
        match p {
            Piece::Break => {
                trim_end(&mut out);
                out.push(Piece::Break);
                space = true;
            }
            Piece::Text(t, s) => {
                let mut text = String::new();
                for ch in t.chars() {
                    if is_invisible(ch) {
                        continue;
                    }
                    if ch.is_whitespace() || ch == '\u{a0}' {
                        if !space {
                            text.push(' ');
                            space = true;
                        }
                    } else {
                        text.push(ch);
                        space = false;
                    }
                }
                if !text.is_empty() {
                    push(&mut out, text, s);
                }
            }
        }
    }
    trim_end(&mut out);
    while matches!(out.first(), Some(Piece::Break)) {
        out.remove(0);
    }
    while matches!(out.last(), Some(Piece::Break)) {
        out.pop();
    }
    out
}

/// Code text as written: only zero-width characters dropped.
pub fn is_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}' | '\u{034f}' | '\u{00ad}'
    )
}

fn push(out: &mut Vec<Piece>, text: String, s: Style) {
    let s = if text.trim().is_empty() {
        Style {
            link: s.link,
            ..Style::default()
        }
    } else {
        s
    };
    if let Some(Piece::Text(last, ls)) = out.last_mut()
        && *ls == s
    {
        last.push_str(&text);
        return;
    }
    out.push(Piece::Text(text, s));
}

/// Drop the trailing spaces before the end of `out`, and the runs they empty.
fn trim_end(out: &mut Vec<Piece>) {
    while let Some(Piece::Text(t, _)) = out.last_mut() {
        let kept = t.trim_end_matches(' ').len();
        t.truncate(kept);
        if t.is_empty() {
            out.pop();
        } else {
            break;
        }
    }
}

/// Whether `pieces` hold any text.
pub fn has_text(pieces: &[Piece]) -> bool {
    pieces
        .iter()
        .any(|p| matches!(p, Piece::Text(t, _) if !t.trim().is_empty()))
}

// ---------------------------------------------------------------------------------------------
// Reading markup back

/// What a run of markup holds: text, or a mention or placeholder shown as its label.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\'
            && let Some(&n) = it.peek()
            && MARKUP_CHARS.contains(n)
        {
            out.push(n);
            it.next();
            continue;
        }
        out.push(c);
    }
    out
}

fn find_unescaped(s: &[char], ch: char, from: usize) -> Option<usize> {
    let mut i = from;
    while i < s.len() {
        if s[i] == '\\' {
            i += 2;
        } else if s[i] == ch {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

fn is_ulid(s: &[char]) -> bool {
    s.len() == 26
        && s.iter()
            .all(|c| c.is_ascii_digit() || (c.is_ascii_uppercase() && !"ILOU".contains(*c)))
}

/// A `[[ULID]]`, `[[ULID|label]]` or `[[+type|title]]` at `i`: the text it shows (the label,
/// the title, or nothing for a bare mention) and the index after it.
fn match_note_link(s: &[char], i: usize) -> Option<(String, usize)> {
    if s.get(i) != Some(&'[') || s.get(i + 1) != Some(&'[') {
        return None;
    }
    let close = |from: usize| -> Option<usize> {
        let mut j = from;
        while j + 1 < s.len() {
            if s[j] == '\\' {
                j += 2;
            } else if s[j] == ']' && s[j + 1] == ']' {
                return Some(j);
            } else {
                j += 1;
            }
        }
        None
    };
    if s.get(i + 2) == Some(&'+') {
        let bar = (i + 3..s.len()).find(|&k| s[k] == '|')?;
        let ty: String = s[i + 3..bar].iter().collect();
        if ty.is_empty()
            || !ty.starts_with(|c: char| c.is_ascii_lowercase())
            || !ty
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return None;
        }
        let end = close(bar + 1)?;
        let title = unescape(&s[bar + 1..end].iter().collect::<String>());
        return (!title.trim().is_empty()).then_some((title, end + 2));
    }
    if i + 28 > s.len() || !is_ulid(&s[i + 2..i + 28]) {
        return None;
    }
    let j = i + 28;
    if s.get(j) == Some(&']') && s.get(j + 1) == Some(&']') {
        return Some((String::new(), j + 2));
    }
    if s.get(j) != Some(&'|') {
        return None;
    }
    let end = close(j + 1)?;
    Some((unescape(&s[j + 1..end].iter().collect::<String>()), end + 2))
}

/// A link opening at `i`: the index of the `]` ending its label, its href and the index after
/// its `)`.
fn match_link(s: &[char], i: usize) -> Option<(usize, String, usize)> {
    let mut j = i + 1;
    while j < s.len() {
        match s[j] {
            '\\' => j += 2,
            '`' => match find_unescaped(s, '`', j + 1) {
                Some(k) => j = k + 1,
                None => j += 1,
            },
            '[' => j = match_note_link(s, j)?.1,
            ']' => {
                if s.get(j + 1) != Some(&'(') {
                    return None;
                }
                let k = find_unescaped(s, ')', j + 2)?;
                return Some((j, unescape(&s[j + 2..k].iter().collect::<String>()), k + 1));
            }
            _ => j += 1,
        }
    }
    None
}

/// Inline markup read into runs, as the web client reads it. Any string parses; unmatched
/// markup is text. A mention or placeholder is the text it shows.
pub fn parse(text: &str) -> Vec<Piece> {
    let s: Vec<char> = text.chars().collect();
    let mut out: Vec<Piece> = Vec::new();
    let mut style = Style::default();
    let mut link_close: Option<(usize, usize)> = None;
    let mut i = 0;
    let emit = |out: &mut Vec<Piece>, t: &str, st: &Style| {
        if let Some(Piece::Text(last, ls)) = out.last_mut()
            && ls == st
        {
            last.push_str(t);
            return;
        }
        out.push(Piece::Text(t.to_owned(), st.clone()));
    };
    while i < s.len() {
        let c = s[i];
        if let Some((close, end)) = link_close
            && i == close
        {
            style.link = None;
            link_close = None;
            i = end;
            continue;
        }
        match c {
            '\\' => {
                if i + 1 < s.len() && MARKUP_CHARS.contains(s[i + 1]) {
                    emit(&mut out, &s[i + 1].to_string(), &style);
                    i += 2;
                } else {
                    emit(&mut out, "\\", &style);
                    i += 1;
                }
            }
            '\n' => {
                out.push(Piece::Break);
                i += 1;
            }
            '*' => {
                let mut n = s[i..].iter().take_while(|&&x| x == '*').count();
                i += n;
                while n >= 3 {
                    style.bold = !style.bold;
                    style.italic = !style.italic;
                    n -= 3;
                }
                if n == 2 {
                    style.bold = !style.bold;
                }
                if n == 1 {
                    style.italic = !style.italic;
                }
            }
            '+' | '~' | '=' => {
                let mut n = s[i..].iter().take_while(|&&x| x == c).count();
                i += n;
                while n >= 2 {
                    match c {
                        '+' => style.underline = !style.underline,
                        '~' => style.strike = !style.strike,
                        _ => style.highlight = !style.highlight,
                    }
                    n -= 2;
                }
                if n == 1 {
                    emit(&mut out, &c.to_string(), &style);
                }
            }
            '`' => {
                let Some(close) = find_unescaped(&s, '`', i + 1) else {
                    emit(&mut out, "`", &style);
                    i += 1;
                    continue;
                };
                let code = Style {
                    code: true,
                    ..style.clone()
                };
                let mut j = i + 1;
                while j < close {
                    if s[j] == '\\' && matches!(s.get(j + 1), Some('`' | '\\')) {
                        emit(&mut out, &s[j + 1].to_string(), &code);
                        j += 2;
                    } else if s[j] == '\n' {
                        out.push(Piece::Break);
                        j += 1;
                    } else {
                        emit(&mut out, &s[j].to_string(), &code);
                        j += 1;
                    }
                }
                i = close + 1;
            }
            '{' => {
                let rest: String = s[i..s.len().min(i + 10)].iter().collect();
                if rest.starts_with("{/}") && style.colour.is_some() {
                    style.colour = None;
                    i += 3;
                    continue;
                }
                let name = rest[1..].split('}').next().unwrap_or("");
                if rest[1..].contains('}')
                    && let Some(p) = PALETTE.iter().find(|p| **p == name)
                {
                    style.colour = Some(p);
                    i += name.len() + 2;
                    continue;
                }
                emit(&mut out, "{", &style);
                i += 1;
            }
            '[' => {
                if let Some((shown, end)) = match_note_link(&s, i) {
                    if !shown.is_empty() {
                        emit(&mut out, &shown, &style);
                    }
                    i = end;
                    continue;
                }
                if link_close.is_none()
                    && let Some((close, href, end)) = match_link(&s, i)
                {
                    style.link = Some(href);
                    link_close = Some((close, end));
                    i += 1;
                    continue;
                }
                emit(&mut out, "[", &style);
                i += 1;
            }
            _ => {
                emit(&mut out, &c.to_string(), &style);
                i += 1;
            }
        }
    }
    out
}
