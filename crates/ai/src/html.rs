//! A web page's readable text for the chat's `fetch_url` (ai D27): Markdown-ish plain text,
//! headings as `#` lines, paragraphs apart, list items as `- ` or `1. ` lines, links as
//! `[text](url)` with the URL made absolute, and table cells apart with ` | `. Scripts,
//! styles, navigation, page headers, footers, asides, forms and hidden elements are dropped.

use reqwest::Url;
use scraper::{ElementRef, Html, Node, Selector};

/// A page read for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The page's `<title>`, when it has one.
    pub title: Option<String>,
    pub text: String,
}

/// Elements whose content is never text for the reader.
const DROPPED: [&str; 19] = [
    "script", "style", "noscript", "template", "head", "nav", "header", "footer", "aside", "svg",
    "canvas", "iframe", "object", "embed", "form", "button", "select", "dialog", "menu",
];

/// Elements that start and end a block of their own.
const BLOCKS: [&str; 27] = [
    "p",
    "div",
    "section",
    "article",
    "main",
    "blockquote",
    "pre",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "figure",
    "figcaption",
    "dl",
    "dt",
    "dd",
    "address",
    "ul",
    "ol",
    "li",
    "hr",
    "details",
    "summary",
    "center",
    "fieldset",
    "body",
];

/// The readable text of `html`, its relative links made absolute against `base`.
pub fn page_text(html: &str, base: &Url) -> Page {
    let doc = Html::parse_document(html);
    let title = Selector::parse("title")
        .ok()
        .and_then(|s| doc.select(&s).next())
        .map(|t| collapse(&t.text().collect::<String>()))
        .filter(|t| !t.is_empty());
    let mut out = Out::default();
    walk(doc.root_element(), base, &mut out, &mut Vec::new());
    Page {
        title,
        text: out.finish(),
    }
}

/// `s` with each run of whitespace one space, trimmed.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text being written.
#[derive(Default)]
struct Out {
    buf: String,
    /// Line breaks owed before the next text: 1 a new line, 2 a blank line.
    owed: u8,
    /// A space owed before the next text, from whitespace in the source.
    space: bool,
    /// What the next line starts with, such as a list marker.
    prefix: String,
    /// Inside `<pre>`, whitespace is kept.
    pre: usize,
}

impl Out {
    fn block(&mut self, breaks: u8) {
        if !self.buf.is_empty() {
            self.owed = self.owed.max(breaks);
        }
        self.space = false;
    }

    fn text(&mut self, s: &str) {
        if self.pre > 0 {
            self.flush();
            self.buf.push_str(s);
            return;
        }
        let starts_space = s.starts_with(char::is_whitespace);
        let ends_space = s.ends_with(char::is_whitespace);
        let words = collapse(s);
        if words.is_empty() {
            if starts_space {
                self.space = true;
            }
            return;
        }
        let at_line = self.owed > 0 || self.buf.is_empty() || self.buf.ends_with('\n');
        self.flush();
        if (self.space || starts_space) && !at_line && !self.buf.ends_with([' ', '(', '[']) {
            self.buf.push(' ');
        }
        self.buf.push_str(&words);
        self.space = ends_space;
    }

    /// Write the line breaks owed and the line's prefix.
    fn flush(&mut self) {
        if self.owed > 0 {
            while self.buf.ends_with(' ') {
                self.buf.pop();
            }
            for _ in 0..self.owed {
                self.buf.push('\n');
            }
            self.owed = 0;
            self.space = false;
        }
        if (self.buf.is_empty() || self.buf.ends_with('\n')) && !self.prefix.is_empty() {
            let p = std::mem::take(&mut self.prefix);
            self.buf.push_str(&p);
        }
    }

    /// Text written straight, such as Markdown punctuation.
    fn raw(&mut self, s: &str) {
        self.flush();
        self.buf.push_str(s);
        self.space = false;
    }

    fn finish(self) -> String {
        // No more than one blank line in a row, and no trailing spaces.
        let mut out = String::new();
        let mut blank = 0;
        for line in self.buf.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                blank += 1;
                if blank > 1 || out.is_empty() {
                    continue;
                }
            } else {
                blank = 0;
            }
            out.push_str(line);
            out.push('\n');
        }
        out.trim_end().to_owned()
    }
}

/// Whether an element is hidden from the reader.
fn hidden(e: &ElementRef) -> bool {
    let v = e.value();
    v.attr("hidden").is_some()
        || v.attr("aria-hidden") == Some("true")
        || matches!(
            v.attr("role"),
            Some("navigation" | "banner" | "contentinfo")
        )
        || v.attr("style").is_some_and(|s| {
            let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            s.contains("display:none")
        })
}

/// Write `el` and what it holds. `lists` holds, per open list, its next number, or `None`
/// for a bulleted list.
fn walk(el: ElementRef, base: &Url, out: &mut Out, lists: &mut Vec<Option<usize>>) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.text(t),
            Node::Element(_) => {
                let Some(e) = ElementRef::wrap(child) else {
                    continue;
                };
                element(e, base, out, lists);
            }
            _ => {}
        }
    }
}

fn element(e: ElementRef, base: &Url, out: &mut Out, lists: &mut Vec<Option<usize>>) {
    let name = e.value().name();
    if DROPPED.contains(&name) || hidden(&e) {
        return;
    }
    match name {
        "br" => {
            if out.pre > 0 {
                out.raw("\n");
            } else {
                out.block(1);
            }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            out.block(2);
            let level = usize::from(name.as_bytes()[1] - b'0');
            let mut inner = Out::default();
            walk(e, base, &mut inner, &mut Vec::new());
            let text = collapse(&inner.finish());
            if !text.is_empty() {
                out.raw(&format!("{} {text}", "#".repeat(level)));
            }
            out.block(2);
        }
        "a" => {
            let mut inner = Out::default();
            walk(e, base, &mut inner, &mut Vec::new());
            let text = collapse(&inner.finish());
            let href = e
                .value()
                .attr("href")
                .map(str::trim)
                .filter(|h| !h.is_empty() && !h.starts_with('#'))
                .and_then(|h| base.join(h).ok())
                .filter(|u| matches!(u.scheme(), "http" | "https" | "mailto"));
            match (text.is_empty(), href) {
                (true, _) => {}
                (false, Some(u)) => out.text(&format!("[{text}]({u})")),
                (false, None) => out.text(&text),
            }
        }
        "ul" | "ol" => {
            out.block(if lists.is_empty() { 2 } else { 1 });
            lists.push((name == "ol").then_some(1));
            walk(e, base, out, lists);
            lists.pop();
            out.block(if lists.is_empty() { 2 } else { 1 });
        }
        "li" => {
            out.block(1);
            let depth = lists.len().saturating_sub(1);
            let marker = match lists.last_mut() {
                Some(Some(n)) => {
                    let m = format!("{n}. ");
                    *n += 1;
                    m
                }
                _ => "- ".to_owned(),
            };
            out.flush();
            out.prefix = format!("{}{marker}", "  ".repeat(depth));
            walk(e, base, out, lists);
            out.prefix.clear();
            out.block(1);
        }
        "pre" => {
            out.block(2);
            out.raw("```\n");
            out.pre += 1;
            walk(e, base, out, lists);
            out.pre -= 1;
            if !out.buf.ends_with('\n') {
                out.buf.push('\n');
            }
            out.buf.push_str("```");
            out.block(2);
        }
        "td" | "th" => {
            if !out.buf.ends_with('\n') && out.owed == 0 && !out.buf.is_empty() {
                out.raw(" | ");
            }
            walk(e, base, out, lists);
        }
        "img" => {
            if let Some(alt) = e
                .value()
                .attr("alt")
                .map(collapse)
                .filter(|a| !a.is_empty())
            {
                out.text(&format!("[image: {alt}]"));
            }
        }
        _ if BLOCKS.contains(&name) => {
            let breaks = if matches!(name, "tr" | "dt" | "dd") {
                1
            } else {
                2
            };
            out.block(breaks);
            walk(e, base, out, lists);
            out.block(breaks);
        }
        _ => walk(e, base, out, lists),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(html: &str) -> Page {
        page_text(
            html,
            &Url::parse("https://example.com/docs/page.html").unwrap(),
        )
    }

    #[test]
    fn a_page_keeps_its_reading_text_and_drops_the_chrome() {
        let p = text(
            r#"<!doctype html><html><head><title> The  Roof </title><style>p{}</style>
            <script>var secret = 1;</script></head><body>
            <header><h1>Site name</h1></header>
            <nav><a href="/">Home</a></nav>
            <main><h1>Slate tiles</h1>
            <p>We chose <b>slate</b> for the
               roof. See <a href="specs.html">the specs</a>.</p>
            <ul><li>Durable</li><li>Heavy<ol><li>Needs a strong frame</li></ol></li></ul>
            <div hidden>Hidden text</div>
            </main>
            <aside>Related posts</aside>
            <footer>Copyright</footer>
            <script>alert(1)</script></body></html>"#,
        );
        assert_eq!(p.title.as_deref(), Some("The Roof"));
        assert_eq!(
            p.text,
            "# Slate tiles\n\nWe chose slate for the roof. See [the specs](https://example.com/docs/specs.html).\n\n- Durable\n- Heavy\n  1. Needs a strong frame"
        );
    }

    #[test]
    fn preformatted_text_keeps_its_lines_and_tables_their_cells() {
        let p = text(
            "<body><pre>a  b\n  c</pre><table><tr><th>Name</th><th>Size</th></tr><tr><td>x</td><td>1</td></tr></table></body>",
        );
        assert_eq!(p.text, "```\na  b\n  c\n```\n\nName | Size\nx | 1");
        assert_eq!(p.title, None);
    }

    #[test]
    fn script_links_and_fragments_are_text_only() {
        let p =
            text(r##"<p><a href="javascript:alert(1)">Click</a> and <a href="#top">top</a></p>"##);
        assert_eq!(p.text, "Click and top");
    }
}
