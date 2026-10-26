//! Text extracted from uploaded assets, for full-text search: PDFs, Word documents (`.docx`),
//! spreadsheets (`.xlsx`, `.xls`, `.ods`) and CSV files, each a [`TextKind`] known by its file
//! name's extension and checked by its first bytes.
//!
//! An asset's text is extracted once, when it is uploaded or when a rebuild finds an asset of
//! one of those kinds with no text yet, and kept in `.vault/asset-text/v1/<ULID>.txt`. Assets
//! never change, so the file stays good until the asset is deleted. A file the extractors could
//! not read keeps an empty file, so a rebuild does not try it again, and `<ULID>.unreadable`
//! beside it holding why. Like the rest of the index, the folder is derived data: delete it and
//! the next rebuild extracts every asset's text again.
//!
//! A note's full-text document takes in the text of every asset it names, in a field or in a
//! block's `asset` item, so a search for a word inside a PDF or a spreadsheet finds the notes
//! holding it.
//!
//! Extraction runs on a thread of its own, capped at [`MAX_EXTRACT_BYTES`] of input,
//! [`EXTRACT_TIMEOUT`] of time and [`MAX_TEXT`] of output; a zip, which a docx, xlsx or ods
//! is, is never unpacked past [`crate::MAX_UNZIPPED_BYTES`]. [`crate::office`] lays out the
//! text of a document or spreadsheet. A CSV file's text is the file itself, read at once.
//!
//! pdf-extract reads a PDF's text first; when it fails or panics, which it does on some real
//! PDFs, lopdf's more tolerant extractor reads what it can. When both fail, as they do on a PDF
//! whose cross-reference table lopdf refuses, the table is rebuilt from the objects in the file
//! and both try again. A panic on an extraction thread is caught and prints nothing: its
//! message becomes the reason the caller logs. A thread that runs past its time cannot be
//! stopped: it is left to finish, and while [`MAX_RUNNING`] are still running no new
//! extraction starts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Once, mpsc};
use std::time::Duration;

use schema::{Doc, Value};

use crate::office::{self, OLE_MAGIC, ZIP_MAGIC};
use crate::{Result, io_err, pdf_repair};

/// The largest file whose text is extracted, in bytes.
pub const MAX_EXTRACT_BYTES: usize = 50 * 1024 * 1024;

/// How long one extraction may take before its file counts as unreadable.
pub const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most text kept from one file, in bytes.
pub const MAX_TEXT: usize = 2 * 1024 * 1024;

/// The most extractions running at once, counting any that ran past their time.
pub const MAX_RUNNING: usize = 4;

static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// The name of the threads extraction runs on, whose panics the hook keeps quiet.
const THREAD: &str = "asset-text";

/// What extracting a file's text came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extracted {
    /// The file's text, at most [`MAX_TEXT`] bytes, a PDF's or a document's with white space
    /// collapsed. It may be empty: a scanned PDF has no text.
    Text(String),
    /// pdf-extract failed for `reason`, and the fallback extractor, or either extractor once
    /// the PDF's xref table was rebuilt, read `text`, tidied as for [`Extracted::Text`] and
    /// never empty.
    Recovered { text: String, reason: String },
    /// The file is too large, is not of its kind, could not be read or ran past its time, for
    /// `reason`, one line. Kept as no text.
    Failed(String),
    /// Too many extractions are running: try again later. Nothing is kept.
    Busy,
}

/// What the index holds of an asset's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredText {
    /// The text extracted, empty for a file with none, such as a scanned PDF.
    Text(String),
    /// The file was kept as unreadable, for the reason given.
    Unreadable(String),
    /// No text has been extracted: the asset is of no [`TextKind`], or its extraction has not
    /// run yet.
    NotExtracted,
}

/// The kinds of file whose text is extracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    /// `.pdf`.
    Pdf,
    /// A Word document, `.docx`.
    Document,
    /// An Excel or OpenDocument spreadsheet: `.xlsx`, `.xls` or `.ods`.
    Spreadsheet,
    /// `.csv`.
    Csv,
}

impl TextKind {
    /// The kind of an asset called `filename`, by its extension, or `None` when its text is
    /// not extracted.
    pub fn of(filename: &str) -> Option<TextKind> {
        let (_, ext) = filename.rsplit_once('.')?;
        Some(match ext.to_ascii_lowercase().as_str() {
            "pdf" => TextKind::Pdf,
            "docx" => TextKind::Document,
            "xlsx" | "xls" | "ods" => TextKind::Spreadsheet,
            "csv" => TextKind::Csv,
            _ => return None,
        })
    }
}

/// Whether an asset called `filename` is a PDF, by its extension.
pub fn is_pdf(filename: &str) -> bool {
    TextKind::of(filename) == Some(TextKind::Pdf)
}

/// Extract the text of `bytes`, a file of `kind`, within the caps in the module documentation.
pub fn extract_text(kind: TextKind, bytes: Vec<u8>) -> Extracted {
    match kind {
        TextKind::Pdf => extract_pdf_text(bytes),
        TextKind::Document => extract_office(bytes, EXTRACT_TIMEOUT, kind, office::docx_text),
        TextKind::Spreadsheet => extract_office(bytes, EXTRACT_TIMEOUT, kind, office::sheet_text),
        TextKind::Csv if bytes.len() > MAX_EXTRACT_BYTES => too_large(),
        TextKind::Csv => Extracted::Text(office::csv_text(&bytes)),
    }
}

fn too_large() -> Extracted {
    Extracted::Failed(format!("larger than {MAX_EXTRACT_BYTES} bytes"))
}

/// The text `read` takes from `bytes`, a document or spreadsheet, on an extraction thread.
pub(crate) fn extract_office(
    bytes: Vec<u8>,
    timeout: Duration,
    kind: TextKind,
    read: Extractor,
) -> Extracted {
    if bytes.len() > MAX_EXTRACT_BYTES {
        return too_large();
    }
    let zip = bytes.starts_with(ZIP_MAGIC);
    match kind {
        TextKind::Document if !zip => {
            return Extracted::Failed(if bytes.starts_with(OLE_MAGIC) {
                "not a Word document: an OLE file, such as an old .doc or a password-protected \
                 document"
                    .to_owned()
            } else {
                "not a Word document: no zip header".to_owned()
            });
        }
        TextKind::Spreadsheet if !zip && !bytes.starts_with(OLE_MAGIC) => {
            return Extracted::Failed("not a spreadsheet: no zip or OLE header".to_owned());
        }
        _ => {}
    }
    let tidied = kind == TextKind::Document;
    run(bytes, timeout, move |bytes| match guarded(read, &bytes) {
        Ok(text) if tidied => Extracted::Text(tidy(&text)),
        Ok(text) => Extracted::Text(office::cap(text)),
        Err(reason) => Extracted::Failed(one_line(&reason)),
    })
}

impl Extracted {
    /// The text extracted, when there is any to keep.
    pub fn text(&self) -> Option<&str> {
        match self {
            Extracted::Text(t) | Extracted::Recovered { text: t, .. } => Some(t),
            Extracted::Failed(_) | Extracted::Busy => None,
        }
    }
}

/// An extractor: the text of a file, or why it has none.
type Extractor = fn(&[u8]) -> std::result::Result<String, String>;

/// Extract the text of the PDF `bytes`, within the caps in the module documentation.
pub fn extract_pdf_text(bytes: Vec<u8>) -> Extracted {
    extract_with(
        bytes,
        EXTRACT_TIMEOUT,
        |b| pdf_extract::extract_text_from_mem(b).map_err(|e| one_line(&e.to_string())),
        lopdf_text,
    )
}

/// The text lopdf finds on every page, skipping the pages and runs it cannot decode and the
/// characters it could not map.
fn lopdf_text(bytes: &[u8]) -> std::result::Result<String, String> {
    let doc = lopdf::Document::load_mem(bytes).map_err(|e| one_line(&e.to_string()))?;
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    let mut out = String::new();
    for chunk in doc.extract_text_chunks(&pages).into_iter().flatten() {
        out.push_str(&chunk);
        out.push('\n');
    }
    Ok(out.replace('\u{FFFD}', " "))
}

/// [`extract_pdf_text`] with the time cap and the extractors given, for tests.
pub(crate) fn extract_with(
    bytes: Vec<u8>,
    timeout: Duration,
    extract: Extractor,
    fallback: Extractor,
) -> Extracted {
    if bytes.len() > MAX_EXTRACT_BYTES {
        return too_large();
    }
    if !looks_like_pdf(&bytes) {
        return Extracted::Failed("no PDF header".to_owned());
    }
    run(bytes, timeout, move |bytes| {
        match read(extract, fallback, &bytes) {
            Extracted::Failed(reason) => match pdf_repair::rebuild_xref(&bytes)
                .map(|fixed| read(extract, fallback, &fixed))
            {
                Some(Extracted::Text(text)) if text.is_empty() => Extracted::Text(text),
                Some(Extracted::Text(text) | Extracted::Recovered { text, .. }) => {
                    Extracted::Recovered {
                        text,
                        reason: format!("{reason}; read after rebuilding its xref table"),
                    }
                }
                _ => Extracted::Failed(reason),
            },
            out => out,
        }
    })
}

/// What `job` makes of `bytes`, run on an extraction thread of its own when a slot is free,
/// or [`Extracted::Failed`] once it runs past `timeout`.
fn run(
    bytes: Vec<u8>,
    timeout: Duration,
    job: impl FnOnce(Vec<u8>) -> Extracted + Send + 'static,
) -> Extracted {
    quiet_panics_on_extraction_threads();
    // Take a slot, or give up when every slot is taken.
    if RUNNING
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < MAX_RUNNING).then_some(n + 1)
        })
        .is_err()
    {
        return Extracted::Busy;
    }
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name(THREAD.to_owned())
        .spawn(move || {
            let out = job(bytes);
            RUNNING.fetch_sub(1, Ordering::SeqCst);
            let _ = tx.send(out);
        });
    if spawned.is_err() {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
        return Extracted::Busy;
    }
    rx.recv_timeout(timeout).unwrap_or_else(|_| {
        Extracted::Failed(format!("ran past {} seconds", timeout.as_secs_f32()))
    })
}

/// The text `extract` reads from `bytes`, or what `fallback` reads when `extract` fails.
fn read(extract: Extractor, fallback: Extractor, bytes: &[u8]) -> Extracted {
    match guarded(extract, bytes) {
        Ok(text) => Extracted::Text(tidy(&text)),
        Err(reason) => match guarded(fallback, bytes).map(|t| tidy(&t)) {
            Ok(text) if !text.is_empty() => Extracted::Recovered { text, reason },
            _ => Extracted::Failed(reason),
        },
    }
}

thread_local! {
    /// The last panic on this extraction thread, as the hook described it.
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Run `extract` on `bytes`, a panic becoming its message as the error.
fn guarded(extract: Extractor, bytes: &[u8]) -> std::result::Result<String, String> {
    match std::panic::catch_unwind(|| extract(bytes)) {
        Ok(out) => out,
        Err(_) => Err(LAST_PANIC
            .with_borrow_mut(Option::take)
            .unwrap_or_else(|| "panicked".to_owned())),
    }
}

/// Install, once, a panic hook that keeps a panic on an extraction thread to itself, for
/// [`guarded`] to report, and hands every other panic to the hook it replaced.
fn quiet_panics_on_extraction_threads() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
            if std::thread::current().name() != Some(THREAD) {
                return previous(info);
            }
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_default();
            let at = info
                .location()
                .map(|l| {
                    format!(
                        " at {}:{}",
                        l.file().rsplit('/').next().unwrap_or(""),
                        l.line()
                    )
                })
                .unwrap_or_default();
            let described = one_line(&format!("panicked{at}: {message}"));
            LAST_PANIC.with_borrow_mut(|p| *p = Some(described));
        }));
    });
}

/// `text` on one line, at most 300 characters.
fn one_line(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(300) {
        Some((i, _)) => format!("{}...", &flat[..i]),
        None => flat,
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

    /// Keep asset `id` as having no text because it could not be read, for `reason`.
    pub fn set_unreadable(&mut self, id: &str, reason: &str) -> Result<()> {
        let path = self.dir.join(format!("{id}.unreadable"));
        std::fs::write(&path, format!("{reason}\n")).map_err(io_err(&path))?;
        self.set(id, "")
    }

    /// Asset `id`'s stored text, or why it has none.
    pub fn stored(&self, id: &str) -> StoredText {
        if !self.known.contains_key(id) {
            return StoredText::NotExtracted;
        }
        let unreadable = self.dir.join(format!("{id}.unreadable"));
        if let Ok(reason) = std::fs::read_to_string(&unreadable) {
            return StoredText::Unreadable(reason.trim().to_owned());
        }
        match std::fs::read_to_string(self.path(id)) {
            Ok(text) => StoredText::Text(text),
            Err(_) => StoredText::NotExtracted,
        }
    }

    /// Whether asset `id`'s text holds `part`, which is already lowercase.
    pub fn matches(&self, id: &str, part: &str) -> bool {
        self.known.get(id) == Some(&true)
            && std::fs::read_to_string(self.path(id))
                .is_ok_and(|t| crate::sql::contains_folded(&t, part))
    }

    /// Forget asset `id`'s text.
    pub fn forget(&mut self, id: &str) {
        if self.known.remove(id).is_some() {
            let _ = std::fs::remove_file(self.path(id));
            let _ = std::fs::remove_file(self.dir.join(format!("{id}.unreadable")));
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
        if !matches!(
            name.as_str(),
            "title" | "created" | "updated" | "archived" | "conflict_of"
        ) {
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
        let ok = |_: &[u8]| Ok("x".to_owned());
        let none = |_: &[u8]| Err("none".to_owned());
        assert_eq!(
            extract_with(b"hello".to_vec(), EXTRACT_TIMEOUT, ok, ok),
            Extracted::Failed("no PDF header".to_owned())
        );
        let mut big = b"%PDF-1.4\n".to_vec();
        big.resize(MAX_EXTRACT_BYTES + 1, b' ');
        assert!(matches!(
            extract_with(big, EXTRACT_TIMEOUT, ok, ok),
            Extracted::Failed(r) if r.starts_with("larger than")
        ));
        assert_eq!(
            extract_with(b"%PDF-1.4\n".to_vec(), EXTRACT_TIMEOUT, ok, none),
            Extracted::Text("x".to_owned())
        );
    }

    #[test]
    fn a_slow_or_panicking_extractor_counts_as_failed() {
        let slow = |_: &[u8]| {
            std::thread::sleep(Duration::from_millis(400));
            Ok("late".to_owned())
        };
        let none = |_: &[u8]| Err("none".to_owned());
        let started = std::time::Instant::now();
        assert!(matches!(
            extract_with(b"%PDF-1.4".to_vec(), Duration::from_millis(20), slow, none),
            Extracted::Failed(r) if r.starts_with("ran past")
        ));
        assert!(started.elapsed() < Duration::from_millis(300));
        let panics = |_: &[u8]| -> std::result::Result<String, String> { panic!("hostile pdf") };
        let Extracted::Failed(reason) =
            extract_with(b"%PDF-1.4".to_vec(), EXTRACT_TIMEOUT, panics, panics)
        else {
            panic!("not failed");
        };
        assert!(reason.starts_with("panicked at asset_text.rs:"), "{reason}");
        assert!(reason.ends_with(": hostile pdf"), "{reason}");
        // The fallback reads what the first extractor panicked on.
        let fallback = |_: &[u8]| Ok("  rescued  text ".to_owned());
        assert_eq!(
            extract_with(b"%PDF-1.4".to_vec(), EXTRACT_TIMEOUT, panics, fallback),
            Extracted::Recovered {
                text: "rescued text".to_owned(),
                reason,
            }
        );
        // A fallback with no text leaves the first extractor's reason.
        let blank = |_: &[u8]| Ok(" \u{FFFD} ".replace('\u{FFFD}', " "));
        assert_eq!(
            extract_with(b"%PDF-1.4".to_vec(), EXTRACT_TIMEOUT, none, blank),
            Extracted::Failed("none".to_owned())
        );
    }

    #[test]
    fn a_corrupt_pdf_has_no_text() {
        let corrupt =
            b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 9 0 R >>\ntrailer\n%%EOF".to_vec();
        assert!(matches!(extract_pdf_text(corrupt), Extracted::Failed(_)));
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
