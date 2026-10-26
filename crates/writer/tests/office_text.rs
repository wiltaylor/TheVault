//! The text inside an uploaded Word document, spreadsheet or CSV file is extracted and searched
//! as part of every note that names it, as a PDF's is, and a rebuild extracts it again when it
//! has gone, or for the first time for an asset uploaded before its kind was read.

use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Input, StoredText, Writer};

#[path = "../../index/tests/support/office.rs"]
mod office;

const DOC: &str = r#"label = "Doc"

field file {
  kind = "asset"
}

template default {
  tab "Main" {
    slot "title"
    slot "file"
  }
}
"#;

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

fn note(w: &Writer, title: &str, asset: &str) -> String {
    w.create(
        "doc",
        &Input::Wcl(format!("title = \"{title}\"\nfile = \"{asset}\"\n")),
        &author(),
    )
    .unwrap()
    .id
}

#[test]
fn words_inside_documents_and_spreadsheets_find_the_notes_that_name_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/doc.wcl"), DOC).unwrap();
    let w = open(root);

    let docx = w.put_asset("Report.docx", office::sample_docx()).unwrap();
    let xlsx = w.put_asset("Budget.xlsx", office::sample_xlsx()).unwrap();
    let csv = w
        .put_asset(
            "people.csv",
            b"name,town\nAda,\"Lower Puddlecombe\"\n".to_vec(),
        )
        .unwrap();
    let broken = w
        .put_asset("broken.xlsx", b"PK\x03\x04 broken".to_vec())
        .unwrap();
    let in_docx = note(&w, "Report", &docx.id);
    let in_xlsx = note(&w, "Budget", &xlsx.id);
    let in_csv = note(&w, "People", &csv.id);
    note(&w, "Broken", &broken.id);

    assert_eq!(searched(&w, "quillwort"), std::slice::from_ref(&in_docx));
    assert_eq!(searched(&w, "bramblefinch"), std::slice::from_ref(&in_docx));
    assert_eq!(searched(&w, "marshwiggle"), std::slice::from_ref(&in_xlsx));
    assert_eq!(searched(&w, "puddlecombe"), std::slice::from_ref(&in_csv));
    assert_eq!(
        w.asset_text(&xlsx.id),
        StoredText::Text(office::SAMPLE_XLSX_TEXT.to_owned())
    );
    assert!(w.asset_text_holds(&docx.id, "Quillwort Seed"));
    assert!(
        matches!(w.asset_text(&broken.id), StoredText::Unreadable(r) if r.starts_with("not a zip"))
    );

    // A rebuild with the stored text gone extracts it again, as it does for assets stored
    // before their kind's text was read.
    drop(w);
    std::fs::remove_dir_all(root.join(".vault/search")).unwrap();
    std::fs::remove_dir_all(root.join(".vault/asset-text")).unwrap();
    let w = open(root);
    assert_eq!(searched(&w, "quillwort"), [in_docx]);
    assert_eq!(searched(&w, "marshwiggle"), [in_xlsx]);
    assert_eq!(searched(&w, "puddlecombe"), [in_csv]);
    let text_dir = root.join(".vault/asset-text/v1");
    assert!(text_dir.join(format!("{}.unreadable", broken.id)).is_file());

    // Deleting the asset takes its text out of the notes it is unlinked from.
    w.delete_asset(&xlsx.id, true, &author()).unwrap();
    assert!(searched(&w, "marshwiggle").is_empty());
}
