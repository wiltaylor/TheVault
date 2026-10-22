//! The OpenAI Responses client, used with an API key and with a ChatGPT plan token (ai D1, D3,
//! D4).
//!
//! Every request sends `store: false`, since the vault keeps no state at the provider, the whole
//! history in `input`, the system prompt as `instructions`, and `include:
//! ["reasoning.encrypted_content"]` so reasoning can be replayed. A ChatGPT plan request always
//! streams and never sends `max_output_tokens`, `previous_response_id`, `metadata`, `user`,
//! `truncation`, `background`, `conversation`, `max_tool_calls`, hosted tools or sampling
//! parameters, and counts as done only once `response.completed` arrives. An API-key request
//! sends `max_output_tokens` when the request sets a cap.
//!
//! An assistant turn whose provider state came from the same model is replayed as its output
//! items, `reasoning` with its `encrypted_content` included, each without its `id`: with
//! `store: false` OpenAI keeps no item an id could name (ai D10).

use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use crate::error::{AiError, ErrorKind, kind_of_code};
use crate::turn::{Turn, parse_input};
use crate::types::{
    Content, Effort, Event, Message, Model, Provider, ProviderState, Request, Role, Stop,
};

/// The request body. `plan` applies the ChatGPT plan's rules.
pub(crate) fn body(req: &Request, plan: bool, stream: bool) -> Value {
    let mut b = Map::new();
    b.insert("model".into(), json!(req.model));
    if let Some(system) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        b.insert("instructions".into(), json!(system));
    }
    b.insert("input".into(), Value::Array(input(req)));
    b.insert("store".into(), json!(false));
    b.insert("stream".into(), json!(plan || stream));
    b.insert("include".into(), json!(["reasoning.encrypted_content"]));
    if !req.tools.is_empty() {
        let tools = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                    "strict": true,
                })
            })
            .collect();
        b.insert("tools".into(), Value::Array(tools));
    }
    if let Some(schema) = &req.schema {
        b.insert(
            "text".into(),
            json!({ "format": { "type": "json_schema", "name": "reply", "schema": schema, "strict": true } }),
        );
    }
    if let Some(effort) = req.effort {
        let effort = match effort {
            Effort::Max => "xhigh",
            e => e.as_str(),
        };
        b.insert("reasoning".into(), json!({ "effort": effort }));
    }
    if !plan && let Some(max) = req.max_tokens {
        b.insert("max_output_tokens".into(), json!(max));
    }
    Value::Object(b)
}

fn input(req: &Request) -> Vec<Value> {
    let mut items = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::User => user_items(m, &mut items),
            Role::Assistant => assistant_items(req, m, &mut items),
        }
    }
    items
}

fn user_items(m: &Message, items: &mut Vec<Value>) {
    for c in &m.content {
        if let Content::ToolResult {
            id,
            content,
            is_error,
        } = c
        {
            let output = if *is_error {
                format!("Error: {content}")
            } else {
                content.clone()
            };
            items.push(json!({ "type": "function_call_output", "call_id": id, "output": output }));
        }
    }
    let texts: Vec<Value> = m
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } if !text.is_empty() => {
                Some(json!({ "type": "input_text", "text": text }))
            }
            _ => None,
        })
        .collect();
    if !texts.is_empty() {
        items.push(json!({ "type": "message", "role": "user", "content": texts }));
    }
}

fn assistant_items(req: &Request, m: &Message, items: &mut Vec<Value>) {
    if let Some(state) = &m.provider_state
        && state.provider == req.provider
        && state.model == req.model
        && let Some(output) = state.output.as_array()
    {
        for item in output {
            let mut item = item.clone();
            if let Some(obj) = item.as_object_mut() {
                obj.remove("id");
            }
            items.push(item);
        }
        return;
    }
    for c in &m.content {
        match c {
            Content::Text { text } if !text.is_empty() => items.push(json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": text }],
            })),
            Content::ToolCall { id, name, input } => items.push(json!({
                "type": "function_call",
                "call_id": id,
                "name": name,
                "arguments": input.to_string(),
            })),
            _ => {}
        }
    }
}

fn usage_of(turn: &mut Turn, response: &Value) {
    if let Some(u) = response.get("usage").filter(|u| u.is_object()) {
        turn.usage.input = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        turn.usage.output = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
    }
}

/// The error a failed response carries.
fn failure(req: &Request, error: Option<&Value>, turn: &Turn) -> AiError {
    let code = error
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = error
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("the response failed");
    let kind = kind_of_code(code).unwrap_or(ErrorKind::Provider);
    let mut text = format!("{} {}: {message}", req.provider.label(), req.model);
    if code == "subscription_sharing_usage_limit_exceeded" {
        text.push_str(". See your ChatGPT usage at https://chatgpt.com/settings/usage");
    }
    AiError::new(kind, text).with_usage(turn.usage)
}

/// The tool calls and refusal among the output items, then the end of the turn.
fn finish_items(
    req: &Request,
    turn: &mut Turn,
    items: Vec<Value>,
    incomplete: Option<&str>,
    out: &mut VecDeque<Event>,
) -> Result<(), AiError> {
    let mut refusal: Option<String> = None;
    let mut calls = false;
    for item in &items {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                calls = true;
                let s = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default();
                turn.tool_call(s("call_id"), s("name"), parse_input(s("arguments")));
            }
            Some("message") => {
                for part in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if part.get("type").and_then(Value::as_str) == Some("refusal") {
                        let text = part.get("refusal").and_then(Value::as_str).unwrap_or("");
                        refusal.get_or_insert_with(String::new).push_str(text);
                    }
                }
            }
            _ => {}
        }
    }
    let stop = match (refusal, incomplete) {
        (Some(text), _) => Stop::Refusal {
            category: None,
            explanation: Some(text).filter(|t| !t.is_empty()),
        },
        (None, Some("content_filter")) => Stop::Refusal {
            category: Some("content_filter".to_owned()),
            explanation: None,
        },
        (None, Some(reason)) if req.provider == Provider::ChatGpt => {
            return Err(AiError::new(
                ErrorKind::Provider,
                format!(
                    "ChatGPT {}: the response ended incomplete ({reason}) before response.completed",
                    req.model
                ),
            )
            .with_usage(turn.usage));
        }
        (None, Some(_)) => Stop::MaxTokens,
        (None, None) if calls => Stop::ToolUse,
        (None, None) => Stop::EndTurn,
    };
    let state = ProviderState {
        provider: req.provider,
        model: req.model.clone(),
        output: Value::Array(items),
    };
    turn.finish(stop, Some(state), out);
    Ok(())
}

/// A whole, non-streamed response as events.
pub(crate) fn parse_response(req: &Request, resp: &Value) -> Result<VecDeque<Event>, AiError> {
    let mut out = VecDeque::new();
    let mut turn = Turn::new(req);
    usage_of(&mut turn, resp);
    let status = resp
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    if status == "failed" {
        return Err(failure(req, resp.get("error"), &turn));
    }
    let items = resp
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in &items {
        if item.get("type").and_then(Value::as_str) == Some("message") {
            for part in item
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if part.get("type").and_then(Value::as_str) == Some("output_text")
                    && let Some(t) = part.get("text").and_then(Value::as_str)
                    && !t.is_empty()
                {
                    out.push_back(Event::TextDelta(t.to_owned()));
                }
            }
        }
    }
    let incomplete = (status == "incomplete").then(|| {
        resp.pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    });
    finish_items(req, &mut turn, items, incomplete, &mut out)?;
    Ok(out)
}

/// The streamed response's events, as its frames arrive.
pub(crate) struct StreamDecoder {
    req: Request,
    turn: Turn,
    items: Vec<(u64, Value)>,
    pub done: bool,
}

impl StreamDecoder {
    pub fn new(req: &Request) -> StreamDecoder {
        // The decoder needs the provider, model and tools, not the history.
        let req = Request {
            messages: Vec::new(),
            ..req.clone()
        };
        StreamDecoder {
            turn: Turn::new(&req),
            req,
            items: Vec::new(),
            done: false,
        }
    }

    pub fn usage(&self) -> crate::Usage {
        self.turn.usage
    }

    pub fn frame(&mut self, data: &Value, out: &mut VecDeque<Event>) -> Result<(), AiError> {
        match data.get("type").and_then(Value::as_str).unwrap_or_default() {
            "response.output_text.delta" => {
                if let Some(d) = data.get("delta").and_then(Value::as_str)
                    && !d.is_empty()
                {
                    out.push_back(Event::TextDelta(d.to_owned()));
                }
            }
            "response.output_item.done" => {
                let index = data
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.items.len() as u64);
                if let Some(item) = data.get("item") {
                    self.items.push((index, item.clone()));
                }
            }
            kind @ ("response.completed" | "response.incomplete") => {
                let resp = data.get("response").cloned().unwrap_or_default();
                usage_of(&mut self.turn, &resp);
                // The final output is the record; the items seen streaming stand in when the
                // provider leaves it out.
                let items = match resp.get("output").and_then(Value::as_array) {
                    Some(o) if !o.is_empty() => o.clone(),
                    _ => {
                        let mut seen = std::mem::take(&mut self.items);
                        seen.sort_by_key(|(i, _)| *i);
                        seen.into_iter().map(|(_, v)| v).collect()
                    }
                };
                let incomplete = (kind == "response.incomplete").then(|| {
                    resp.pointer("/incomplete_details/reason")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                });
                self.done = true;
                finish_items(&self.req, &mut self.turn, items, incomplete, out)?;
            }
            "response.failed" => {
                let resp = data.get("response").cloned().unwrap_or_default();
                usage_of(&mut self.turn, &resp);
                self.done = true;
                return Err(failure(&self.req, resp.get("error"), &self.turn));
            }
            "error" => {
                self.done = true;
                let error = match data.get("error") {
                    Some(e) if e.is_object() => e.clone(),
                    _ => data.clone(),
                };
                return Err(failure(&self.req, Some(&error), &self.turn));
            }
            _ => {}
        }
        Ok(())
    }
}

/// The models of `GET /v1/models`: `data[].id` with an API key, and `models[]` with
/// `visibility == "list"` with a ChatGPT plan token (ai D6).
pub(crate) fn models(provider: Provider, page: &Value) -> Vec<Model> {
    let mut models: Vec<Model> = if provider == Provider::ChatGpt {
        page.get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|m| m.get("visibility").and_then(Value::as_str) == Some("list"))
            .filter_map(|m| {
                Some(Model {
                    id: m.get("slug")?.as_str()?.to_owned(),
                    display_name: m
                        .get("display_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect()
    } else {
        let mut m: Vec<Model> = page
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(Model {
                    id: m.get("id")?.as_str()?.to_owned(),
                    display_name: None,
                })
            })
            .collect();
        m.sort_by(|a, b| a.id.cmp(&b.id));
        m
    };
    models.dedup_by(|a, b| a.id == b.id);
    models
}
