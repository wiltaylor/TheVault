//! A note with `toggle` and `admonition` blocks: written in canonical form from the JSON form the
//! notes API and scripts send, read back as the same blocks, the same bytes on a second write, and
//! refused when an item is wrong.

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

const CANONICAL: &str = r#"content body {
  toggle {
    level = 2
    text  = "Details **here**"

    paragraph {
      text = "Hidden until opened"
    }

    list {
      style = "bullet"

      item {
        text = "One"
      }
    }
  }

  toggle {
    text = "Plain summary"
  }

  admonition {
    kind = "warning"

    paragraph {
      text = "Mind the gap"
    }
  }

  admonition {
    collapsible = true
    colour      = "pink"
    icon        = "flame"
    kind        = "tip"
    title       = "A **custom** title"

    toggle {
      text = "Nested"

      quote {
        text = "Deep"
      }
    }
  }
}
"#;

// The JSON form's `fields` is an object, read in sorted key order, so the items of a block sent
// as JSON are written in that order.

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

fn body() -> serde_json::Value {
    json!([
        { "kind": "toggle", "fields": { "level": 2, "text": "Details **here**" }, "blocks": [
            { "kind": "paragraph", "fields": { "text": "Hidden until opened" } },
            { "kind": "list", "fields": { "style": "bullet" }, "blocks": [
                { "kind": "item", "fields": { "text": "One" } },
            ] },
        ] },
        { "kind": "toggle", "fields": { "text": "Plain summary" } },
        { "kind": "admonition", "fields": { "kind": "warning" }, "blocks": [
            { "kind": "paragraph", "fields": { "text": "Mind the gap" } },
        ] },
        { "kind": "admonition", "fields": {
            "kind": "tip", "title": "A **custom** title", "colour": "pink", "icon": "flame", "collapsible": true,
        }, "blocks": [
            { "kind": "toggle", "fields": { "text": "Nested" }, "blocks": [
                { "kind": "quote", "fields": { "text": "Deep" } },
            ] },
        ] },
    ])
}

#[test]
fn toggles_and_admonitions_round_trip_through_json_and_the_canonical_writer() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let note = json!({ "fields": { "title": "Folded" },
                       "blocks": [{ "kind": "content", "labels": ["body"], "blocks": body() }] });
    let saved = w.create("page", &Input::Json(note), &author).unwrap();
    assert!(saved.wcl.ends_with(CANONICAL), "{}", saved.wcl);

    let read = w.read(&saved.id).unwrap();
    let back = writer::doc_to_json(&read.doc);
    assert_eq!(back["blocks"][0]["blocks"], body());
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
fn a_wrong_kind_colour_or_icon_is_refused_with_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let with = |block: serde_json::Value| {
        json!({ "fields": { "title": "Bad" },
                "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [block] }] })
    };
    for (block, want) in [
        (
            json!({ "kind": "admonition", "fields": { "kind": "loud" } }),
            "\"loud\" is not one of note, info, tip",
        ),
        (
            json!({ "kind": "admonition", "fields": { "kind": "tip", "colour": "teal" } }),
            "\"teal\" is not a palette colour",
        ),
        (
            json!({ "kind": "admonition", "fields": { "kind": "tip", "icon": "nope-nope" } }),
            "\"nope-nope\" is not an icon name",
        ),
        (
            json!({ "kind": "toggle", "fields": { "text": "x", "level": 7 } }),
            "must be from 0 to 3",
        ),
        (
            json!({ "kind": "callout", "fields": {} }),
            "\"callout\" is not a content block kind",
        ),
    ] {
        let err = w
            .create("page", &Input::Json(with(block)), &author)
            .unwrap_err();
        assert!(err.to_string().contains(want), "{want}: {err}");
    }
}
