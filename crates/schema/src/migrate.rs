//! Reading canvases written before canvas nodes held content blocks.
//!
//! A canvas node used to be one of six kinds: `text`, `note`, `query`, `asset`, `link` and `ink`.
//! Now a node is a `card`, which holds a sequence of content blocks exactly as a content field
//! does, or a `note`. [`canvases`] rewrites every node of an older kind, at any depth of a file, as
//! the card that shows the same thing:
//!
//! - `text`: one `paragraph` per run of text between blank lines, inline markup kept.
//! - `asset`: an `image` block naming the asset.
//! - `link`: a paragraph holding the link, labelled with its URL.
//! - `query`: a `query_view` block with the query and one view of the node's view kind; a cards
//!   view keeps the node's template.
//! - `ink`: an `ink` block of the node's size holding its strokes.
//!
//! The node keeps its id, box, group and colour, and edges are untouched. Every vault file is
//! read through [`crate::Doc::from_source`], which applies this, so an old canvas loads, indexes
//! and validates as cards, and is written in the new form the next time its file is saved.

use crate::doc::{Doc, DocBlock, Value};

/// The node items every kind shares, which a converted node keeps.
const KEPT: [&str; 7] = ["kind", "x", "y", "width", "height", "group", "colour"];

/// Rewrite every old-kind node of every canvas in `doc`, at any depth.
pub fn canvases(doc: &mut Doc) {
    for b in &mut doc.blocks {
        if b.kind == "canvas" {
            for n in b.body.blocks.iter_mut().filter(|n| n.kind == "node") {
                node(&mut n.body);
            }
        }
        canvases(&mut b.body);
    }
}

fn str_of<'a>(d: &'a Doc, name: &str) -> Option<&'a str> {
    d.field(name).and_then(Value::as_str)
}

fn text_block(kind: &str, text: String) -> DocBlock {
    DocBlock {
        kind: kind.to_owned(),
        labels: Vec::new(),
        body: Doc {
            fields: vec![("text".into(), Value::Str(text))],
            blocks: Vec::new(),
        },
    }
}

/// One old node's body, rewritten as a card. A `note`, a `card` or an unknown kind is left as it is.
fn node(d: &mut Doc) {
    let Some(kind) = str_of(d, "kind").map(str::to_owned) else {
        return;
    };
    let blocks: Vec<DocBlock> = match kind.as_str() {
        "text" => paragraphs(str_of(d, "text").unwrap_or("")),
        "link" => match str_of(d, "url") {
            Some(url) => vec![text_block(
                "paragraph",
                format!(
                    "[{}]({})",
                    escape(url),
                    url.replace('(', "%28").replace(')', "%29")
                ),
            )],
            None => Vec::new(),
        },
        "asset" => match d.field("asset") {
            Some(asset) => vec![DocBlock {
                kind: "image".into(),
                labels: Vec::new(),
                body: Doc {
                    fields: vec![("asset".into(), asset.clone())],
                    blocks: Vec::new(),
                },
            }],
            None => Vec::new(),
        },
        "query" => vec![query_view(d)],
        "ink" => vec![ink(d)],
        _ => return,
    };
    d.fields.retain(|(n, _)| KEPT.contains(&n.as_str()));
    for (n, v) in &mut d.fields {
        if n == "kind" {
            *v = Value::Str("card".into());
        }
    }
    d.blocks = blocks;
}

/// The paragraphs of a text node: each run of lines between blank lines, without the blank lines.
pub fn paragraphs(text: &str) -> Vec<DocBlock> {
    let mut out = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let flush = |run: &mut Vec<&str>, out: &mut Vec<DocBlock>| {
        if !run.is_empty() {
            out.push(text_block("paragraph", run.join("\n")));
            run.clear();
        }
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut run, &mut out);
        } else {
            run.push(line);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// `s` as literal text in inline markup: every markup character escaped.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '*' | '+' | '~' | '`' | '=' | '{' | '}' | '[' | ']' | '(' | ')' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn query_view(d: &Doc) -> DocBlock {
    let view = str_of(d, "view")
        .filter(|v| matches!(*v, "list" | "table"))
        .unwrap_or("cards");
    let mut view_fields = vec![("kind".to_owned(), Value::Str(view.into()))];
    if view == "cards"
        && let Some(t) = d.field("template")
    {
        view_fields.push(("template".into(), t.clone()));
    }
    let mut fields = Vec::new();
    if let Some(q) = d.field("query") {
        fields.push(("query".to_owned(), q.clone()));
    }
    DocBlock {
        kind: "query_view".into(),
        labels: Vec::new(),
        body: Doc {
            fields,
            blocks: vec![DocBlock {
                kind: "view".into(),
                labels: vec![view.to_owned()],
                body: Doc {
                    fields: view_fields,
                    blocks: Vec::new(),
                },
            }],
        },
    }
}

fn ink(d: &Doc) -> DocBlock {
    let size = |name: &str| {
        let v = d.field(name).and_then(Value::as_f64).unwrap_or(1.0);
        Value::Int((v.round() as i64).max(1))
    };
    DocBlock {
        kind: "ink".into(),
        labels: Vec::new(),
        body: Doc {
            fields: vec![
                ("width".into(), size("width")),
                ("height".into(), size("height")),
            ],
            blocks: d.blocks_of("stroke").cloned().collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Doc {
        Doc::parse(src, "n.wcl").unwrap()
    }

    fn canvas_of(src: &str) -> Doc {
        parse(&format!(
            "canvas board {{\n viewport {{\n x = 0.0\n y = 0.0\n zoom = 1.0\n }}\n{src}\n edge e {{\n from = \"n\"\n to = \"n\"\n }}\n}}\n"
        ))
        .blocks
        .remove(0)
        .body
    }

    fn the_node(c: &Doc) -> &Doc {
        &c.blocks_of("node").next().unwrap().body
    }

    fn fields(d: &Doc) -> Vec<String> {
        d.fields.iter().map(|(n, _)| n.clone()).collect()
    }

    #[test]
    fn a_text_node_becomes_a_card_of_paragraphs() {
        let c = canvas_of(
            " node n {\n kind = \"text\"\n x = 1.0\n y = 2.0\n width = 3.0\n height = 4.0\n group = \"g\"\n colour = \"red\"\n text = \"First **bold**\\nline two\\n\\n  \\nSecond [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZA]]\"\n }",
        );
        let n = the_node(&c);
        assert_eq!(n.field("kind"), Some(&Value::Str("card".into())));
        assert_eq!(
            fields(n),
            ["kind", "x", "y", "width", "height", "group", "colour"]
        );
        let texts: Vec<_> = n
            .blocks
            .iter()
            .map(|b| {
                (
                    b.kind.as_str(),
                    b.body.field("text").and_then(Value::as_str).unwrap(),
                )
            })
            .collect();
        assert_eq!(
            texts,
            [
                ("paragraph", "First **bold**\nline two"),
                ("paragraph", "Second [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZA]]"),
            ]
        );
        // The edge and the viewport are as they were.
        assert_eq!(c.blocks_of("edge").count(), 1);
        assert_eq!(c.blocks_of("viewport").count(), 1);
    }

    #[test]
    fn an_asset_node_becomes_a_card_with_an_image() {
        let c = canvas_of(
            " node n {\n kind = \"asset\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n asset = \"01JB9A0C3D4E5F6G7H8J9K0M1N\"\n }",
        );
        let n = the_node(&c);
        assert_eq!(fields(n), ["kind", "x", "y", "width", "height"]);
        let [img] = n.blocks.as_slice() else { panic!() };
        assert_eq!(img.kind, "image");
        assert_eq!(
            img.body.fields,
            [(
                "asset".into(),
                Value::Str("01JB9A0C3D4E5F6G7H8J9K0M1N".into())
            )]
        );
    }

    #[test]
    fn a_link_node_becomes_a_card_with_a_linked_paragraph() {
        let c = canvas_of(
            " node n {\n kind = \"link\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n url = \"https://example.com/a_(b)\"\n }",
        );
        let [p] = the_node(&c).blocks.as_slice() else {
            panic!()
        };
        assert_eq!(p.kind, "paragraph");
        assert_eq!(
            p.body.field("text").and_then(Value::as_str),
            Some("[https://example.com/a_\\(b\\)](https://example.com/a_%28b%29)")
        );
    }

    #[test]
    fn a_query_node_becomes_a_card_with_a_query_view() {
        let c = canvas_of(
            " node n {\n kind = \"query\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n query = \"from task\"\n view = \"cards\"\n template = \"card\"\n }",
        );
        let [q] = the_node(&c).blocks.as_slice() else {
            panic!()
        };
        assert_eq!(q.kind, "query_view");
        assert_eq!(
            q.body.fields,
            [("query".into(), Value::Str("from task".into()))]
        );
        let [v] = q.body.blocks.as_slice() else {
            panic!()
        };
        assert_eq!(
            (v.kind.as_str(), v.labels.as_slice()),
            ("view", ["cards".to_owned()].as_slice())
        );
        assert_eq!(
            v.body.fields,
            [
                ("kind".into(), Value::Str("cards".into())),
                ("template".into(), Value::Str("card".into()))
            ]
        );

        // A list or table view takes no template; no view at all is cards.
        let c = canvas_of(
            " node n {\n kind = \"query\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n query = \"from task\"\n view = \"list\"\n template = \"card\"\n }",
        );
        let v = &the_node(&c).blocks[0].body.blocks[0];
        assert_eq!(v.labels, ["list"]);
        assert_eq!(v.body.fields, [("kind".into(), Value::Str("list".into()))]);
        let c = canvas_of(
            " node n {\n kind = \"query\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n query = \"from task\"\n }",
        );
        assert_eq!(the_node(&c).blocks[0].body.blocks[0].labels, ["cards"]);
    }

    #[test]
    fn an_ink_node_becomes_a_card_with_its_drawing() {
        let c = canvas_of(
            " node n {\n kind = \"ink\"\n x = 0.0\n y = 0.0\n width = 200.4\n height = 120.0\n stroke {\n tool = \"pen\"\n colour = \"#dc2626\"\n size = 3.0\n points = [1.0, 2.0, 0.5]\n }\n }",
        );
        let [ink] = the_node(&c).blocks.as_slice() else {
            panic!()
        };
        assert_eq!(ink.kind, "ink");
        assert_eq!(
            ink.body.fields,
            [
                ("width".into(), Value::Int(200)),
                ("height".into(), Value::Int(120))
            ]
        );
        assert_eq!(ink.body.blocks_of("stroke").count(), 1);
    }

    #[test]
    fn note_and_card_nodes_and_canvases_in_content_are_handled() {
        let src = "content body {\n canvas {\n viewport {\n x = 0.0\n y = 0.0\n zoom = 1.0\n }\n node t {\n kind = \"text\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n text = \"hi\"\n }\n node m {\n kind = \"note\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n note = \"01JB8Z4K2MQ7V3X9T5R6W8Y2ZA\"\n }\n node c {\n kind = \"card\"\n x = 0.0\n y = 0.0\n width = 3.0\n height = 4.0\n paragraph {\n text = \"kept\"\n }\n }\n }\n}\n";
        let raw = Doc::from_source(&store::parse_data_only(src, "n.wcl").unwrap());
        let canvas = &raw.blocks[0].body.blocks[0].body;
        let kinds: Vec<_> = canvas
            .blocks_of("node")
            .map(|n| n.body.field("kind").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(kinds, ["card", "note", "card"]);
        let note = canvas.blocks_of("node").nth(1).unwrap();
        assert!(note.body.field("note").is_some());
        let card = canvas.blocks_of("node").nth(2).unwrap();
        assert_eq!(
            card.body.blocks[0]
                .body
                .field("text")
                .and_then(Value::as_str),
            Some("kept")
        );
    }
}
