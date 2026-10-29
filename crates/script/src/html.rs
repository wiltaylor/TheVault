//! `vault::html_to_blocks`, `vault::text_to_blocks`, `vault::blocks_to_html` and
//! `vault::complete_images` (rich-import H1, H3, H4): imported HTML and plain text as content
//! blocks, through `crates/html-blocks`, with the images they hold stored as assets.
//!
//! The image fetch policy is the host's own, not the script's `allow_domains` (rich-import H3):
//!
//! - Only `after_*` handlers and tasks download remote images; anywhere else an image stays a
//!   link. Only `http` and `https` URLs with no user name or password.
//! - The address rule of the chat's web tools ([`ai::web`]): no loopback, private, link-local
//!   or other local address, unless Settings' `fetch_private` allows it.
//! - At most [`IMAGE_LIMIT`] bytes, [`IMAGE_TIMEOUT`] and [`IMAGE_REDIRECTS`] redirects each,
//!   no cookies and no credentials. The bytes must start as an image does (PNG, JPEG, GIF, WebP,
//!   BMP, AVIF or ICO), whatever the server calls them; a 1 or 2 pixel PNG or GIF is a tracking
//!   pixel, and dropped.
//! - Each URL is fetched once per run, and each image stored once per run, by its bytes.
//! - Past the run's deadline or the `deadline` option, an image is left pending, as a paragraph
//!   `vault::complete_images` finds again on a later run.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::ai::web::{FetchError, Fetcher};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use html_blocks::{Images, Options, Placed, Src};
use serde_json::{Value as Json, json};
use wscript::std_modules::DynValue;
use wscript::{Fault, Module};

use crate::ai_module::Ai;
use crate::{Kind, Reply, Vault, convert, host_call, run};

/// The largest image downloaded or stored.
pub const IMAGE_LIMIT: u64 = 5 * 1024 * 1024;

/// How long one image download may take, redirects included.
pub const IMAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// The most redirects one image download follows.
pub const IMAGE_REDIRECTS: usize = 3;

/// The functions this module adds to `vault`.
pub fn register(m: &mut Module, v: &Arc<dyn Vault>, ai: &Ai) {
    let (vv, a) = (v.clone(), ai.clone());
    m.fn_named(
        "html_to_blocks",
        ["html", "options"],
        move |html: String, options: DynValue| -> Reply<DynValue> {
            Fault(host_call(|_, _| {
                let o = HostOptions::read(convert::from_dyn(options))?;
                let html = if o.base64 { decode(&html)? } else { html };
                let mut images = HostImages::new(&vv, &a, &o);
                let c = if o.detect && !looks_like_html(&html) {
                    html_blocks::text_to_blocks(&html, &o.conv)
                } else {
                    html_blocks::html_to_blocks(&html, &o.conv, &mut images)
                };
                Ok(convert::to_dyn(json!({
                    "blocks": c.blocks,
                    "images": images.report,
                    "warnings": c.warnings,
                    "cut": c.cut,
                    "pending": c.pending,
                })))
            }))
        },
    );
    m.fn_named(
        "text_to_blocks",
        ["text", "options"],
        move |text: String, options: DynValue| -> Reply<DynValue> {
            Fault(host_call(|_, _| {
                let o = HostOptions::read(convert::from_dyn(options))?;
                let text = if o.base64 { decode(&text)? } else { text };
                let c = html_blocks::text_to_blocks(&text, &o.conv);
                Ok(convert::to_dyn(json!({
                    "blocks": c.blocks,
                    "warnings": c.warnings,
                    "cut": c.cut,
                })))
            }))
        },
    );
    m.fn_named("blocks_to_html", ["blocks"], |blocks: DynValue| -> String {
        match convert::from_dyn(blocks) {
            Json::Array(b) => html_blocks::blocks_to_html(&b),
            b @ Json::Object(_) => html_blocks::blocks_to_html(&[b]),
            _ => String::new(),
        }
    });
    let (vv, a) = (v.clone(), ai.clone());
    m.fn_named(
        "complete_images",
        ["blocks", "options"],
        move |blocks: DynValue, options: DynValue| -> Reply<DynValue> {
            Fault(host_call(|_, _| {
                let mut o = HostOptions::read(convert::from_dyn(options))?;
                o.download = true;
                let blocks = match convert::from_dyn(blocks) {
                    Json::Array(b) => b,
                    _ => return Err("complete_images takes a list of blocks".into()),
                };
                let mut images = HostImages::new(&vv, &a, &o);
                let (out, pending) = html_blocks::complete_pending(&blocks, &mut images);
                Ok(convert::to_dyn(json!({
                    "changed": out != blocks,
                    "blocks": out,
                    "pending": pending,
                    "images": images.report,
                })))
            }))
        },
    );
}

/// Whether `s` holds a tag, so reads as HTML rather than plain text: a `<` before a letter, `/`
/// or `!`, and a `>` after it.
pub fn looks_like_html(s: &str) -> bool {
    let b = s.as_bytes();
    b.windows(2).enumerate().any(|(k, w)| {
        w[0] == b'<'
            && (w[1].is_ascii_alphabetic() || w[1] == b'/' || w[1] == b'!')
            && b[k..].contains(&b'>')
    })
}

/// Base64 in either alphabet, padded or not, as UTF-8 text.
fn decode(data: &str) -> Result<String, String> {
    let clean: String = data
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let bytes = STANDARD_NO_PAD
        .decode(clean)
        .map_err(|e| format!("the data is not base64: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The options a script passes, as `docs/scripts.md` lists them.
struct HostOptions {
    conv: Options,
    base64: bool,
    detect: bool,
    download: bool,
    cid: HashMap<String, String>,
    deadline: Option<i64>,
}

impl HostOptions {
    fn read(v: Json) -> Result<HostOptions, String> {
        let v = match v {
            Json::Null => json!({}),
            Json::Object(_) => v,
            _ => return Err("the options are a map".into()),
        };
        let flag = |k: &str| v[k].as_bool().unwrap_or(false);
        let mut conv = Options {
            quotes: flag("quotes"),
            ..Options::default()
        };
        if let Some(t) = v["cut_text"].as_str() {
            conv.cut_text = t.to_owned();
        }
        if let Some(n) = v["max_images"].as_u64() {
            conv.max_images = usize::try_from(n).unwrap_or(0).min(html_blocks::MAX_IMAGES);
        }
        let download = match v["images"].as_str() {
            None | Some("link") => false,
            Some("download") => true,
            Some(other) => {
                return Err(format!(
                    "images is {other:?}: it must be \"download\" or \"link\""
                ));
            }
        };
        let cid = v["cid"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, a)| Some((cid_key(k), a.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        Ok(HostOptions {
            conv,
            base64: flag("base64"),
            detect: flag("detect"),
            download,
            cid,
            deadline: v["deadline"].as_i64(),
        })
    }
}

/// A Content-ID as the `cid` option and a `cid:` URL compare: no angle brackets, lower case.
fn cid_key(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_ascii_lowercase()
}

thread_local! {
    /// The run this cache belongs to, by its deadline, and the asset stored for each URL and
    /// each image's bytes in it.
    static STORED: RefCell<(Option<Instant>, HashMap<String, String>)> =
        RefCell::new((None, HashMap::new()));
}

/// The asset stored for `key` earlier in this run.
fn stored(key: &str) -> Option<String> {
    let run = run::peek(|r| r.deadline);
    STORED.with(|s| {
        let mut s = s.borrow_mut();
        if s.0 != run {
            *s = (run, HashMap::new());
        }
        s.1.get(key).cloned()
    })
}

fn remember(key: String, asset: &str) {
    STORED.with(|s| {
        s.borrow_mut().1.insert(key, asset.to_owned());
    });
}

fn bytes_key(bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("bytes:{}:{:016x}", bytes.len(), h.finish())
}

/// The image kind `bytes` start as, as a file extension.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") && bytes.len() > 26 {
        Some("bmp")
    } else if bytes.len() > 12
        && &bytes[4..8] == b"ftyp"
        && matches!(&bytes[8..12], b"avif" | b"avis")
    {
        Some("avif")
    } else if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() > 6 {
        Some("ico")
    } else {
        None
    }
}

/// The size of a PNG or GIF from its header.
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG") && bytes.len() >= 24 {
        let n = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        return Some((n(16), n(20)));
    }
    if bytes.starts_with(b"GIF8") && bytes.len() >= 10 {
        let n = |i: usize| u32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
        return Some((n(6), n(8)));
    }
    None
}

/// A file name for an image from `url`: its last path segment, with the extension of its kind.
fn filename(url: Option<&str>, ext: &str) -> String {
    let segment = url
        .and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| u.path_segments()?.next_back().map(str::to_owned))
        .map(|s| percent_decode(&s))
        .unwrap_or_default();
    let base: String = segment
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .take(100)
        .collect();
    let stem = base.rsplit_once('.').map_or(base.as_str(), |(s, _)| s);
    let stem = if stem.trim().is_empty() || stem == "." || stem == ".." {
        "image"
    } else {
        stem
    };
    format!("{stem}.{ext}")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
        {
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(b[i + 1]) * 16 + hex(b[i + 2]));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Why a download got nothing, as a note's paragraph says it.
fn reason(e: &FetchError) -> String {
    match e {
        FetchError::LocalNetwork => "the address is on the local network".to_owned(),
        FetchError::TooManyRedirects => format!("it redirected more than {IMAGE_REDIRECTS} times"),
        FetchError::Timeout | FetchError::Stalled => {
            format!("it took longer than {} seconds", IMAGE_TIMEOUT.as_secs())
        }
        FetchError::Status(code, why) => format!("the server answered {code} {why}")
            .trim_end()
            .to_owned(),
        FetchError::BadUrl(m) | FetchError::Failed(m) => m.clone(),
    }
}

/// The images of one conversion: looked up, decoded or downloaded, and stored.
struct HostImages<'a> {
    vault: &'a Arc<dyn Vault>,
    ai: &'a Ai,
    download: bool,
    cid: &'a HashMap<String, String>,
    deadline: Option<i64>,
    /// The runtime and fetcher of this call's downloads, made at the first.
    fetch: Option<(tokio::runtime::Runtime, Fetcher)>,
    report: Vec<Json>,
}

impl<'a> HostImages<'a> {
    fn new(vault: &'a Arc<dyn Vault>, ai: &'a Ai, o: &'a HostOptions) -> HostImages<'a> {
        HostImages {
            vault,
            ai,
            download: o.download,
            cid: &o.cid,
            deadline: o.deadline,
            fetch: None,
            report: Vec::new(),
        }
    }

    fn note(&mut self, src: &Src, alt: &str, placed: &Placed) {
        let src = match src {
            Src::Cid(id) => format!("cid:{id}"),
            Src::Data { media_type, .. } => format!("data:{media_type}"),
            Src::Remote(u) => u.clone(),
        };
        let mut entry = json!({ "src": src, "alt": alt });
        match placed {
            Placed::Asset(id) => {
                entry["status"] = json!("stored");
                entry["asset"] = json!(id);
            }
            Placed::Link => entry["status"] = json!("linked"),
            Placed::Failed(why) => {
                entry["status"] = json!("failed");
                entry["reason"] = json!(why);
            }
            Placed::Pending => entry["status"] = json!("pending"),
            Placed::Skip => entry["status"] = json!("skipped"),
        }
        self.report.push(entry);
    }

    /// Store `bytes`, once per run.
    fn store(&self, bytes: Vec<u8>, url: Option<&str>) -> Placed {
        if bytes.len() as u64 > IMAGE_LIMIT {
            return Placed::Failed("it is larger than 5 MB".into());
        }
        let Some(ext) = sniff(&bytes) else {
            return Placed::Failed("it is not an image".into());
        };
        if dimensions(&bytes).is_some_and(|(w, h)| w <= 2 || h <= 2) {
            return Placed::Skip;
        }
        let key = bytes_key(&bytes);
        if let Some(id) = stored(&key) {
            return Placed::Asset(id);
        }
        match self.vault.upload_asset(&filename(url, ext), bytes) {
            Ok(a) => match a["id"].as_str() {
                Some(id) => {
                    remember(key, id);
                    Placed::Asset(id.to_owned())
                }
                None => Placed::Failed("the vault stored no asset".into()),
            },
            Err(e) => Placed::Failed(e),
        }
    }

    fn remote(&mut self, url: &str) -> Placed {
        if let Some(id) = stored(&format!("url:{url}")) {
            return Placed::Asset(id);
        }
        // Network only where a script reaches it: after_* handlers and tasks.
        if !matches!(
            crate::current_job(|j| j.kind),
            Some(Kind::After | Kind::Task)
        ) {
            return Placed::Link;
        }
        if self.deadline.is_some_and(|d| self.vault.now_ms() >= d) {
            return Placed::Pending;
        }
        let left =
            run::peek(|r| r.deadline.saturating_duration_since(Instant::now())).unwrap_or_default();
        if left < Duration::from_secs(2) {
            return Placed::Pending;
        }
        let timeout = IMAGE_TIMEOUT.min(left - Duration::from_secs(1));
        if self.fetch.is_none() {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => return Placed::Failed(format!("no runtime to fetch with: {e}")),
            };
            let fetcher = Fetcher::new(self.ai.fetch_private()).limited(timeout, IMAGE_REDIRECTS);
            self.fetch = Some((rt, fetcher));
        }
        let Some((rt, fetcher)) = &self.fetch else {
            return Placed::Failed("no fetcher".into());
        };
        let fetcher = fetcher.clone().limited(timeout, IMAGE_REDIRECTS);
        let got = rt.block_on(fetcher.get(url, IMAGE_LIMIT + 1));
        let placed = match got {
            Ok(f) if f.cut => Placed::Failed("it is larger than 5 MB".into()),
            Ok(f) => self.store(f.body, Some(f.url.as_str())),
            Err(e) => Placed::Failed(reason(&e)),
        };
        if let Placed::Asset(id) = &placed {
            remember(format!("url:{url}"), id);
        }
        placed
    }
}

impl Images for HostImages<'_> {
    fn place(&mut self, src: &Src, alt: &str) -> Placed {
        let placed = if !self.download {
            Placed::Link
        } else {
            match src {
                Src::Cid(id) => match self.cid.get(&cid_key(id)) {
                    Some(asset) => Placed::Asset(asset.clone()),
                    None => Placed::Failed("the message holds no such part".into()),
                },
                Src::Data { bytes, .. } => self.store(bytes.clone(), None),
                Src::Remote(url) => self.remote(url),
            }
        };
        self.note(src, alt, &placed);
        placed
    }
}

#[cfg(test)]
mod tests {
    use super::{dimensions, filename, looks_like_html, sniff};

    #[test]
    fn html_is_text_with_a_tag() {
        assert!(looks_like_html("a <b>c</b>"));
        assert!(looks_like_html("<!-- x -->"));
        assert!(!looks_like_html("a < b and c > d"));
        assert!(!looks_like_html("1<2"));
    }

    #[test]
    fn images_are_known_by_their_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(sniff(b"GIF89a.."), Some("gif"));
        assert_eq!(sniff(b"<html>"), None);
        assert_eq!(dimensions(b"GIF89a\x01\x00\x01\x00"), Some((1, 1)));
        assert_eq!(
            filename(Some("https://e.example/a/hero%20shot.jpeg?x=1"), "jpg"),
            "hero shot.jpg"
        );
        assert_eq!(filename(Some("https://e.example/"), "png"), "image.png");
        assert_eq!(filename(None, "gif"), "image.gif");
    }
}
