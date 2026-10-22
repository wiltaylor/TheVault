//! The provider-neutral request, transcript (ai D10) and event model both clients speak.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The three credentials (ai D1). `anthropic` speaks the Messages API; `openai` and `chatgpt`
/// speak the Responses API, with an API key and with a ChatGPT plan token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "chatgpt")]
    ChatGpt,
}

impl Provider {
    /// Every provider, in the order Settings > AI shows them.
    pub const ALL: [Provider; 3] = [Provider::Anthropic, Provider::OpenAi, Provider::ChatGpt];

    /// `anthropic`, `openai` or `chatgpt`: the name in settings, routes, scripts and trail lines.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::ChatGpt => "chatgpt",
        }
    }

    /// The name the user reads: `Anthropic`, `OpenAI` or `ChatGPT`.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "OpenAI",
            Provider::ChatGpt => "ChatGPT",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Provider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "anthropic" => Ok(Provider::Anthropic),
            "openai" => Ok(Provider::OpenAi),
            "chatgpt" => Ok(Provider::ChatGpt),
            other => Err(format!(
                "unknown AI provider {other:?}: expected anthropic, openai or chatgpt"
            )),
        }
    }
}

/// How hard the model thinks. Unset leaves the provider's default (ai D5). Anthropic takes each
/// value as `output_config.effort`; OpenAI takes it as `reasoning.effort`, where `max` becomes
/// `xhigh`, OpenAI's highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

impl FromStr for Effort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "low" => Ok(Effort::Low),
            "medium" => Ok(Effort::Medium),
            "high" => Ok(Effort::High),
            "xhigh" => Ok(Effort::XHigh),
            "max" => Ok(Effort::Max),
            other => Err(format!(
                "unknown effort {other:?}: expected low, medium, high, xhigh or max"
            )),
        }
    }
}

/// Who said a [`Message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a [`Message`] (ai D10). As JSON it is tagged by `type`: `text`, `tool_call`,
/// `tool_result` or `notice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    /// A tool the assistant called, with its parsed input.
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    /// What a tool returned, in a user message, answering the [`Content::ToolCall`] with the
    /// same `id`.
    ///
    /// `parts` follow `content` (ai D19): an image the tool read, by its asset id, or a text
    /// standing in for one. A transcript from before parts deserializes with none.
    ToolResult {
        id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        parts: Vec<Part>,
    },
    /// Something the app tells the user, such as that an answer was cut off. Never sent to a
    /// provider.
    Notice {
        kind: String,
        text: String,
    },
}

impl Content {
    pub fn text(text: impl Into<String>) -> Content {
        Content::Text { text: text.into() }
    }

    /// A tool result of text alone.
    pub fn tool_result(
        id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Content {
        Content::ToolResult {
            id: id.into(),
            content: content.into(),
            is_error,
            parts: Vec::new(),
        }
    }
}

/// A piece of a [`Content::ToolResult`] beyond its text (ai D19). As JSON it is tagged by
/// `type`: `text` or `image`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// An image asset, by reference. The transcript holds only the asset id and its media
    /// type, so the bytes never travel to the browser. The server loads them into `data` each
    /// time it builds a provider request; `data` is never serialized. An image whose `data` is
    /// unset when the request is built goes to the provider as a text saying so.
    Image {
        asset_id: String,
        media_type: String,
        #[serde(skip)]
        data: Option<ImageData>,
    },
}

impl Part {
    /// A reference to image asset `asset_id`, not yet loaded.
    pub fn image(asset_id: impl Into<String>, media_type: impl Into<String>) -> Part {
        Part::Image {
            asset_id: asset_id.into(),
            media_type: media_type.into(),
            data: None,
        }
    }
}

/// An image as it is sent: its media type, which differs from the asset's when it was scaled
/// down, and its bytes in standard base64 with no line breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub media_type: String,
    pub base64: String,
}

/// The text an image part with no loaded data is sent as.
pub(crate) fn unloaded(asset_id: &str) -> String {
    format!("[image asset {asset_id} was not loaded]")
}

/// The provider's own record of an assistant turn (ai D10): Anthropic's content blocks with
/// `thinking` and its `signature`, or OpenAI's output items with `reasoning` and its
/// `encrypted_content`. It is opaque to everyone but this crate, which replays `output` when
/// the next request goes to the same provider and model and drops it otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderState {
    pub provider: Provider,
    pub model: String,
    /// A JSON array: the content blocks or output items.
    pub output: Value,
}

/// One turn of a transcript (ai D10). Transcripts are append-only: nothing edits or reorders
/// an earlier turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<ProviderState>,
}

impl Message {
    /// A user message of one text.
    pub fn user(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: vec![Content::text(text)],
            provider_state: None,
        }
    }

    /// An assistant message of one text, with no provider state.
    pub fn assistant(text: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![Content::text(text)],
            provider_state: None,
        }
    }

    /// The user message answering tool calls with their results.
    pub fn tool_results(results: Vec<Content>) -> Message {
        Message {
            role: Role::User,
            content: results,
            provider_state: None,
        }
    }
}

/// A tool the model may call. `input_schema` is a JSON Schema object. Both clients send tools
/// as strict, so every object in it must say `additionalProperties: false` and list all its
/// properties in `required` (Anthropic's and OpenAI's strict mode rules).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// One call to a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub provider: Provider,
    pub model: String,
    /// The system prompt: Anthropic's `system`, OpenAI's `instructions`.
    #[serde(default)]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    /// A JSON Schema the reply's text must follow: Anthropic's `output_config.format`, OpenAI's
    /// `text.format`. The reply is parsed and checked against it ([`Reply::json`]).
    #[serde(default)]
    pub schema: Option<Value>,
    #[serde(default)]
    pub effort: Option<Effort>,
    /// The output cap. Unset is 64000 for a streamed Anthropic request and 16000 for a
    /// non-streamed one (ai D5), and no cap for OpenAI. A ChatGPT plan request never sends one
    /// (ai D4).
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl Request {
    /// A request of one user message, with nothing else set.
    pub fn new(provider: Provider, model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Request {
            provider,
            model: model.into(),
            system: None,
            messages: vec![Message::user(prompt)],
            tools: Vec::new(),
            schema: None,
            effort: None,
            max_tokens: None,
        }
    }

    /// The short request Settings > AI's Test button sends: "Reply with OK", at most 16 tokens.
    pub fn test(provider: Provider, model: impl Into<String>) -> Self {
        Request {
            max_tokens: Some(16),
            ..Request::new(provider, model, "Reply with OK.")
        }
    }
}

/// Tokens a call used. `input` counts every input token, cached ones included.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

/// A tool call the model made, complete, with its input parsed strictly and checked against the
/// tool's schema (ai D5). When `error` is set, the input was not JSON (then `input` is the raw
/// text as a string), named no tool of the request, or broke the schema: do not run the tool,
/// answer it with `Content::ToolResult { is_error: true, content: error }` so the model can try
/// again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Why a turn ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Stop {
    EndTurn,
    /// The model wants the tool calls of this turn run.
    ToolUse,
    /// The output cap cut the turn off.
    MaxTokens,
    /// The provider refused (ai D11). Anthropic gives a category, such as `cyber`, and sometimes
    /// an explanation; OpenAI's refusal text is the explanation.
    Refusal {
        category: Option<String>,
        explanation: Option<String>,
    },
}

impl Stop {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stop::EndTurn => "end_turn",
            Stop::ToolUse => "tool_use",
            Stop::MaxTokens => "max_tokens",
            Stop::Refusal { .. } => "refusal",
        }
    }
}

/// What a streamed call emits, in order: text as it arrives, then the turn's tool calls, then
/// one `Usage`, then `Done`. Tool calls come only on a turn that ends `ToolUse`: never on one
/// cut off by `MaxTokens` or a `Refusal` (ai D5). An error ends the stream early instead.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    TextDelta(String),
    ToolCall(ToolCall),
    Usage(Usage),
    /// The turn is over. `provider_state` is set when the turn ended `EndTurn` or `ToolUse`;
    /// keep it on the assistant [`Message`] so the next request can replay it.
    Done {
        stop: Stop,
        provider_state: Option<ProviderState>,
    },
}

/// A finished call, as [`crate::complete`] and [`crate::complete_blocking`] return it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub provider: Provider,
    pub model: String,
    /// All the text, joined.
    pub text: String,
    /// The text parsed as JSON and checked against the request's schema, when it had one.
    pub json: Option<Value>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// `EndTurn`, `ToolUse` or `MaxTokens`: a refusal is an error ([`crate::ErrorKind::Refusal`]).
    pub stop: Stop,
    pub provider_state: Option<ProviderState>,
}

impl Reply {
    /// The assistant turn to append to the transcript: the text, then the tool calls, with the
    /// provider state.
    pub fn message(&self) -> Message {
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(Content::text(self.text.clone()));
        }
        for call in &self.tool_calls {
            content.push(Content::ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            });
        }
        Message {
            role: Role::Assistant,
            content,
            provider_state: self.provider_state.clone(),
        }
    }
}

/// A model a provider offers (ai D6). `display_name` is Anthropic's and the ChatGPT plan's;
/// OpenAI's API-key list has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub display_name: Option<String>,
}
