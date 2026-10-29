//! A `text/plain` body as content blocks (rich-import H1, H2): a paragraph per run of lines
//! between blank lines, its lines kept as line breaks; a run of `>` lines as a quote; URLs as
//! links. With [`Options::quotes`], the quoted history at the end (an "On … wrote:" line and the
//! `>` lines after it, or Outlook's "-----Original Message-----") and the signature after a
//! `-- ` line fold into collapsed toggles.

use serde_json::json;

use crate::inline::{self, Piece, Style};
use crate::{Converted, Options, QUOTED_TITLE, SIGNATURE_TITLE, paragraph, toggle};

/// `text` as content blocks.
pub fn text_to_blocks(text: &str, opts: &Options) -> Converted {
    let mut cut = false;
    let mut warnings = Vec::new();
    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.len() > opts.max_html {
        cut = true;
        warnings.push(format!(
            "The text is {} bytes; only the first {} were converted.",
            text.len(),
            opts.max_html
        ));
        let mut end = opts.max_html;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut budget = opts.max_blocks;
    let mut blocks = if opts.quotes {
        folded(&lines, &mut budget)
    } else {
        plain(&lines, &mut budget)
    };
    if budget == 0 {
        cut = true;
        warnings.push(format!(
            "The message makes more than {} blocks; the rest was left out.",
            opts.max_blocks
        ));
    }
    if cut {
        blocks.push(paragraph(inline::escape_text(&opts.cut_text)));
    }
    Converted {
        blocks,
        warnings,
        cut,
        pending: 0,
    }
}

/// Whether `line` starts an attribution: "On Mon, 5 Jan 2026, Alex <a@example.com> wrote:", which
/// a mail client may wrap over two lines.
fn attribution(lines: &[&str], at: usize) -> Option<usize> {
    let l = lines[at].trim();
    if !l.starts_with("On ") {
        return None;
    }
    if l.ends_with("wrote:") {
        return Some(at + 1);
    }
    let next = lines.get(at + 1)?.trim();
    (next.ends_with("wrote:") && !next.starts_with('>')).then_some(at + 2)
}

fn is_quoted(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

/// The blocks of `lines` with the quoted history and the signature folded.
fn folded(lines: &[&str], budget: &mut usize) -> Vec<serde_json::Value> {
    // The quoted history: an attribution followed by quoted lines to the end, or an Outlook
    // header.
    let mut quote_at = None;
    for at in 0..lines.len() {
        let l = lines[at].trim();
        if l.starts_with("-----Original Message-----")
            || (l.starts_with("________________________________")
                && lines
                    .get(at + 1)
                    .is_some_and(|n| n.trim_start().starts_with("From:")))
        {
            quote_at = Some(at);
            break;
        }
        if let Some(after) = attribution(lines, at) {
            let rest = &lines[after..];
            let quoted = rest.iter().filter(|r| is_quoted(r)).count();
            let blank = rest.iter().filter(|r| r.trim().is_empty()).count();
            if quoted > 0 && quoted + blank == rest.len() {
                quote_at = Some(at);
                break;
            }
        }
    }
    let (body, quoted) = match quote_at {
        Some(at) => (&lines[..at], Some(&lines[at..])),
        None => (lines, None),
    };
    // The signature: from the last `-- ` line of the body.
    let sig_at = body.iter().rposition(|l| *l == "-- " || *l == "--");
    let (main, signature) = match sig_at {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let mut out = plain(main, budget);
    if let Some(sig) = signature {
        let inner = plain(sig, budget);
        if !inner.is_empty() {
            out.push(toggle(SIGNATURE_TITLE, inner));
        }
    }
    if let Some(q) = quoted {
        // Inside, the quoted lines lose one level of `>` and read as a message of their own.
        let mut head = Vec::new();
        let mut inner_lines = Vec::new();
        // The attribution heads the toggle; Outlook's separator line is left out. Either way
        // the lines inside are fewer, so the folding inside ends.
        let k = match attribution(q, 0) {
            Some(after) => {
                head.push(
                    q[..after]
                        .iter()
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                after
            }
            None => 1,
        };
        for l in &q[k..] {
            let t = l.trim_start();
            inner_lines.push(
                t.strip_prefix("> ")
                    .or_else(|| t.strip_prefix('>'))
                    .unwrap_or(l),
            );
        }
        let mut inner = Vec::new();
        for h in head {
            inner.push(paragraph(line_markup(&[h.as_str()])));
        }
        // Older history inside it folds again.
        inner.extend(folded(&inner_lines, budget));
        if !inner.is_empty() {
            out.push(toggle(QUOTED_TITLE, inner));
        }
    }
    out
}

/// A paragraph per run of lines between blank lines, and a quote per run of `>` lines.
fn plain(lines: &[&str], budget: &mut usize) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    // The run of lines in progress, and whether it is a quote.
    let mut run: Vec<&str> = Vec::new();
    let mut quoting = false;
    for l in lines {
        let quoted = is_quoted(l);
        if l.trim().is_empty() || (!run.is_empty() && quoted != quoting) {
            end_run(&mut run, quoting, budget, &mut out);
        }
        if l.trim().is_empty() {
            continue;
        }
        quoting = quoted;
        if quoted {
            let t = l.trim_start();
            run.push(
                t.strip_prefix("> ")
                    .or_else(|| t.strip_prefix('>'))
                    .unwrap_or(t),
            );
        } else {
            run.push(l);
        }
    }
    end_run(&mut run, quoting, budget, &mut out);
    out
}

/// The run of lines `run` as a paragraph, or a quote, emptying it.
fn end_run(run: &mut Vec<&str>, quote: bool, budget: &mut usize, out: &mut Vec<serde_json::Value>) {
    if run.is_empty() {
        return;
    }
    let text = line_markup(run);
    run.clear();
    if text.is_empty() || *budget == 0 {
        return;
    }
    *budget -= 1;
    out.push(if quote {
        json!({ "kind": "quote", "fields": { "text": text } })
    } else {
        paragraph(text)
    });
}

/// Lines as one text of markup: spaces collapsed, the lines as line breaks, URLs as links.
fn line_markup(lines: &[&str]) -> String {
    let mut pieces = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        if k > 0 {
            pieces.push(Piece::Break);
        }
        linked(l, &mut pieces);
    }
    let pieces = inline::normalise(pieces);
    if inline::has_text(&pieces) {
        inline::serialize(&pieces)
    } else {
        String::new()
    }
}

/// `line` as pieces, each `http://` or `https://` URL in it a link to itself. A URL ends at a
/// space, a quote or an angle bracket, and leaves out the punctuation that ends a sentence.
fn linked(line: &str, out: &mut Vec<Piece>) {
    let mut rest = line;
    while let Some(at) = find_url(rest) {
        let (before, from) = rest.split_at(at);
        let end = from
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
            .unwrap_or(from.len());
        let mut url = &from[..end];
        // A closing bracket ends the URL unless the URL opened one.
        loop {
            let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
            let trimmed = if trimmed.ends_with(')')
                && trimmed.matches('(').count() < trimmed.matches(')').count()
            {
                &trimmed[..trimmed.len() - 1]
            } else {
                trimmed
            };
            if trimmed.len() == url.len() {
                break;
            }
            url = trimmed;
        }
        out.push(Piece::Text(before.to_owned(), Style::default()));
        out.push(Piece::Text(
            url.to_owned(),
            Style {
                link: Some(url.to_owned()),
                ..Style::default()
            },
        ));
        rest = &from[url.len()..];
    }
    out.push(Piece::Text(rest.to_owned(), Style::default()));
}

fn find_url(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    [lower.find("https://"), lower.find("http://")]
        .into_iter()
        .flatten()
        .filter(|&at| s.len() > at + 8)
        .min()
}
