//! The two transports: an async one over `reqwest::Client` for the chat route, and a blocking
//! one over `reqwest::blocking::Client` for scripts and the token refresh. Both retry a 429, a
//! 5xx or a connection failure at most twice, waiting `retry-after-ms` or `retry-after` when the
//! provider sends one, and never once an answer has begun to stream.

use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::credentials::Credential;
use crate::error::{self, AiError, ErrorKind};
use crate::types::Provider;

/// Retries after the first attempt.
pub const MAX_RETRIES: u32 = 2;

/// The longest wait a `retry-after` may ask for: longer is not retried.
pub const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// How long a connection may take to open.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn user_agent() -> String {
    format!("TheVault/{}", env!("CARGO_PKG_VERSION"))
}

/// An async client for [`crate::stream`], [`crate::complete`] and [`crate::list_models`]. Build
/// one and keep it: it holds the connection pool.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// A blocking client for [`crate::complete_blocking`] and [`crate::list_models_blocking`]. Do not
/// build or drop one inside an async runtime: use it from a plain thread or `spawn_blocking`.
pub fn blocking_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(None)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// One HTTP request to a provider, before it is sent.
pub(crate) struct Call {
    pub get: bool,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<Value>,
    pub provider: Provider,
    pub model: String,
    /// The key or token, scrubbed out of any error message.
    pub secret: String,
    /// The path under the base URL, such as `/v1/responses`.
    pub path: String,
    /// Whether the call goes to a custom API URL (ai D24).
    pub custom: bool,
}

impl Call {
    /// A GET of `path` under `cred`'s base URL. An empty secret, a custom API URL with no key,
    /// sends no auth header (ai D24).
    pub fn new(cred: &Credential, model: &str, path: &str) -> Call {
        let mut headers = Vec::new();
        let secret = cred.secret.expose();
        let keyed = !cred.secret.is_empty();
        match cred.provider {
            Provider::Anthropic => {
                if keyed {
                    headers.push(("x-api-key", secret.to_owned()));
                }
                headers.push(("anthropic-version", crate::anthropic::VERSION.to_owned()));
            }
            Provider::OpenAi | Provider::ChatGpt => {
                if keyed {
                    headers.push(("authorization", format!("Bearer {secret}")));
                }
            }
        }
        Call {
            get: true,
            url: crate::settings::api_url(&cred.base_url, path),
            headers,
            body: None,
            provider: cred.provider,
            model: model.to_owned(),
            secret: secret.to_owned(),
            path: path.to_owned(),
            custom: cred.custom_host.is_some(),
        }
    }

    pub fn post(mut self, body: Value, stream: bool) -> Call {
        self.get = false;
        self.headers
            .push(("content-type", "application/json".to_owned()));
        self.headers.push((
            "accept",
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            }
            .to_owned(),
        ));
        self.body = Some(body);
        self
    }

    pub fn fail(&self, e: AiError) -> AiError {
        e.on(self.provider, &self.model).scrubbed(&self.secret)
    }

    fn status_error(&self, status: u16, body: &str) -> AiError {
        if self.custom
            && !self.get
            && matches!(status, 404 | 405)
            && !error::names_model(body)
            && let Some(api) = match self.path.as_str() {
                "/v1/responses" => Some("the OpenAI Responses API (POST /v1/responses)"),
                "/v1/messages" => Some("the Anthropic Messages API (POST /v1/messages)"),
                _ => None,
            }
        {
            return self.fail(
                AiError::invalid(format!("This service does not support {api}."))
                    .with_status(status),
            );
        }
        self.fail(error::from_response(
            self.provider,
            &self.model,
            status,
            body,
        ))
    }

    fn send_error(&self, e: &reqwest::Error) -> AiError {
        let kind = if e.is_timeout() {
            ErrorKind::Timeout
        } else if e.is_connect() || e.is_request() {
            ErrorKind::Unavailable
        } else {
            ErrorKind::Provider
        };
        let what = match kind {
            ErrorKind::Timeout => "timed out",
            ErrorKind::Unavailable => "could not be reached",
            _ => "failed",
        };
        let detail = without_url(e);
        self.fail(AiError::new(
            kind,
            format!("{} {what}: {detail}", self.provider.label()),
        ))
    }

    fn async_request(&self, http: &reqwest::Client) -> reqwest::RequestBuilder {
        let mut b = if self.get {
            http.get(&self.url)
        } else {
            http.post(&self.url)
        };
        for (n, v) in &self.headers {
            b = b.header(*n, v);
        }
        if let Some(body) = &self.body {
            b = b.body(body.to_string());
        }
        b
    }

    fn blocking_request(
        &self,
        http: &reqwest::blocking::Client,
    ) -> reqwest::blocking::RequestBuilder {
        let mut b = if self.get {
            http.get(&self.url)
        } else {
            http.post(&self.url)
        };
        for (n, v) in &self.headers {
            b = b.header(*n, v);
        }
        if let Some(body) = &self.body {
            b = b.body(body.to_string());
        }
        b
    }
}

/// A reqwest error's text without the URL, which carries no secret but adds only noise.
fn without_url(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    if let Some(url) = e.url() {
        text = text
            .replace(&format!(" ({url})"), "")
            .replace(url.as_str(), "");
    }
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        text.push_str(&format!(": {s}"));
        source = s.source();
    }
    text
}

/// Whether a failed answer is worth another attempt: a 429 or a 5xx, but not a usage or quota
/// limit, which waiting a second does not lift.
fn retryable(status: u16, body: &str) -> bool {
    let limit = [
        "subscription_sharing_usage_limit_exceeded",
        "usage_limit_reached",
        "insufficient_quota",
    ]
    .iter()
    .any(|c| body.contains(c));
    (status == 429 || (500..600).contains(&status)) && !limit
}

/// The wait the provider asked for, or doubling waits from half a second.
fn wait_for(headers: &HeaderMap, attempt: u32) -> Duration {
    let header = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    if let Some(ms) = header("retry-after-ms") {
        return Duration::from_millis(ms as u64);
    }
    if let Some(s) = header("retry-after") {
        return Duration::from_secs_f64(s.min(3600.0));
    }
    backoff(attempt)
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 << attempt.min(6))
}

/// Send `call` and return the answer once it is a success. A failure is read whole and mapped.
pub(crate) async fn send(
    http: &reqwest::Client,
    call: &Call,
) -> Result<reqwest::Response, AiError> {
    let mut attempt = 0;
    loop {
        let wait = match call.async_request(http).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status().as_u16();
                let wait = wait_for(resp.headers(), attempt);
                let body = resp.text().await.unwrap_or_default();
                if !(retryable(status, &body) && attempt < MAX_RETRIES && wait <= MAX_RETRY_WAIT) {
                    return Err(call.status_error(status, &body));
                }
                wait
            }
            Err(e) => {
                if !((e.is_connect() || e.is_timeout()) && attempt < MAX_RETRIES) {
                    return Err(call.send_error(&e));
                }
                backoff(attempt)
            }
        };
        tokio::time::sleep(wait).await;
        attempt += 1;
    }
}

/// The time left before `deadline`, or a `timeout` error when there is none.
pub(crate) fn remaining(
    deadline: Option<Instant>,
    call: &Call,
) -> Result<Option<Duration>, AiError> {
    let Some(d) = deadline else {
        return Ok(None);
    };
    match d
        .checked_duration_since(Instant::now())
        .filter(|l| !l.is_zero())
    {
        Some(left) => Ok(Some(left)),
        None => Err(out_of_time(call)),
    }
}

fn out_of_time(call: &Call) -> AiError {
    call.fail(AiError::new(
        ErrorKind::Timeout,
        format!("the {} call ran out of time", call.provider.label()),
    ))
}

/// Like [`send`], blocking, giving up at `deadline`. The whole answer must arrive by then: the
/// deadline is the request's timeout. A retry whose wait would pass the deadline is not made.
pub(crate) fn send_blocking(
    http: &reqwest::blocking::Client,
    call: &Call,
    deadline: Option<Instant>,
) -> Result<reqwest::blocking::Response, AiError> {
    let mut attempt = 0;
    loop {
        let mut req = call.blocking_request(http);
        if let Some(left) = remaining(deadline, call)? {
            req = req.timeout(left);
        }
        let (wait, failure) = match req.send() {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status().as_u16();
                let wait = wait_for(resp.headers(), attempt);
                let body = resp.text().unwrap_or_default();
                let failure = call.status_error(status, &body);
                if !(retryable(status, &body) && attempt < MAX_RETRIES && wait <= MAX_RETRY_WAIT) {
                    return Err(failure);
                }
                (wait, failure)
            }
            Err(e) => {
                let failure = call.send_error(&e);
                if !((e.is_connect() || e.is_timeout()) && attempt < MAX_RETRIES) {
                    return Err(failure);
                }
                (backoff(attempt), failure)
            }
        };
        if let Some(left) = remaining(deadline, call)?
            && wait >= left
        {
            return Err(failure);
        }
        std::thread::sleep(wait);
        attempt += 1;
    }
}

/// The error a read of the streamed body failed with.
pub(crate) fn read_error(call: &Call, e: &dyn std::fmt::Display, timeout: bool) -> AiError {
    let kind = if timeout {
        ErrorKind::Timeout
    } else {
        ErrorKind::Unavailable
    };
    call.fail(AiError::new(
        kind,
        format!("{} answer broke off: {e}", call.provider.label()),
    ))
}

/// The error for a reqwest failure mid-body.
pub(crate) fn body_error(call: &Call, e: &reqwest::Error) -> AiError {
    read_error(call, &without_url(e), e.is_timeout())
}
