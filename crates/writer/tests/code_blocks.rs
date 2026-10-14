//! Shell sessions as two `code` blocks: a `bash` block for the command and a `stdout` block for
//! its output. Both round-trip through the canonical writer; the old `terminal` block is refused
//! as an unknown content block kind.

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
  code {
    language = "bash"
    source   = "vault import --dry-run export/"
  }

  code {
    language = "stdout"
    source   = <<'END'
    42 notes would be created
      3 skipped
    END
  }
}
"#;

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
        { "kind": "code", "fields": { "language": "bash", "source": "vault import --dry-run export/" } },
        { "kind": "code", "fields": { "language": "stdout", "source": "42 notes would be created\n  3 skipped\n" } },
    ])
}

#[test]
fn bash_and_stdout_code_blocks_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let note = json!({ "fields": { "title": "Dry run" },
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
fn a_terminal_block_is_an_unknown_content_block() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let author = Author::Session("test".into());
    let wcl = "title = \"Old\"\n\ncontent body {\n  terminal {\n    command {\n      line = \"ls\"\n    }\n  }\n}\n";
    let err = w
        .create("page", &Input::Wcl(wcl.into()), &author)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("\"terminal\" is not a content block kind"),
        "{err}"
    );
}
