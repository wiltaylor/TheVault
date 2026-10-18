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

#[path = "../../index/tests/support/hostile_pdf.rs"]
mod hostile_pdf;

/// The child half of [`a_pdf_that_panics_the_extractor_is_logged_once_and_not_retried`]: one
/// server start over the vault at `PDF_CHILD_ROOT`. The first start uploads the PDFs and names
/// them in notes; every start then rebuilds the index, as a start with no index does.
#[test]
fn hostile_pdf_child() {
    let Some(root) = std::env::var_os("PDF_CHILD_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    if !root.join("types/doc.wcl").exists() {
        std::fs::create_dir_all(root.join("types")).unwrap();
        std::fs::write(root.join("types/doc.wcl"), DOC).unwrap();
        let w = open(root);
        for (name, bytes) in [
            (
                "unreadable.pdf",
                hostile_pdf::font_without_encoding_or_text(),
            ),
            ("colour.pdf", hostile_pdf::colour_space_by_reference()),
            ("font.pdf", hostile_pdf::font_without_encoding()),
        ] {
            let asset = w.put_asset(name, bytes).unwrap();
            w.create(
                "doc",
                &Input::Wcl(format!("title = \"{name}\"\nfile = \"{}\"\n", asset.id)),
                &author(),
            )
            .unwrap();
            println!("ASSET {name} {}", asset.id);
        }
    }
    std::fs::remove_dir_all(root.join(".vault/search")).unwrap();
    drop(open(root));
}

#[test]
fn a_pdf_that_panics_the_extractor_is_logged_once_and_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let exe = std::env::current_exe().unwrap();
    let run = || {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "hostile_pdf_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("PDF_CHILD_ROOT", &root)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        (
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    let (first_out, first_err) = run();
    let (_, again_err) = run();
    let stderr = format!("{first_err}{again_err}");
    assert!(!stderr.contains("thread '"), "{stderr}");
    assert!(!stderr.contains("RUST_BACKTRACE"), "{stderr}");

    let id_of = |name: &str| {
        first_out
            .lines()
            .find_map(|l| l.split_once(&format!("ASSET {name} ")).map(|(_, id)| id))
            .unwrap_or_else(|| panic!("no {name} in {first_out}"))
            .to_owned()
    };
    // Each PDF that panicked pdf-extract is logged once, in one line naming the asset and why,
    // over both starts: neither start after the first tries it again.
    for (name, reason) in [
        ("unreadable.pdf", "missing unicode map and encoding"),
        ("colour.pdf", "second arg must be a dict"),
        ("font.pdf", "missing unicode map and encoding"),
    ] {
        let id = id_of(name);
        let lines: Vec<&str> = stderr.lines().filter(|l| l.contains(&id)).collect();
        assert_eq!(lines.len(), 1, "{name}: {stderr}");
        assert!(lines[0].contains(reason), "{name}: {}", lines[0]);
    }

    // The unreadable one is kept as having no text, with its reason beside it.
    let unreadable = id_of("unreadable.pdf");
    let text_dir = root.join(".vault/asset-text/v1");
    assert_eq!(
        std::fs::read(text_dir.join(format!("{unreadable}.txt")))
            .unwrap()
            .len(),
        0
    );
    let why = std::fs::read_to_string(text_dir.join(format!("{unreadable}.unreadable"))).unwrap();
    assert!(why.contains("missing unicode map and encoding"), "{why}");

    // The fallback's text is searchable in the notes naming those PDFs.
    let w = open(&root);
    assert_eq!(searched(&w, "marmalade").len(), 1);
    assert_eq!(searched(&w, "wombat").len(), 1);
    // Deleting an unreadable asset takes its reason with it.
    w.delete_asset(&unreadable, true, &author()).unwrap();
    assert!(!text_dir.join(format!("{unreadable}.unreadable")).exists());
}
