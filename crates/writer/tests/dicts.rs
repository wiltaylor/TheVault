//! The writer puts a dict's entries in canonical order: declared keys in declaration order, then
//! the others by name (`docs/vault-format.md` "dict"), at every depth, whichever form the note
//! was submitted in.

use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const CRATE: &str = r#"label = "Crate"

field spec {
  kind   = "dict"
  values = "any"

  key weight {
    kind = "float"
  }

  key dims {
    kind = "dict"

    key w {
      kind = "float"
    }

    key h {
      kind = "float"
    }
  }

  key boxes {
    kind = "list"

    item {
      kind = "dict"

      key size {
        kind = "string"
      }

      key colour {
        kind = "string"
      }
    }
  }
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/crate.wcl"), CRATE).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

/// The text of the note's `dict spec { ... }` block.
fn spec_block(text: &str) -> String {
    let start = text.find("dict spec {").expect("a spec block");
    let end = start + text[start..].find("\n}").expect("the block ends");
    text[start..end].to_owned()
}

#[test]
fn declared_keys_come_first_in_declaration_order_at_every_depth() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let json = serde_json::json!({
        "fields": { "title": "Tools" },
        "blocks": [{
            "kind": "dict",
            "labels": ["spec"],
            "fields": {
                "zone": "B",
                "boxes": [{ "colour": "red", "size": "m" }],
                "dims": { "h": 3.0, "w": 4.0 },
                "aisle": 7,
                "weight": 12.5
            }
        }]
    });
    let saved = w
        .create("crate", &Input::Json(json), &Author::Session("test".into()))
        .unwrap();
    let text =
        std::fs::read_to_string(dir.path().join(format!("notes/crate/{}.wcl", saved.id))).unwrap();
    let block = spec_block(&text);
    let order: Vec<&str> = block
        .lines()
        .skip(1)
        .map(|l| l.trim().split(' ').next().unwrap())
        .collect();
    assert_eq!(
        order,
        ["weight", "dims", "boxes", "aisle", "zone"],
        "{block}"
    );
    assert!(block.contains("{ w: 4.0, h: 3.0 }"), "{block}");
    assert!(
        block.contains("[{ size: \"m\", colour: \"red\" }]"),
        "{block}"
    );
}
