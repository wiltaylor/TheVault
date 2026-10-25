//! The Anthropic Messages client (ai D1, D3, D5).
//!
//! Requests carry `anthropic-version: 2026-10-21`, which rejects `temperature`, `top_p` and
//! `top_k`, so none is ever sent; nor is `fallbacks`: a refusal ends the call (ai D3, D11).
//! `max_tokens` is 64000 streamed and 16000 not, unless the request sets it. `thinking:
//! {type: "adaptive"}` and `output_config.effort` are sent only when the request sets an
//! effort; a schema goes in `output_config.format`. Tools are strict, and streamed requests
//! turn on `eager_input_streaming`, so each tool input is parsed strictly at
//! `content_block_stop` and checked against its schema here.
//!
//! An assistant turn whose provider state came from the same model is replayed as its content
//! blocks, `thinking` with its `signature` included (ai D10).

use std::collections::{HashMap, VecDeque};

use serde_json::{Map, Value, json};

use crate::credentials::Credential;
use crate::error::{AiError, ErrorKind, kind_of_code};
use crate::turn::{Turn, parse_input};
use crate::types::{
    Content, Event, Message, Model, Part, ProviderState, Request, Role, Stop, Wire, unloaded,
};

/// The API version every request names.
pub const VERSION: &str = "2026-10-21";

/// The output cap of a streamed request that sets none.
pub const STREAM_MAX_TOKENS: u32 = 64000;

/// The output cap of a non-streamed request that sets none.
pub const MAX_TOKENS: u32 = 16000;

/// The request body.
pub(crate) fn body(cred: &Credential, req: &Request, stream: bool) -> Value {
    let max = req.max_tokens.unwrap_or(if stream {
        STREAM_MAX_TOKENS
    } else {
        MAX_TOKENS
    });
    let mut b = Map::new();
    b.insert("model".into(), json!(req.model));
    b.insert("max_tokens".into(), json!(max));
    if let Some(system) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        b.insert("system".into(), json!(system));
    }
    b.insert("messages".into(), Value::Array(messages(cred, req)));
    if stream {
        b.insert("stream".into(), json!(true));
    }
    if !req.tools.is_empty() {
        let tools = req
            .tools
            .iter()
            .map(|t| {
                let mut tool = json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                    "strict": true,
                });
                if stream {
                    tool["eager_input_streaming"] = json!(true);
                }
                tool
            })
            .collect();
        b.insert("tools".into(), Value::Array(tools));
    }
    let mut output = Map::new();
    if let Some(effort) = req.effort {
        b.insert("thinking".into(), json!({ "type": "adaptive" }));
        output.insert("effort".into(), json!(effort.as_str()));
    }
    if let Some(schema) = &req.schema {
        output.insert(
            "format".into(),
            json!({ "type": "json_schema", "schema": schema }),
        );
    }
    if !output.is_empty() {
        b.insert("output_config".into(), Value::Object(output));
    }
    Value::Object(b)
}

fn messages(cred: &Credential, req: &Request) -> Vec<Value> {
    req.messages
        .iter()
        .filter_map(|m| {
            let content = match m.role {
                Role::Assistant => assistant_content(cred, req, m),
                Role::User => user_content(m),
            };
            if content.is_empty() {
                return None;
            }
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            Some(json!({ "role": role, "content": content }))
        })
        .collect()
}

fn assistant_content(cred: &Credential, req: &Request, m: &Message) -> Vec<Value> {
    if let Some(state) = &m.provider_state
        && cred.replays(state, &req.model)
        && let Some(blocks) = state.output.as_array()
    {
        return blocks.clone();
    }
    m.content
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } if !text.is_empty() => {
                Some(json!({ "type": "text", "text": text }))
            }
            Content::ToolCall { id, name, input } => Some(json!({
                "type": "tool_use", "id": id, "name": name, "input": object_or_empty(input),
            })),
            _ => None,
        })
        .collect()
}

/// Tool results first, as the API wants them, then the text.
fn user_content(m: &Message) -> Vec<Value> {
    let results = m.content.iter().filter_map(|c| match c {
        Content::ToolResult {
            id,
            content,
            is_error,
            parts,
        } => Some(json!({
            "type": "tool_result", "tool_use_id": id, "content": result_content(content, parts),
            "is_error": is_error,
        })),
        _ => None,
    });
    let texts = m.content.iter().filter_map(|c| match c {
        Content::Text { text } if !text.is_empty() => Some(json!({ "type": "text", "text": text })),
        _ => None,
    });
    results.chain(texts).collect()
}

/// A tool result's `content`: the text alone, or with parts (ai D19), an array of blocks, the
/// text first, each image an `image` block with a base64 source.
fn result_content(content: &str, parts: &[Part]) -> Value {
    if parts.is_empty() {
        return json!(content);
    }
    let mut blocks = Vec::with_capacity(parts.len() + 1);
    if !content.is_empty() {
        blocks.push(json!({ "type": "text", "text": content }));
    }
    for p in parts {
        blocks.push(match p {
            Part::Text { text } => json!({ "type": "text", "text": text }),
            Part::Image { data: Some(d), .. } => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": d.media_type, "data": d.base64 },
            }),
            Part::Image {
                asset_id,
                data: None,
                ..
            } => json!({ "type": "text", "text": unloaded(asset_id) }),
        });
    }
    Value::Array(blocks)
}

fn object_or_empty(v: &Value) -> Value {
    if v.is_object() { v.clone() } else { json!({}) }
}

fn stop_of(reason: Option<&str>, details: Option<&Value>) -> Stop {
    match reason.unwrap_or("end_turn") {
        "tool_use" => Stop::ToolUse,
        "max_tokens" | "model_context_window_exceeded" => Stop::MaxTokens,
        "refusal" => {
            let field = |k: &str| {
                details
                    .and_then(|d| d.get(k))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            Stop::Refusal {
                category: field("category"),
                explanation: field("explanation"),
            }
        }
        _ => Stop::EndTurn,
    }
}

/// Input tokens with the cached ones counted in.
fn input_tokens(usage: &Value) -> Option<u64> {
    let n = |k: &str| usage.get(k).and_then(Value::as_u64);
    let base = n("input_tokens")?;
    Some(
        base + n("cache_creation_input_tokens").unwrap_or(0)
            + n("cache_read_input_tokens").unwrap_or(0),
    )
}

fn state(account: &str, model: &str, blocks: Vec<Value>) -> ProviderState {
    ProviderState {
        account: account.to_owned(),
        wire: Wire::Messages,
        model: model.to_owned(),
        output: Value::Array(blocks),
    }
}

/// A whole, non-streamed message as events.
pub(crate) fn parse_message(
    cred: &Credential,
    req: &Request,
    msg: &Value,
) -> Result<VecDeque<Event>, AiError> {
    let mut out = VecDeque::new();
    let mut turn = Turn::new(req);
    let blocks = msg
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            AiError::invalid(format!("{} answered a message with no content", cred.name))
        })?;
    for b in &blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = b.get("text").and_then(Value::as_str).unwrap_or_default();
                if !text.is_empty() {
                    out.push_back(Event::TextDelta(text.to_owned()));
                }
            }
            Some("tool_use") => {
                let id = b.get("id").and_then(Value::as_str).unwrap_or_default();
                let name = b.get("name").and_then(Value::as_str).unwrap_or_default();
                let input = b.get("input").cloned().unwrap_or_else(|| json!({}));
                turn.tool_call(id, name, Ok(input));
            }
            _ => {}
        }
    }
    if let Some(usage) = msg.get("usage") {
        turn.usage.input = input_tokens(usage).unwrap_or(0);
        turn.usage.output = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    let stop = stop_of(
        msg.get("stop_reason").and_then(Value::as_str),
        msg.get("stop_details"),
    );
    turn.finish(
        stop,
        Some(state(&cred.account, &req.model, blocks)),
        &mut out,
    );
    Ok(out)
}

/// The streamed message's events, as its frames arrive.
pub(crate) struct StreamDecoder {
    account: String,
    name: String,
    model: String,
    turn: Turn,
    blocks: Vec<Value>,
    partial: HashMap<usize, String>,
    stop: Option<Stop>,
    pub done: bool,
}

impl StreamDecoder {
    pub fn new(cred: &Credential, req: &Request) -> StreamDecoder {
        StreamDecoder {
            account: cred.account.clone(),
            name: cred.name.clone(),
            model: req.model.clone(),
            turn: Turn::new(req),
            blocks: Vec::new(),
            partial: HashMap::new(),
            stop: None,
            done: false,
        }
    }

    pub fn usage(&self) -> crate::Usage {
        self.turn.usage
    }

    fn block(&mut self, index: usize) -> &mut Value {
        if self.blocks.len() <= index {
            self.blocks.resize(index + 1, json!({}));
        }
        &mut self.blocks[index]
    }

    pub fn frame(&mut self, data: &Value, out: &mut VecDeque<Event>) -> Result<(), AiError> {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        match data.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message_start" => {
                if let Some(usage) = data.pointer("/message/usage") {
                    self.turn.usage.input = input_tokens(usage).unwrap_or(0);
                    self.turn.usage.output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            "content_block_start" => {
                let block = data.get("content_block").cloned().unwrap_or_default();
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    self.partial.insert(index, String::new());
                }
                *self.block(index) = block;
            }
            "content_block_delta" => {
                let delta = data.get("delta").cloned().unwrap_or_default();
                let s = |k: &str| delta.get(k).and_then(Value::as_str).unwrap_or_default();
                match s("type") {
                    "text_delta" => {
                        let text = s("text").to_owned();
                        append(self.block(index), "text", &text);
                        if !text.is_empty() {
                            out.push_back(Event::TextDelta(text));
                        }
                    }
                    "input_json_delta" => {
                        self.partial
                            .entry(index)
                            .or_default()
                            .push_str(s("partial_json"));
                    }
                    "thinking_delta" => {
                        let t = s("thinking").to_owned();
                        append(self.block(index), "thinking", &t);
                    }
                    "signature_delta" => {
                        let sig = s("signature").to_owned();
                        append(self.block(index), "signature", &sig);
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(raw) = self.partial.remove(&index) {
                    let parsed = parse_input(&raw);
                    let block = self.block(index);
                    let id = block["id"].as_str().unwrap_or_default().to_owned();
                    let name = block["name"].as_str().unwrap_or_default().to_owned();
                    block["input"] = match &parsed {
                        Ok(v) => object_or_empty(v),
                        Err(_) => json!({}),
                    };
                    self.turn.tool_call(&id, &name, parsed);
                }
            }
            "message_delta" => {
                let details = data
                    .pointer("/delta/stop_details")
                    .or_else(|| data.get("stop_details"))
                    .filter(|d| !d.is_null());
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop = Some(stop_of(Some(reason), details));
                }
                if let Some(usage) = data.get("usage") {
                    if let Some(o) = usage.get("output_tokens").and_then(Value::as_u64) {
                        self.turn.usage.output = o;
                    }
                    if let Some(i) = input_tokens(usage) {
                        self.turn.usage.input = i;
                    }
                }
            }
            "message_stop" => {
                let stop = self.stop.take().unwrap_or(Stop::EndTurn);
                let blocks = std::mem::take(&mut self.blocks);
                self.turn
                    .finish(stop, Some(state(&self.account, &self.model, blocks)), out);
                self.done = true;
            }
            "error" => {
                let error = data.get("error").cloned().unwrap_or_default();
                let code = error
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the answer stream reported an error");
                let kind = kind_of_code(code).unwrap_or(ErrorKind::Provider);
                return Err(
                    AiError::new(kind, format!("{} {}: {message}", self.name, self.model))
                        .with_usage(self.turn.usage),
                );
            }
            _ => {}
        }
        Ok(())
    }
}

fn append(block: &mut Value, key: &str, text: &str) {
    if let Some(obj) = block.as_object_mut() {
        let mut now = obj
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        now.push_str(text);
        obj.insert(key.to_owned(), Value::String(now));
    }
}

/// One page of `GET /v1/models`: the models, and the id to continue after when there are more.
pub(crate) fn models_page(page: &Value) -> (Vec<Model>, Option<String>) {
    let models = page
        .get("data")
        .and_then(Value::as_array)
        .map(|d| {
            d.iter()
                .filter_map(|m| {
                    Some(Model {
                        id: m.get("id")?.as_str()?.to_owned(),
                        display_name: m
                            .get("display_name")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let more = page.get("has_more").and_then(Value::as_bool) == Some(true);
    let last = page
        .get("last_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    (models, last.filter(|_| more))
}
