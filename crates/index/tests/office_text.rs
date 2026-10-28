//! The text of Word documents, spreadsheets and CSV files: what each extracts to, and that a
//! corrupt file or a zip bomb is kept as unreadable.

use index::{Extracted, MAX_UNZIPPED_BYTES, TextKind, extract_text};

use crate::support::{office, when_free};

fn text(kind: TextKind, bytes: Vec<u8>) -> String {
    match when_free(|| extract_text(kind, bytes.clone())) {
        Extracted::Text(t) => t,
        other => panic!("no text: {other:?}"),
    }
}

fn failed(kind: TextKind, bytes: Vec<u8>) -> String {
    match when_free(|| extract_text(kind, bytes.clone())) {
        Extracted::Failed(reason) => reason,
        other => panic!("not failed: {other:?}"),
    }
}

#[test]
fn kinds_come_from_the_extension() {
    assert_eq!(TextKind::of("a.PDF"), Some(TextKind::Pdf));
    assert_eq!(TextKind::of("Plan.docx"), Some(TextKind::Document));
    for name in ["b.xlsx", "c.XLS", "d.ods"] {
        assert_eq!(TextKind::of(name), Some(TextKind::Spreadsheet), "{name}");
    }
    assert_eq!(TextKind::of("e.csv"), Some(TextKind::Csv));
    for name in ["old.doc", "notes.txt", "noext", "x.odt"] {
        assert_eq!(TextKind::of(name), None, "{name}");
    }
}

#[test]
fn a_docx_is_its_paragraphs_and_table_rows_in_order() {
    assert_eq!(
        text(TextKind::Document, office::sample_docx()),
        office::SAMPLE_DOCX_TEXT
    );
}

#[test]
fn an_xlsx_is_each_sheet_as_csv() {
    assert_eq!(
        text(TextKind::Spreadsheet, office::sample_xlsx()),
        office::SAMPLE_XLSX_TEXT
    );
}

#[test]
fn an_xls_reads_its_dates_and_durations() {
    // tests/date.xls from calamine (MIT), a BIFF8 workbook.
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dates.xls"
    ))
    .unwrap();
    let got = text(TextKind::Spreadsheet, bytes);
    assert!(
        got.starts_with("# Sheet: Sheet1\n2021-01-01,15\n2021-01-02,16\n"),
        "{got}"
    );
    assert!(got.contains("\n255:10:10,17\n"), "{got}");
}

#[test]
fn an_ods_reads_as_an_xlsx_does() {
    assert_eq!(
        text(TextKind::Spreadsheet, office::sample_ods()),
        "# Sheet: Plants\nFernhollow,4,2024-04-01\n"
    );
}

#[test]
fn a_csv_is_the_file_itself() {
    let csv = "name,quote\n\"Smith, J\",\"He said \"\"hi\"\"\"\n";
    assert_eq!(text(TextKind::Csv, csv.as_bytes().to_vec()), csv);
}

#[test]
fn a_corrupt_or_mislabelled_file_is_unreadable() {
    let r = failed(TextKind::Document, b"PK\x03\x04 not really a zip".to_vec());
    assert!(r.starts_with("not a zip"), "{r}");
    let r = failed(TextKind::Document, b"plain text".to_vec());
    assert_eq!(r, "not a Word document: no zip header");
    let ole = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0].to_vec();
    let r = failed(TextKind::Document, ole.clone());
    assert!(r.contains("OLE file"), "{r}");
    let r = failed(TextKind::Document, office::zip_of(&[("readme.txt", "hi")]));
    assert!(r.contains("no word/document.xml"), "{r}");
    let r = failed(
        TextKind::Document,
        office::docx("<w:p><w:r><w:t>unclosed</w:r></w:p>"),
    );
    assert!(r.starts_with("word/document.xml:"), "{r}");

    let r = failed(TextKind::Spreadsheet, b"a,b\n1,2\n".to_vec());
    assert_eq!(r, "not a spreadsheet: no zip or OLE header");
    let r = failed(TextKind::Spreadsheet, ole);
    assert!(r.starts_with("not a readable spreadsheet"), "{r}");
    let r = failed(TextKind::Spreadsheet, office::zip_of(&[("a.txt", "hi")]));
    assert!(r.contains("_rels/.rels"), "{r}");
    assert!(r.starts_with("not a readable spreadsheet"), "{r}");
}

#[test]
fn a_zip_bomb_is_not_unpacked_past_the_cap() {
    let size = MAX_UNZIPPED_BYTES as usize + 1;
    let bomb = office::zip_bomb(&[], "word/document.xml", size);
    assert!(bomb.len() < 1024 * 1024, "{}", bomb.len());
    let r = failed(TextKind::Document, bomb);
    assert!(r.contains("unzips to more than"), "{r}");

    let sheet = office::xlsx(&[("One", "")]);
    let mut parts = Vec::new();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(sheet)).unwrap();
    for i in 0..zip.len() {
        use std::io::Read;
        let mut f = zip.by_index(i).unwrap();
        let mut body = String::new();
        f.read_to_string(&mut body).unwrap();
        parts.push((f.name().unwrap().into_owned(), body));
    }
    let parts: Vec<(&str, &str)> = parts
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_str()))
        .collect();
    let bomb = office::zip_bomb(&parts, "xl/sharedStrings.xml", size);
    let r = failed(TextKind::Spreadsheet, bomb);
    assert!(r.contains("unzips to more than"), "{r}");
}
