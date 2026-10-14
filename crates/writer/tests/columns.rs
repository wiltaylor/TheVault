//! A note with a `columns` block: written in canonical form from JSON, read back as the same
//! blocks, the same bytes on a second write, and refused when its shape is wrong.

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
    columns {
      column {
        width = 2
        slot "title"
      }
      column {
        slot "body"
      }
    }
  }
}
"#;

const CANONICAL: &str = r#"content body {
  paragraph {
    text = "Above"
  }

  columns {
    column {
      width = 1.5

      heading {
        level = 2
        text  = "Left"
      }

      paragraph {
        text = "Beside **this**"
      }
    }

    column {
      width = 1
    }

    column {
      width = 0.5

      list {
        style = "bullet"

        item {
          text = "Right"
        }
      }
    }
  }
}
"#;

/// A clock that never moves, so a write and its rewrite share `created` and `updated` and a
/// byte comparison between them cannot depend on a second ticking over in between.
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
        { "kind": "paragraph", "fields": { "text": "Above" } },
        { "kind": "columns", "blocks": [
            { "kind": "column", "fields": { "width": 1.5 }, "blocks": [
                { "kind": "heading", "fields": { "level": 2, "text": "Left" } },
                { "kind": "paragraph", "fields": { "text": "Beside **this**" } },
            ] },
            { "kind": "column", "fields": { "width": 1 } },
            { "kind": "column", "fields": { "width": 0.5 }, "blocks": [
                { "kind": "list", "fields": { "style": "bullet" }, "blocks": [
                    { "kind": "item", "fields": { "text": "Right" } },
                ] },
            ] },
        ] },
    ])
}

#[test]
fn columns_round_trip_through_the_canonical_writer() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let note = json!({ "fields": { "title": "Laid out" },
                       "blocks": [{ "kind": "content", "labels": ["body"], "blocks": body() }] });
    let saved = w.create("page", &Input::Json(note), &author).unwrap();
    assert!(saved.wcl.ends_with(CANONICAL), "{}", saved.wcl);

    // Read back, the blocks are the ones sent; written again, the file is the same bytes.
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
fn a_columns_block_of_the_wrong_shape_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let one = json!({ "fields": { "title": "One" }, "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
        { "kind": "columns", "blocks": [{ "kind": "column", "fields": { "width": 1 } }] },
    ] }] });
    let err = w.create("page", &Input::Json(one), &author).unwrap_err();
    assert!(
        err.to_string().contains("needs 2 to 6 columns, not 1"),
        "{err}"
    );
}
