//! `control` blocks through the writer: a type whose template lays out a form, and a note whose
//! own content holds controls, written canonically and refused at write time when a control
//! names no field or does not fit it.

use std::path::Path;

use serde_json::json;
use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const FORM: &str = r#"label = "Form"

field done {
  kind = "bool"
}

field stars {
  kind = "int"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    columns {
      column {
        control {
          field = "done"
          kind  = "toggle"
          label = "Finished"
        }
      }
      column {
        control {
          field = "stars"
          kind  = "rating"
          max   = 3
        }
      }
    }
    slot "body"
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/form.wcl"), FORM).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

#[test]
fn a_note_with_controls_in_its_content_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let body = json!([
        { "kind": "control", "fields": { "field": "stars", "kind": "slider", "min": 0, "max": 10, "step": 1 } },
    ]);
    let note = json!({ "fields": { "title": "Filled", "stars": 2 },
                       "blocks": [{ "kind": "content", "labels": ["body"], "blocks": body }] });
    let saved = w.create("form", &Input::Json(note), &author).unwrap();
    assert!(
        saved.wcl.contains(
            "content body {\n  control {\n    field = \"stars\"\n    kind  = \"slider\"\n    max   = 10\n    min   = 0\n    step  = 1\n  }\n}\n"
        ),
        "{}",
        saved.wcl
    );
    let read = w.read(&saved.id).unwrap();
    assert_eq!(writer::doc_to_json(&read.doc)["blocks"][0]["blocks"], body);
}

#[test]
fn a_control_that_does_not_fit_its_field_is_refused_at_write_time() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    for (control, reason) in [
        (
            json!({ "field": "done", "kind": "rating" }),
            "a rating control fits an int field, and \"done\" is a bool field",
        ),
        (
            json!({ "field": "gone", "kind": "toggle" }),
            "the type has no field \"gone\"",
        ),
        (
            json!({ "field": "stars", "kind": "slider" }),
            "control#1.min: is required by a slider control",
        ),
    ] {
        let note = json!({ "fields": { "title": "Bad" },
                           "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [{ "kind": "control", "fields": control }] }] });
        let err = w.create("form", &Input::Json(note), &author).unwrap_err();
        assert!(err.to_string().contains(reason), "{err}");
    }
}
