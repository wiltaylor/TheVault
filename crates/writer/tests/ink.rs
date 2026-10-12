//! The writer rounds ink stroke coordinates to two decimal places and pressure to three, as
//! `docs/vault-format.md` "Ink" says, wherever a stroke sits: an ink field, an ink content block
//! or an ink canvas node.

use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const SKETCH: &str = r#"label = "Sketch"

field drawing {
  kind = "ink"
}

field body {
  kind = "content"
}

field board {
  kind = "canvas"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/sketch.wcl"), SKETCH).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn stroke(points: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "kind": "stroke",
        "fields": { "tool": "pen", "colour": "#1f2937", "size": 4.0, "points": points }
    })
}

fn ink(label: Option<&str>, points: serde_json::Value) -> serde_json::Value {
    let mut b = serde_json::json!({
        "kind": "ink",
        "fields": { "width": 800, "height": 300 },
        "blocks": [stroke(points)]
    });
    if let Some(label) = label {
        b["labels"] = serde_json::json!([label]);
    }
    b
}

/// The `points = [...]` lines of a note file, in file order.
fn points(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("points"))
        .map(|l| l.split_once('=').unwrap().1.trim().to_owned())
        .collect()
}

#[test]
fn create_and_update_round_every_stroke() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let raw = serde_json::json!([10.123456, 20.006, 0.123456, 14.999, 22.0, 0.6666666]);
    let rounded = "[10.12, 20.01, 0.123, 15.0, 22.0, 0.667]";
    let canvas_node = serde_json::json!({
        "kind": "node", "labels": ["sketch"],
        "fields": { "kind": "ink", "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0 },
        "blocks": [stroke(raw.clone())]
    });
    let json = serde_json::json!({
        "fields": { "title": "Drawn" },
        "blocks": [
            ink(Some("drawing"), raw.clone()),
            { "kind": "content", "labels": ["body"], "blocks": [ink(None, raw.clone())] },
            { "kind": "canvas", "labels": ["board"], "blocks": [
                { "kind": "viewport", "fields": { "x": 0.0, "y": 0.0, "zoom": 1.0 } },
                canvas_node
            ] }
        ]
    });
    let saved = w.create("sketch", &Input::Json(json), &author()).unwrap();
    let path = dir.path().join(format!("notes/sketch/{}.wcl", saved.id));
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(points(&text), vec![rounded; 3], "{text}");

    let edited = text.replace(rounded, "[1.23456, 2.0, 0.98765]");
    w.update(
        &saved.id,
        &Input::Wcl(edited),
        Some(&saved.version),
        &author(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(points(&text)[0], "[1.23, 2.0, 0.988]", "{text}");
}
