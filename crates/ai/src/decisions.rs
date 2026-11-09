//! OpenRouter's alpha Decisions API. All wire details live here, separately from chat.
//! Contract: OpenRouter Python SDK decisions README and DecisionsResponse, read 2026-11-08
//! (TV-113). The endpoint returns one JSON document, not chat SSE. No retries or tools.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AiError, Credential, ErrorKind, Kind, Outcome, Usage};

pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Questions supported by note classifiers. Image-region (`box`) decisions are not supported.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Question {
    Noul {
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<BTreeMap<String, Value>>,
    },
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub model: String,
    pub state: Value,
    pub questions: BTreeMap<String, Question>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
    Score {
        score: f64,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
}

impl Answer {
    /// Typed value written to the configured note field. Noul is a probability, not a bool.
    pub fn value(&self) -> Value {
        match self {
            Self::Noul { noul } => Value::from(*noul),
            Self::Choice { choice, .. } => Value::from(choice.clone()),
            Self::Score { score, .. } => Value::from(*score),
        }
    }

    /// A notify threshold tests yes probability for noul, confidence for choice/score.
    pub fn confidence(&self) -> f64 {
        match self {
            Self::Noul { noul } => *noul,
            Self::Choice { confidence, .. } | Self::Score { confidence, .. } => *confidence,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    pub id: String,
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

fn probability(n: f64) -> bool {
    n.is_finite() && (0.0..=1.0).contains(&n)
}

fn context(value: &Value) -> bool {
    value.is_string() || value.is_object() || value.is_array()
}

impl Request {
    pub fn validate(&self) -> Result<(), AiError> {
        if self.model.trim().is_empty()
            || self.questions.is_empty()
            || self.questions.len() > 64
            || !(self.state.is_string() || self.state.is_object() || self.state.is_array())
        {
            return Err(AiError::invalid(
                "Invalid decisions model, state or questions",
            ));
        }
        for (id, question) in &self.questions {
            let (instructions, valid) = match question {
                Question::Noul {
                    instructions,
                    criteria,
                } => (
                    instructions,
                    criteria.as_ref().is_none_or(|c| {
                        c.len() == 2
                            && c.contains_key("true")
                            && c.contains_key("false")
                            && c.values().all(context)
                    }),
                ),
                Question::Choice {
                    instructions,
                    criteria,
                } => (
                    instructions,
                    criteria.len() >= 2
                        && criteria.keys().all(|k| !k.trim().is_empty())
                        && criteria.values().all(context),
                ),
                Question::Score {
                    instructions,
                    criteria,
                } => (
                    instructions,
                    criteria.len() >= 2 && criteria.iter().all(context),
                ),
            };
            if id.is_empty()
                || !valid
                || !(instructions.is_string()
                    || instructions.is_object()
                    || instructions.is_array())
            {
                return Err(AiError::invalid("Invalid decision question"));
            }
        }
        Ok(())
    }

    fn check(&self, response: &Response) -> Result<(), AiError> {
        if response.answers.len() != self.questions.len()
            || response.id.is_empty()
            || response.model.is_empty()
        {
            return Err(AiError::invalid("Incomplete decisions response"));
        }
        for (id, question) in &self.questions {
            let valid = match (question, response.answers.get(id)) {
                (Question::Noul { .. }, Some(Answer::Noul { noul })) => probability(*noul),
                (
                    Question::Choice { criteria, .. },
                    Some(Answer::Choice {
                        choice,
                        confidence,
                        probabilities,
                    }),
                ) => {
                    criteria.contains_key(choice)
                        && probability(*confidence)
                        && probabilities.len() == criteria.len()
                        && probabilities
                            .iter()
                            .all(|(k, p)| criteria.contains_key(k) && probability(*p))
                }
                (
                    Question::Score { .. },
                    Some(Answer::Score {
                        score,
                        confidence,
                        probabilities,
                    }),
                ) => {
                    score.is_finite()
                        && probability(*confidence)
                        && !probabilities.is_empty()
                        && probabilities.values().all(|p| probability(*p))
                }
                _ => false,
            };
            if !valid {
                return Err(AiError::invalid("Invalid or missing decision answer"));
            }
        }
        Ok(())
    }
}

/// One blocking HTTP attempt. Call outside an async runtime. Redirects and retries are off.
/// Errors never include provider response bodies, state, header values or credentials.
pub fn decide(
    credential: &Credential,
    request: &Request,
    timeout: Duration,
) -> Result<Response, AiError> {
    request.validate()?;
    if credential.kind != Kind::OpenAiCompatible {
        return Err(AiError::invalid(
            "Decisions require an OpenAI-compatible account",
        ));
    }
    let base = crate::check_url(&credential.base_url)?;
    let mut url =
        reqwest::Url::parse(&base).map_err(|_| AiError::invalid("Invalid decisions base URL"))?;
    if !url.path().trim_end_matches('/').ends_with("/api/v1") {
        return Err(AiError::invalid(
            "Decisions require an OpenRouter /api/v1 base URL",
        ));
    }
    let root = url.path().trim_end_matches('/').trim_end_matches("/v1");
    url.set_path(&format!("{root}/alpha/decisions"));
    let body =
        serde_json::to_vec(request).map_err(|_| AiError::invalid("Invalid decisions request"))?;
    if body.len() as u64 > MAX_BYTES {
        return Err(AiError::invalid("Decisions request is too large"));
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(network_error)?;
    let mut call = client.post(url).header("content-type", "application/json");
    if !credential.secret.is_empty() {
        call = call.bearer_auth(credential.secret.expose());
    }
    for (name, value) in &credential.headers {
        call = call.header(name, value);
    }
    let response = call.body(body).send().map_err(network_error)?;
    let status = response.status().as_u16();
    if status != 200 {
        let kind = match status {
            401 | 403 => ErrorKind::Auth,
            402 | 429 => ErrorKind::RateLimit,
            400 | 404 | 405 | 413 => ErrorKind::Invalid,
            500..=599 => ErrorKind::Unavailable,
            _ => ErrorKind::Provider,
        };
        let mut error = AiError::new(kind, format!("Decisions HTTP {status}"));
        error.status = Some(status);
        return Err(error);
    }
    if response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_none_or(|s| s.split(';').next().unwrap_or_default().trim() != "application/json")
    {
        return Err(AiError::invalid(
            "Decisions require a JSON response, not a stream",
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AiError::new(ErrorKind::Unavailable, "Decisions response read failed"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AiError::invalid("Decisions response is too large"));
    }
    let response: Response = serde_json::from_slice(&bytes)
        .map_err(|_| AiError::invalid("Malformed decisions response"))?;
    request.check(&response)?;
    Ok(response)
}

fn network_error(error: reqwest::Error) -> AiError {
    AiError::new(
        if error.is_timeout() {
            ErrorKind::Timeout
        } else {
            ErrorKind::Unavailable
        },
        "Decisions request failed",
    )
}

pub fn trail(
    credential: &Credential,
    request: &Request,
    result: &Result<Response, AiError>,
    took: Duration,
) -> String {
    let (usage, outcome) = match result {
        Ok(r) => (
            Usage {
                input: r.usage.input_tokens,
                output: r.usage.output_tokens,
            },
            Outcome::Ok,
        ),
        Err(e) => (e.usage, Outcome::of_error(e)),
    };
    crate::trail_line(
        &credential.name,
        credential.kind,
        &request.model,
        usage,
        outcome,
        took,
        credential.custom_host.as_deref(),
    )
}
