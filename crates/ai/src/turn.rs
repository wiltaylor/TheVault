//! What both clients share while a turn comes in: checking each tool call, and the events that
//! end the turn.

use std::collections::VecDeque;

use serde_json::Value;

use crate::error::AiError;
use crate::schema;
use crate::types::{Event, ProviderState, Reply, Request, Stop, ToolCall, ToolSpec, Usage};

/// One turn in progress.
pub(crate) struct Turn {
    tools: Vec<ToolSpec>,
    calls: Vec<ToolCall>,
    pub usage: Usage,
}

impl Turn {
    pub fn new(req: &Request) -> Turn {
        Turn {
            tools: req.tools.clone(),
            calls: Vec::new(),
            usage: Usage::default(),
        }
    }

    /// A complete tool call: `input` is the parsed input, or the raw text that did not parse.
    pub fn tool_call(&mut self, id: &str, name: &str, input: Result<Value, (String, String)>) {
        let (input, error) = match input {
            Ok(v) => {
                let error = match self.tools.iter().find(|t| t.name == name) {
                    None => Some(format!("there is no tool named {name:?}")),
                    Some(t) => schema::validate(&t.input_schema, &v)
                        .err()
                        .map(|e| format!("the input does not match the tool's schema: {e}")),
                };
                (v, error)
            }
            Err((raw, why)) => (
                Value::String(raw),
                Some(format!("the input is not valid JSON: {why}")),
            ),
        };
        self.calls.push(ToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            input,
            error,
        });
    }

    /// End the turn: its tool calls when it stopped for them, then usage, then `Done`.
    pub fn finish(&mut self, stop: Stop, state: Option<ProviderState>, out: &mut VecDeque<Event>) {
        let calls = std::mem::take(&mut self.calls);
        if stop == Stop::ToolUse {
            out.extend(calls.into_iter().map(Event::ToolCall));
        }
        out.push_back(Event::Usage(self.usage));
        let provider_state = match stop {
            Stop::EndTurn | Stop::ToolUse => state,
            Stop::MaxTokens | Stop::Refusal { .. } => None,
        };
        out.push_back(Event::Done {
            stop,
            provider_state,
        });
    }
}

/// Parse a tool input strictly: empty is `{}`, anything else must be one JSON value.
pub(crate) fn parse_input(raw: &str) -> Result<Value, (String, String)> {
    if raw.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(raw).map_err(|e| (raw.to_owned(), e.to_string()))
}

/// Builds a [`Reply`] from a turn's events.
pub(crate) struct Collector {
    reply: Reply,
    schema: Option<Value>,
    done: bool,
}

impl Collector {
    pub fn new(req: &Request) -> Collector {
        Collector {
            reply: Reply {
                provider: req.provider,
                model: req.model.clone(),
                text: String::new(),
                json: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
                stop: Stop::EndTurn,
                provider_state: None,
            },
            schema: req.schema.clone(),
            done: false,
        }
    }

    pub fn event(&mut self, e: &Event) {
        match e {
            Event::TextDelta(t) => self.reply.text.push_str(t),
            Event::ToolCall(c) => self.reply.tool_calls.push(c.clone()),
            Event::Usage(u) => self.reply.usage = *u,
            Event::Done {
                stop,
                provider_state,
            } => {
                self.reply.stop = stop.clone();
                self.reply.provider_state = provider_state.clone();
                self.done = true;
            }
        }
    }

    /// The reply, or the refusal as an error (ai D11), or an `invalid` error when the text
    /// breaks the request's schema.
    pub fn finish(mut self) -> Result<Reply, AiError> {
        let r = &self.reply;
        if !self.done {
            return Err(AiError::new(
                crate::ErrorKind::Provider,
                "the answer ended before the turn did",
            )
            .on(r.provider, &r.model)
            .with_usage(r.usage));
        }
        if let Stop::Refusal {
            category,
            explanation,
        } = &r.stop
        {
            return Err(AiError::refusal(
                r.provider,
                &r.model,
                category.clone(),
                explanation.clone(),
            )
            .with_usage(r.usage));
        }
        if let Some(schema) = &self.schema
            && r.stop == Stop::EndTurn
        {
            let parsed: Value = serde_json::from_str(r.text.trim()).map_err(|e| {
                AiError::invalid(format!(
                    "the reply is not the JSON the schema asks for: {e}"
                ))
                .on(r.provider, &r.model)
                .with_usage(r.usage)
            })?;
            schema::validate(schema, &parsed).map_err(|e| {
                AiError::invalid(format!("the reply does not match the schema: {e}"))
                    .on(r.provider, &r.model)
                    .with_usage(r.usage)
            })?;
            self.reply.json = Some(parsed);
        }
        Ok(self.reply)
    }
}
