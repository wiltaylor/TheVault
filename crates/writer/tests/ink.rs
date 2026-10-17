//! The writer rounds ink stroke coordinates to two decimal places and pressure to three, as
//! `docs/vault-format.md` "Ink" says, wherever a stroke sits: an ink field, an ink content block
//! or an ink block inside a canvas card.

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
        "fields": { "kind": "card", "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0 },
        "blocks": [ink(None, raw.clone())]
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

/// A resized ink drawing and canvas keep their `height` items: the JSON a client sends is written
/// as is, and writing the file's own text back leaves it byte for byte the same.
#[test]
fn ink_and_canvas_heights_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let mut drawing = ink(Some("drawing"), serde_json::json!([10.0, 3900.0, 0.5]));
    drawing["fields"]["height"] = serde_json::json!(4000);
    let mut inline = ink(None, serde_json::json!([1.0, 2.0, 0.5]));
    inline["fields"]["height"] = serde_json::json!(120);
    let json = serde_json::json!({
        "fields": { "title": "Tall" },
        "blocks": [
            drawing,
            { "kind": "content", "labels": ["body"], "blocks": [inline] },
            { "kind": "canvas", "labels": ["board"], "fields": { "height": 900 }, "blocks": [
                { "kind": "viewport", "fields": { "x": 0.0, "y": 0.0, "zoom": 1.0 } }
            ] }
        ]
    });
    let saved = w.create("sketch", &Input::Json(json), &author()).unwrap();
    let path = dir.path().join(format!("notes/sketch/{}.wcl", saved.id));
    let text = std::fs::read_to_string(&path).unwrap();
    let heights: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("height"))
        .collect();
    assert_eq!(
        heights,
        ["height = 4000", "height = 120", "height = 900"],
        "{text}"
    );

    let again = w.update(
        &saved.id,
        &Input::Wcl(text.clone()),
        Some(&saved.version),
        &author(),
    );
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

    // Past the bound is refused, and the file stays as it was.
    let too_tall = text.replace("height = 4000", "height = 4001");
    let version = again.unwrap().version;
    let refused = w.update(&saved.id, &Input::Wcl(too_tall), Some(&version), &author());
    assert!(refused.is_err(), "4001 was accepted");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// An endless drawing's saved view, and strokes far from the origin with negative coordinates,
/// are written as the client sends them and read back byte for byte.
#[test]
fn ink_viewport_and_negative_coordinates_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let mut drawing = ink(
        Some("drawing"),
        serde_json::json!([-1500.25, -320.5, 0.5, 4200.0, -7.0, 0.75]),
    );
    drawing["fields"] = serde_json::json!({ "height": 300 });
    drawing["blocks"].as_array_mut().unwrap().insert(
        0,
        serde_json::json!({ "kind": "viewport", "fields": { "x": -1620.5, "y": -400.0, "zoom": 0.375 } }),
    );
    let json = serde_json::json!({ "fields": { "title": "Far" }, "blocks": [drawing] });
    let saved = w.create("sketch", &Input::Json(json), &author()).unwrap();
    let path = dir.path().join(format!("notes/sketch/{}.wcl", saved.id));
    let text = std::fs::read_to_string(&path).unwrap();
    let squeezed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        squeezed.contains("viewport { x = -1620.5 y = -400.0 zoom = 0.375 }"),
        "{text}"
    );
    assert_eq!(
        points(&text),
        vec!["[-1500.25, -320.5, 0.5, 4200.0, -7.0, 0.75]"],
        "{text}"
    );
    // The viewport stays ahead of the strokes, as it was sent.
    assert!(text.find("viewport {").unwrap() < text.find("stroke {").unwrap());

    let again = w.update(
        &saved.id,
        &Input::Wcl(text.clone()),
        Some(&saved.version),
        &author(),
    );
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// The default ink is stored as the token `ink`, next to strokes in a hex colour, and both are
/// written as sent and read back byte for byte.
#[test]
fn the_default_ink_token_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let mut drawing = ink(Some("drawing"), serde_json::json!([1.0, 2.0, 0.5]));
    drawing["fields"] = serde_json::json!({ "height": 300 });
    drawing["blocks"][0]["fields"]["colour"] = serde_json::json!("ink");
    let mut red = stroke(serde_json::json!([3.0, 4.0, 0.5]));
    red["fields"]["colour"] = serde_json::json!("#dc2626");
    drawing["blocks"].as_array_mut().unwrap().push(red);
    let json = serde_json::json!({ "fields": { "title": "Inked" }, "blocks": [drawing] });
    let saved = w.create("sketch", &Input::Json(json), &author()).unwrap();
    let path = dir.path().join(format!("notes/sketch/{}.wcl", saved.id));
    let text = std::fs::read_to_string(&path).unwrap();
    let colours: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("colour"))
        .collect();
    assert_eq!(
        colours,
        ["colour = \"ink\"", "colour = \"#dc2626\""],
        "{text}"
    );

    let again = w.update(
        &saved.id,
        &Input::Wcl(text.clone()),
        Some(&saved.version),
        &author(),
    );
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}
