//! A PDF whose cross-reference table lopdf refuses, rebuilt from the objects in the file.
//!
//! lopdf reads a PDF through the table at its end that gives every object's byte offset. Some
//! writers get the table's shape wrong: one bank's statements end every entry with a bare
//! newline, 19 bytes where the format wants 20, and lopdf then reports "invalid file trailer"
//! and reads nothing. [`rebuild_xref`] finds every `N G obj` in the file itself and appends a
//! fresh table and a copy of the last trailer pointing at it, so a second attempt reads the
//! PDF the way a viewer that repairs such files would.

use std::collections::BTreeMap;

/// `bytes`, from its `%PDF-` header on, with a rebuilt cross-reference table and trailer
/// appended, or `None` when it has no objects or no trailer dictionary to copy.
pub(crate) fn rebuild_xref(bytes: &[u8]) -> Option<Vec<u8>> {
    // lopdf counts offsets from the header, which may come after some leading bytes.
    let start = bytes.windows(5).position(|w| w == b"%PDF-")?;
    let pdf = &bytes[start..];
    let objects = scan_objects(pdf);
    let max_id = *objects.keys().next_back()?;
    let trailer = last_trailer(pdf)?;

    let mut out = pdf.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", max_id + 1).as_bytes());
    for id in 1..=max_id {
        let entry = match objects.get(&id) {
            Some(&(offset, generation)) => format!("{offset:010} {generation:05} n \n"),
            None => "0000000000 00000 f \n".to_owned(),
        };
        out.extend_from_slice(entry.as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {}", max_id + 1).as_bytes());
    out.extend_from_slice(&trailer);
    out.extend_from_slice(format!("\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    Some(out)
}

/// Every object `N G obj` at the start of a line in `pdf`, by number, to its offset and
/// generation. A later object of the same number wins, as an incremental update's does. A
/// stream's data is skipped, so bytes inside it that look like an object are not taken.
fn scan_objects(pdf: &[u8]) -> BTreeMap<u32, (usize, u16)> {
    let mut found = BTreeMap::new();
    let mut i = 0;
    while i < pdf.len() {
        let line_start = i == 0 || matches!(pdf[i - 1], b'\n' | b'\r');
        if line_start && let Some((id, generation, body)) = object_header(pdf, i) {
            found.insert(id, (i, generation));
            i = skip_object(pdf, body);
            continue;
        }
        i += 1;
    }
    found
}

/// The number, generation and body offset of an `N G obj` header at `at`.
fn object_header(pdf: &[u8], at: usize) -> Option<(u32, u16, usize)> {
    let (id, i) = digits(pdf, at)?;
    let i = spaces(pdf, i, 1)?;
    let (generation, i) = digits(pdf, i)?;
    let i = spaces(pdf, i, 1)?;
    let after = i.checked_add(3)?;
    if pdf.get(i..after)? != b"obj" {
        return None;
    }
    // "obj" ends at a delimiter, not inside a longer word.
    if pdf
        .get(after)
        .is_some_and(|c| !c.is_ascii_whitespace() && !b"<[(/%".contains(c))
    {
        return None;
    }
    let id = u32::try_from(id).ok().filter(|&n| n > 0)?;
    let generation = u16::try_from(generation).ok()?;
    Some((id, generation, after))
}

/// Where scanning resumes after an object whose body starts at `body`: past its `endobj`,
/// jumping over the data of a stream first.
fn skip_object(pdf: &[u8], body: usize) -> usize {
    let endobj = find(pdf, body, b"endobj");
    let stream = find(pdf, body, b"stream");
    let from = match (stream, endobj) {
        (Some(s), Some(e)) if s < e => find(pdf, s + 6, b"endstream").map_or(s + 6, |n| n + 9),
        (Some(s), None) => find(pdf, s + 6, b"endstream").map_or(s + 6, |n| n + 9),
        _ => body,
    };
    find(pdf, from, b"endobj").map_or(from, |n| n + 6)
}

/// The body of the last trailer dictionary in `pdf` without its opening `<<`, with its `/Size`,
/// `/Prev` and `/XRefStm` entries blanked, since the rebuilt table replaces what they describe.
fn last_trailer(pdf: &[u8]) -> Option<Vec<u8>> {
    let mut at = rfind(pdf, b"trailer")?;
    at = spaces(pdf, at + 7, 0)?;
    if pdf.get(at..at + 2)? != b"<<" {
        return None;
    }
    let end = dictionary_end(pdf, at)?;
    let mut dict = pdf[at + 2..end].to_vec();
    for key in [&b"/Size"[..], b"/Prev", b"/XRefStm"] {
        blank_entry(&mut dict, key);
    }
    Some(dict)
}

/// The offset just past the `>>` that closes the dictionary opening at `at`, skipping strings.
fn dictionary_end(pdf: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = at;
    while i < pdf.len() {
        match pdf[i] {
            b'<' if pdf.get(i + 1) == Some(&b'<') => {
                depth += 1;
                i += 2;
            }
            b'>' if pdf.get(i + 1) == Some(&b'>') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'<' => i = find(pdf, i, b">")? + 1,
            b'(' => i = literal_string_end(pdf, i)?,
            _ => i += 1,
        }
    }
    None
}

/// The offset past the `)` closing the literal string opening at `at`.
fn literal_string_end(pdf: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = at;
    while i < pdf.len() {
        match pdf[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Overwrite with spaces every `key` in `dict` followed by an integer value, key and value.
fn blank_entry(dict: &mut [u8], key: &[u8]) {
    let mut from = 0;
    while let Some(k) = find(dict, from, key) {
        from = k + key.len();
        let Some(v) = spaces(dict, from, 0) else {
            continue;
        };
        let v = if dict.get(v) == Some(&b'-') { v + 1 } else { v };
        let Some((_, end)) = digits(dict, v) else {
            continue;
        };
        dict[k..end].fill(b' ');
        from = end;
    }
}

/// The decimal number at `at` and the offset after it.
fn digits(pdf: &[u8], at: usize) -> Option<(u64, usize)> {
    let len = pdf
        .get(at..)?
        .iter()
        .take(20)
        .take_while(|c| c.is_ascii_digit())
        .count();
    if len == 0 || len > 10 {
        return None;
    }
    let n = std::str::from_utf8(&pdf[at..at + len]).ok()?.parse().ok()?;
    Some((n, at + len))
}

/// The offset after at least `min` PDF white-space bytes from `at`.
fn spaces(pdf: &[u8], at: usize, min: usize) -> Option<usize> {
    let n = pdf
        .get(at..)?
        .iter()
        .take_while(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0'))
        .count();
    (n >= min).then_some(at + n)
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_objects_at_line_starts_and_skips_stream_data() {
        let pdf = b"%PDF-1.3\n1 0 obj\n<< /Type /Catalog >>\nendobj\n\
2 0 obj\n<< /Length 16 >>\nstream\n\n9 0 obj fake\nendstream\nendobj\n\
1 0 obj<< /Type /Catalog /Later true >>endobj\n12 3 obj\n(x)\nendobj\n";
        let found = scan_objects(pdf);
        assert_eq!(found.keys().copied().collect::<Vec<_>>(), vec![1, 2, 12]);
        let later = find(pdf, 0, b"1 0 obj<<").unwrap();
        assert_eq!(found[&1], (later, 0));
        assert_eq!(found[&12].1, 3);
    }

    #[test]
    fn copies_the_last_trailer_without_size_prev_or_xrefstm() {
        let pdf = b"trailer\n<< /Size 3 >>\n\
trailer\n<< /Size 11 /Prev 400 /Root 1 0 R /Info (a >> b) /ID [<AB> <CD>] /D << /X 1 >> >>\nstartxref";
        let dict = last_trailer(pdf).unwrap();
        let text = String::from_utf8(dict).unwrap();
        assert!(!text.contains("/Size") && !text.contains("/Prev"), "{text}");
        assert!(
            text.contains("/Root 1 0 R /Info (a >> b) /ID [<AB> <CD>]"),
            "{text}"
        );
        assert!(text.trim_end().ends_with("/D << /X 1 >> >>"), "{text}");
    }

    #[test]
    fn rebuilds_nothing_without_objects_or_a_trailer() {
        assert_eq!(rebuild_xref(b"%PDF-1.4\n"), None);
        assert_eq!(rebuild_xref(b"%PDF-1.4\n1 0 obj\n<< >>\nendobj\n"), None);
        assert_eq!(
            rebuild_xref(b"no header\n1 0 obj\n<< >>\nendobj\ntrailer\n<< >>"),
            None
        );
    }
}
