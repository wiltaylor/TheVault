//! Content blocks as the HTML Google Calendar takes for a description, and back (rich-import H4):
//! a description compared in that form round-trips, so an untouched one is never sent again.

use html_blocks::{LinkImages, Options, blocks_to_html, html_to_blocks, text_to_blocks};
use serde_json::{Value, json};

/// A description as the sync reads it: plain text when it holds no tags.
fn blocks_of(description: &str) -> Vec<Value> {
    let looks_html = description.contains('<') && description.contains('>');
    if looks_html {
        html_to_blocks(description, &Options::default(), &mut LinkImages).blocks
    } else {
        text_to_blocks(description, &Options::default()).blocks
    }
}

fn canonical(description: &str) -> String {
    blocks_to_html(&blocks_of(description))
}

const DESCRIPTIONS: [&str; 14] = [
    "",
    "Plain line\nSecond line\n\nNew paragraph with https://meet.example.com/abc-def",
    "Bring <b>snacks</b> &amp; <i>drinks</i><br>See <a href=\"https://example.com/map\">the map</a>",
    "<ul><li>One</li><li>Two<ul><li>Two point one</li></ul></li></ul><ol><li>First</li></ol>",
    "<p>Para one</p><p>Para <u>two</u></p><br><br><p>three</p>",
    "<h1>Agenda</h1><p>Intro</p><hr><blockquote>Quoted<br>words</blockquote>",
    "<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>",
    "Literal *stars* [brackets] {braces} `ticks` a+b ==eq== \\back",
    "<b>bold <i>both</i></b><i> italic</i> plain <s>struck</s> <code>code</code> <mark>hi</mark>",
    "  spaced   out\t text  \n\n\n  lines  ",
    "<img src=\"https://img.example.com/x.png\" alt=\"Map\"><p>after</p>",
    "<div>Gmail<div><br></div><div>style</div></div>",
    "> quoted line\n> more\nnot quoted",
    "<a href=\"https://example.com/p(1)\">paren link</a> and <a href=\"javascript:x()\">bad</a>",
];

#[test]
fn the_written_form_reads_back_as_itself() {
    for d in DESCRIPTIONS {
        let c = canonical(d);
        assert_eq!(d.trim().is_empty(), c.is_empty(), "{d:?} wrote {c:?}");
        assert_eq!(canonical(&c), c, "{d:?} wrote {c:?}");
        // The blocks read from the written form write it again.
        assert_eq!(blocks_to_html(&blocks_of(&c)), c, "{d:?}");
    }
}

#[test]
fn blocks_are_the_simple_html_google_takes() {
    let blocks = json!([
        { "kind": "heading", "fields": { "level": 2, "text": "Plan *now*" } },
        { "kind": "paragraph", "fields": { "text": "**Bold** [link](https://example.com/?a=1&b=2) <tag>\nnext {red}red{/} ~~gone~~ [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB|Alex]]" } },
        { "kind": "list", "fields": { "style": "todo" }, "blocks": [
            { "kind": "item", "fields": { "text": "Done", "checked": true } },
            { "kind": "item", "fields": { "text": "Open" } },
        ] },
        { "kind": "table", "fields": { "header": true }, "blocks": [
            { "kind": "row", "fields": { "cells": ["a", "b"] } },
            { "kind": "row", "fields": { "cells": ["1", "**2**"] } },
        ] },
        { "kind": "divider" },
        { "kind": "image", "fields": { "asset": "01JB9A0C3D4E5F6G7H8J9K0M1N", "alt": "Map" } },
        { "kind": "code", "fields": { "source": "a < b\n  c" } },
    ]);
    let html = blocks_to_html(blocks.as_array().unwrap());
    assert_eq!(
        html,
        "<p><b>Plan <i>now</i></b></p>\
         <p><b>Bold</b> <a href=\"https://example.com/?a=1&amp;b=2\">link</a> &lt;tag&gt;<br>next red gone Alex</p>\
         <ul><li>\u{2611} Done</li><li>\u{2610} Open</li></ul>\
         <p>a | b<br>1 | <b>2</b></p>\
         <hr>\
         <p>Image: Map</p>\
         <p>a &lt; b<br>c</p>"
    );
    assert_eq!(canonical(&html), html);
}
