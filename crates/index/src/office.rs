//! The text of Word documents, spreadsheets and CSV files, for [`crate::asset_text`].
//!
//! - A Word document (`.docx`) is a zip of XML parts. Its headers, body, footnotes, endnotes
//!   and footers are read in that order: each paragraph is a line, and each table row a line of
//!   its cells joined by ` | `. Text in a `mc:Fallback`, which repeats what the part before it
//!   holds for older readers, is left out.
//! - A spreadsheet (`.xlsx`, `.xls`, `.ods`) is read by calamine. Each sheet is a heading line,
//!   `# Sheet: <name>`, then its rows as CSV (RFC 4180 quoting), without the empty rows and
//!   columns after the last cell holding anything. A date is `YYYY-MM-DD`, a date with a time
//!   `YYYY-MM-DDTHH:MM:SS`, a time alone `HH:MM:SS`, a duration `H:MM:SS` and a number as the
//!   shortest decimal that reads back as it.
//! - A CSV file is its own text, read as UTF-8 with any invalid bytes replaced.
//!
//! A zip is never unpacked past [`MAX_UNZIPPED_BYTES`]: a docx's parts are cut off there as
//! they are read, and a spreadsheet's are counted out before calamine opens it, its pictures
//! and thumbnails aside, as calamine does not read them.

use std::io::{Cursor, Read, Seek};

use calamine::{Data, Ods, Reader, Sheets, Xls, Xlsb, Xlsx};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use zip::ZipArchive;

/// The most bytes read out of one zip, all its parts told.
pub const MAX_UNZIPPED_BYTES: u64 = 128 * 1024 * 1024;

/// How a zip file starts.
pub(crate) const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// How an OLE compound file, such as an `.xls`, starts.
pub(crate) const OLE_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

type Out = std::result::Result<String, String>;

/// The text of the Word document `bytes`, as the module documentation lays out.
pub(crate) fn docx_text(bytes: &[u8]) -> Out {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a zip: {e}"))?;
    let mut budget = MAX_UNZIPPED_BYTES;
    let mut parts: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| part_order(n).is_some())
        .collect();
    parts.sort_by_key(|n| (part_order(n), n.clone()));
    if !parts.iter().any(|n| n == "word/document.xml") {
        return Err("not a Word document: it has no word/document.xml".to_owned());
    }
    let mut out = String::new();
    for name in parts {
        let xml = read_part(&mut zip, &name, &mut budget)?;
        let text = paragraphs(&xml).map_err(|e| format!("{name}: {e}"))?;
        if !text.trim().is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&text);
        }
        if out.len() > crate::MAX_TEXT {
            break;
        }
    }
    Ok(out)
}

/// Where the docx part `name` comes in the text, or `None` when its text is not read.
fn part_order(name: &str) -> Option<u8> {
    let file = name.strip_prefix("word/")?;
    let xml = |prefix: &str| {
        file.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(".xml"))
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
    };
    match file {
        "document.xml" => Some(1),
        "footnotes.xml" => Some(2),
        "endnotes.xml" => Some(3),
        _ if xml("header") => Some(0),
        _ if xml("footer") => Some(4),
        _ => None,
    }
}

/// The bytes of zip entry `name`, taken from `budget`.
fn read_part<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    budget: &mut u64,
) -> std::result::Result<Vec<u8>, String> {
    let entry = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut buf = Vec::new();
    entry
        .take(*budget + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("{name}: {e}"))?;
    take_budget(budget, buf.len() as u64)?;
    Ok(buf)
}

fn take_budget(budget: &mut u64, n: u64) -> std::result::Result<(), String> {
    if n > *budget {
        return Err(format!(
            "unzips to more than {MAX_UNZIPPED_BYTES} bytes, so it is not read"
        ));
    }
    *budget -= n;
    Ok(())
}

/// The text of a WordprocessingML part: paragraphs as lines, table rows as cells joined by
/// ` | `, tabs and breaks kept.
fn paragraphs(xml: &[u8]) -> std::result::Result<String, String> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = String::new();
    // The tables being read, innermost last: the cells of the row so far, and the cell's text.
    let mut tables: Vec<(Vec<String>, String)> = Vec::new();
    let mut in_text = false;
    // How deep inside runs the reader is: a tab or break outside one is a tab stop or a setting.
    let mut runs = 0usize;
    // How deep inside `mc:Fallback` elements the reader is.
    let mut fallback = 0usize;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?;
        match event {
            Event::Eof => break,
            Event::Start(e) if e.local_name().as_ref() == "Fallback" => fallback += 1,
            Event::End(e) if e.local_name().as_ref() == "Fallback" => {
                fallback = fallback.saturating_sub(1);
            }
            _ if fallback > 0 => {}
            Event::Start(e) => match e.local_name().as_ref() {
                "t" => in_text = true,
                "r" => runs += 1,
                "tbl" => tables.push((Vec::new(), String::new())),
                "tc" => {
                    if let Some((_, cell)) = tables.last_mut() {
                        cell.clear();
                    }
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                "tab" if runs > 0 => text_sink(&mut out, &mut tables).push('\t'),
                "br" | "cr" if runs > 0 => text_sink(&mut out, &mut tables).push('\n'),
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                "t" => in_text = false,
                "r" => runs = runs.saturating_sub(1),
                "p" => text_sink(&mut out, &mut tables).push('\n'),
                "tc" => {
                    if let Some((row, cell)) = tables.last_mut() {
                        let text = cell.split_whitespace().collect::<Vec<_>>().join(" ");
                        row.push(text);
                        cell.clear();
                    }
                }
                "tr" => {
                    let line = tables
                        .last_mut()
                        .map(|(row, _)| std::mem::take(row).join(" | "))
                        .unwrap_or_default();
                    let n = tables.len();
                    match n.checked_sub(2).and_then(|i| tables.get_mut(i)) {
                        Some((_, cell)) => {
                            cell.push_str(&line);
                            cell.push(' ');
                        }
                        None => {
                            out.push_str(&line);
                            out.push('\n');
                        }
                    }
                }
                "tbl" => {
                    tables.pop();
                }
                _ => {}
            },
            Event::Text(t) if in_text => {
                text_sink(&mut out, &mut tables).push_str(&t.xml10_content());
            }
            Event::CData(t) if in_text => {
                text_sink(&mut out, &mut tables).push_str(&t.xml10_content());
            }
            Event::GeneralRef(r) if in_text => {
                let s = text_sink(&mut out, &mut tables);
                if r.is_char_ref() {
                    if let Ok(Some(c)) = r.resolve_char_ref() {
                        s.push(c);
                    }
                } else if let Some(v) = resolve_predefined_entity(&r.xml10_content()) {
                    s.push_str(v);
                }
            }
            _ => {}
        }
        buf.clear();
        if out.len() > crate::MAX_TEXT {
            break;
        }
    }
    Ok(out)
}

/// Where text read now goes: the cell being read, or the text outside any table.
fn text_sink<'a>(out: &'a mut String, tables: &'a mut [(Vec<String>, String)]) -> &'a mut String {
    match tables.last_mut() {
        Some((_, cell)) => cell,
        None => out,
    }
}

/// The text of the spreadsheet `bytes`, as the module documentation lays out.
pub(crate) fn sheet_text(bytes: &[u8]) -> Out {
    let unreadable = |e: &dyn std::fmt::Display| format!("not a readable spreadsheet: {e}");
    let data = Cursor::new(bytes);
    let mut book = if bytes.starts_with(OLE_MAGIC) {
        Sheets::Xls(Xls::new(data).map_err(|e| unreadable(&e))?)
    } else {
        let names = check_unzipped(bytes)?;
        let has = |n: &str| names.iter().any(|m| m == n);
        if has("content.xml") && has("mimetype") {
            Sheets::Ods(Ods::new(data).map_err(|e| unreadable(&e))?)
        } else if has("xl/workbook.bin") {
            Sheets::Xlsb(Xlsb::new(data).map_err(|e| unreadable(&e))?)
        } else {
            Sheets::Xlsx(Xlsx::new(data).map_err(|e| unreadable(&e))?)
        }
    };
    let mut out = String::new();
    let mut failed = None;
    for name in book.sheet_names() {
        let range = match book.worksheet_range(&name) {
            Ok(r) => r,
            Err(e) => {
                failed.get_or_insert_with(|| format!("sheet {name}: {e}"));
                continue;
            }
        };
        out.push_str("# Sheet: ");
        out.push_str(&name);
        out.push('\n');
        let rows: Vec<&[Data]> = range.rows().collect();
        let used = |c: &Data| match c {
            Data::Empty => false,
            Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => !s.is_empty(),
            _ => true,
        };
        let height = rows
            .iter()
            .rposition(|r| r.iter().any(used))
            .map_or(0, |i| i + 1);
        let width = rows[..height]
            .iter()
            .filter_map(|r| r.iter().rposition(used))
            .max()
            .map_or(0, |i| i + 1);
        for row in &rows[..height] {
            for (i, c) in row.iter().take(width).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_csv_field(&mut out, &cell_text(c));
            }
            for _ in row.len().min(width)..width {
                out.push(',');
            }
            out.push('\n');
            if out.len() > crate::MAX_TEXT {
                return Ok(out);
            }
        }
    }
    match failed {
        Some(reason) if out.is_empty() => Err(reason),
        _ => Ok(out),
    }
}

/// Unpack every part of the zip `bytes` calamine may read, to count what it unzips to. Returns
/// the names of the zip's entries, lowercase.
fn check_unzipped(bytes: &[u8]) -> std::result::Result<Vec<String>, String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a zip: {e}"))?;
    let mut budget = MAX_UNZIPPED_BYTES;
    let mut names = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry
            .name()
            .map_err(|e| e.to_string())?
            .to_ascii_lowercase();
        let skipped = ["media/", "pictures/", "thumbnails/", "docprops/thumbnail"]
            .iter()
            .any(|p| name.contains(p));
        let dir = entry.is_dir();
        names.push(name);
        if dir || skipped {
            continue;
        }
        let n = std::io::copy(&mut entry.take(budget + 1), &mut std::io::sink())
            .map_err(|e| e.to_string())?;
        take_budget(&mut budget, n)?;
    }
    Ok(names)
}

/// A cell as text.
fn cell_text(c: &Data) -> String {
    match c {
        Data::Empty => String::new(),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::Bool(true) => "TRUE".to_owned(),
        Data::Bool(false) => "FALSE".to_owned(),
        Data::Error(e) => e.to_string(),
        Data::DateTime(d) if d.is_duration() => {
            let secs = (d.as_f64() * 86_400.0).round() as i64;
            let sign = if secs < 0 { "-" } else { "" };
            let secs = secs.unsigned_abs();
            format!(
                "{sign}{}:{:02}:{:02}",
                secs / 3600,
                secs / 60 % 60,
                secs % 60
            )
        }
        Data::DateTime(d) => {
            let (y, mo, day, h, mi, s, _) = d.to_ymd_hms_milli();
            let has_time = (h, mi, s) != (0, 0, 0);
            if d.as_f64() < 1.0 {
                format!("{h:02}:{mi:02}:{s:02}")
            } else if has_time {
                format!("{y:04}-{mo:02}-{day:02}T{h:02}:{mi:02}:{s:02}")
            } else {
                format!("{y:04}-{mo:02}-{day:02}")
            }
        }
    }
}

/// Append `field` to `out` as one CSV field, quoted when RFC 4180 needs it.
fn push_csv_field(out: &mut String, field: &str) {
    if field.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&field.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(field);
    }
}

/// The text of the CSV file `bytes`: the file itself, without a byte order mark, invalid UTF-8
/// replaced and NUL bytes dropped, cut to at most [`crate::MAX_TEXT`] bytes.
pub(crate) fn csv_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes).replace('\0', "");
    cap(text)
}

/// `text` cut to at most [`crate::MAX_TEXT`] bytes, on a character boundary.
pub(crate) fn cap(mut text: String) -> String {
    if text.len() > crate::MAX_TEXT {
        let mut end = crate::MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_paragraphs_tables_and_entities() {
        let xml = br#"<w:document xmlns:w="w" xmlns:mc="mc"><w:body>
            <w:p><w:pPr><w:tabs><w:tab w:val="left"/></w:tabs></w:pPr>
              <w:r><w:t>Fish</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve"> &amp; chips</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>c&#233;</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            <w:p><mc:AlternateContent><mc:Choice><w:t>shown</w:t></mc:Choice>
              <mc:Fallback><w:t>repeated</w:t></mc:Fallback></mc:AlternateContent></w:p>
            </w:body></w:document>"#;
        let text = paragraphs(xml).unwrap();
        assert_eq!(text, "Fish\t & chips\na b | cé\nshown\n");
    }

    #[test]
    fn quotes_csv_fields_that_need_it() {
        let mut out = String::new();
        push_csv_field(&mut out, "plain");
        out.push(',');
        push_csv_field(&mut out, "say \"hi\", then\nleave");
        assert_eq!(out, "plain,\"say \"\"hi\"\", then\nleave\"");
    }

    #[test]
    fn csv_text_is_the_file() {
        assert_eq!(
            csv_text(b"\xEF\xBB\xBFa,b\n1,\xFF\0\n"),
            "a,b\n1,\u{FFFD}\n"
        );
    }
}
