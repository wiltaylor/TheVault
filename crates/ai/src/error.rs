//! [`AiError`]: every failure of a provider call, sorted into the kinds scripts and the chat
//! panel branch on.

use std::fmt;

use serde_json::Value;

use crate::types::{Provider, Usage};

/// What went wrong, as scripts see it in `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The provider has no credentials, or no model is chosen.
    NotConfigured,
    /// The key or token was refused, or the ChatGPT sign-in expired.
    Auth,
    /// A rate or usage limit, the ChatGPT plan's included.
    RateLimit,
    /// The provider refused the request (ai D11).
    Refusal,
    /// The request, or what came back, was malformed: a bad model, schema or parameter.
    Invalid,
    /// The call ran out of time.
    Timeout,
    /// The provider could not be reached or is overloaded, or this host has no AI at all.
    Unavailable,
    /// Anything else the provider reported.
    Provider,
}

impl ErrorKind {
    /// `not_configured`, `auth`, `rate_limit`, `refusal`, `invalid`, `timeout`, `unavailable` or
    /// `provider`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::NotConfigured => "not_configured",
            ErrorKind::Auth => "auth",
            ErrorKind::RateLimit => "rate_limit",
            ErrorKind::Refusal => "refusal",
            ErrorKind::Invalid => "invalid",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Unavailable => "unavailable",
            ErrorKind::Provider => "provider",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed AI call. `message` is for the user and never holds a key or token: messages built
/// from a provider's answer are scrubbed of the credential and of anything shaped like a key.
#[derive(Debug, Clone, PartialEq)]
pub struct AiError {
    pub kind: ErrorKind,
    pub message: String,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    /// The HTTP status, when the provider answered one.
    pub status: Option<u16>,
    /// A refusal's category, such as `cyber`, when the provider named one.
    pub category: Option<String>,
    /// A refusal's explanation, when the provider gave one.
    pub explanation: Option<String>,
    /// The tokens the call used before it failed, for the trail line.
    pub usage: Usage,
}

impl AiError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> AiError {
        AiError {
            kind,
            message: message.into(),
            provider: None,
            model: None,
            status: None,
            category: None,
            explanation: None,
            usage: Usage::default(),
        }
    }

    pub fn not_configured(message: impl Into<String>) -> AiError {
        AiError::new(ErrorKind::NotConfigured, message)
    }

    pub fn invalid(message: impl Into<String>) -> AiError {
        AiError::new(ErrorKind::Invalid, message)
    }

    /// The error a refusal surfaces as (ai D11).
    pub fn refusal(
        provider: Provider,
        model: &str,
        category: Option<String>,
        explanation: Option<String>,
    ) -> AiError {
        let mut message = format!("{} refused the request", model_label(provider, model));
        if let Some(c) = &category {
            message.push_str(&format!(" (category {c})"));
        }
        if let Some(e) = explanation.as_deref().filter(|e| !e.trim().is_empty()) {
            message.push_str(&format!(": {}", e.trim()));
        }
        AiError {
            category,
            explanation,
            ..AiError::new(ErrorKind::Refusal, message).on(provider, model)
        }
    }

    /// The same error, naming the provider and model it came from.
    pub fn on(mut self, provider: Provider, model: &str) -> AiError {
        self.provider = Some(provider);
        if !model.is_empty() {
            self.model = Some(model.to_owned());
        }
        self
    }

    pub fn with_status(mut self, status: u16) -> AiError {
        self.status = Some(status);
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> AiError {
        self.usage = usage;
        self
    }

    /// Remove `secret` and anything shaped like a key or token from the message.
    pub(crate) fn scrubbed(mut self, secret: &str) -> AiError {
        self.message = scrub(&self.message, secret);
        self.explanation = self.explanation.map(|e| scrub(&e, secret));
        self
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AiError {}

fn model_label(provider: Provider, model: &str) -> String {
    if model.is_empty() {
        provider.label().to_owned()
    } else {
        format!("{} {model}", provider.label())
    }
}

/// The prefixes of the keys and tokens the providers issue: Anthropic and OpenAI keys, and JWTs.
const KEY_PREFIXES: [&str; 4] = ["sk-", "sk_", "eyJ", "oaiat"];

/// `text` with `secret` replaced, and every word that starts like a key or token (OpenAI's
/// "Incorrect API key provided: sk-ab***cd" quotes part of one) replaced by `[redacted]`.
pub(crate) fn scrub(text: &str, secret: &str) -> String {
    let mut out = if secret.len() >= 4 {
        text.replace(secret, "[redacted]")
    } else {
        text.to_owned()
    };
    for prefix in KEY_PREFIXES {
        let mut from = 0;
        while let Some(at) = out[from..].find(prefix).map(|i| i + from) {
            let word_start = at == 0
                || !out[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric());
            let end = out[at..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ')' | ']'))
                .map_or(out.len(), |i| i + at);
            if word_start && end - at > prefix.len() + 2 {
                out.replace_range(at..end, "[redacted]");
                from = at + "[redacted]".len();
            } else {
                from = at + prefix.len();
            }
        }
    }
    out
}

/// The HTTP status and the provider's error body, sorted into an [`AiError`]. Understands
/// Anthropic's `{type: "error", error: {type, message}}`, OpenAI's `{error: {message, type,
/// code, param}}` and the ChatGPT plan's admission failures `{detail}`.
pub(crate) fn from_response(provider: Provider, model: &str, status: u16, body: &str) -> AiError {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = json.get("error");
    let code = error
        .and_then(|e| {
            e.get("code")
                .and_then(Value::as_str)
                .or_else(|| e.get("type").and_then(Value::as_str))
        })
        .or_else(|| error.and_then(Value::as_str))
        .unwrap_or_default();
    let message = error
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .or_else(|| json.get("detail").and_then(Value::as_str))
        .or_else(|| json.get("message").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let text = body.trim();
            if text.is_empty() || text.len() > 300 {
                format!("HTTP {status}")
            } else {
                text.to_owned()
            }
        });
    let param = error.and_then(|e| e.get("param")).and_then(Value::as_str);
    let kind = kind_of_code(code).unwrap_or(match status {
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimit,
        408 | 504 => ErrorKind::Timeout,
        400 | 404 | 409 | 413 | 422 => ErrorKind::Invalid,
        500 | 502 | 503 | 529 => ErrorKind::Unavailable,
        _ => ErrorKind::Provider,
    });
    let mut text = format!("{}: {message}", model_label(provider, model));
    if let Some(p) = param {
        text.push_str(&format!(" (parameter {p})"));
    }
    if code == "subscription_sharing_usage_limit_exceeded" {
        text.push_str(". See your ChatGPT usage at https://chatgpt.com/settings/usage");
    }
    AiError::new(kind, text)
        .on(provider, model)
        .with_status(status)
}

/// Whether an error body says the model was not found (OpenAI's `model_not_found`, or an error
/// message that starts `model:` as Anthropic's does), so that a 404 from a custom API URL is a
/// missing model rather than a missing API (ai D24).
pub(crate) fn names_model(body: &str) -> bool {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = json.get("error");
    let field = |k: &str| error.and_then(|e| e.get(k)).and_then(Value::as_str);
    field("code") == Some("model_not_found")
        || field("message").is_some_and(|m| m.trim_start().starts_with("model:"))
}

/// The kind a provider's error code or type means, when it says more than the status does.
pub(crate) fn kind_of_code(code: &str) -> Option<ErrorKind> {
    Some(match code {
        "subscription_sharing_usage_limit_exceeded"
        | "usage_limit_reached"
        | "rate_limit_exceeded"
        | "rate_limit_error"
        | "insufficient_quota" => ErrorKind::RateLimit,
        "subscription_sharing_user_not_eligible"
        | "authentication_error"
        | "permission_error"
        | "invalid_api_key"
        | "usage_not_included" => ErrorKind::Auth,
        "subscription_sharing_unsupported_capability"
        | "subscription_sharing_route_not_supported"
        | "invalid_request_error"
        | "not_found_error"
        | "request_too_large"
        | "invalid_prompt"
        | "model_not_found"
        | "context_length_exceeded" => ErrorKind::Invalid,
        "overloaded_error" | "server_is_overloaded" | "service_unavailable" => {
            ErrorKind::Unavailable
        }
        "timeout_error" => ErrorKind::Timeout,
        "api_error" | "server_error" => ErrorKind::Provider,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_the_secret_and_key_shapes() {
        let out = scrub(
            "Incorrect API key provided: sk-ab***wxyz. token eyJhbGciOi.x.y and my-own-secret-1",
            "my-own-secret-1",
        );
        assert!(!out.contains("sk-ab"), "{out}");
        assert!(!out.contains("eyJhbG"), "{out}");
        assert!(!out.contains("my-own-secret-1"), "{out}");
        assert!(
            out.contains("Incorrect API key provided: [redacted]"),
            "{out}"
        );
    }

    #[test]
    fn scrub_keeps_ordinary_words() {
        assert_eq!(
            scrub("task-runner sk- desk-top", "zz"),
            "task-runner sk- desk-top"
        );
    }

    #[test]
    fn plan_codes_map_to_kinds() {
        let e = from_response(
            Provider::ChatGpt,
            "gpt-x",
            429,
            r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"Limit reached"}}"#,
        );
        assert_eq!(e.kind, ErrorKind::RateLimit);
        assert_eq!(e.status, Some(429));
        let e = from_response(Provider::ChatGpt, "gpt-x", 503, r#"{"detail":"Try later"}"#);
        assert_eq!(e.kind, ErrorKind::Unavailable);
        assert!(e.message.contains("Try later"));
        let e = from_response(
            Provider::ChatGpt,
            "gpt-x",
            400,
            r#"{"error":{"code":"subscription_sharing_unsupported_capability","message":"no","param":"tools"}}"#,
        );
        assert_eq!(e.kind, ErrorKind::Invalid);
        assert!(e.message.contains("parameter tools"));
        let e = from_response(
            Provider::Anthropic,
            "m",
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(e.kind, ErrorKind::Unavailable);
    }
}
