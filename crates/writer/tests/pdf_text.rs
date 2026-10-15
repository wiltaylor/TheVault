//! The text inside an uploaded PDF is extracted and searched as part of every note that names
//! it, in a field or in an `asset` block, and a rebuild extracts it again when it has gone.

use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const DOC: &str = r#"label = "Doc"

field file {
  kind = "asset"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "file"
    slot "body"
  }
}
"#;

fn pdf() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../index/tests/fixtures/three-pages.pdf"
    ))
    .unwrap()
}

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn searched(w: &Writer, text: &str) -> Vec<String> {
    let mut ids: Vec<String> = w.read_index(|index, _| {
        index
            .search(text, None)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect()
    });
    ids.sort();
    ids
}

#[test]
fn a_pdf_s_words_find_the_notes_that_name_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/doc.wcl"), DOC).unwrap();
    let w = open(root);

    let asset = w.put_asset("sample.pdf", pdf()).unwrap();
    let in_field = w
        .create(
            "doc",
            &Input::Wcl(format!("title = \"Attached\"\nfile = \"{}\"\n", asset.id)),
            &author(),
        )
        .unwrap()
        .id;
    let in_block = w
        .create(
            "doc",
            &Input::Wcl(format!(
                "title = \"Embedded\"\n\ncontent body {{\n  asset {{\n    asset = \"{}\"\n  }}\n}}\n",
                asset.id
            )),
            &author(),
        )
        .unwrap()
        .id;
    let plain = w
        .create("doc", &Input::Wcl("title = \"Plain\"\n".into()), &author())
        .unwrap()
        .id;

    let mut both = vec![in_field.clone(), in_block.clone()];
    both.sort();
    assert_eq!(searched(&w, "zephyrine"), both);
    assert!(!searched(&w, "zephyrine").contains(&plain));

    // A corrupt PDF is stored all the same, and has no text.
    let corrupt = w
        .put_asset("broken.pdf", b"%PDF-1.7\nnot really a pdf".to_vec())
        .unwrap();
    assert_eq!(w.asset(&corrupt.id).unwrap().1.len(), 25);

    // A rebuild with the stored text gone extracts it again.
    drop(w);
    std::fs::remove_dir_all(root.join(".vault/search")).unwrap();
    std::fs::remove_dir_all(root.join(".vault/asset-text")).unwrap();
    let w = open(root);
    assert_eq!(searched(&w, "zephyrine"), both);
    assert!(
        root.join(format!(".vault/asset-text/v1/{}.txt", asset.id))
            .is_file()
    );
    // The corrupt one is kept as having no text, so it is not tried again.
    let kept = root.join(format!(".vault/asset-text/v1/{}.txt", corrupt.id));
    assert_eq!(std::fs::read(kept).unwrap().len(), 0);

    // Deleting the asset takes its text out of the notes it is unlinked from.
    w.delete_asset(&asset.id, true, &author()).unwrap();
    assert!(searched(&w, "zephyrine").is_empty());
}
