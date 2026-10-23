//! The images a request to a fake provider carries (ai D19), read and checked as the real API
//! reads them: Anthropic's `image` blocks with a base64 source, and OpenAI's `input_image`
//! items with a base64 data URL, in a tool result or a user message.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

/// The media types both providers take.
const MEDIA_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// An image a request sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeImage {
    /// The tool call whose result held it, or empty for an image in a user message.
    pub tool_call_id: String,
    pub media_type: String,
    /// The decoded bytes.
    pub bytes: Vec<u8>,
}

fn decode(media_type: &str, data: &str, call: &str) -> Result<FakeImage, String> {
    if !MEDIA_TYPES.contains(&media_type) {
        return Err(format!("image media type {media_type:?} is not supported"));
    }
    if data.contains(['\n', '\r']) {
        return Err("image data must be base64 with no line breaks".to_owned());
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|e| format!("image data is not base64: {e}"))?;
    Ok(FakeImage {
        tool_call_id: call.to_owned(),
        media_type: media_type.to_owned(),
        bytes,
    })
}

fn anthropic_block(b: &Value, call: &str, out: &mut Vec<FakeImage>) -> Result<(), String> {
    if b["type"] != "image" {
        return Ok(());
    }
    let source = &b["source"];
    if source["type"] != "base64" {
        return Err("image source.type must be base64".to_owned());
    }
    let media = source["media_type"].as_str().unwrap_or_default();
    let data = source["data"]
        .as_str()
        .ok_or("image source.data is required")?;
    out.push(decode(media, data, call)?);
    Ok(())
}

/// The images of a Messages request body.
pub(crate) fn anthropic_images(body: &Value) -> Result<Vec<FakeImage>, String> {
    let mut out = Vec::new();
    for m in body["messages"].as_array().into_iter().flatten() {
        for c in m["content"].as_array().into_iter().flatten() {
            if c["type"] == "tool_result" {
                let call = c["tool_use_id"].as_str().unwrap_or_default();
                for b in c["content"].as_array().into_iter().flatten() {
                    anthropic_block(b, call, &mut out)?;
                }
            } else {
                anthropic_block(c, "", &mut out)?;
            }
        }
    }
    Ok(out)
}

fn data_url(url: &str, call: &str, out: &mut Vec<FakeImage>) -> Result<(), String> {
    let rest = url
        .strip_prefix("data:")
        .ok_or("this fake takes only data URLs")?;
    let (media, data) = rest
        .split_once(";base64,")
        .ok_or("the data URL must be base64")?;
    out.push(decode(media, data, call)?);
    Ok(())
}

fn openai_item(i: &Value, call: &str, out: &mut Vec<FakeImage>) -> Result<(), String> {
    if i["type"] != "input_image" {
        return Ok(());
    }
    let url = i["image_url"]
        .as_str()
        .ok_or("input_image needs an image_url")?;
    data_url(url, call, out)
}

/// The images of a Chat Completions request body (ai D25): `image_url` parts of `user`
/// messages. A `tool` message takes text only, so an array there is a 400.
pub(crate) fn chat_images(body: &Value) -> Result<Vec<FakeImage>, String> {
    let mut out = Vec::new();
    for m in body["messages"].as_array().into_iter().flatten() {
        match (&m["role"], &m["content"]) {
            (r, Value::Array(_)) if r == "tool" => {
                return Err("a tool message's content must be text".into());
            }
            (_, Value::Array(parts)) => {
                for p in parts {
                    if p["type"] == "image_url" {
                        let url = p["image_url"]["url"]
                            .as_str()
                            .ok_or("image_url needs a url")?;
                        data_url(url, "", &mut out)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The images of a Responses request body.
pub(crate) fn openai_images(body: &Value) -> Result<Vec<FakeImage>, String> {
    let mut out = Vec::new();
    for item in body["input"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("function_call_output") => {
                let call = item["call_id"].as_str().unwrap_or_default();
                match &item["output"] {
                    Value::String(_) => {}
                    Value::Array(parts) => {
                        for p in parts {
                            openai_item(p, call, &mut out)?;
                        }
                    }
                    _ => {
                        return Err(
                            "function_call_output.output must be a string or an array".into()
                        );
                    }
                }
            }
            Some("message") => {
                for p in item["content"].as_array().into_iter().flatten() {
                    openai_item(p, "", &mut out)?;
                }
            }
            _ => {}
        }
    }
    Ok(out)
}
