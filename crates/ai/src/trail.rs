//! The one line every provider call leaves (ai D12). Scripts write it to the run log, the chat
//! route with `tracing::info!` under target `ai`. It never holds prompt text, keys or tokens.

use std::time::Duration;

use crate::error::{AiError, ErrorKind};
use crate::types::{Provider, Reply, Stop, Usage};

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Refused,
    Error(ErrorKind),
}

impl Outcome {
    /// The outcome of a finished call: a refusal error is `Refused`.
    pub fn of(result: &Result<Reply, AiError>) -> Outcome {
        match result {
            Ok(_) => Outcome::Ok,
            Err(e) => Outcome::of_error(e),
        }
    }

    pub fn of_error(e: &AiError) -> Outcome {
        match e.kind {
            ErrorKind::Refusal => Outcome::Refused,
            k => Outcome::Error(k),
        }
    }

    /// The outcome of a streamed turn that ended with `stop`.
    pub fn of_stop(stop: &Stop) -> Outcome {
        match stop {
            Stop::Refusal { .. } => Outcome::Refused,
            _ => Outcome::Ok,
        }
    }

    /// `ok`, `refused` or `error:<kind>`.
    pub fn as_string(&self) -> String {
        match self {
            Outcome::Ok => "ok".to_owned(),
            Outcome::Refused => "refused".to_owned(),
            Outcome::Error(k) => format!("error:{}", k.as_str()),
        }
    }
}

/// `ai: provider=<p> model=<m> in=<input tokens> out=<output tokens> outcome=ok|refused|error:<kind> ms=<duration>`.
/// The model id is cut to the characters model ids use, so nothing can break the line.
pub fn trail_line(
    provider: Provider,
    model: &str,
    usage: Usage,
    outcome: Outcome,
    took: Duration,
) -> String {
    let model: String = model
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '@'))
        .take(100)
        .collect();
    let model = if model.is_empty() {
        "-".to_owned()
    } else {
        model
    };
    format!(
        "ai: provider={provider} model={model} in={} out={} outcome={} ms={}",
        usage.input,
        usage.output,
        outcome.as_string(),
        took.as_millis()
    )
}

/// [`trail_line`] for a finished call of `provider` and `model`.
pub fn trail_for(
    provider: Provider,
    model: &str,
    result: &Result<Reply, AiError>,
    took: Duration,
) -> String {
    let usage = match result {
        Ok(r) => r.usage,
        Err(e) => e.usage,
    };
    trail_line(provider, model, usage, Outcome::of(result), took)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_line() {
        let line = trail_line(
            Provider::Anthropic,
            "claude-opus-5",
            Usage {
                input: 12,
                output: 34,
            },
            Outcome::Ok,
            Duration::from_millis(1234),
        );
        assert_eq!(
            line,
            "ai: provider=anthropic model=claude-opus-5 in=12 out=34 outcome=ok ms=1234"
        );
        let line = trail_line(
            Provider::ChatGpt,
            "gpt x\nsecret=1",
            Usage::default(),
            Outcome::Error(ErrorKind::RateLimit),
            Duration::ZERO,
        );
        assert_eq!(
            line,
            "ai: provider=chatgpt model=gptxsecret1 in=0 out=0 outcome=error:rate_limit ms=0"
        );
        assert!(
            trail_line(
                Provider::OpenAi,
                "m",
                Usage::default(),
                Outcome::Refused,
                Duration::ZERO
            )
            .contains("outcome=refused")
        );
    }
}
