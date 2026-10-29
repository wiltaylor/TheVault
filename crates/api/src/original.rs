//! `GET /api/notes/{id}/original`: the HTML a note was imported from, as an email's View original
//! shows it (rich-import H6). The note's `html` asset is served as `text/html` only here, never by
//! the assets route, which serves HTML as plain text.
//!
//! - Each `cid:` reference the note's `inline_images` names (`<asset id> <Content-ID>`) becomes a
//!   `data:` URI of the stored image, so the page needs no request of its own to show it.
//! - The answer's policy is a sandbox with no scripts, forms or plugins, no requests but `data:`
//!   images and inline styles, and no referrer. The web client puts the page in a sandboxed
//!   iframe's `srcdoc` with a policy of its own, which lets remote images in only when the user
//!   asks.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use schema::Value;
use writer::Writer;

use crate::assets::content_type;
use crate::{ApiError, blocking};

/// The policy the page is served with.
pub const ORIGINAL_CSP: &str = "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; form-action 'none'; base-uri 'none'";

fn not_found(message: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
}

/// `html` with each `cid:` URL that `images` names, by Content-ID ignoring case, replaced by its
/// `data:` URI, others left as they were, and without the `<meta http-equiv="refresh">` and
/// `<base>` elements that would send the frame elsewhere.
pub fn rewrite_cids(html: &str, images: &HashMap<String, String>) -> String {
    let away =
        regex::Regex::new(r#"(?is)<meta[^>]*http-equiv\s*=\s*["']?refresh[^>]*>|<base\b[^>]*>"#)
            .expect("the refresh pattern");
    let html = away.replace_all(html, "");
    let re = regex::Regex::new(r#"(?i)cid:([^"'\s>)]+)"#).expect("the cid pattern");
    re.replace_all(&html, |c: &regex::Captures| {
        let id = c[1].trim_start_matches('<').trim_end_matches('>');
        let decoded = percent_decode(id).to_ascii_lowercase();
        images
            .get(&decoded)
            .cloned()
            .unwrap_or_else(|| c[0].to_owned())
    })
    .into_owned()
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

/// The page of note `id`: its `html` asset with its inline images in place.
fn page(w: &Writer, id: &str) -> Result<String, ApiError> {
    let note = w.read(id).map_err(ApiError::from)?;
    let Some(Value::Str(html_id)) = note.doc.field("html") else {
        return Err(not_found("this note holds no original HTML"));
    };
    let (_, bytes) = w.asset(html_id).map_err(ApiError::from)?;
    let mut images = HashMap::new();
    if let Some(Value::List(entries)) = note.doc.field("inline_images") {
        for e in entries {
            let Value::Str(e) = e else { continue };
            let Some((asset, cid)) = e.split_once(' ') else {
                continue;
            };
            let Ok((a, data)) = w.asset(asset.trim()) else {
                continue;
            };
            let media = content_type(&a.filename);
            if !media.starts_with("image/") || media == "image/svg+xml" {
                continue;
            }
            images.insert(
                cid.trim().to_ascii_lowercase(),
                format!("data:{media};base64,{}", STANDARD.encode(data)),
            );
        }
    }
    Ok(rewrite_cids(&String::from_utf8_lossy(&bytes), &images))
}

/// `GET /api/notes/{id}/original`.
pub async fn original(
    State(w): State<Arc<Writer>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let html = blocking(w, move |w| page(w, &id)).await?;
    let mut res = html.into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(ORIGINAL_CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::rewrite_cids;

    #[test]
    fn cid_urls_become_the_stored_images() {
        let images = HashMap::from([(
            "logo@mail.example".to_owned(),
            "data:image/png;base64,AA".to_owned(),
        )]);
        assert_eq!(
            rewrite_cids(
                r#"<img src="cid:Logo%40mail.example"><img src='cid:other'><p>cid: text</p>"#,
                &images
            ),
            r#"<img src="data:image/png;base64,AA"><img src='cid:other'><p>cid: text</p>"#
        );
        assert_eq!(
            rewrite_cids(
                r#"<META HTTP-EQUIV="Refresh" content="0;url=https://e.example"><base href="https://e.example/"><p>x</p>"#,
                &images
            ),
            "<p>x</p>"
        );
    }
}
