//! A note with `table` blocks: written in canonical form from the JSON form the notes API and
//! scripts send, with short rows padded to the widest and `align` cut to the columns there are,
//! read back as the same blocks, the same bytes on a second write, and refused when wrong.

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

// The rows of a table follow its items line after line, as a template tab's items do.
const CANONICAL: &str = r#"content body {
  table {
    align  = ["left", "right", "center"]
    header = true
    row {
      cells = ["Item", "Qty", "Notes"]
    }
    row {
      cells = ["Apples", "3", "**ripe**, a \"good\" | batch"]
    }
    row {
      cells = ["Pears", "", ""]
    }
  }

  table {
    row {
      cells = ["Only"]
    }
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
    json!({ "fields": { "title": "Tabled" },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": blocks }] })
}

#[test]
fn tables_are_written_square_and_round_trip_through_json_and_the_canonical_writer() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let sent = json!([
        { "kind": "table", "fields": { "header": true, "align": ["left", "right", "center", "right"] }, "blocks": [
            { "kind": "row", "fields": { "cells": ["Item", "Qty", "Notes"] } },
            { "kind": "row", "fields": { "cells": ["Apples", "3", "**ripe**, a \"good\" | batch"] } },
            { "kind": "row", "fields": { "cells": ["Pears"] } },
        ] },
        { "kind": "table", "blocks": [{ "kind": "row", "fields": { "cells": ["Only"] } }] },
    ]);
    let saved = w.create("page", &Input::Json(note(sent)), &author).unwrap();
    assert!(saved.wcl.ends_with(CANONICAL), "{}", saved.wcl);

    let read = w.read(&saved.id).unwrap();
    let back = writer::doc_to_json(&read.doc);
    assert_eq!(
        back["blocks"][0]["blocks"][0]["blocks"][2],
        json!({ "kind": "row", "fields": { "cells": ["Pears", "", ""] } })
    );
    assert_eq!(
        back["blocks"][0]["blocks"][0]["fields"]["align"],
        json!(["left", "right", "center"])
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
fn a_wrong_table_is_refused_with_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    for (block, want) in [
        (
            json!({ "kind": "table", "fields": { "header": true } }),
            "needs at least one row",
        ),
        (
            json!({ "kind": "table", "fields": { "align": ["middle"] },
                    "blocks": [{ "kind": "row", "fields": { "cells": ["a"] } }] }),
            "\"middle\" is not one of left, center, right",
        ),
        (
            json!({ "kind": "table", "blocks": [{ "kind": "row", "fields": { "cells": [1] } }] }),
            "must be a list of strings",
        ),
    ] {
        let err = w
            .create("page", &Input::Json(note(json!([block]))), &author)
            .unwrap_err();
        assert!(err.to_string().contains(want), "{want}: {err}");
    }
}
