//! A canvas written before nodes held content blocks: it loads as cards, indexes without a
//! failure, and is written in the new form the next time its note is saved.

use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const BOARD: &str = r#"label = "Board"

field board {
  kind = "canvas"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const ID: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZF";

const OLD: &str = r#"title = "Old board"

canvas board {
  viewport {
    x    = 0.0
    y    = 0.0
    zoom = 1.0
  }

  node idea {
    kind   = "text"
    x      = 0.0
    y      = 0.0
    width  = 240.0
    height = 120.0
    colour = "yellow"
    text   = "An **idea**"
  }

  node work {
    kind   = "query"
    x      = 300.0
    y      = 0.0
    width  = 240.0
    height = 300.0
    query  = "from board"
    view   = "list"
  }

  node site {
    kind   = "link"
    x      = 0.0
    y      = 200.0
    width  = 240.0
    height = 80.0
    url    = "https://example.com/docs"
  }

  edge idea_to_work {
    from  = "idea"
    to    = "work"
    arrow = "both"
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::create_dir_all(root.join("notes/board")).unwrap();
    std::fs::write(root.join("types/board.wcl"), BOARD).unwrap();
    std::fs::write(root.join(format!("notes/board/{ID}.wcl")), OLD).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

#[test]
fn an_old_canvas_loads_as_cards_and_saves_in_the_new_form() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    // The index took the note: no failure, and the old text is searchable.
    w.read_index(|index, _| {
        assert!(index.failures().unwrap().is_empty());
        assert_eq!(index.search("idea", None).unwrap()[0].id, ID);
    });

    let note = w.read(ID).unwrap();
    let json = writer::doc_to_json(&note.doc);
    let nodes = json["blocks"][0]["blocks"].as_array().unwrap();
    let kinds: Vec<_> = nodes
        .iter()
        .filter(|b| b["kind"] == "node")
        .map(|b| {
            (
                b["labels"][0].as_str().unwrap(),
                b["fields"]["kind"].as_str().unwrap(),
                b["blocks"][0]["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("idea", "card", "paragraph"),
            ("work", "card", "query_view"),
            ("site", "card", "paragraph")
        ]
    );
    assert_eq!(nodes[1]["fields"]["colour"], "yellow");
    assert_eq!(nodes[1]["blocks"][0]["fields"]["text"], "An **idea**");

    // The file is untouched until the note is saved; then it holds cards.
    let path = dir.path().join(format!("notes/board/{ID}.wcl"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), OLD);
    w.update(
        ID,
        &Input::Json(json),
        Some(&note.version),
        &Author::Session("test".into()),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("kind   = \"text\""), "{text}");
    assert!(!text.contains("url"), "{text}");
    assert_eq!(text.matches("kind   = \"card\"").count(), 3, "{text}");
    assert!(text.contains("query_view {"), "{text}");
    assert!(
        text.contains("text = \"[https://example.com/docs](https://example.com/docs)\""),
        "{text}"
    );
}
