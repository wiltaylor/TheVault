//! What another app shares to TheVault, with or without a connection (D19). The Kotlin share
//! activity has already copied each shared `content://` file into a file of the app's own;
//! [`store`] keeps those files as assets of the local vault, and [`payload`] builds the
//! `share_received` payload of `docs/scripts.md` (`{ text, subject, url, attachments }`), each
//! attachment `{ asset, filename, mime, size }`.
//!
//! [`crate::core::Core::share`] queues that payload as a `share_received` device event and runs
//! it at once, so the device file's handler, by default `scripts/device_share.ws`, decides what
//! the share becomes. When no device runtime is open, or the device file declares no
//! `share_received` handler, [`write_note`] makes the fleeting note the default script would, so a
//! share is never silently lost. Either way exactly one of the two runs. The writer hooks queue
//! whatever is written for the home server like any local write.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vocab::Author;
use writer::{FLEETING_PROCESSED_FIELD, FLEETING_TYPE, Input, Writer};

/// The longest title a share's first line gives, in characters, before it is cut. As the web
/// client's capture dialog does.
const TITLE_MAX: usize = 80;

/// The session the app's own writes are recorded under: the owner, as in the webview.
const OWNER: &str = "owner";

/// What the share activity hands over.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Shared {
    /// `EXTRA_TEXT`, or `""`.
    pub text: String,
    /// `EXTRA_SUBJECT`, or `""`.
    pub subject: String,
    /// Each shared file, already copied out of its `content://` URI.
    pub files: Vec<SharedFile>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SharedFile {
    /// The copy the share activity made. [`import`] removes it once it is stored.
    pub path: PathBuf,
    /// The name the sharing app gave the file.
    pub filename: String,
    /// The media type the sharing app gave the file, or `""`, when it is guessed from the name.
    pub mime: String,
}

/// What became of a share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// The queued `share_received` event that ran, or `None` when the share fell back to
    /// [`write_note`].
    pub event: Option<i64>,
    /// How the event stands: `done`, or `queued` when its handler failed and the event waits for
    /// the next pass (`failed` once it has failed every pass). Always `done` for the fallback.
    pub status: String,
    /// Why the handler failed.
    pub message: Option<String>,
    /// The notes the share made: the ones the handler created, or the fallback's one note.
    pub notes: Vec<String>,
}

impl Shared {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.subject.trim().is_empty() && self.files.is_empty()
    }
}

/// The media type of a shared file: the one its app gave, else a guess from its name.
pub fn mime(file: &SharedFile) -> String {
    let given = file.mime.trim();
    if !given.is_empty() && given != "*/*" && !given.ends_with("/*") {
        return given.to_ascii_lowercase();
    }
    let ext = file
        .filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// The file name endings of a media type, the usual one first, or none for a type not listed.
/// The Android MMS reader (`MmsReader.kt`, `FileNames`) keeps the same table.
fn extensions(mime: &str) -> &'static [&'static str] {
    let base = mime.split(';').next().unwrap_or_default().trim();
    match base.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => &["jpg", "jpeg"],
        "image/png" => &["png"],
        "image/gif" => &["gif"],
        "image/webp" => &["webp"],
        "image/avif" => &["avif"],
        "image/heic" => &["heic"],
        "image/heif" => &["heif"],
        "image/bmp" => &["bmp"],
        "image/svg+xml" => &["svg"],
        "application/pdf" => &["pdf"],
        "text/plain" => &["txt", "md"],
        "text/csv" => &["csv"],
        "text/x-vcard" | "text/vcard" => &["vcf"],
        "application/json" => &["json"],
        "application/zip" => &["zip"],
        "audio/mpeg" => &["mp3"],
        "audio/amr" => &["amr"],
        "audio/aac" => &["aac"],
        "audio/mp4" => &["m4a"],
        "audio/ogg" => &["ogg"],
        "audio/wav" => &["wav"],
        "video/mp4" => &["mp4"],
        "video/3gpp" => &["3gp"],
        "video/3gpp2" => &["3g2"],
        "video/quicktime" => &["mov"],
        "video/webm" => &["webm"],
        _ => &[],
    }
}

/// `filename` with the extension of its media type `mime` added when it lacks one of its
/// endings, so a part named `image000001` sent as `image/jpeg` is kept as `image000001.jpg` and
/// shows as an image. A type with no known ending leaves the name as it is.
pub fn with_extension(filename: &str, mime: &str) -> String {
    let wanted = extensions(mime);
    let Some(first) = wanted.first() else {
        return filename.to_owned();
    };
    let has = filename.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && wanted.contains(&ext.to_ascii_lowercase().as_str())
    });
    if has {
        filename.to_owned()
    } else {
        format!("{filename}.{first}")
    }
}

/// The first `http://` or `https://` link in `text`, without trailing punctuation.
pub fn first_url(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let start = word.find("https://").or_else(|| word.find("http://"))?;
        let url = word[start..]
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '>', '"', '\'']);
        (url.len() > "https://".len()).then(|| url.to_owned())
    })
}

/// `filename` made safe as an asset's file name: no folders, no control characters, not empty.
pub fn safe_filename(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .to_owned();
    let clean: String = clean.chars().take(200).collect();
    if clean.is_empty() {
        "shared".to_owned()
    } else {
        clean
    }
}

/// Plain text as inline markup: the characters that would start markup are escaped, as the web
/// editor's `serializeInline` does for a text node.
pub fn escape_inline(text: &str) -> String {
    const ALWAYS: &[char] = &['\\', '*', '`', '{', '[', ']'];
    const PAIRED: &[char] = &['+', '~', '='];
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (k, &ch) in chars.iter().enumerate() {
        let paired = PAIRED.contains(&ch)
            && (k == 0
                || k == chars.len() - 1
                || chars[k - 1] == ch
                || chars.get(k + 1) == Some(&ch));
        if ALWAYS.contains(&ch) || paired {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Plain text as paragraph blocks: a blank line starts a paragraph, a newline breaks a line.
fn paragraphs(text: &str) -> Vec<Value> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut flush = |current: &mut Vec<&str>| {
        let p = current.join("\n");
        let p = p.trim_end();
        if !p.trim().is_empty() {
            out.push(json!({ "kind": "paragraph", "fields": { "text": escape_inline(p) } }));
        }
        current.clear();
    };
    for line in text.split('\n') {
        if line.trim().is_empty() {
            flush(&mut current);
        } else {
            current.push(line);
        }
    }
    flush(&mut current);
    out
}

/// The note's title: the subject, else the first line of text, else the first file's name, cut
/// at a word to at most [`TITLE_MAX`] characters.
pub fn title(shared: &Shared) -> String {
    let line = [shared.subject.as_str(), shared.text.as_str()]
        .iter()
        .flat_map(|t| t.lines())
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|l| !l.is_empty())
        .or_else(|| shared.files.first().map(|f| safe_filename(&f.filename)))
        .unwrap_or_else(|| "Shared".to_owned());
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= TITLE_MAX {
        return line;
    }
    let cut: String = chars[..TITLE_MAX - 1].iter().collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > TITLE_MAX / 2 => &cut[..space],
        _ => &cut,
    };
    format!("{}…", cut.trim_end())
}

/// Store each shared file as an asset of the local vault, named with the extension of its media
/// type ([`with_extension`]), and answer them as the payload's
/// `attachments`: `[{ asset, filename, mime, size }]`, in the order they were shared.
pub fn store(writer: &Writer, shared: &Shared) -> anyhow::Result<Vec<Value>> {
    let mut attachments = Vec::new();
    for file in &shared.files {
        let bytes = std::fs::read(&file.path)
            .map_err(|e| anyhow::anyhow!("reading shared file {}: {e}", file.path.display()))?;
        let mime = mime(file);
        let filename = with_extension(&safe_filename(&file.filename), &mime);
        let asset = writer.put_asset(&filename, bytes)?;
        attachments.push(json!({
            "asset": asset.id,
            "filename": asset.filename,
            "mime": mime,
            "size": asset.size,
        }));
    }
    Ok(attachments)
}

/// Remove the copies the share activity made, once [`store`] kept them.
pub fn remove_copies(shared: &Shared) {
    for file in &shared.files {
        let _ = std::fs::remove_file(&file.path);
    }
}

/// The `share_received` payload for `shared`, whose files [`store`] kept as `attachments`.
pub fn payload(shared: &Shared, attachments: &[Value]) -> Value {
    json!({
        "text": shared.text,
        "subject": shared.subject,
        "url": first_url(&shared.text),
        "attachments": attachments,
    })
}

/// The fleeting note a `share_received` payload becomes when no handler takes it, in the JSON
/// form of [`Input::Json`]: the note the default `device_share` script makes. The shared images
/// are its `image` field, and every other file an `asset` block after the text.
pub fn fleeting_note(shared: &Shared, payload: &Value) -> Value {
    let mut fields = serde_json::Map::new();
    fields.insert("title".into(), json!(title(shared)));
    fields.insert(FLEETING_PROCESSED_FIELD.into(), json!(false));
    let attachments = payload["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let is_image = |a: &Value| a["mime"].as_str().is_some_and(|m| m.starts_with("image/"));
    let images: Vec<Value> = attachments
        .iter()
        .filter(|a| is_image(a))
        .map(|a| a["asset"].clone())
        .collect();
    if !images.is_empty() {
        fields.insert("image".into(), Value::Array(images));
    }
    let mut body = paragraphs(&shared.text);
    if let Some(url) = payload["url"].as_str()
        && !shared.text.contains(url)
    {
        body.extend(paragraphs(url));
    }
    for a in attachments.iter().filter(|a| !is_image(a)) {
        body.push(json!({ "kind": "asset", "fields": { "asset": a["asset"] } }));
    }
    let mut blocks = Vec::new();
    if !body.is_empty() {
        blocks.push(json!({ "kind": "content", "labels": ["body"], "blocks": body }));
    }
    json!({ "fields": fields, "blocks": blocks })
}

/// The fallback: write the fleeting note for `payload` directly, with no handler. Returns the
/// note's id.
pub fn write_note(writer: &Writer, shared: &Shared, payload: &Value) -> anyhow::Result<String> {
    let note = fleeting_note(shared, payload);
    let saved = writer.create(
        FLEETING_TYPE,
        &Input::Json(note),
        &Author::Session(OWNER.to_owned()),
    )?;
    Ok(saved.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Shared {
        Shared {
            text: t.to_owned(),
            ..Shared::default()
        }
    }

    #[test]
    fn finds_the_first_link() {
        assert_eq!(
            first_url("look at https://example.net/a?b=1, then http://x.org").as_deref(),
            Some("https://example.net/a?b=1")
        );
        assert_eq!(
            first_url("(see https://e.net/x).").as_deref(),
            Some("https://e.net/x")
        );
        assert_eq!(first_url("no link here"), None);
        assert_eq!(first_url("https://"), None);
    }

    #[test]
    fn titles_come_from_subject_text_or_file() {
        let mut s = text("  first   line \nsecond");
        assert_eq!(title(&s), "first line");
        s.subject = "A page".into();
        assert_eq!(title(&s), "A page");
        let files = Shared {
            files: vec![SharedFile {
                path: "/tmp/x".into(),
                filename: "photo.jpg".into(),
                mime: String::new(),
            }],
            ..Shared::default()
        };
        assert_eq!(title(&files), "photo.jpg");
        let long = text(&"word ".repeat(40));
        let t = title(&long);
        assert!(t.ends_with('…') && t.chars().count() <= TITLE_MAX, "{t}");
    }

    #[test]
    fn escapes_markup_like_the_web_editor() {
        assert_eq!(
            escape_inline("a*b [c] `d` {e} \\"),
            "a\\*b \\[c\\] \\`d\\` \\{e} \\\\"
        );
        assert_eq!(escape_inline("a+b ++c +"), "a+b \\+\\+c \\+");
    }

    #[test]
    fn filenames_lose_folders_and_control_characters() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("a\u{0}b.txt"), "ab.txt");
        assert_eq!(safe_filename(".hidden"), "hidden");
        assert_eq!(safe_filename(""), "shared");
    }

    #[test]
    fn media_types_come_from_the_app_or_the_name() {
        let f = |filename: &str, mime: &str| SharedFile {
            path: "/tmp/x".into(),
            filename: filename.into(),
            mime: mime.into(),
        };
        assert_eq!(mime(&f("a.JPG", "")), "image/jpeg");
        assert_eq!(mime(&f("a.bin", "image/png")), "image/png");
        assert_eq!(mime(&f("report.pdf", "*/*")), "application/pdf");
        assert_eq!(mime(&f("noext", "")), "application/octet-stream");
    }

    #[test]
    fn stored_names_carry_the_extension_of_their_media_type() {
        assert_eq!(
            with_extension("image000001", "image/jpeg"),
            "image000001.jpg"
        );
        assert_eq!(with_extension("photo.JPEG", "image/jpeg"), "photo.JPEG");
        assert_eq!(with_extension("photo.png", "image/png"), "photo.png");
        assert_eq!(with_extension("photo.dat", "image/png"), "photo.dat.png");
        assert_eq!(with_extension("shared", "image/png; name=x"), "shared.png");
        assert_eq!(with_extension("contact", "text/x-vcard"), "contact.vcf");
        assert_eq!(with_extension("blob", "application/octet-stream"), "blob");
    }

    #[test]
    fn a_shared_image_without_an_extension_is_stored_with_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let server = embedded::Embedded::open(embedded::Config {
            vault: dir.path().to_owned(),
            web: None,
            secret_key: None,
        })
        .unwrap();
        let writer = server.writer();
        let copies = tempfile::TempDir::new().unwrap();
        let path = copies.path().join("copy");
        std::fs::write(&path, b"\x89PNG").unwrap();
        let shared = Shared {
            files: vec![SharedFile {
                path,
                filename: "IMG_0001".into(),
                mime: "image/png".into(),
            }],
            ..Shared::default()
        };
        let stored = store(writer, &shared).unwrap();
        assert_eq!(stored[0]["filename"], "IMG_0001.png");
        assert_eq!(stored[0]["mime"], "image/png");
    }

    #[test]
    fn the_note_carries_body_assets_and_is_unprocessed() {
        let s = text("Hello\n\nhttps://e.net");
        let attachments = [
            json!({ "asset": "01J00000000000000000000000", "filename": "a.png", "mime": "image/png", "size": 1 }),
            json!({ "asset": "01J00000000000000000000001", "filename": "b.pdf", "mime": "application/pdf", "size": 1 }),
        ];
        let p = payload(&s, &attachments);
        assert_eq!(p["url"], "https://e.net");
        assert_eq!(p["attachments"][1]["filename"], "b.pdf");
        let note = fleeting_note(&s, &p);
        assert_eq!(note["fields"]["processed"], false);
        assert_eq!(
            note["fields"]["image"],
            json!(["01J00000000000000000000000"])
        );
        assert_eq!(note["blocks"][0]["labels"][0], "body");
        let body = note["blocks"][0]["blocks"].as_array().unwrap();
        assert_eq!(body.len(), 3);
        assert_eq!(body[2]["kind"], "asset");
        assert_eq!(body[2]["fields"]["asset"], "01J00000000000000000000001");
    }
}
