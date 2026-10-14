//! An `image` block in a template that shows an asset field of the note (`field`) in place of a
//! fixed `asset`: checked when a type or template is written or a change is planned, refused in a
//! note's own content, and written byte for byte as sent.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use state::{Author, State};
use store::Vault;
use writer::{Input, TypeChange, Writer};

const ALBUM: &str = r#"label = "Album"

field cover {
  kind = "asset"
}

field photos {
  kind     = "asset"
  multiple = true
}

field artist {
  kind = "string"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    image {
      field   = "cover"
      alt     = "Cover of {{title}}"
      caption = "By {{artist}}"
      width   = 320
    }
    columns {
      column {
        image {
          field = "photos"
        }
      }
      column {
        image {
          asset = "01JB9A0C3D4E5F6G7H8J9K0M1N"
        }
      }
    }
    slot "body"
  }
}

template card {
  tab "Card" {
    image {
      field = "cover"
    }
    slot "artist"
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/album.wcl"), ALBUM).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

/// `ALBUM` with its default template's first image block replaced by `image`.
fn with_image(image: &str) -> String {
    let start = ALBUM
        .find("    image {\n      field   = \"cover\"")
        .unwrap();
    let end = start + ALBUM[start..].find("    }\n").unwrap() + "    }\n".len();
    format!("{}{image}{}", &ALBUM[..start], &ALBUM[end..])
}

#[test]
fn a_type_with_images_from_fields_round_trips_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    assert_eq!(w.put_type("album", ALBUM).unwrap(), ALBUM);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("types/album.wcl")).unwrap(),
        ALBUM
    );
    let card = "template card {\n  tab \"Card\" {\n    image {\n      field = \"photos\"\n      alt   = \"{{artist}}\"\n    }\n  }\n}\n";
    assert_eq!(w.put_template("album", "card", card).unwrap(), card);
}

#[test]
fn an_image_takes_an_asset_or_a_field_of_the_right_kind() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    for (image, reason) in [
        (
            "    image {\n      field = \"cover\"\n      asset = \"01JB9A0C3D4E5F6G7H8J9K0M1N\"\n    }\n",
            "takes `asset` or `field`, not both",
        ),
        (
            "    image {\n      alt = \"Nothing\"\n    }\n",
            "needs `asset` or `field`",
        ),
        (
            "    image {\n      field = \"sleeve\"\n    }\n",
            "image#1.field: \"sleeve\" names no field of the type",
        ),
        (
            "    image {\n      field = \"artist\"\n    }\n",
            "image#1.field: an image shows an asset field, and \"artist\" is a string field",
        ),
        (
            "    image {\n      field   = \"cover\"\n      caption = \"{{label}}\"\n    }\n",
            "image#1.caption: `{{label}}` names no field of the type",
        ),
        (
            "    image {\n      field = \"cover\"\n      alt   = \"{{nope}}\"\n    }\n",
            "image#1.alt: `{{nope}}` names no field of the type",
        ),
    ] {
        let source = with_image(image);
        let err = w.put_type("album", &source).unwrap_err();
        assert!(err.to_string().contains(reason), "{reason}: {err}");

        // A planned change and a template on its own are checked the same way.
        let change = TypeChange {
            wcl: source.clone(),
            renames: BTreeMap::new(),
            migration: None,
        };
        let err = w.plan_type_change("album", &change).unwrap_err();
        assert!(err.to_string().contains(reason), "plan: {reason}: {err}");
        let template = format!("template card {{\n  tab \"Card\" {{\n{image}  }}\n}}\n");
        let err = w.put_template("album", "card", &template).unwrap_err();
        assert!(
            err.to_string().contains(reason),
            "template: {reason}: {err}"
        );
    }
}

#[test]
fn a_note_image_cannot_name_a_field() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let note = json!({ "fields": { "title": "Bad" },
                       "blocks": [{ "kind": "content", "labels": ["body"],
                                    "blocks": [{ "kind": "image", "fields": { "field": "cover" } }] }] });
    let err = w.create("album", &Input::Json(note), &author).unwrap_err();
    assert!(
        err.to_string()
            .contains("image#1.field: is allowed only in a template"),
        "{err}"
    );

    // A note's image still needs its asset.
    let note = json!({ "fields": { "title": "Bad" },
                       "blocks": [{ "kind": "content", "labels": ["body"],
                                    "blocks": [{ "kind": "image", "fields": { "alt": "x" } }] }] });
    let err = w.create("album", &Input::Json(note), &author).unwrap_err();
    assert!(
        err.to_string().contains("image#1.asset: is required"),
        "{err}"
    );
}
