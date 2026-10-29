//! HTML to content blocks (rich-import H1, H2, H3), on the small anonymised fixtures in
//! `tests/fixtures/`.

use html_blocks::{
    Images, LinkImages, Options, Placed, Src, complete_pending, html_to_blocks, pending_image,
};
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn mail() -> Options {
    Options {
        quotes: true,
        ..Options::default()
    }
}

fn convert(html: &str, opts: &Options) -> Vec<Value> {
    html_to_blocks(html, opts, &mut LinkImages).blocks
}

fn para(text: &str) -> Value {
    json!({ "kind": "paragraph", "fields": { "text": text } })
}

fn toggle(text: &str, blocks: Vec<Value>) -> Value {
    json!({ "kind": "toggle", "fields": { "text": text }, "blocks": blocks })
}

/// Each block's kind, and a toggle's or a list's blocks' kinds after it in brackets.
fn shape(blocks: &[Value]) -> Vec<String> {
    blocks
        .iter()
        .map(|b| {
            let kind = b["kind"].as_str().unwrap().to_owned();
            match b["blocks"].as_array() {
                Some(inner) if kind != "table" && kind != "list" => {
                    format!("{kind}[{}]", shape(inner).join(","))
                }
                _ => kind,
            }
        })
        .collect()
}

#[test]
fn a_gmail_reply_folds_its_quote_and_signature_into_toggles() {
    let blocks = convert(&fixture("gmail_reply.html"), &mail());
    assert_eq!(
        blocks,
        [
            para("Thanks, that works for me."),
            para("See you **Thursday** at the [library](https://maps.example.com/?q=library)."),
            toggle("Signature", vec![para("Sam Sample"), para("Example Co.")]),
            toggle(
                "Quoted text",
                vec![
                    para(
                        "On Mon, 5 Jan 2026 at 10:00, Alex Example <[alex@example.com](mailto:alex@example.com)> wrote:"
                    ),
                    para("Does Thursday suit you?"),
                ]
            ),
        ]
    );
    // Without folding, the quote and signature are blocks like any other.
    let open = convert(&fixture("gmail_reply.html"), &Options::default());
    assert!(open.iter().all(|b| b["kind"] != "toggle"), "{open:#?}");
    assert!(
        open.contains(&json!({ "kind": "quote", "fields": { "text": "Does Thursday suit you?" } })),
        "{open:#?}"
    );
}

#[test]
fn an_outlook_reply_folds_everything_from_its_header_on() {
    let blocks = convert(&fixture("outlook_reply.html"), &mail());
    assert_eq!(blocks[0], para("The report is attached."));
    assert_eq!(blocks[1], para("Regards,\nJo"));
    assert_eq!(shape(&blocks[2..]), ["toggle[divider,paragraph,paragraph]"]);
    assert_eq!(
        blocks[2]["blocks"][1],
        para(
            "**From:** Alex Example <alex@example.com>\n**Sent:** Monday, 5 January 2026 10:00\n**Subject:** Report"
        )
    );
    assert_eq!(blocks[2]["blocks"][2], para("Could you send the report?"));
    // The style element's text is never content.
    assert!(!serde_json::to_string(&blocks).unwrap().contains("margin"));
}

#[test]
fn an_apple_mail_reply_folds_the_cited_quote_with_its_attribution() {
    let blocks = convert(&fixture("apple_reply.html"), &mail());
    assert_eq!(
        blocks,
        [
            para("Sounds good."),
            toggle(
                "Quoted text",
                vec![
                    para("On 5 Jan 2026, at 10:00, Alex Example <alex@example.com> wrote:"),
                    para("Lunch on Friday?"),
                ]
            ),
        ]
    );
}

#[test]
fn a_newsletter_unwraps_its_layout_tables_and_drops_what_is_hidden() {
    let blocks = convert(&fixture("newsletter.html"), &mail());
    assert_eq!(
        shape(&blocks),
        [
            "paragraph",
            "heading",
            "paragraph",
            "paragraph",
            "heading",
            "paragraph",
            "heading",
            "paragraph",
            "paragraph",
            "paragraph",
            "paragraph"
        ]
    );
    assert_eq!(
        blocks[1]["fields"],
        json!({ "level": 1, "text": "Trails of the week" })
    );
    assert_eq!(blocks[2], para("Three **new** routes opened this weekend."));
    assert_eq!(
        blocks[3],
        para("[Image: A forest trail](https://img.example.com/hero.jpg)")
    );
    // A link in a filled cell, and a link styled with a background, are bold buttons.
    assert_eq!(
        blocks[8],
        para("[**Claim your voucher**](https://shop.example.com/voucher?u=1)")
    );
    assert_eq!(
        blocks[9],
        para("[**Get the maps**](https://shop.example.com/maps)")
    );
    // A small icon is its alt text, in its link.
    assert_eq!(
        blocks[10],
        para(
            "[Social](https://social.example.com/outdoors) [Unsubscribe](https://example.com/unsubscribe)"
        )
    );
    let all = serde_json::to_string(&blocks).unwrap();
    // The preheader, Outlook's conditional comment, the title and the tracking pixels are gone.
    for gone in [
        "voucher\u{34f}",
        "This week",
        "Outlook only",
        "Weekly digest",
        "track.example.com",
    ] {
        assert!(!all.contains(gone), "{gone}: {all}");
    }
}

/// Every image placed in turn: stored, failed, pending, then skipped.
struct Script(Vec<Placed>, Vec<Src>);

impl Images for Script {
    fn place(&mut self, src: &Src, _alt: &str) -> Placed {
        self.1.push(src.clone());
        if self.0.is_empty() {
            Placed::Link
        } else {
            self.0.remove(0)
        }
    }
}

#[test]
fn images_are_placed_where_they_stood_and_tracking_pixels_are_never_asked_for() {
    let mut images = Script(
        vec![
            Placed::Asset("01JB9A0C3D4E5F6G7H8J9K0M1N".into()),
            Placed::Failed("the server answered 404".into()),
        ],
        Vec::new(),
    );
    let c = html_to_blocks(&fixture("newsletter.html"), &mail(), &mut images);
    // The logo and the hero only: never the icon or the 1x1 pixels.
    assert_eq!(
        images.1,
        [
            Src::Remote("https://img.example.com/logo.png".into()),
            Src::Remote("https://img.example.com/hero.jpg".into())
        ]
    );
    assert_eq!(
        c.blocks[0],
        json!({ "kind": "image", "fields": { "asset": "01JB9A0C3D4E5F6G7H8J9K0M1N", "alt": "Example Outdoors" } })
    );
    assert_eq!(
        c.blocks[3],
        para(
            "[Image: A forest trail](https://img.example.com/hero.jpg) (not downloaded: the server answered 404)"
        )
    );

    // cid: and data: images reach the store as the part's id and the decoded bytes.
    let mut images = Script(vec![Placed::Pending], Vec::new());
    let c = html_to_blocks(
        r#"<p>a</p><img src="cid:logo%40mail" alt="Logo"><img src="data:image/png;base64,iVBORw0K" alt="Dot"><img src="file:///etc/passwd" alt="Local">"#,
        &Options::default(),
        &mut images,
    );
    assert_eq!(
        images.1,
        [
            Src::Cid("logo@mail".into()),
            Src::Data {
                media_type: "image/png".into(),
                bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a]
            }
        ]
    );
    assert_eq!(c.pending, 1);
    // A pending image is a paragraph a later run finds again; one with no URL says why not; a
    // local file is its alt text.
    assert_eq!(c.blocks[1], para("Image: Logo (not downloaded yet)"));
    assert_eq!(c.blocks[2], para("Image: Dot"));
    assert_eq!(c.blocks[3], para("Local"));
}

#[test]
fn a_pending_image_is_completed_later_in_place() {
    let mut images = Script(vec![Placed::Pending], Vec::new());
    let c = html_to_blocks(
        r#"<p>Hi</p><img src="https://img.example.com/a.png" alt="A [b]"><p>Bye</p>"#,
        &Options::default(),
        &mut images,
    );
    assert_eq!(c.pending, 1);
    let text = c.blocks[1]["fields"]["text"].as_str().unwrap();
    assert_eq!(
        pending_image(text),
        Some((
            "A [b]".to_owned(),
            "https://img.example.com/a.png".to_owned()
        ))
    );
    assert_eq!(
        pending_image("[Image: x](https://e.example) (not downloaded: 404)"),
        None
    );
    assert_eq!(pending_image("Image: x (not downloaded yet)"), None);

    // Inside a toggle too.
    let nested = vec![toggle("Quoted text", c.blocks.clone())];
    let mut later = Script(vec![Placed::Pending], Vec::new());
    let (again, pending) = complete_pending(&nested, &mut later);
    assert_eq!((again, pending), (nested.clone(), 1));
    let mut done = Script(
        vec![Placed::Asset("01JB9A0C3D4E5F6G7H8J9K0M1P".into())],
        Vec::new(),
    );
    let (blocks, pending) = complete_pending(&nested, &mut done);
    assert_eq!(pending, 0);
    assert_eq!(
        blocks[0]["blocks"][1],
        json!({ "kind": "image", "fields": { "asset": "01JB9A0C3D4E5F6G7H8J9K0M1P", "alt": "A [b]" } })
    );
}

#[test]
fn a_data_table_is_a_table_with_its_header_and_alignments() {
    let blocks = convert(&fixture("data_table.html"), &mail());
    assert_eq!(shape(&blocks), ["paragraph", "table", "paragraph"]);
    assert_eq!(
        blocks[1],
        json!({ "kind": "table", "fields": { "header": true, "align": ["left", "right", "center"] }, "blocks": [
            { "kind": "row", "fields": { "cells": ["Venue", "Price", "Seats"] } },
            { "kind": "row", "fields": { "cells": ["**Town hall**", "$400", "120"] } },
            { "kind": "row", "fields": { "cells": ["Library *annex*", "$150", "40"] } },
            { "kind": "row", "fields": { "cells": ["Park | pavilion", "", "200"] } },
        ] })
    );
    // One column, one row, a nested table or an image in a cell: a layout, unwrapped.
    for layout in [
        "<table><tr><td>a</td></tr><tr><td>b</td></tr></table>",
        "<table><tr><td>a</td><td>b</td></tr></table>",
        "<table><tr><td>a</td><td><table><tr><td>b</td></tr></table></td></tr><tr><td>c</td><td>d</td></tr></table>",
        "<table role=\"presentation\"><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
        "<table><tr><td><img src=\"https://e.example/x.png\" alt=\"x\"></td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
    ] {
        let blocks = convert(layout, &mail());
        assert!(
            blocks.iter().all(|b| b["kind"] != "table"),
            "{layout}: {blocks:#?}"
        );
    }
}

#[test]
fn lists_nest_and_keep_their_style() {
    let blocks = convert(&fixture("lists.html"), &mail());
    assert_eq!(
        blocks,
        [
            json!({ "kind": "list", "fields": { "style": "bullet" }, "blocks": [
                { "kind": "item", "fields": { "text": "Fruit" }, "blocks": [
                    { "kind": "item", "fields": { "text": "Apples" } },
                    { "kind": "item", "fields": { "text": "Pears **(ripe)**" } },
                ] },
                { "kind": "item", "fields": { "text": "Bread" } },
            ] }),
            json!({ "kind": "list", "fields": { "style": "numbered" }, "blocks": [
                { "kind": "item", "fields": { "text": "First" } },
                { "kind": "item", "fields": { "text": "Second" }, "blocks": [
                    { "kind": "item", "fields": { "text": "Second, part one" } },
                ] },
            ] }),
        ]
    );
}

#[test]
fn pre_is_code_with_its_language_and_inline_code_is_marked() {
    let blocks = convert(&fixture("code.html"), &mail());
    assert_eq!(
        blocks[1],
        json!({ "kind": "code", "fields": { "language": "rust", "source": "fn main() {\n    println!(\"hi & bye\");\n}" } })
    );
    assert_eq!(
        blocks[2],
        para("Then `cargo run`, or `Ctrl`\\+`C` to stop.")
    );
}

#[test]
fn headings_take_levels_one_to_three_and_empty_ones_are_dropped() {
    let blocks = convert(&fixture("headings.html"), &mail());
    let levels: Vec<(i64, &str)> = blocks
        .iter()
        .map(|b| {
            (
                b["fields"]["level"].as_i64().unwrap(),
                b["fields"]["text"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        levels,
        [
            (1, "One"),
            (2, "Two"),
            (3, "Three"),
            (3, "Four"),
            (3, "Five"),
            (3, "Six")
        ]
    );
}

#[test]
fn only_web_mail_and_phone_links_are_links() {
    let blocks = convert(&fixture("links.html"), &mail());
    assert_eq!(
        blocks,
        [para(
            "[web](https://example.com/a) [plain](http://example.com/b) [mail](mailto:alex@example.com) [call](tel:+15550100) script ftp relative data mixed [paren](https://example.com/p(1\\))"
        )]
    );
}

#[test]
fn clear_colours_map_to_the_palette_and_greys_and_links_stay_plain() {
    let blocks = convert(&fixture("colours.html"), &mail());
    assert_eq!(
        blocks,
        [para(
            "{red}Red alert{/} {blue}blue note{/} black grey white [pink link](https://example.com) {green}green word{/}"
        )]
    );
}

#[test]
fn rules_are_dividers_and_a_figure_caption_goes_with_its_image() {
    let blocks = convert(&fixture("figure.html"), &mail());
    assert_eq!(
        shape(&blocks),
        [
            "paragraph",
            "divider",
            "paragraph",
            "paragraph",
            "divider",
            "paragraph"
        ]
    );
    assert_eq!(blocks[3], para("Sales by month"));
    let mut stored = Script(
        vec![Placed::Asset("01JB9A0C3D4E5F6G7H8J9K0M1Q".into())],
        Vec::new(),
    );
    let c = html_to_blocks(&fixture("figure.html"), &mail(), &mut stored);
    assert_eq!(
        c.blocks[2],
        json!({ "kind": "image", "fields": { "asset": "01JB9A0C3D4E5F6G7H8J9K0M1Q", "alt": "Sales chart", "caption": "Sales by month" } })
    );
    assert_eq!(
        shape(&c.blocks),
        ["paragraph", "divider", "image", "divider", "paragraph"]
    );
}

#[test]
fn entities_are_decoded_and_markup_characters_escaped() {
    let blocks = convert(&fixture("entities.html"), &mail());
    assert_eq!(
        blocks,
        [para(
            "Fish & chips <3 \"quoted\" © 2026 © © café — spaced out 5 \\* 3 = 15 \\[x\\] \\{y} \\`z\\` a+b ~c~ \\=\\=d\\=\\= \\\\e"
        )]
    );
}

#[test]
fn malformed_html_reads_as_a_browser_reads_it() {
    let blocks = convert(&fixture("malformed.html"), &mail());
    assert_eq!(blocks[0], para("Unclosed paragraph **bold *both***"));
    assert!(blocks.iter().any(|b| b["kind"] == "list"), "{blocks:#?}");
    assert!(blocks.iter().all(|b| b["kind"] != "table"), "{blocks:#?}");
}

#[test]
fn scripts_styles_frames_forms_and_script_urls_never_reach_the_blocks() {
    let blocks = convert(&fixture("hostile.html"), &mail());
    assert_eq!(
        blocks,
        [
            para("Safe text"),
            para("Click me and bad image"),
            para("<script>still text</script>"),
        ]
    );
    let all = serde_json::to_string(&blocks).unwrap();
    for gone in [
        "alert",
        "bad\"",
        "frame text",
        "object text",
        "secret",
        "Form text",
        "svg text",
        "javascript",
        "evil",
    ] {
        assert!(!all.contains(gone), "{gone}: {all}");
    }
}

#[test]
fn line_breaks_stay_in_a_paragraph_and_a_blank_line_starts_another() {
    let blocks = convert(
        "<div>one<br>two<br><br>three<br></div><div>&nbsp;</div><p>\u{200b}</p>",
        &mail(),
    );
    assert_eq!(blocks, [para("one\ntwo"), para("three")]);
}

#[test]
fn a_huge_input_is_cut_with_a_closing_paragraph() {
    let html = "<p>Some words here.</p>".repeat(200_000);
    let c = html_to_blocks(&html, &mail(), &mut LinkImages);
    assert!(c.cut);
    assert!(
        c.blocks.len() <= html_blocks::MAX_BLOCKS + 1,
        "{}",
        c.blocks.len()
    );
    assert_eq!(
        c.blocks.last().unwrap(),
        &para("The rest is longer than the import keeps; see View original.")
    );
    assert!(!c.warnings.is_empty());
    // Deep nesting stops at the depth limit without overflowing.
    let deep = "<div>".repeat(5000) + "deep" + &"</div>".repeat(5000);
    let c = html_to_blocks(&deep, &mail(), &mut LinkImages);
    assert!(
        c.warnings.iter().any(|w| w.contains("nested")),
        "{:?}",
        c.warnings
    );
    // More images than the cap: the rest say why.
    let many = "<img src=\"https://img.example.com/a.png\" alt=\"a\">".repeat(45);
    let mut count = Script(Vec::new(), Vec::new());
    let c = html_to_blocks(&many, &mail(), &mut count);
    assert_eq!(count.1.len(), html_blocks::MAX_IMAGES);
    assert_eq!(
        c.blocks.last().unwrap(),
        &para(
            "[Image: a](https://img.example.com/a.png) (not downloaded: more than 40 images in one message)"
        )
    );
}
