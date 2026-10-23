//! The chat's `read_asset` tool and the images it sends (ai D19), and `find_assets` (ai D29).
//!
//! What a read returns depends on the asset's media type, which comes from its file name as a
//! download's does:
//!
//! - PNG, JPEG, GIF and WebP images go to the model as images. One over [`MAX_IMAGE_BYTES`] or
//!   with a long edge over [`MAX_EDGE`] pixels is decoded and sent as a JPEG of quality
//!   [`JPEG_QUALITY`] at a [`SCALED_EDGE`] long edge. Another image type is converted the same
//!   way when it can be decoded, which with the decoders built in it cannot, so it is metadata.
//! - A PDF is the text the index extracted, cut at [`TEXT_LIMIT`] characters, or metadata when
//!   the PDF was kept as unreadable.
//! - `text/*` and JSON (Markdown is `text/*`) is the file's text, cut at [`TEXT_LIMIT`].
//! - Anything else is metadata: the id, file name, media type and size.
//!
//! The transcript holds an image only as a reference, [`Part::Image`]: [`hydrate`] loads it
//! from the vault, scaled the same way, each time a provider request is built, so the bytes
//! never reach the browser. An asset deleted since is sent as a text saying so.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use ai::{Content, ImageData, Message, Part};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{ImageReader, codecs::jpeg::JpegEncoder};
use serde_json::{Value, json};
use writer::{StoredText, Writer};

/// The most images `read_asset` sends in one user turn.
pub const IMAGE_LIMIT: usize = 8;

/// The largest image file sent as it is: 3.75 MiB, which is 5 MiB in base64.
pub const MAX_IMAGE_BYTES: u64 = 3_932_160;

/// The longest edge, in pixels, of an image sent as it is.
pub const MAX_EDGE: u32 = 2000;

/// The long edge, in pixels, of an image scaled down.
pub const SCALED_EDGE: u32 = 1568;

/// The JPEG quality of an image scaled down.
pub const JPEG_QUALITY: u8 = 85;

/// The most characters of text a read returns.
pub const TEXT_LIMIT: usize = 40_000;

/// The largest image file decoded at all; a larger one is metadata.
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// The image types sent as they are when small enough.
const NATIVE: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Images prepared this turn, by asset id: what to send, or why there is nothing to send.
pub type Images = Arc<Mutex<HashMap<String, Result<ImageData, String>>>>;

/// What one `read_asset` call came to.
pub struct Read {
    /// The text result, JSON.
    pub content: String,
    pub is_error: bool,
    /// The image reference, when the asset went as an image.
    pub parts: Vec<Part>,
    /// The `tool_result` summary: the asset's metadata, never its bytes or text.
    pub summary: Value,
}

impl Read {
    fn error(message: impl Into<String>) -> Read {
        let message = message.into();
        Read {
            content: json!({ "error": message }).to_string(),
            is_error: true,
            parts: Vec::new(),
            summary: json!({ "notes": [], "message": message }),
        }
    }
}

/// The media type of a file called `filename`, without parameters.
pub fn media_type(filename: &str) -> String {
    let t = crate::assets::content_type(filename);
    t.split(';').next().unwrap_or(t).trim().to_owned()
}

fn is_text(media_type: &str) -> bool {
    media_type.starts_with("text/") || media_type == "application/json"
}

/// `s` cut to `limit` characters, with a marker saying the rest was left out. `partial` says
/// `s` is only the start of the text already.
fn cut(s: &str, limit: usize, size: u64, partial: bool) -> String {
    if !partial && s.chars().count() <= limit {
        return s.to_owned();
    }
    let kept: String = s.chars().take(limit).collect();
    format!("{kept}\n[truncated at {limit} characters; the whole text is longer ({size} bytes)]")
}

/// The most assets `find_assets` returns, and how many it returns when the call says nothing.
pub const FIND_LIMIT: usize = 50;
pub const FIND_DEFAULT: usize = 20;

/// `find_assets {text, limit}` (ai D29): the assets whose file name holds `text`, ignoring case,
/// newest first, at most `limit` (1 to [`FIND_LIMIT`], [`FIND_DEFAULT`] when absent). Empty text
/// lists the newest. Each is its id, file name, media type, size and when it was stored.
pub fn find_assets(w: &Writer, input: &Value) -> Read {
    let text = input["text"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let limit = input["limit"]
        .as_u64()
        .map_or(FIND_DEFAULT, |n| usize::try_from(n).unwrap_or(FIND_LIMIT))
        .clamp(1, FIND_LIMIT);
    let all = match w.assets() {
        Ok(a) => a,
        Err(e) => return Read::error(e.to_string()),
    };
    let matched: Vec<_> = all
        .iter()
        .rev()
        .filter(|a| text.is_empty() || a.filename.to_lowercase().contains(&text))
        .collect();
    let total = matched.len();
    let assets: Vec<Value> = matched
        .into_iter()
        .take(limit)
        .map(|a| {
            let created = crate::assets::ulid_ms(&a.id)
                .and_then(|ms| i64::try_from(ms).ok())
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            json!({
                "id": a.id, "filename": a.filename, "media_type": media_type(&a.filename),
                "size": a.size, "created": created,
            })
        })
        .collect();
    let message = format!(
        "{} of {total} asset{} found",
        assets.len(),
        if total == 1 { "" } else { "s" }
    );
    Read {
        content: json!({ "assets": assets, "total": total }).to_string(),
        is_error: false,
        parts: Vec::new(),
        summary: json!({ "notes": [], "assets": assets, "total": total, "message": message }),
    }
}

/// `read_asset {id}`. `may_send_image` is false once the turn has sent [`IMAGE_LIMIT`] images.
pub fn read_asset(w: &Writer, input: &Value, may_send_image: bool, images: &Images) -> Read {
    let Some(id) = input["id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Read::error("id must be a non-empty string");
    };
    let asset = match w.asset_info(id) {
        Ok(a) => a,
        Err(e) => return Read::error(e.to_string()),
    };
    let media = media_type(&asset.filename);
    let mut meta = json!({
        "id": asset.id, "filename": asset.filename, "media_type": media, "size": asset.size,
    });
    let summary = |kind: &str| {
        json!({ "notes": [], "asset": {
            "id": asset.id, "filename": asset.filename, "media_type": media,
            "size": asset.size, "kind": kind,
        } })
    };
    let done = |content: Value, kind: &str, parts: Vec<Part>| Read {
        content: content.to_string(),
        is_error: false,
        parts,
        summary: summary(kind),
    };
    if media.starts_with("image/") {
        if !may_send_image {
            return Read::error(format!(
                "the limit of {IMAGE_LIMIT} images per question is reached: this image was not \
                 sent. Answer from what you have, or ask the user to ask again."
            ));
        }
        match load_image(w, &asset.id, &media, images) {
            Ok(sent) => {
                meta["sent_as"] = json!(sent.media_type);
                meta["note"] = json!("The image follows.");
                return done(meta, "image", vec![Part::image(&asset.id, &media)]);
            }
            Err(why) => {
                meta["note"] = json!(format!("The image cannot be sent: {why}."));
                return done(meta, "metadata", Vec::new());
            }
        }
    }
    if media == "application/pdf" {
        match w.asset_text(&asset.id) {
            StoredText::Text(text) if !text.trim().is_empty() => {
                meta["text"] = json!(cut(&text, TEXT_LIMIT, asset.size, false));
                return done(meta, "pdf", Vec::new());
            }
            StoredText::Text(_) => {
                meta["note"] = json!("The PDF has no text layer, such as a scan.");
            }
            StoredText::Unreadable(reason) => {
                meta["note"] = json!(format!(
                    "The PDF's text could not be read and it is kept as unreadable: {reason}"
                ));
            }
            StoredText::NotExtracted => {
                meta["note"] = json!("The PDF's text has not been extracted yet.");
            }
        }
        return done(meta, "metadata", Vec::new());
    }
    if is_text(&media) {
        // Four bytes a character at most, and one more to tell a longer file.
        let want = (TEXT_LIMIT as u64 * 4 + 1).min(asset.size);
        return match w.asset_range(&asset.id, 0, want) {
            Ok((_, bytes)) => {
                let mut text = String::from_utf8_lossy(&bytes).into_owned();
                let partial = want < asset.size;
                if partial {
                    // The read may have split the last character.
                    text.pop();
                }
                meta["text"] = json!(cut(&text, TEXT_LIMIT, asset.size, partial));
                done(meta, "text", Vec::new())
            }
            Err(e) => Read::error(e.to_string()),
        };
    }
    done(meta, "metadata", Vec::new())
}

/// Image asset `id` as it is sent, prepared once a turn.
fn load_image(w: &Writer, id: &str, media: &str, images: &Images) -> Result<ImageData, String> {
    if let Some(got) = images.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        return got.clone();
    }
    if id.starts_with(crate::ai_web::WEB_IMAGE) {
        return Err("it was fetched in an earlier question".to_owned());
    }
    if id.starts_with(crate::ai_browser::BROWSER_IMAGE) {
        return Err("it was read from the browser box in an earlier question".to_owned());
    }
    let got = match w.asset_info(id) {
        Err(_) => Err(format!("asset {id} no longer exists")),
        Ok(a) if a.size > MAX_DECODE_BYTES => Err(format!(
            "the file is {} bytes, more than the {MAX_DECODE_BYTES} an image may be",
            a.size
        )),
        Ok(_) => w
            .asset(id)
            .map_err(|e| e.to_string())
            .and_then(|(_, bytes)| prepare(bytes, media)),
    };
    images
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_owned(), got.clone());
    got
}

/// `bytes`, an image of type `media`, as it is sent: as it is when it is a type both providers
/// take and small enough, else decoded and re-encoded as a JPEG at most [`SCALED_EDGE`] on its
/// long edge.
pub fn prepare(bytes: Vec<u8>, media: &str) -> Result<ImageData, String> {
    let reader = || {
        ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (width, height) = reader()?
        .into_dimensions()
        .map_err(|e| format!("it cannot be decoded ({e})"))?;
    let big = bytes.len() as u64 > MAX_IMAGE_BYTES || width.max(height) > MAX_EDGE;
    if NATIVE.contains(&media) && !big {
        return Ok(ImageData {
            media_type: media.to_owned(),
            base64: STANDARD.encode(&bytes),
        });
    }
    let mut img = reader()?
        .decode()
        .map_err(|e| format!("it cannot be decoded ({e})"))?;
    if img.width().max(img.height()) > SCALED_EDGE {
        img = img.thumbnail(SCALED_EDGE, SCALED_EDGE);
    }
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| format!("it could not be re-encoded ({e})"))?;
    Ok(ImageData {
        media_type: "image/jpeg".to_owned(),
        base64: STANDARD.encode(&out),
    })
}

/// Load every image part of `messages` from the vault for a provider request. An asset gone
/// since, or one that no longer decodes, becomes a text part saying so; the transcript the
/// client holds is not changed.
pub fn hydrate(w: &Writer, messages: &mut [Message], images: &Images) {
    for m in messages {
        for c in &mut m.content {
            let Content::ToolResult { parts, .. } = c else {
                continue;
            };
            for p in parts.iter_mut() {
                let Part::Image {
                    asset_id,
                    media_type,
                    data,
                } = p
                else {
                    continue;
                };
                match load_image(w, asset_id, media_type, images) {
                    Ok(d) => *data = Some(d),
                    Err(why) => {
                        let text = if asset_id.starts_with(crate::ai_web::WEB_IMAGE) {
                            "an image fetched from the web in an earlier question is not kept: \
                             fetch it again to see it"
                                .to_owned()
                        } else if asset_id.starts_with(crate::ai_browser::BROWSER_IMAGE) {
                            "an image read from the browser box in an earlier question is not \
                             kept: read it again with browser_file to see it"
                                .to_owned()
                        } else if w.asset_info(asset_id).is_err() {
                            format!("asset {asset_id} no longer exists")
                        } else {
                            format!("asset {asset_id} could not be sent: {why}")
                        };
                        *p = Part::Text { text };
                    }
                }
            }
        }
    }
}

/// The assets note JSON `note` names, in a field or in a block's `asset` item, at any depth,
/// with their file names, media types and sizes: what `read_asset` can read.
pub fn note_assets(w: &Writer, note: &Value) -> Vec<Value> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) if schema::is_ulid(s) => {
                if !out.contains(s) {
                    out.push(s.clone());
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut ids = Vec::new();
    walk(&note["fields"], &mut ids);
    walk(&note["blocks"], &mut ids);
    ids.iter()
        .filter_map(|id| w.asset_info(id).ok())
        .map(|a| {
            json!({
                "id": a.id, "filename": a.filename,
                "media_type": media_type(&a.filename), "size": a.size,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = RgbImage::from_pixel(width, height, image::Rgb([200, 100, 50]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn decoded(d: &ImageData) -> image::DynamicImage {
        image::load_from_memory(&STANDARD.decode(&d.base64).unwrap()).unwrap()
    }

    #[test]
    fn a_small_image_goes_as_it_is() {
        let bytes = png(40, 30);
        let d = prepare(bytes.clone(), "image/png").unwrap();
        assert_eq!(d.media_type, "image/png");
        assert_eq!(STANDARD.decode(&d.base64).unwrap(), bytes);
    }

    #[test]
    fn a_wide_image_is_scaled_to_a_jpeg() {
        let d = prepare(png(2400, 600), "image/png").unwrap();
        assert_eq!(d.media_type, "image/jpeg");
        let img = decoded(&d);
        assert_eq!((img.width(), img.height()), (SCALED_EDGE, 392));
    }

    #[test]
    fn an_image_type_it_cannot_decode_is_refused() {
        assert!(prepare(b"<svg/>".to_vec(), "image/svg+xml").is_err());
    }

    #[test]
    fn media_types_drop_their_parameters() {
        assert_eq!(media_type("a.md"), "text/plain");
        assert_eq!(media_type("a.PNG"), "image/png");
        assert_eq!(media_type("a.bin"), "application/octet-stream");
    }
}
