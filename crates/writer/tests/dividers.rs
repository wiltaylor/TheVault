//! A note with `divider` blocks (rich-import H5): written in canonical form from the JSON form
//! the notes API and scripts send, read back as the same blocks, the same bytes on a second
//! write, and refused with items or blocks inside.

use std::path::Path;

use serde_json::json;
use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const PAGE: &str = r#"label = "Page"

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
  }
}
"#;

/// A clock that never moves, so a write and its rewrite share `created` and `updated`.
struct FixedClock;

impl state::Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        1_790_000_000_000
    }
}

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/page.wcl"), PAGE).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state)
        .unwrap()
        .with_clock(std::sync::Arc::new(FixedClock))
}

fn note(blocks: serde_json::Value) -> serde_json::Value {
    json!({ "fields": { "title": "Divided" },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": blocks }] })
}

#[test]
fn dividers_round_trip_through_json_and_the_canonical_writer() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let sent = json!([
        { "kind": "paragraph", "fields": { "text": "Above" } },
        { "kind": "divider" },
        { "kind": "paragraph", "fields": { "text": "Below" } },
        { "kind": "toggle", "fields": { "text": "More" }, "blocks": [{ "kind": "divider" }] },
    ]);
    let saved = w
        .create("page", &Input::Json(note(sent.clone())), &author)
        .unwrap();
    assert!(saved.wcl.contains("\n  divider\n"), "{}", saved.wcl);

    let read = w.read(&saved.id).unwrap();
    let back = writer::doc_to_json(&read.doc);
    assert_eq!(back["blocks"][0]["blocks"][1]["kind"], "divider");
    assert_eq!(
        back["blocks"][0]["blocks"][3]["blocks"][0]["kind"],
        "divider"
    );
    let path = dir.path().join(format!("notes/page/{}.wcl", saved.id));
    let first = std::fs::read(&path).unwrap();
    w.update(
        &saved.id,
        &Input::Wcl(String::from_utf8(first.clone()).unwrap()),
        Some(&saved.version),
        &author,
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
}

#[test]
fn a_divider_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    for block in [
        json!({ "kind": "divider", "fields": { "style": "dashed" } }),
        json!({ "kind": "divider", "blocks": [{ "kind": "paragraph", "fields": { "text": "x" } }] }),
    ] {
        let err = w
            .create("page", &Input::Json(note(json!([block]))), &author)
            .unwrap_err();
        assert!(err.to_string().contains("divider"), "{err}");
    }
}
