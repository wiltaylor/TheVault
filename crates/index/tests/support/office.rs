//! Small Word documents and spreadsheets built in code with the zip crate, so no binary
//! fixture is needed: a docx with paragraphs, a table and a footnote, an xlsx with two sheets
//! holding text, numbers and a date, and zips that unzip past the extractors' cap.

#![allow(dead_code)]

use std::io::{Cursor, Write};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// A zip of `parts`, each deflated.
pub fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in parts {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// A docx whose body is `body`, WordprocessingML inside `w:body`.
pub fn docx(body: &str) -> Vec<u8> {
    docx_with(body, &[])
}

/// A docx whose body is `body`, with the further `parts`.
pub fn docx_with(body: &str, parts: &[(&str, &str)]) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#
    );
    let mut all = vec![
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        ("word/document.xml", document.as_str()),
    ];
    all.extend_from_slice(parts);
    zip_of(&all)
}

/// A paragraph of `text`.
pub fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

/// A table of `rows`.
pub fn table(rows: &[&[&str]]) -> String {
    let mut out = String::from("<w:tbl>");
    for row in rows {
        out.push_str("<w:tr>");
        for cell in *row {
            out.push_str(&format!("<w:tc>{}</w:tc>", p(cell)));
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

/// A docx with a heading, a paragraph, a table, a header and a footnote. Its words found
/// nowhere else: `bramblefinch` in a paragraph, `quillwort` in a table cell.
pub fn sample_docx() -> Vec<u8> {
    let body = [
        p("Quarterly report"),
        p("The bramblefinch budget grew."),
        table(&[&["Item", "Cost"], &["Quillwort seed", "12"]]),
        p("Signed off."),
    ]
    .concat();
    let header = format!(r#"<w:hdr xmlns:w="{W}">{}</w:hdr>"#, p("Acme Ltd"));
    let notes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1">{}</w:footnote></w:footnotes>"#,
        p("Figures are estimates.")
    );
    docx_with(
        &body,
        &[
            ("word/header1.xml", &header),
            ("word/footnotes.xml", &notes),
        ],
    )
}

/// The text [`sample_docx`] extracts to.
pub const SAMPLE_DOCX_TEXT: &str = "Acme Ltd\nQuarterly report\nThe bramblefinch budget grew.\nItem | Cost\nQuillwort seed | 12\nSigned off.\nFigures are estimates.";

/// An xlsx of `sheets`, each a name and the `<row>` elements of its `sheetData`. Style 1 is a
/// date (number format 14).
pub fn xlsx(sheets: &[(&str, &str)]) -> Vec<u8> {
    let mut book = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut files = Vec::new();
    for (i, (name, rows)) in sheets.iter().enumerate() {
        let n = i + 1;
        book.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
        files.push((
            format!("xl/worksheets/sheet{n}.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
            ),
        ));
    }
    book.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/></cellXfs></styleSheet>"#;
    let types = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;
    let package = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let mut parts: Vec<(&str, &str)> = vec![
        ("[Content_Types].xml", types),
        ("_rels/.rels", package),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/styles.xml", styles),
    ];
    for (name, body) in &files {
        parts.push((name, body));
    }
    zip_of(&parts)
}

/// An inline string cell.
pub fn s(r: &str, text: &str) -> String {
    format!(r#"<c r="{r}" t="inlineStr"><is><t>{text}</t></is></c>"#)
}

/// A number cell, of style `style`.
pub fn n(r: &str, value: &str, style: u32) -> String {
    format!(r#"<c r="{r}" s="{style}"><v>{value}</v></c>"#)
}

/// A workbook of two sheets: `Budget`, with a header row, a number, a date and a cell holding a
/// comma and quotes, and `Notes`, with the word `marshwiggle` found nowhere else.
pub fn sample_xlsx() -> Vec<u8> {
    let budget = [
        format!(
            "<row r=\"1\">{}{}{}</row>",
            s("A1", "Item"),
            s("B1", "Cost"),
            s("C1", "Due")
        ),
        format!(
            "<row r=\"2\">{}{}{}</row>",
            s("A2", "Seed, &quot;best&quot;"),
            n("B2", "12.5", 0),
            n("C2", "45383", 1)
        ),
        format!("<row r=\"3\">{}{}</row>", s("A3", "Pots"), n("B3", "3", 0)),
    ]
    .concat();
    let notes = format!("<row r=\"2\">{}</row>", s("B2", "Ask about marshwiggle"));
    xlsx(&[("Budget", &budget), ("Notes", &notes)])
}

/// The text [`sample_xlsx`] extracts to.
pub const SAMPLE_XLSX_TEXT: &str = "# Sheet: Budget\nItem,Cost,Due\n\"Seed, \"\"best\"\"\",12.5,2024-04-01\nPots,3,\n# Sheet: Notes\nAsk about marshwiggle\n";

/// A zip holding `name`, `size` bytes of spaces that deflate to almost nothing, after `parts`.
pub fn zip_bomb(parts: &[(&str, &str)], name: &str, size: usize) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (part, body) in parts {
        zip.start_file(*part, SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.start_file(name, SimpleFileOptions::default()).unwrap();
    let chunk = vec![b' '; 1024 * 1024];
    let mut left = size;
    while left > 0 {
        let n = left.min(chunk.len());
        zip.write_all(&chunk[..n]).unwrap();
        left -= n;
    }
    zip.finish().unwrap().into_inner()
}

/// An OpenDocument spreadsheet of one sheet, `Plants`, with a text cell, a number and a date.
pub fn sample_ods() -> Vec<u8> {
    let content = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet><table:table table:name="Plants"><table:table-row><table:table-cell office:value-type="string"><text:p>Fernhollow</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="4"><text:p>4</text:p></table:table-cell><table:table-cell office:value-type="date" office:date-value="2024-04-01"><text:p>01/04/24</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
    zip_of(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content),
        (
            "META-INF/manifest.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
        ),
    ])
}
