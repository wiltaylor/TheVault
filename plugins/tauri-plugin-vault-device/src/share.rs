//! What another app shares to TheVault becomes a fleeting note in the local vault, with or without
//! a connection (D19). The Kotlin share activity has already copied each shared `content://` file
//! into a file of the app's own; [`import`] stores those files as assets of the local vault and
//! writes the note, which the writer hooks queue for the home server like any local write.
//!
//! The note is built from the `share_received` payload of `docs/scripts.md`
//! (`{ text, subject, url, attachments }`), so routing the share through the device runtime's
//! `share_received` handler later changes where the note is made, not what it is built from.

use std::path::PathBuf;

use serde::Deserialize;
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
}

impl Shared {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.subject.trim().is_empty() && self.files.is_empty()
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

/// The `share_received` payload for `shared`, whose files were stored as `attachments`.
pub fn payload(shared: &Shared, attachments: &[String]) -> Value {
    json!({
        "text": shared.text,
        "subject": shared.subject,
        "url": first_url(&shared.text),
        "attachments": attachments,
    })
}

/// The fleeting note a `share_received` payload becomes, in the JSON form of [`Input::Json`].
pub fn fleeting_note(shared: &Shared, payload: &Value) -> Value {
    let mut fields = serde_json::Map::new();
    fields.insert("title".into(), json!(title(shared)));
    fields.insert(FLEETING_PROCESSED_FIELD.into(), json!(false));
    let attachments = payload["attachments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !attachments.is_empty() {
        fields.insert("image".into(), Value::Array(attachments));
    }
    let mut blocks = Vec::new();
    let body = paragraphs(&shared.text);
    if !body.is_empty() {
        blocks.push(json!({ "kind": "content", "labels": ["body"], "blocks": body }));
    }
    json!({ "fields": fields, "blocks": blocks })
}

/// Store the shared files as assets and write the fleeting note. Returns the note's id.
pub fn import(writer: &Writer, shared: &Shared) -> anyhow::Result<String> {
    if shared.is_empty() {
        anyhow::bail!("nothing was shared");
    }
    let mut attachments = Vec::new();
    for file in &shared.files {
        let bytes = std::fs::read(&file.path)
            .map_err(|e| anyhow::anyhow!("reading shared file {}: {e}", file.path.display()))?;
        let asset = writer.put_asset(&safe_filename(&file.filename), bytes)?;
        attachments.push(asset.id);
    }
    let payload = payload(shared, &attachments);
    let note = fleeting_note(shared, &payload);
    let saved = writer.create(
        FLEETING_TYPE,
        &Input::Json(note),
        &Author::Session(OWNER.to_owned()),
    )?;
    for file in &shared.files {
        let _ = std::fs::remove_file(&file.path);
    }
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
    fn the_note_carries_body_assets_and_is_unprocessed() {
        let s = text("Hello\n\nhttps://e.net");
        let p = payload(&s, &["01J00000000000000000000000".to_owned()]);
        assert_eq!(p["url"], "https://e.net");
        let note = fleeting_note(&s, &p);
        assert_eq!(note["fields"]["processed"], false);
        assert_eq!(note["fields"]["image"][0], "01J00000000000000000000000");
        assert_eq!(note["blocks"][0]["labels"][0], "body");
        assert_eq!(note["blocks"][0]["blocks"].as_array().unwrap().len(), 2);
    }
}
