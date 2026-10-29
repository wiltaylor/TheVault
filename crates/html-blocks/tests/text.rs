//! A `text/plain` body as content blocks (rich-import H1, H2).

use html_blocks::{Options, text_to_blocks};
use serde_json::{Value, json};

fn mail() -> Options {
    Options {
        quotes: true,
        ..Options::default()
    }
}

fn para(text: &str) -> Value {
    json!({ "kind": "paragraph", "fields": { "text": text } })
}

#[test]
fn paragraphs_keep_their_lines_and_urls_become_links() {
    let blocks = text_to_blocks(
        "Hi Sam,\r\nthe agenda is at https://docs.example.com/agenda.\r\n\r\n\r\nSee (https://example.com/a_(b)) and <https://example.com/c>  now *really*\n",
        &mail(),
    )
    .blocks;
    assert_eq!(
        blocks,
        [
            para(
                "Hi Sam,\nthe agenda is at [https://docs.example.com/agenda](https://docs.example.com/agenda)."
            ),
            para(
                "See ([https://example.com/a_(b)](https://example.com/a_(b\\))) and <[https://example.com/c](https://example.com/c)> now \\*really\\*"
            ),
        ]
    );
}

#[test]
fn quoted_lines_are_a_quote() {
    let blocks = text_to_blocks("Before\n> one\n> two\nAfter", &Options::default()).blocks;
    assert_eq!(
        blocks,
        [
            para("Before"),
            json!({ "kind": "quote", "fields": { "text": "one\ntwo" } }),
            para("After"),
        ]
    );
}

#[test]
fn the_quoted_history_and_signature_fold_into_toggles() {
    let text = "Yes, Thursday.\n\n-- \nSam Sample\nExample Co.\n\nOn Mon, 5 Jan 2026 at 10:00, Alex Example <alex@example.com>\nwrote:\n> Does Thursday suit you?\n>\n> On Sun, 4 Jan 2026, Sam wrote:\n>> Shall we meet?\n";
    let blocks = text_to_blocks(text, &mail()).blocks;
    assert_eq!(
        blocks,
        [
            para("Yes, Thursday."),
            json!({ "kind": "toggle", "fields": { "text": "Signature" }, "blocks": [para("Sam Sample\nExample Co.")] }),
            json!({ "kind": "toggle", "fields": { "text": "Quoted text" }, "blocks": [
                para("On Mon, 5 Jan 2026 at 10:00, Alex Example <alex@example.com> wrote:"),
                para("Does Thursday suit you?"),
                { "kind": "toggle", "fields": { "text": "Quoted text" }, "blocks": [
                    para("On Sun, 4 Jan 2026, Sam wrote:"),
                    para("Shall we meet?"),
                ] },
            ] }),
        ]
    );
    // An Outlook reply's history, from its separator on.
    let outlook = text_to_blocks(
        "Attached.\n\n-----Original Message-----\nFrom: Alex\nSubject: Report\n\nSend it?",
        &mail(),
    )
    .blocks;
    assert_eq!(
        outlook,
        [
            para("Attached."),
            json!({ "kind": "toggle", "fields": { "text": "Quoted text" }, "blocks": [
                para("From: Alex\nSubject: Report"),
                para("Send it?"),
            ] }),
        ]
    );
}

#[test]
fn a_huge_text_is_cut_with_a_closing_paragraph() {
    let text = "word\n\n".repeat(10_000);
    let c = text_to_blocks(&text, &mail());
    assert!(c.cut);
    assert_eq!(c.blocks.len(), html_blocks::MAX_BLOCKS + 1);
}
