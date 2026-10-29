//! Content blocks as the simple HTML Google Calendar takes for a description (rich-import H4):
//! paragraphs, line breaks, bold, italic, underline, links, lists, quotes and rules. A heading,
//! a toggle's summary and a callout's title are bold paragraphs, a table's rows are lines of
//! cells between ` | `, code is its lines, and an image is its alt text. Other marks, mentions
//! and placeholders are their text. Nothing else is written, so what [`crate::html_to_blocks`]
//! reads back from this writes this again.

use serde_json::Value;

use crate::inline::{self, Piece, Style};

/// `blocks` as one HTML string, empty when they hold nothing to write.
pub fn blocks_to_html(blocks: &[Value]) -> String {
    let mut out = String::new();
    for b in blocks {
        block(b, &mut out);
    }
    out
}

fn text_of<'a>(b: &'a Value, key: &str) -> &'a str {
    b["fields"][key].as_str().unwrap_or("")
}

fn children(b: &Value) -> &[Value] {
    b["blocks"].as_array().map_or(&[], Vec::as_slice)
}

fn para(pieces: &[Piece], out: &mut String) {
    let html = runs(pieces);
    if !html.is_empty() {
        out.push_str("<p>");
        out.push_str(&html);
        out.push_str("</p>");
    }
}

fn bolded(text: &str) -> Vec<Piece> {
    inline::parse(text)
        .into_iter()
        .map(|p| match p {
            Piece::Text(t, s) => Piece::Text(t, Style { bold: true, ..s }),
            b => b,
        })
        .collect()
}

fn block(b: &Value, out: &mut String) {
    match b["kind"].as_str().unwrap_or("") {
        "paragraph" => para(&inline::parse(text_of(b, "text")), out),
        "heading" => para(&bolded(text_of(b, "text")), out),
        "quote" => {
            let html = runs(&inline::parse(text_of(b, "text")));
            if !html.is_empty() {
                out.push_str("<blockquote>");
                out.push_str(&html);
                out.push_str("</blockquote>");
            }
        }
        "list" => list(b, out),
        "code" | "mermaid" => {
            let source = text_of(b, "source");
            let pieces: Vec<Piece> = source
                .lines()
                .enumerate()
                .flat_map(|(k, l)| {
                    let t = Piece::Text(l.to_owned(), Style::default());
                    if k == 0 {
                        vec![t]
                    } else {
                        vec![Piece::Break, t]
                    }
                })
                .collect();
            para(&pieces, out);
        }
        "table" => {
            let mut pieces = Vec::new();
            for row in children(b) {
                let cells: Vec<&str> = row["fields"]["cells"]
                    .as_array()
                    .map(|c| c.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                if cells.iter().all(|c| c.trim().is_empty()) {
                    continue;
                }
                if !pieces.is_empty() {
                    pieces.push(Piece::Break);
                }
                for (k, c) in cells.iter().enumerate() {
                    if k > 0 {
                        pieces.push(Piece::Text(" | ".to_owned(), Style::default()));
                    }
                    pieces.extend(
                        inline::parse(c)
                            .into_iter()
                            .filter(|p| !matches!(p, Piece::Break)),
                    );
                }
            }
            para(&pieces, out);
        }
        "image" => {
            let alt = b["fields"]["caption"]
                .as_str()
                .or(b["fields"]["alt"].as_str())
                .unwrap_or("");
            let label = if alt.is_empty() {
                "Image".to_owned()
            } else {
                format!("Image: {alt}")
            };
            para(&[Piece::Text(label, Style::default())], out);
        }
        "toggle" => {
            para(&bolded(text_of(b, "text")), out);
            for c in children(b) {
                block(c, out);
            }
        }
        "admonition" => {
            let title = text_of(b, "title");
            let title = if title.is_empty() {
                text_of(b, "kind")
            } else {
                title
            };
            para(&bolded(title), out);
            for c in children(b) {
                block(c, out);
            }
        }
        "columns" | "column" => {
            for c in children(b) {
                block(c, out);
            }
        }
        "divider" => out.push_str("<hr>"),
        _ => {}
    }
}

fn list(b: &Value, out: &mut String) {
    let tag = if text_of(b, "style") == "numbered" {
        "ol"
    } else {
        "ul"
    };
    let todo = text_of(b, "style") == "todo";
    let mut items = String::new();
    for item in children(b) {
        items_html(item, tag, todo, &mut items);
    }
    if !items.is_empty() {
        out.push('<');
        out.push_str(tag);
        out.push('>');
        out.push_str(&items);
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
}

fn items_html(item: &Value, tag: &str, todo: bool, out: &mut String) {
    let mut pieces = inline::parse(text_of(item, "text"));
    if todo {
        let mark = if item["fields"]["checked"] == true {
            "\u{2611} "
        } else {
            "\u{2610} "
        };
        pieces.insert(0, Piece::Text(mark.to_owned(), Style::default()));
    }
    let text = runs(&pieces);
    let mut nested = String::new();
    for child in children(item) {
        items_html(child, tag, todo, &mut nested);
    }
    if text.is_empty() && nested.is_empty() {
        return;
    }
    out.push_str("<li>");
    out.push_str(&text);
    if !nested.is_empty() {
        out.push('<');
        out.push_str(tag);
        out.push('>');
        out.push_str(&nested);
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
    out.push_str("</li>");
}

fn escape(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The tags a style is written with, outermost first: a link, then bold, italic and underline.
fn tags(s: &Style) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(href) = &s.link {
        out.push(format!(
            "a href=\"{}\"",
            escape(href).replace('"', "&quot;")
        ));
    }
    for (on, t) in [(s.bold, "b"), (s.italic, "i"), (s.underline, "u")] {
        if on {
            out.push(t.to_owned());
        }
    }
    out
}

fn close(open: &str) -> String {
    format!("</{}>", open.split(' ').next().unwrap_or(open))
}

/// Inline pieces as HTML, whitespace collapsed as HTML shows it, the tags of runs in a row kept
/// open between them.
fn runs(pieces: &[Piece]) -> String {
    let pieces = inline::normalise(pieces.to_vec());
    let mut out = String::new();
    let mut open: Vec<String> = Vec::new();
    for p in &pieces {
        match p {
            Piece::Break => out.push_str("<br>"),
            Piece::Text(t, s) => {
                let want = tags(s);
                let same = open.iter().zip(&want).take_while(|(a, b)| a == b).count();
                while open.len() > same {
                    let t = open.pop().expect("an open tag");
                    out.push_str(&close(&t));
                }
                for t in &want[same..] {
                    out.push('<');
                    out.push_str(t);
                    out.push('>');
                    open.push(t.clone());
                }
                out.push_str(&escape(t));
            }
        }
    }
    while let Some(t) = open.pop() {
        out.push_str(&close(&t));
    }
    out
}
