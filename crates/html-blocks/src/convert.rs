//! HTML to content blocks (rich-import H1, H2, H3): a walk of the HTML5 parse that keeps a line
//! of inline content, ending it as a block wherever a block element starts or ends.

use scraper::node::Element;
use scraper::{Html, Node};
use serde_json::{Value, json};

use crate::css::{self, declarations};
use crate::inline::{self, Piece, Style};
use crate::{
    Converted, Images, Options, Placed, QUOTED_TITLE, SIGNATURE_TITLE, Src, colour, image_block,
    image_text, paragraph, toggle,
};

type NodeRef<'a> = ego_tree::NodeRef<'a, Node>;

/// Elements whose content is never shown: scripts, styles, forms, frames and embedded objects.
const DROPPED: [&str; 22] = [
    "head", "script", "style", "title", "meta", "link", "noscript", "template", "form", "input",
    "select", "textarea", "button", "iframe", "frame", "frameset", "object", "embed", "applet",
    "svg", "canvas", "map",
];

/// Elements that end the line before them and after them.
const BLOCKS: [&str; 30] = [
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "nav",
    "aside",
    "center",
    "address",
    "details",
    "summary",
    "dl",
    "dt",
    "dd",
    "fieldset",
    "legend",
    "body",
    "html",
    "tr",
    "td",
    "th",
    "thead",
    "tbody",
    "tfoot",
    "caption",
    "li",
    "figcaption",
    "hgroup",
];

/// Elements inside a table cell that make the table a layout: a cell of a data table holds a
/// line of inline content.
const CELL_BLOCKS: [&str; 13] = [
    "table",
    "img",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "blockquote",
    "pre",
    "hr",
];

/// Where the walk puts what it finds: blocks in sequence, or the one line of a list item or a
/// data table's cell, where a block element ends a line rather than a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Blocks,
    Line,
}

struct Conv<'o> {
    opts: &'o Options,
    images: &'o mut dyn Images,
    out: Vec<Value>,
    line: Vec<Piece>,
    style: Style,
    mode: Mode,
    /// Blocks met in [`Mode::Line`], such as an image in a list item, put after the list.
    deferred: Vec<Value>,
    /// Blocks made so far, list items and table rows included.
    count: usize,
    /// Set once [`Options::max_blocks`] is reached: the walk stops.
    full: bool,
    deep: bool,
    cut: bool,
    warnings: Vec<String>,
    /// Most of the document's text is inside tables, so every table is a layout.
    newsletter: bool,
    images_seen: usize,
    pending: usize,
    /// The index in `out` from which the rest of the message is an Outlook reply's history.
    rest_from: Option<usize>,
    /// Sub-conversions open, such as a toggle's or a quote's: Outlook's history only starts at
    /// the top.
    nesting: usize,
    /// The caption of the figure being walked.
    in_figure: bool,
}

/// `html` as content blocks. Images go to `images`. A document or a fragment; malformed HTML
/// reads as a browser reads it.
pub fn html_to_blocks(html: &str, opts: &Options, images: &mut dyn Images) -> Converted {
    let mut cut = false;
    let mut warnings = Vec::new();
    let html = if html.len() > opts.max_html {
        cut = true;
        warnings.push(format!(
            "The HTML is {} bytes; only the first {} were converted.",
            html.len(),
            opts.max_html
        ));
        let mut end = opts.max_html;
        while !html.is_char_boundary(end) {
            end -= 1;
        }
        &html[..end]
    } else {
        html
    };
    let doc = Html::parse_document(html);
    let mut c = Conv {
        opts,
        images,
        out: Vec::new(),
        line: Vec::new(),
        style: Style::default(),
        mode: Mode::Blocks,
        deferred: Vec::new(),
        count: 0,
        full: false,
        deep: false,
        cut,
        warnings,
        newsletter: newsletter(doc.tree.root()),
        images_seen: 0,
        pending: 0,
        rest_from: None,
        nesting: 0,
        in_figure: false,
    };
    c.walk(doc.tree.root(), 0);
    c.flush();
    if let Some(at) = c.rest_from
        && at < c.out.len()
    {
        let rest = c.out.split_off(at);
        c.out.push(toggle(QUOTED_TITLE, rest));
    }
    if c.full {
        c.cut = true;
        c.warnings.push(format!(
            "The message makes more than {} blocks; the rest was left out.",
            opts.max_blocks
        ));
    }
    if c.deep {
        c.warnings.push(format!(
            "Content nested deeper than {} elements was left out.",
            opts.max_depth
        ));
    }
    if c.cut {
        c.out.push(paragraph(inline::escape_text(&opts.cut_text)));
    }
    Converted {
        blocks: c.out,
        warnings: c.warnings,
        cut: c.cut,
        pending: c.pending,
    }
}

/// Whether `root` is built as a newsletter is: tables inside tables, holding most of its text.
fn newsletter(root: NodeRef) -> bool {
    fn count(n: NodeRef, in_table: bool, all: &mut usize, tabled: &mut usize) {
        for child in n.children() {
            match child.value() {
                Node::Text(t) => {
                    let len = t.trim().len();
                    *all += len;
                    if in_table {
                        *tabled += len;
                    }
                }
                Node::Element(e) => {
                    if DROPPED.contains(&e.name()) {
                        continue;
                    }
                    count(child, in_table || e.name() == "table", all, tabled);
                }
                _ => count(child, in_table, all, tabled),
            }
        }
    }
    let nested = root.descendants().any(|d| {
        matches!(d.value(), Node::Element(e) if e.name() == "table") && holds(d, &["table"])
    });
    if !nested {
        return false;
    }
    let (mut all, mut tabled) = (0, 0);
    count(root, false, &mut all, &mut tabled);
    all > 0 && tabled * 10 > all * 6
}

fn has_class(e: &Element, wanted: &[&str]) -> bool {
    e.attr("class")
        .is_some_and(|c| c.split_whitespace().any(|k| wanted.contains(&k)))
}

/// Whether element `e` is hidden: by its style, the `hidden` attribute, or as a preheader.
fn hidden(e: &Element) -> bool {
    e.attr("hidden").is_some()
        || e.attr("style")
            .is_some_and(|s| css::hides(&declarations(s)))
        || has_class(
            e,
            &["preheader", "preheader-text", "preview-text", "previewtext"],
        )
}

fn is_heading(name: &str) -> Option<i64> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" | "h4" | "h5" | "h6" => Some(3),
        _ => None,
    }
}

/// A link target the markup keeps: http, https, mailto and tel.
fn safe_href(href: &str) -> Option<String> {
    let href: String = href.chars().filter(|c| !c.is_control()).collect();
    let href = href.trim();
    let scheme = href.split(':').next()?.to_ascii_lowercase();
    (href.contains(':') && matches!(scheme.as_str(), "http" | "https" | "mailto" | "tel"))
        .then(|| href.to_owned())
}

/// The text of `n` as a browser would read it, for `pre`: line breaks for `<br>`.
fn raw_text(n: NodeRef, out: &mut String) {
    for child in n.children() {
        match child.value() {
            Node::Text(t) => out.extend(t.chars().filter(|c| !inline::is_invisible(*c))),
            Node::Element(e) if e.name() == "br" => out.push('\n'),
            Node::Element(e) if DROPPED.contains(&e.name()) || hidden(e) => {}
            _ => raw_text(child, out),
        }
    }
}

fn text_len(n: NodeRef) -> usize {
    let mut s = String::new();
    raw_text(n, &mut s);
    s.trim().chars().count()
}

/// Whether `n` holds a descendant element named in `names`.
fn holds(n: NodeRef, names: &[&str]) -> bool {
    n.descendants().skip(1).any(|d| match d.value() {
        Node::Element(e) => names.contains(&e.name()),
        _ => false,
    })
}

/// The rows of a table, in order, from its `thead`, `tbody` and `tfoot` sections or directly
/// under it, with whether each is in `thead`. Rows of nested tables are their table's.
fn rows(table: NodeRef) -> Vec<(NodeRef, bool)> {
    let mut out = Vec::new();
    for child in table.children() {
        let Node::Element(e) = child.value() else {
            continue;
        };
        match e.name() {
            "tr" => out.push((child, false)),
            "thead" | "tbody" | "tfoot" => {
                for r in child.children() {
                    if matches!(r.value(), Node::Element(e) if e.name() == "tr") {
                        out.push((r, e.name() == "thead"));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn cells(row: NodeRef) -> Vec<NodeRef> {
    row.children()
        .filter(|c| matches!(c.value(), Node::Element(e) if matches!(e.name(), "td" | "th")))
        .collect()
}

fn span(cell: NodeRef) -> usize {
    match cell.value() {
        Node::Element(e) => e
            .attr("colspan")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 100),
        _ => 1,
    }
}

impl Conv<'_> {
    /// Add a finished block, unless the limit is reached. In [`Mode::Line`] it waits for the
    /// end of the list or table it is in.
    fn emit(&mut self, block: Value) {
        if self.full {
            return;
        }
        let weight = 1 + block["blocks"].as_array().map_or(0, Vec::len);
        if self.count + weight > self.opts.max_blocks {
            self.full = true;
            return;
        }
        self.count += weight;
        if self.mode == Mode::Line {
            self.deferred.push(block);
        } else {
            self.out.push(block);
        }
    }

    /// End the line: a paragraph per run of it between double breaks. In [`Mode::Line`] a block
    /// boundary is a line break.
    fn flush(&mut self) {
        if self.mode == Mode::Line {
            if inline::has_text(&self.line) && !matches!(self.line.last(), Some(Piece::Break)) {
                self.line.push(Piece::Break);
            }
            return;
        }
        let line = std::mem::take(&mut self.line);
        for para in split_paragraphs(line) {
            let para = inline::normalise(para);
            if inline::has_text(&para) {
                self.emit(paragraph(inline::serialize(&para)));
            }
        }
    }

    /// The line so far as one text, ending it.
    fn take_line(&mut self) -> String {
        let line = inline::normalise(std::mem::take(&mut self.line));
        if inline::has_text(&line) {
            inline::serialize(&line)
        } else {
            String::new()
        }
    }

    fn text(&mut self, t: &str) {
        self.line
            .push(Piece::Text(t.to_owned(), self.style.clone()));
    }

    /// The blocks `f` makes, walked apart from the line and blocks around them.
    fn sub(&mut self, f: impl FnOnce(&mut Self)) -> Vec<Value> {
        self.flush();
        let out = std::mem::take(&mut self.out);
        let line = std::mem::take(&mut self.line);
        let mode = std::mem::replace(&mut self.mode, Mode::Blocks);
        let deferred = std::mem::take(&mut self.deferred);
        self.nesting += 1;
        f(self);
        self.flush();
        self.nesting -= 1;
        self.mode = mode;
        self.deferred = deferred;
        self.line = line;
        std::mem::replace(&mut self.out, out)
    }

    fn children(&mut self, n: NodeRef, depth: usize) {
        for child in n.children() {
            if self.full {
                return;
            }
            self.walk(child, depth + 1);
        }
    }

    fn walk(&mut self, n: NodeRef, depth: usize) {
        if self.full {
            return;
        }
        if depth > self.opts.max_depth {
            self.deep = true;
            return;
        }
        match n.value() {
            Node::Text(t) => {
                let t: &str = t;
                self.text(t);
            }
            Node::Element(e) => self.element(n, e, depth),
            Node::Document | Node::Fragment => self.children(n, depth),
            _ => {}
        }
    }

    fn element(&mut self, n: NodeRef, e: &Element, depth: usize) {
        let name = e.name();
        if DROPPED.contains(&name) || hidden(e) {
            return;
        }
        if self.opts.quotes && self.mode == Mode::Blocks {
            if Self::quoted(e) {
                self.fold(n, depth, QUOTED_TITLE);
                return;
            }
            // Gmail's "-- " before a signature, which the toggle stands for.
            if has_class(e, &["gmail_signature_prefix"]) {
                return;
            }
            if has_class(e, &["gmail_signature"])
                || e.attr("data-smartmail") == Some("gmail_signature")
            {
                self.fold(n, depth, SIGNATURE_TITLE);
                return;
            }
            if self.nesting == 0 && self.rest_from.is_none() && outlook_history(n, e) {
                self.flush();
                self.rest_from = Some(self.out.len());
            }
        }
        let saved = self.style.clone();
        self.apply_style(e);
        match name {
            "br" => self.line.push(Piece::Break),
            "hr" => {
                self.flush();
                if self.mode == Mode::Blocks {
                    self.emit(json!({ "kind": "divider" }));
                }
            }
            "img" => self.image(e, None),
            "a" => self.link(n, e, depth),
            "ul" | "ol" => {
                self.flush();
                self.list(n, depth);
            }
            "blockquote" => self.blockquote(n, depth),
            "pre" => self.pre(n, e),
            "table" => self.table(n, e, depth),
            "figure" => self.figure(n, depth),
            "figcaption" if self.in_figure => {}
            _ if is_heading(name).is_some() => {
                let level = is_heading(name).unwrap_or(3);
                self.flush();
                self.children(n, depth);
                if self.mode == Mode::Line {
                    self.flush();
                } else {
                    let text = self.take_line();
                    if !text.is_empty() {
                        self.emit(json!({ "kind": "heading", "fields": { "level": level, "text": text } }));
                    }
                }
            }
            _ if BLOCKS.contains(&name) => {
                self.flush();
                self.children(n, depth);
                self.flush();
            }
            _ => self.children(n, depth),
        }
        self.style = saved;
    }

    /// The marks an element and its `style` give its text.
    fn apply_style(&mut self, e: &Element) {
        match e.name() {
            "b" | "strong" => self.style.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" => self.style.italic = true,
            "u" | "ins" => self.style.underline = true,
            "s" | "strike" | "del" => self.style.strike = true,
            "code" | "kbd" | "tt" | "samp" => self.style.code = true,
            "mark" => self.style.highlight = true,
            "th" => self.style.bold = false,
            _ => {}
        }
        let decls = e.attr("style").map(declarations).unwrap_or_default();
        if let Some(b) = css::bold(&decls) {
            self.style.bold = b;
        }
        if let Some(i) = css::italic(&decls) {
            self.style.italic = i;
        }
        let (underline, strike) = css::decoration(&decls);
        self.style.underline |= underline;
        self.style.strike |= strike;
        let colour = css::get(&decls, "color").or(if e.name() == "font" {
            e.attr("color")
        } else {
            None
        });
        if let Some(c) = colour {
            self.style.colour = colour::nearest(c);
        }
    }

    /// Whether `e` starts quoted reply history: Gmail's `gmail_quote`, or a `blockquote` with
    /// `type="cite"` as Apple Mail and Thunderbird write it.
    fn quoted(e: &Element) -> bool {
        has_class(e, &["gmail_quote", "gmail_quote_container"])
            || (e.name() == "blockquote"
                && e.attr("type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("cite")))
    }

    /// `n` walked into a collapsed toggle titled `title`. The "On … wrote:" line just before a
    /// quote goes in with it.
    fn fold(&mut self, n: NodeRef, depth: usize, title: &str) {
        self.flush();
        let mut lead = Vec::new();
        if title == QUOTED_TITLE
            && let Some(last) = self.out.last()
            && last["kind"] == "paragraph"
            && last["fields"]["text"]
                .as_str()
                .is_some_and(|t| t.trim_end().ends_with("wrote:") && t.len() < 400)
        {
            lead.push(self.out.pop().expect("a last block"));
        }
        let mut inner = self.sub(|c| c.children(n, depth));
        // Gmail wraps a quote in a container that is a quote too: one toggle, not two.
        if let [only] = inner.as_slice()
            && only["kind"] == "toggle"
            && only["fields"]["text"] == crate::escape_text(title)
        {
            inner = only["blocks"].as_array().cloned().unwrap_or_default();
            self.count = self.count.saturating_sub(1);
        }
        lead.extend(inner);
        if !lead.is_empty() {
            self.emit(toggle(title, lead));
        }
    }

    fn link(&mut self, n: NodeRef, e: &Element, depth: usize) {
        let Some(href) = e.attr("href").and_then(safe_href) else {
            self.children(n, depth);
            return;
        };
        if self.mode == Mode::Blocks && self.button(n, e) {
            self.flush();
            self.style.link = Some(href);
            self.style.bold = true;
            self.style.colour = None;
            self.children(n, depth);
            self.flush();
            return;
        }
        self.style.link = Some(href);
        // A link's text keeps the link's look, not a colour.
        self.style.colour = None;
        self.children(n, depth);
    }

    /// Whether link `n` looks like a button: styled as one, named one by its class, or the only
    /// text of a cell with a background.
    fn button(&self, n: NodeRef, e: &Element) -> bool {
        if holds(n, &["img"]) {
            return false;
        }
        if e.attr("style")
            .is_some_and(|s| css::button(&declarations(s)))
            || e.attr("class").is_some_and(|c| {
                c.split_whitespace()
                    .any(|k| k.to_ascii_lowercase().contains("button") || k == "btn")
            })
        {
            return true;
        }
        let cell = n
            .ancestors()
            .find(|a| matches!(a.value(), Node::Element(x) if matches!(x.name(), "td" | "th")));
        cell.is_some_and(|c| {
            let Node::Element(ce) = c.value() else {
                return false;
            };
            let filled = ce.attr("bgcolor").is_some()
                || ce
                    .attr("style")
                    .is_some_and(|s| css::button(&declarations(s)));
            filled && text_len(c) == text_len(n) && text_len(n) > 0
        })
    }

    fn image(&mut self, e: &Element, caption: Option<&str>) {
        let decls = e.attr("style").map(declarations).unwrap_or_default();
        let width =
            css::attr_px(e.attr("width")).or_else(|| css::get(&decls, "width").and_then(css::px));
        let height =
            css::attr_px(e.attr("height")).or_else(|| css::get(&decls, "height").and_then(css::px));
        // A tracking pixel: never fetched.
        if width.is_some_and(|w| w <= 2.0) || height.is_some_and(|h| h <= 2.0) {
            return;
        }
        let alt: String = e
            .attr("alt")
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // An icon, such as a social network's in a footer: its alt text, in its link if any.
        if width.is_some_and(|w| w <= 32.0) && height.is_some_and(|h| h <= 32.0) {
            if !alt.is_empty() {
                self.text(&alt);
            }
            return;
        }
        let Some(src) = e.attr("src").and_then(parse_src) else {
            if !alt.is_empty() {
                self.text(&alt);
            }
            return;
        };
        let placed = if self.images_seen >= self.opts.max_images {
            Placed::Failed(format!(
                "more than {} images in one message",
                self.opts.max_images
            ))
        } else {
            self.images_seen += 1;
            self.images.place(&src, &alt)
        };
        if placed == Placed::Skip {
            return;
        }
        if placed == Placed::Pending {
            self.pending += 1;
        }
        match (placed, self.mode) {
            (Placed::Asset(id), _) => {
                self.flush();
                self.emit(image_block(&id, &alt, caption));
            }
            (placed, Mode::Line) => {
                // The link stands in the line, as text of the list item or cell.
                let text = image_text(&src, &alt, &placed);
                self.line.extend(inline::parse(&text));
            }
            (placed, Mode::Blocks) => {
                self.flush();
                self.emit(paragraph(image_text(&src, &alt, &placed)));
            }
        }
    }

    fn figure(&mut self, n: NodeRef, depth: usize) {
        self.flush();
        let caption = n
            .descendants()
            .find(|d| matches!(d.value(), Node::Element(e) if e.name() == "figcaption"))
            .map(|c| {
                let mut s = String::new();
                raw_text(c, &mut s);
                s.split_whitespace().collect::<Vec<_>>().join(" ")
            })
            .filter(|c| !c.is_empty());
        let before = self.out.len();
        let was = std::mem::replace(&mut self.in_figure, true);
        self.children(n, depth);
        self.flush();
        self.in_figure = was;
        if let Some(caption) = caption {
            match self
                .out
                .get_mut(before..)
                .and_then(|s| s.iter_mut().rev().find(|b| b["kind"] == "image"))
            {
                Some(img) => img["fields"]["caption"] = json!(caption),
                None => self.emit(paragraph(inline::escape_text(&caption))),
            }
        }
    }

    fn pre(&mut self, n: NodeRef, e: &Element) {
        self.flush();
        let mut source = String::new();
        raw_text(n, &mut source);
        let source = source.trim_end_matches('\n').to_owned();
        if source.trim().is_empty() {
            return;
        }
        if self.mode == Mode::Line {
            for (k, l) in source.lines().enumerate() {
                if k > 0 {
                    self.line.push(Piece::Break);
                }
                self.text(l);
            }
            return;
        }
        let language = std::iter::once(e)
            .chain(n.children().filter_map(|c| match c.value() {
                Node::Element(x) if x.name() == "code" => Some(x),
                _ => None,
            }))
            .find_map(|x| {
                x.attr("class")?
                    .split_whitespace()
                    .find_map(|k| {
                        k.strip_prefix("language-")
                            .or_else(|| k.strip_prefix("lang-"))
                    })
                    .map(str::to_owned)
            });
        let mut fields = json!({ "source": source });
        if let Some(l) = language.filter(|l| !l.is_empty()) {
            fields["language"] = json!(l);
        }
        self.emit(json!({ "kind": "code", "fields": fields }));
    }

    fn blockquote(&mut self, n: NodeRef, depth: usize) {
        self.flush();
        if self.mode == Mode::Line {
            self.children(n, depth);
            self.flush();
            return;
        }
        let inner = self.sub(|c| c.children(n, depth));
        // A quote block holds one text: a quote of paragraphs is that, anything else stays as
        // its blocks.
        let texts: Option<Vec<String>> = inner
            .iter()
            .map(|b| match b["kind"].as_str() {
                Some("paragraph" | "heading" | "quote") => {
                    b["fields"]["text"].as_str().map(str::to_owned)
                }
                _ => None,
            })
            .collect();
        match texts {
            Some(t) if !t.is_empty() => {
                // The blocks just made count once, as the quote.
                self.count = self.count.saturating_sub(inner.len());
                self.emit(json!({ "kind": "quote", "fields": { "text": t.join("\n") } }));
            }
            _ => {
                self.count = self.count.saturating_sub(inner.len());
                for b in inner {
                    self.emit(b);
                }
            }
        }
    }

    // ---- lists ----------------------------------------------------------------------------

    fn list(&mut self, n: NodeRef, depth: usize) {
        let Node::Element(e) = n.value() else {
            return;
        };
        let ordered = e.name() == "ol";
        let outer = self.mode;
        if outer == Mode::Line {
            // A list inside a cell: its items as lines.
            for item in self.items(n, depth) {
                self.line_of_item(&item, 0);
            }
            return;
        }
        let items = self.items(n, depth);
        let deferred = std::mem::take(&mut self.deferred);
        if !items.is_empty() {
            let weight = count_items(&items);
            if self.count + weight > self.opts.max_blocks {
                self.full = true;
                return;
            }
            self.count += weight;
            self.out.push(json!({
                "kind": "list",
                "fields": { "style": if ordered { "numbered" } else { "bullet" } },
                "blocks": items,
            }));
        }
        for b in deferred {
            self.out.push(b);
        }
    }

    /// An item already made, as lines of the current line.
    fn line_of_item(&mut self, item: &Value, level: usize) {
        if let Some(t) = item["fields"]["text"].as_str() {
            if inline::has_text(&self.line) {
                self.line.push(Piece::Break);
            }
            self.line.push(Piece::Text(
                format!("{}- ", "  ".repeat(level)),
                Style::default(),
            ));
            self.line.extend(inline::parse(t));
        }
        for child in item["blocks"].as_array().into_iter().flatten() {
            self.line_of_item(child, level + 1);
        }
    }

    /// The items of list `n`: each `li`, with the lists inside it as its nested items. A list
    /// directly inside a list nests under the item before it.
    fn items(&mut self, n: NodeRef, depth: usize) -> Vec<Value> {
        let mut items: Vec<Value> = Vec::new();
        for child in n.children() {
            if self.full {
                break;
            }
            match child.value() {
                Node::Element(e) if DROPPED.contains(&e.name()) || hidden(e) => {}
                Node::Element(e) if e.name() == "li" => {
                    if let Some(item) = self.item(child, depth + 1) {
                        items.push(item);
                    }
                }
                Node::Element(e) if matches!(e.name(), "ul" | "ol") => {
                    let nested = self.items(child, depth + 1);
                    match items.last_mut() {
                        Some(last) => {
                            let mut list = last["blocks"].as_array().cloned().unwrap_or_default();
                            list.extend(nested);
                            last["blocks"] = Value::Array(list);
                        }
                        None => items.extend(nested),
                    }
                }
                Node::Text(t) if t.trim().is_empty() => {}
                // Stray content in a list reads as an item of its own.
                _ => {
                    if let Some(item) = self.item_of(|c| c.walk(child, depth + 1), Vec::new()) {
                        items.push(item);
                    }
                }
            }
        }
        items
    }

    fn item(&mut self, li: NodeRef, depth: usize) -> Option<Value> {
        let Node::Element(e) = li.value() else {
            return None;
        };
        if depth > self.opts.max_depth {
            self.deep = true;
            return None;
        }
        let saved = self.style.clone();
        self.apply_style(e);
        let mut nested = Vec::new();
        let item = self.item_of(
            |c| {
                for child in li.children() {
                    match child.value() {
                        Node::Element(x) if matches!(x.name(), "ul" | "ol") => {
                            nested.extend(c.items(child, depth + 1));
                        }
                        _ => c.walk(child, depth + 1),
                    }
                }
            },
            Vec::new(),
        );
        self.style = saved;
        match item {
            Some(mut item) => {
                if !nested.is_empty() {
                    item["blocks"] = Value::Array(nested);
                }
                Some(item)
            }
            None if !nested.is_empty() => {
                Some(json!({ "kind": "item", "fields": { "text": "" }, "blocks": nested }))
            }
            None => None,
        }
    }

    /// The item `f` walks, its content one line; `None` when it holds no text.
    fn item_of(&mut self, f: impl FnOnce(&mut Self), blocks: Vec<Value>) -> Option<Value> {
        let line = std::mem::take(&mut self.line);
        let mode = std::mem::replace(&mut self.mode, Mode::Line);
        f(self);
        self.mode = mode;
        let text = self.take_line();
        self.line = line;
        if text.is_empty() && blocks.is_empty() {
            return None;
        }
        let mut item = json!({ "kind": "item", "fields": { "text": text } });
        if !blocks.is_empty() {
            item["blocks"] = Value::Array(blocks);
        }
        Some(item)
    }

    // ---- tables ---------------------------------------------------------------------------

    /// Whether table `n` lays out the message rather than holding data (rich-import H1): marked
    /// `role="presentation"` or `none`, inside a newsletter, holding a table, fewer than two
    /// rows, one column, a cell with a block in it (an image, a heading, a list, a quote, code,
    /// a rule, or more than one paragraph or `div`), or more columns or rows than a table block
    /// takes.
    fn is_layout(&self, n: NodeRef, e: &Element) -> bool {
        if e.attr("role")
            .is_some_and(|r| matches!(r.trim(), "presentation" | "none"))
            || self.newsletter
        {
            return true;
        }
        if holds(n, &["table"]) {
            return true;
        }
        let rows = rows(n);
        if rows.len() < 2 || rows.len() > schema::TABLE_ROWS_MAX {
            return true;
        }
        let width = rows
            .iter()
            .map(|(r, _)| cells(*r).iter().map(|c| span(*c)).sum::<usize>())
            .max()
            .unwrap_or(0);
        if width <= 1 || width > schema::TABLE_COLUMNS_MAX {
            return true;
        }
        rows.iter().any(|(r, _)| {
            cells(*r).iter().any(|c| {
                holds(*c, &CELL_BLOCKS)
                    || c.children()
                        .filter(|x| matches!(x.value(), Node::Element(e) if matches!(e.name(), "p" | "div")))
                        .count()
                        > 1
            })
        })
    }

    fn table(&mut self, n: NodeRef, e: &Element, depth: usize) {
        self.flush();
        if self.mode == Mode::Line || self.is_layout(n, e) {
            // A layout: each cell's content in reading order, row by row.
            for (row, _) in rows(n) {
                for cell in cells(row) {
                    if self.full {
                        return;
                    }
                    self.walk(cell, depth + 2);
                    self.flush();
                }
            }
            return;
        }
        let rows = rows(n);
        let mut out_rows: Vec<Vec<String>> = Vec::new();
        let mut aligns: Vec<Option<&'static str>> = Vec::new();
        let mut header = false;
        for (k, (row, in_head)) in rows.iter().enumerate() {
            let cs = cells(*row);
            if k == 0 {
                header = *in_head
                    || cs
                        .iter()
                        .all(|c| matches!(c.value(), Node::Element(e) if e.name() == "th"));
            }
            let mut texts = Vec::new();
            for c in cs {
                let Node::Element(ce) = c.value() else {
                    continue;
                };
                let decls = ce.attr("style").map(declarations).unwrap_or_default();
                let align = css::align(ce.attr("align"), &decls);
                let column = texts.len();
                if aligns.len() <= column {
                    aligns.resize(column + 1, None);
                }
                if aligns[column].is_none() {
                    aligns[column] = align;
                }
                let saved = self.style.clone();
                self.apply_style(ce);
                let text = self
                    .item_of(|conv| conv.children(c, depth + 2), Vec::new())
                    .and_then(|i| i["fields"]["text"].as_str().map(str::to_owned))
                    .unwrap_or_default();
                self.style = saved;
                texts.push(text);
                texts.extend(std::iter::repeat_n(String::new(), span(c) - 1));
            }
            if texts.iter().any(|t| !t.is_empty()) {
                out_rows.push(texts);
            } else if k == 0 {
                header = false;
            }
        }
        let deferred = std::mem::take(&mut self.deferred);
        if !out_rows.is_empty() {
            let width = out_rows.iter().map(Vec::len).max().unwrap_or(0);
            let mut fields = json!({});
            if header {
                fields["header"] = json!(true);
            }
            let align: Vec<&str> = (0..width)
                .map(|c| aligns.get(c).copied().flatten().unwrap_or("left"))
                .collect();
            if align.iter().any(|a| *a != "left") {
                fields["align"] = json!(align);
            }
            let rows: Vec<Value> = out_rows
                .into_iter()
                .map(|mut r| {
                    r.resize(width, String::new());
                    json!({ "kind": "row", "fields": { "cells": r } })
                })
                .collect();
            self.emit(json!({ "kind": "table", "fields": fields, "blocks": rows }));
        }
        for b in deferred {
            self.emit(b);
        }
    }
}

fn count_items(items: &[Value]) -> usize {
    items
        .iter()
        .map(|i| 1 + i["blocks"].as_array().map_or(0, |b| count_items(b)))
        .sum()
}

/// Whether `e` starts an Outlook reply's history, which runs to the end of the message:
/// `div#appendonsend`, `div#divRplyFwdMsg`, or a `div` with a solid top border whose text starts
/// with `From:`, as Outlook on the desktop writes it.
fn outlook_history(n: NodeRef, e: &Element) -> bool {
    if e.name() != "div" {
        return false;
    }
    if e.id()
        .is_some_and(|id| id == "appendonsend" || id.starts_with("divRplyFwdMsg"))
    {
        return true;
    }
    let bordered = e.attr("style").is_some_and(|s| {
        let s = s.to_ascii_lowercase().replace(' ', "");
        s.contains("border-top:solid")
    });
    bordered && {
        let mut t = String::new();
        raw_text(n, &mut t);
        t.trim_start().starts_with("From:")
    }
}

/// The source of an image: `cid:`, a base64 `data:` image, or an http or https URL.
fn parse_src(src: &str) -> Option<Src> {
    let src = src.trim();
    let lower = src.to_ascii_lowercase();
    if let Some(id) = lower.strip_prefix("cid:").map(|_| &src[4..]) {
        let id = percent_decode(id.trim().trim_start_matches('<').trim_end_matches('>'));
        return (!id.is_empty()).then_some(Src::Cid(id));
    }
    if lower.starts_with("data:") {
        let (head, data) = src[5..].split_once(',')?;
        let media_type = head.split(';').next()?.trim().to_ascii_lowercase();
        if !media_type.starts_with("image/") || !head.to_ascii_lowercase().contains(";base64") {
            return None;
        }
        let bytes = base64(data)?;
        return Some(Src::Data { media_type, bytes });
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(Src::Remote(src.to_owned()));
    }
    if lower.starts_with("//") {
        return Some(Src::Remote(format!("https:{src}")));
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
        {
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(b[i + 1]) * 16 + hex(b[i + 2]));
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Standard or URL-safe base64, with or without padding and whitespace.
fn base64(data: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in data.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            b'%' => return None,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A line cut into paragraphs where two breaks meet, with only spaces between them.
fn split_paragraphs(line: Vec<Piece>) -> Vec<Vec<Piece>> {
    let mut out = vec![Vec::new()];
    let mut breaks = 0;
    for p in line {
        match &p {
            Piece::Break => breaks += 1,
            // Spaces between breaks are nothing; spaces between words are kept.
            Piece::Text(t, _)
                if t.chars()
                    .all(|c| c.is_whitespace() || c == '\u{a0}' || inline::is_invisible(c)) =>
            {
                if breaks == 0 {
                    out.last_mut().expect("one paragraph").push(p);
                }
            }
            Piece::Text(..) => {
                if breaks >= 2 {
                    out.push(Vec::new());
                } else if breaks == 1 {
                    out.last_mut().expect("one paragraph").push(Piece::Break);
                }
                breaks = 0;
                out.last_mut().expect("one paragraph").push(p);
            }
        }
    }
    out
}
