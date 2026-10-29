//! Imported HTML and plain text as content blocks (rich-import H1, H4): the body of an email and
//! the description of a calendar event become headings, paragraphs, lists, quotes, code, tables,
//! images, dividers and toggles in the JSON form the writer accepts, and content blocks become
//! the simple HTML Google Calendar takes.
//!
//! - [`html_to_blocks`] walks the HTML5 parse of a document or fragment. Scripts, styles, forms,
//!   frames, objects and hidden content never reach the output, which is blocks of inline markup
//!   with its markup characters escaped, so no raw HTML or `javascript:` URL survives. Images go
//!   to an [`Images`] the caller gives, which stores them or says why not.
//! - [`text_to_blocks`] reads a `text/plain` body: a paragraph per run of lines, `>` lines as
//!   quotes, URLs as links, and with [`Options::quotes`] the quoted history and signature folded
//!   into toggles.
//! - [`blocks_to_html`] writes content blocks as the HTML Google Calendar takes for a description.
//!   For any input `x`, `blocks_to_html(html_to_blocks(c))` is `c` again when `c` is
//!   `blocks_to_html(html_to_blocks(x))`, so a description compared in that form round-trips.
//!
//! `docs/vault-format.md` "Imported HTML" lists the rules, the layout table heuristic among them.

mod colour;
mod convert;
mod css;
mod html_out;
mod inline;
mod text;

use serde_json::{Value, json};

pub use convert::html_to_blocks;
pub use html_out::blocks_to_html;
pub use inline::escape_text;
pub use text::text_to_blocks;

/// The version of the conversion, which notes and snapshots record so a newer one can redo
/// them (rich-import H8). 1 was the scripts' own line-per-paragraph text.
pub const CONVERTER_VERSION: i64 = 2;

/// The most HTML converted, in bytes. The rest is cut.
pub const MAX_HTML: usize = 2 * 1024 * 1024;

/// The most blocks one conversion makes, list items and table rows included.
pub const MAX_BLOCKS: usize = 5000;

/// The deepest element converted; anything deeper is left out.
pub const MAX_DEPTH: usize = 128;

/// The most images one conversion places.
pub const MAX_IMAGES: usize = 40;

/// The summary of the toggle that holds quoted reply history.
pub const QUOTED_TITLE: &str = "Quoted text";

/// The summary of the toggle that holds a signature.
pub const SIGNATURE_TITLE: &str = "Signature";

/// How a conversion runs.
#[derive(Debug, Clone)]
pub struct Options {
    /// Fold quoted reply history and signatures into collapsed toggles (rich-import H2).
    pub quotes: bool,
    /// The paragraph added at the end when a limit cut the conversion short.
    pub cut_text: String,
    pub max_html: usize,
    pub max_blocks: usize,
    pub max_depth: usize,
    pub max_images: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            quotes: false,
            cut_text: "The rest is longer than the import keeps; see View original.".to_owned(),
            max_html: MAX_HTML,
            max_blocks: MAX_BLOCKS,
            max_depth: MAX_DEPTH,
            max_images: MAX_IMAGES,
        }
    }
}

/// Where an image's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Src {
    /// `cid:<id>`: a part of the message, by its Content-ID without angle brackets.
    Cid(String),
    /// A `data:` URI's decoded bytes and its media type.
    Data { media_type: String, bytes: Vec<u8> },
    /// An `http` or `https` URL.
    Remote(String),
}

impl Src {
    /// The `src` as the HTML gave it, for a link: the URL of a remote image, else nothing.
    pub fn url(&self) -> Option<&str> {
        match self {
            Src::Remote(u) => Some(u),
            _ => None,
        }
    }
}

/// What became of an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    /// Stored as this asset: an image block.
    Asset(String),
    /// Left as a link to it, or as its alt text when it has no URL.
    Link,
    /// Not stored, for this reason: a paragraph saying so.
    Failed(String),
    /// Not fetched yet, for a later run: a paragraph [`complete_pending`] finds again.
    Pending,
    /// Nothing at all, such as a tracking pixel found once fetched.
    Skip,
}

/// Stores the images a conversion meets.
pub trait Images {
    fn place(&mut self, src: &Src, alt: &str) -> Placed;
}

/// Every image as a link, or its alt text: nothing fetched or stored.
pub struct LinkImages;

impl Images for LinkImages {
    fn place(&mut self, _src: &Src, _alt: &str) -> Placed {
        Placed::Link
    }
}

/// What a conversion made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Converted {
    /// Content blocks in the JSON form the writer accepts.
    pub blocks: Vec<Value>,
    /// What was left out, for the run log.
    pub warnings: Vec<String>,
    /// Whether a limit cut the conversion short.
    pub cut: bool,
    /// Images left [`Placed::Pending`].
    pub pending: usize,
}

// ---------------------------------------------------------------------------------------------
// Blocks

pub(crate) fn paragraph(text: String) -> Value {
    json!({ "kind": "paragraph", "fields": { "text": text } })
}

pub(crate) fn toggle(text: &str, blocks: Vec<Value>) -> Value {
    json!({ "kind": "toggle", "fields": { "text": escape_text(text) }, "blocks": blocks })
}

/// The markup of an image that is a link to its URL, as `[Image: <alt>](<url>)`, or the plain
/// `Image: <alt>` when it has no URL.
fn image_label(alt: &str, url: Option<&str>) -> String {
    let label = if alt.is_empty() {
        "Image".to_owned()
    } else {
        format!("Image: {alt}")
    };
    match url {
        Some(u) => inline::serialize(&[inline::Piece::Text(
            label,
            inline::Style {
                link: Some(u.to_owned()),
                ..inline::Style::default()
            },
        )]),
        None => escape_text(&label),
    }
}

/// What a paragraph standing for an image says: its link, then why it is not stored.
pub(crate) fn image_text(src: &Src, alt: &str, placed: &Placed) -> String {
    let label = image_label(alt, src.url());
    match placed {
        Placed::Failed(why) => match src.url() {
            Some(_) => format!(
                "{label} {}",
                escape_text(&format!("(not downloaded: {why})"))
            ),
            None => format!(
                "{label} {}",
                escape_text(&format!("(not available: {why})"))
            ),
        },
        Placed::Pending => format!("{label} {}", escape_text(PENDING_NOTE)),
        _ => label,
    }
}

/// What a paragraph holding a remote image not fetched yet ends with.
const PENDING_NOTE: &str = "(not downloaded yet)";

/// The alt text and URL of a paragraph [`html_to_blocks`] left for an image not fetched yet, or
/// `None` for any other text.
pub fn pending_image(text: &str) -> Option<(String, String)> {
    let label = text.strip_suffix(&format!(" {}", escape_text(PENDING_NOTE)))?;
    let pieces = inline::parse(label);
    let [inline::Piece::Text(t, s)] = pieces.as_slice() else {
        return None;
    };
    let url = s.link.clone()?;
    let alt = t.strip_prefix("Image: ").unwrap_or("").to_owned();
    if !(t == "Image" || t.starts_with("Image: ")) {
        return None;
    }
    Some((alt, url))
}

/// `blocks` with each paragraph [`pending_image`] finds, at any depth, placed again through
/// `images`: an image block once stored, a paragraph saying why not when it fails, and as it was
/// while still pending. Returns the blocks and how many are still pending.
pub fn complete_pending(blocks: &[Value], images: &mut dyn Images) -> (Vec<Value>, usize) {
    let mut pending = 0;
    let out = blocks
        .iter()
        .filter_map(|b| complete_one(b, images, &mut pending))
        .collect();
    (out, pending)
}

fn complete_one(b: &Value, images: &mut dyn Images, pending: &mut usize) -> Option<Value> {
    if b["kind"] == "paragraph"
        && let Some((alt, url)) = b["fields"]["text"].as_str().and_then(pending_image)
    {
        let src = Src::Remote(url);
        return match images.place(&src, &alt) {
            Placed::Asset(id) => Some(image_block(&id, &alt, None)),
            Placed::Skip => None,
            placed => {
                if placed == Placed::Pending {
                    *pending += 1;
                }
                Some(paragraph(image_text(&src, &alt, &placed)))
            }
        };
    }
    let mut out = b.clone();
    if let Some(children) = b["blocks"].as_array() {
        out["blocks"] = Value::Array(
            children
                .iter()
                .filter_map(|c| complete_one(c, images, pending))
                .collect(),
        );
    }
    Some(out)
}

pub(crate) fn image_block(asset: &str, alt: &str, caption: Option<&str>) -> Value {
    let mut fields = json!({ "asset": asset });
    if !alt.is_empty() {
        fields["alt"] = json!(alt);
    }
    if let Some(c) = caption.filter(|c| !c.is_empty()) {
        fields["caption"] = json!(c);
    }
    json!({ "kind": "image", "fields": fields })
}
