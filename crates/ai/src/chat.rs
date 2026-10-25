//! The OpenAI Chat Completions client (ai D25), for an OpenAI-compatible service with a custom
//! API URL that offers no Responses API. It speaks the same [`Request`], [`Event`] and
//! [`AiError`] as the other clients.
//!
//! - `POST {root}/chat/completions` with the system prompt as the first `system` message, the
//!   history as `user`, `assistant` (with `tool_calls`, whose `arguments` are JSON strings) and
//!   `tool` messages, and tools as `{type: "function", function: {name, description,
//!   parameters}}` without `strict`, which many services reject. No sampling parameters (ai D3).
//! - A schema goes as `response_format: {type: "json_schema", ...}`. A service that refuses that
//!   with a 400 is asked once more with `{type: "json_object"}` and the schema in the system
//!   text ([`body`] with `json_object`); the reply is checked against the schema either way.
//! - A tool message takes text only, so the images of a tool result (ai D19) follow the tool
//!   messages in one `user` message of `image_url` parts with base64 data URLs.
//! - Streaming reads `data:` frames up to `data: [DONE]`: text deltas, tool calls accumulated by
//!   `index`, the `finish_reason`, a `refusal` delta, and the usage chunk `stream_options:
//!   {include_usage: true}` asks for. `reasoning_content` is not part of the answer.
//! - The provider state (ai D10) is the assistant message as it is sent back, `content` and
//!   `tool_calls`, so a replay is exact.

use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use crate::credentials::Credential;
use crate::error::{AiError, ErrorKind, kind_of_code};
use crate::turn::{Turn, Who, parse_input};
use crate::types::{
    Content, Effort, Event, Message, Part, ProviderState, Request, Role, Stop, Usage, Wire,
    unloaded,
};

/// The path a Chat Completions call posts to, under the API root.
pub(crate) const PATH: &str = "/v1/chat/completions";

/// The request body. `json_object` is the fallback for a service that refused `json_schema`:
/// `response_format: {type: "json_object"}` with the schema in the system text.
pub(crate) fn body(cred: &Credential, req: &Request, stream: bool, json_object: bool) -> Value {
    let mut b = Map::new();
    b.insert("model".into(), json!(req.model));
    b.insert(
        "messages".into(),
        Value::Array(messages(cred, req, json_object)),
    );
    if stream {
        b.insert("stream".into(), json!(true));
        b.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    if !req.tools.is_empty() {
        let tools = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    },
                })
            })
            .collect();
        b.insert("tools".into(), Value::Array(tools));
    }
    if let Some(max) = req.max_tokens {
        b.insert("max_tokens".into(), json!(max));
    }
    if let Some(schema) = &req.schema {
        let format = if json_object {
            json!({ "type": "json_object" })
        } else {
            json!({
                "type": "json_schema",
                "json_schema": { "name": "reply", "schema": schema, "strict": true },
            })
        };
        b.insert("response_format".into(), format);
    }
    if let Some(effort) = req.effort {
        let effort = match effort {
            Effort::Max => "xhigh",
            e => e.as_str(),
        };
        b.insert("reasoning_effort".into(), json!(effort));
    }
    Value::Object(b)
}

/// The system text of the `json_object` fallback: the request's own, then the schema.
fn system_text(req: &Request, json_object: bool) -> Option<String> {
    let own = req
        .system
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    match (&req.schema, json_object) {
        (Some(schema), true) => {
            let rule = format!(
                "Reply with one JSON object and nothing else. It must match this JSON Schema:\n{schema}"
            );
            Some(match own {
                Some(s) => format!("{s}\n\n{rule}"),
                None => rule,
            })
        }
        _ => own,
    }
}

fn messages(cred: &Credential, req: &Request, json_object: bool) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(system) = system_text(req, json_object) {
        out.push(json!({ "role": "system", "content": system }));
    }
    for m in &req.messages {
        match m.role {
            Role::User => user_messages(m, &mut out),
            Role::Assistant => {
                if let Some(msg) = assistant_message(cred, req, m) {
                    out.push(msg);
                }
            }
        }
    }
    out
}

/// A user turn: a `tool` message per tool result, then one `user` message holding every image
/// the results carried, then the user's text.
fn user_messages(m: &Message, out: &mut Vec<Value>) {
    let mut images = Vec::new();
    for c in &m.content {
        if let Content::ToolResult {
            id,
            content,
            is_error,
            parts,
        } = c
        {
            let mut texts = Vec::new();
            if !content.is_empty() {
                texts.push(if *is_error {
                    format!("Error: {content}")
                } else {
                    content.clone()
                });
            }
            let mut these = Vec::new();
            for p in parts {
                match p {
                    Part::Text { text } => texts.push(text.clone()),
                    Part::Image { data: Some(d), .. } => these.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", d.media_type, d.base64) },
                    })),
                    Part::Image {
                        asset_id,
                        data: None,
                        ..
                    } => texts.push(unloaded(asset_id)),
                }
            }
            if !these.is_empty() {
                let n = these.len();
                texts.push(format!(
                    "{n} image{} follow{} in the next user message.",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "s" } else { "" }
                ));
                images.push(json!({
                    "type": "text",
                    "text": format!("The image{} tool call {id} returned:", if n == 1 { "" } else { "s" }),
                }));
                images.extend(these);
            }
            out.push(json!({ "role": "tool", "tool_call_id": id, "content": texts.join("\n\n") }));
        }
    }
    if !images.is_empty() {
        out.push(json!({ "role": "user", "content": images }));
    }
    let texts: Vec<&str> = m
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if !texts.is_empty() {
        out.push(json!({ "role": "user", "content": texts.join("\n\n") }));
    }
}

/// Whether `state`'s output is a Chat Completions assistant message this request can replay.
fn replayable<'a>(cred: &Credential, req: &Request, m: &'a Message) -> Option<&'a Value> {
    let state = m.provider_state.as_ref()?;
    if !cred.replays(state, &req.model) {
        return None;
    }
    match state.output.as_array().map(Vec::as_slice) {
        Some([msg])
            if msg.get("role").and_then(Value::as_str) == Some("assistant")
                && msg.get("type").is_none() =>
        {
            Some(msg)
        }
        _ => None,
    }
}

fn assistant_message(cred: &Credential, req: &Request, m: &Message) -> Option<Value> {
    if let Some(msg) = replayable(cred, req, m) {
        return Some(msg.clone());
    }
    let text: String = m
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let calls: Vec<Value> = m
        .content
        .iter()
        .filter_map(|c| match c {
            Content::ToolCall { id, name, input } => Some(json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": input.to_string() },
            })),
            _ => None,
        })
        .collect();
    if text.is_empty() && calls.is_empty() {
        return None;
    }
    Some(assistant_json(&text, calls))
}

/// An assistant message: its text, or null when it has only tool calls, and its tool calls.
fn assistant_json(text: &str, calls: Vec<Value>) -> Value {
    let mut msg = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { json!(text) },
    });
    if !calls.is_empty() {
        msg["tool_calls"] = Value::Array(calls);
    }
    msg
}

/// The error an `{error: {code, message}}` body or frame carries.
fn failure(who: &Who, req: &Request, error: &Value, usage: Usage) -> AiError {
    let s = |k: &str| error.get(k).and_then(Value::as_str);
    let code = s("code").or_else(|| s("type")).unwrap_or_default();
    let message = s("message").unwrap_or("the request failed");
    let kind = kind_of_code(code).unwrap_or(ErrorKind::Provider);
    AiError::new(kind, format!("{} {}: {message}", who.name, req.model)).with_usage(usage)
}

/// A tool call coming in, piece by piece.
#[derive(Default)]
struct PartialCall {
    index: u64,
    id: String,
    name: String,
    arguments: String,
}

/// A Chat Completions answer as it arrives, streamed or whole.
pub(crate) struct StreamDecoder {
    who: Who,
    req: Request,
    turn: Turn,
    text: String,
    refusal: Option<String>,
    calls: Vec<PartialCall>,
    finish_reason: Option<String>,
    pub done: bool,
}

impl StreamDecoder {
    pub fn new(cred: &Credential, req: &Request) -> StreamDecoder {
        // The decoder needs the provider, model, tools and schema, not the history.
        let req = Request {
            messages: Vec::new(),
            ..req.clone()
        };
        StreamDecoder {
            who: Who::of(cred),
            turn: Turn::new(&req),
            req,
            text: String::new(),
            refusal: None,
            calls: Vec::new(),
            finish_reason: None,
            done: false,
        }
    }

    pub fn usage(&self) -> Usage {
        self.turn.usage
    }

    /// Whether a `finish_reason` arrived, so the end of the body ends the turn.
    pub fn finished(&self) -> bool {
        self.finish_reason.is_some()
    }

    fn usage_of(&mut self, data: &Value) {
        if let Some(u) = data.get("usage").filter(|u| u.is_object()) {
            self.turn.usage.input = u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0);
            self.turn.usage.output = u
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
        }
    }

    fn text(&mut self, t: &str, out: &mut VecDeque<Event>) {
        if !t.is_empty() {
            self.text.push_str(t);
            out.push_back(Event::TextDelta(t.to_owned()));
        }
    }

    fn refusal(&mut self, t: &str) {
        self.refusal.get_or_insert_with(String::new).push_str(t);
    }

    /// One `tool_calls[]` entry of a delta or a whole message.
    fn call(&mut self, position: usize, c: &Value) {
        let index = c
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or(position as u64);
        let at = match self.calls.iter().position(|p| p.index == index) {
            Some(i) => i,
            None => {
                self.calls.push(PartialCall {
                    index,
                    ..PartialCall::default()
                });
                self.calls.len() - 1
            }
        };
        let p = &mut self.calls[at];
        if let Some(id) = c
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            p.id = id.to_owned();
        }
        let f = c.get("function");
        if let Some(name) = f
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            p.name = name.to_owned();
        }
        if let Some(a) = f.and_then(|f| f.get("arguments")).and_then(Value::as_str) {
            p.arguments.push_str(a);
        }
    }

    /// One `data:` frame other than `[DONE]`.
    pub fn frame(&mut self, data: &Value, out: &mut VecDeque<Event>) -> Result<(), AiError> {
        if let Some(e) = data.get("error").filter(|e| e.is_object()) {
            self.done = true;
            return Err(failure(&self.who, &self.req, e, self.turn.usage));
        }
        self.usage_of(data);
        let Some(choice) = data
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            return Ok(());
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                self.text(t, out);
            }
            if let Some(r) = delta.get("refusal").and_then(Value::as_str) {
                self.refusal(r);
            }
            for (i, c) in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                self.call(i, c);
            }
        }
        if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(r.to_owned());
        }
        Ok(())
    }

    /// `data: [DONE]`, or the end of a body after the `finish_reason`: the turn is over.
    pub fn finish(&mut self, out: &mut VecDeque<Event>) -> Result<(), AiError> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        let calls = std::mem::take(&mut self.calls);
        let mut sent = Vec::new();
        for (i, c) in calls.iter().enumerate() {
            let id = if c.id.is_empty() {
                format!("call_{i}")
            } else {
                c.id.clone()
            };
            self.turn.tool_call(&id, &c.name, parse_input(&c.arguments));
            sent.push(json!({
                "id": id,
                "type": "function",
                "function": { "name": c.name, "arguments": c.arguments },
            }));
        }
        let stop = match (self.refusal.take(), self.finish_reason.as_deref()) {
            (Some(text), _) => Stop::Refusal {
                category: None,
                explanation: Some(text).filter(|t| !t.trim().is_empty()),
            },
            (None, Some("content_filter")) => Stop::Refusal {
                category: Some("content_filter".to_owned()),
                explanation: None,
            },
            (None, Some("length")) => Stop::MaxTokens,
            (None, _) if !sent.is_empty() => Stop::ToolUse,
            (None, _) => Stop::EndTurn,
        };
        let state = ProviderState {
            account: self.who.account.clone(),
            wire: Wire::ChatCompletions,
            model: self.req.model.clone(),
            output: json!([assistant_json(&self.text, sent)]),
        };
        self.turn.finish(stop, Some(state), out);
        Ok(())
    }
}

/// A whole, non-streamed `chat.completion` as events.
pub(crate) fn parse_completion(
    cred: &Credential,
    req: &Request,
    body: &Value,
) -> Result<VecDeque<Event>, AiError> {
    let mut out = VecDeque::new();
    let mut d = StreamDecoder::new(cred, req);
    if let Some(e) = body.get("error").filter(|e| e.is_object()) {
        return Err(failure(&d.who, req, e, Usage::default()));
    }
    d.usage_of(body);
    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .ok_or_else(|| {
            AiError::new(
                ErrorKind::Provider,
                format!("{} {}: the answer has no choices", d.who.name, req.model),
            )
            .with_usage(d.usage())
        })?;
    let msg = choice.get("message").cloned().unwrap_or_default();
    if let Some(t) = msg.get("content").and_then(Value::as_str) {
        d.text(t, &mut out);
    }
    if let Some(r) = msg.get("refusal").and_then(Value::as_str) {
        d.refusal(r);
    }
    for (i, c) in msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        d.call(i, c);
    }
    d.finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    d.finish(&mut out)?;
    Ok(out)
}
