//! Text extracted from uploaded PDF assets, for full-text search.
//!
//! A PDF's text is extracted once, when it is uploaded or when a rebuild finds a PDF with no
//! text yet, and kept in `.vault/asset-text/v1/<ULID>.txt`. Assets never change, so the file
//! stays good until the asset is deleted. A PDF the extractor could not read keeps an empty
//! file, so a rebuild does not try it again. Like the rest of the index, the folder is derived
//! data: delete it and the next rebuild extracts every PDF again.
//!
//! A note's full-text document takes in the text of every PDF it names, in a field or in a
//! block's `asset` item, so a search for a word inside a PDF finds the notes holding it.
//!
//! Extraction runs on a thread of its own, capped at [`MAX_PDF_BYTES`] of input,
//! [`EXTRACT_TIMEOUT`] of time and [`MAX_TEXT`] of output, and a panic in the extractor is
//! caught. A thread that runs past its time cannot be stopped: it is left to finish, and while
//! [`MAX_RUNNING`] are still running no new extraction starts.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use schema::{Doc, Value};

use crate::{Result, io_err};

/// The largest PDF whose text is extracted, in bytes.
pub const MAX_PDF_BYTES: usize = 50 * 1024 * 1024;

/// How long one extraction may take before its PDF counts as unreadable.
pub const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most text kept from one PDF, in bytes.
pub const MAX_TEXT: usize = 2 * 1024 * 1024;

/// The most extractions running at once, counting any that ran past their time.
pub const MAX_RUNNING: usize = 4;

static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// What extracting a PDF's text came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extracted {
    /// The PDF's text, white space collapsed, at most [`MAX_TEXT`] bytes. It may be empty: a
    /// scanned PDF has no text.
    Text(String),
    /// The file is too large, is not a PDF, could not be read, panicked the extractor or ran
    /// past its time. Kept as no text.
    Failed,
    /// Too many extractions are running: try again later. Nothing is kept.
    Busy,
}

/// Whether an asset called `filename` is a PDF, by its extension.
pub fn is_pdf(filename: &str) -> bool {
    filename
        .rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case("pdf"))
}

/// Extract the text of the PDF `bytes`, within the caps in the module documentation.
pub fn extract_pdf_text(bytes: Vec<u8>) -> Extracted {
    extract_with(bytes, EXTRACT_TIMEOUT, |b| {
        pdf_extract::extract_text_from_mem(b).ok()
    })
}

/// [`extract_pdf_text`] with the time cap and the extractor given, for tests.
pub(crate) fn extract_with(
    bytes: Vec<u8>,
    timeout: Duration,
    extract: fn(&[u8]) -> Option<String>,
) -> Extracted {
    if bytes.len() > MAX_PDF_BYTES || !looks_like_pdf(&bytes) {
        return Extracted::Failed;
    }
    // Take a slot, or give up when every slot is taken.
    if RUNNING
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < MAX_RUNNING).then_some(n + 1)
        })
        .is_err()
    {
        return Extracted::Busy;
    }
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("pdf-text".to_owned())
        .spawn(move || {
            let out = std::panic::catch_unwind(|| extract(&bytes)).ok().flatten();
            RUNNING.fetch_sub(1, Ordering::SeqCst);
            let _ = tx.send(out);
        });
    if spawned.is_err() {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
        return Extracted::Busy;
    }
    match rx.recv_timeout(timeout) {
        Ok(Some(text)) => Extracted::Text(tidy(&text)),
        Ok(None) | Err(_) => Extracted::Failed,
    }
}

/// Whether `bytes` start with a PDF header, which a PDF may put anywhere in its first 1024 bytes.
fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)]
        .windows(5)
        .any(|w| w == b"%PDF-")
}

/// Text with every run of white space made one space, or one newline when the run held one,
/// cut to at most [`MAX_TEXT`] bytes on a character boundary.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT));
    let mut gap: Option<char> = None;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            if c == '\n' || gap == Some('\n') {
                gap = Some('\n');
            } else {
                gap = Some(' ');
            }
            continue;
        }
        if let Some(g) = gap.take()
            && !out.is_empty()
        {
            out.push(g);
        }
        if out.len() + c.len_utf8() > MAX_TEXT {
            break;
        }
        out.push(c);
    }
    out
}

/// The stored texts in `.vault/asset-text/v1/`.
pub(crate) struct AssetTexts {
    dir: PathBuf,
    /// Every asset with a text file, and whether that text is not empty.
    known: HashMap<String, bool>,
}

impl AssetTexts {
    /// Open the folder, creating it when it is missing.
    pub fn open(dir: &Path) -> Result<AssetTexts> {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        let mut known = HashMap::new();
        for e in std::fs::read_dir(dir).map_err(io_err(dir))?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".txt") else {
                continue;
            };
            if schema::is_ulid(id) {
                let full = e.metadata().is_ok_and(|m| m.len() > 0);
                known.insert(id.to_owned(), full);
            }
        }
        Ok(AssetTexts {
            dir: dir.to_owned(),
            known,
        })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.txt"))
    }

    /// Whether asset `id` has had its text extracted, or found to have none.
    pub fn has(&self, id: &str) -> bool {
        self.known.contains_key(id)
    }

    /// Keep `text` as asset `id`'s text. Empty text records that the asset has none.
    pub fn set(&mut self, id: &str, text: &str) -> Result<()> {
        let path = self.path(id);
        let tmp = self.dir.join(format!("{id}.tmp"));
        std::fs::write(&tmp, text).map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io_err(&path))?;
        self.known.insert(id.to_owned(), !text.is_empty());
        Ok(())
    }

    /// Forget asset `id`'s text.
    pub fn forget(&mut self, id: &str) {
        if self.known.remove(id).is_some() {
            let _ = std::fs::remove_file(self.path(id));
        }
    }

    /// The texts of the assets `doc` names that have any, joined by newlines.
    pub fn of_doc(&self, doc: &Doc) -> String {
        if !self.known.values().any(|&full| full) {
            return String::new();
        }
        let mut ids = Vec::new();
        doc_assets(doc, &mut ids);
        let mut seen = HashSet::new();
        let mut out = String::new();
        for id in ids {
            if !seen.insert(id) || self.known.get(id) != Some(&true) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(self.path(id)) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&text);
            }
        }
        out
    }
}

/// Every ULID among `doc`'s field values, and in the `asset` item of its blocks at any depth.
/// Note references are ULIDs too; they have no text, so they are passed over.
fn doc_assets<'a>(doc: &'a Doc, out: &mut Vec<&'a str>) {
    fn push<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
        match v {
            Value::Str(s) if schema::is_ulid(s) => out.push(s),
            Value::List(items) => items.iter().for_each(|i| push(i, out)),
            _ => {}
        }
    }
    for (name, v) in &doc.fields {
        if !matches!(name.as_str(), "title" | "created" | "updated") {
            push(v, out);
        }
    }
    for b in &doc.blocks {
        doc_assets(&b.body, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidies_white_space_and_caps_the_text() {
        assert_eq!(tidy("  one\t two \n\n three  "), "one two\nthree");
        let long = "é".repeat(MAX_TEXT);
        let t = tidy(&long);
        assert!(t.len() <= MAX_TEXT && t.len() > MAX_TEXT - 2);
    }

    #[test]
    fn refuses_what_is_not_a_pdf_or_too_large() {
        let ok = |_: &[u8]| Some("x".to_owned());
        assert_eq!(
            extract_with(b"hello".to_vec(), EXTRACT_TIMEOUT, ok),
            Extracted::Failed
        );
        let mut big = b"%PDF-1.4\n".to_vec();
        big.resize(MAX_PDF_BYTES + 1, b' ');
        assert_eq!(extract_with(big, EXTRACT_TIMEOUT, ok), Extracted::Failed);
        assert_eq!(
            extract_with(b"%PDF-1.4\n".to_vec(), EXTRACT_TIMEOUT, ok),
            Extracted::Text("x".to_owned())
        );
    }

    #[test]
    fn a_slow_or_panicking_extractor_counts_as_failed() {
        let slow = |_: &[u8]| {
            std::thread::sleep(Duration::from_millis(400));
            Some("late".to_owned())
        };
        let started = std::time::Instant::now();
        assert_eq!(
            extract_with(b"%PDF-1.4".to_vec(), Duration::from_millis(20), slow),
            Extracted::Failed
        );
        assert!(started.elapsed() < Duration::from_millis(300));
        let panics = |_: &[u8]| -> Option<String> { panic!("hostile pdf") };
        assert_eq!(
            extract_with(b"%PDF-1.4".to_vec(), EXTRACT_TIMEOUT, panics),
            Extracted::Failed
        );
    }

    #[test]
    fn a_corrupt_pdf_has_no_text() {
        let corrupt =
            b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 9 0 R >>\ntrailer\n%%EOF".to_vec();
        assert_eq!(extract_pdf_text(corrupt), Extracted::Failed);
    }

    #[test]
    fn extracts_the_text_of_a_real_pdf() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/three-pages.pdf"
        ))
        .unwrap();
        let Extracted::Text(text) = extract_pdf_text(bytes) else {
            panic!("no text");
        };
        assert!(text.contains("zephyrine"), "{text}");
        assert!(text.contains("Page 3"), "{text}");
    }
}
