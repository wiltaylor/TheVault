//! The one line every provider call leaves (ai D12). Scripts write it to the run log, the chat
//! route with `tracing::info!` under target `ai`. It never holds prompt text, keys or tokens.

use std::time::Duration;

use crate::error::{AiError, ErrorKind};
use crate::types::{Kind, Reply, Stop, Usage};

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

/// `ai: account=<name> kind=<kind> model=<m> in=<input tokens> out=<output tokens> outcome=ok|refused|error:<kind> ms=<duration>`,
/// with ` host=<host>` after it when the call went to an account's own base URL (ai D24): pass
/// [`crate::Credential::custom_host`]. The account name has each space as `_` and the name,
/// model id and host are cut to the characters they use, so nothing can break the line
/// (accounts A8).
pub fn trail_line(
    name: &str,
    kind: Kind,
    model: &str,
    usage: Usage,
    outcome: Outcome,
    took: Duration,
    host: Option<&str>,
) -> String {
    let token = |s: &str, extra: &[char], max: usize, spaces: bool| {
        let t: String = s
            .chars()
            .map(|c| if spaces && c == ' ' { '_' } else { c })
            .filter(|c| c.is_alphanumeric() || extra.contains(c))
            .take(max)
            .collect();
        if t.is_empty() { "-".to_owned() } else { t }
    };
    let name = token(name, &['-', '_', '.', '@'], 60, true);
    let model = token(model, &['-', '_', '.', ':', '/', '@'], 100, false);
    let mut line = format!(
        "ai: account={name} kind={kind} model={model} in={} out={} outcome={} ms={}",
        usage.input,
        usage.output,
        outcome.as_string(),
        took.as_millis()
    );
    let host: String = host
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '[' | ']'))
        .take(255)
        .collect();
    if !host.is_empty() {
        line.push_str(&format!(" host={host}"));
    }
    line
}

/// [`trail_line`] for a finished call with credential `cred` to `model`.
pub fn trail_for(
    cred: &crate::Credential,
    model: &str,
    result: &Result<Reply, AiError>,
    took: Duration,
) -> String {
    let usage = match result {
        Ok(r) => r.usage,
        Err(e) => e.usage,
    };
    trail_line(
        &cred.name,
        cred.kind,
        model,
        usage,
        Outcome::of(result),
        took,
        cred.custom_host.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_line() {
        let line = trail_line(
            "Anthropic",
            Kind::Anthropic,
            "claude-opus-5",
            Usage {
                input: 12,
                output: 34,
            },
            Outcome::Ok,
            Duration::from_millis(1234),
            None,
        );
        assert_eq!(
            line,
            "ai: account=Anthropic kind=anthropic model=claude-opus-5 in=12 out=34 outcome=ok ms=1234"
        );
        let line = trail_line(
            "My plan\nx=1",
            Kind::ChatGpt,
            "gpt x\nsecret=1",
            Usage::default(),
            Outcome::Error(ErrorKind::RateLimit),
            Duration::ZERO,
            None,
        );
        assert_eq!(
            line,
            "ai: account=My_planx1 kind=chatgpt model=gptxsecret1 in=0 out=0 outcome=error:rate_limit ms=0"
        );
        assert!(
            trail_line(
                "",
                Kind::OpenAi,
                "m",
                Usage::default(),
                Outcome::Refused,
                Duration::ZERO,
                None
            )
            .contains("account=- kind=openai model=m in=0 out=0 outcome=refused")
        );
        assert_eq!(
            trail_line(
                "Ollama",
                Kind::OpenAiCompatible,
                "m",
                Usage::default(),
                Outcome::Ok,
                Duration::ZERO,
                Some("10.0.0.5:11434\nx=1 y")
            ),
            "ai: account=Ollama kind=openai_compatible model=m in=0 out=0 outcome=ok ms=0 host=10.0.0.5:11434x1y"
        );
    }
}
