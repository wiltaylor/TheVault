//! Settings > Integrations > Brave Search, and the chat's `web_search` tool (ai D26).
//!
//! - `GET /api/integrations/brave` answers `{ configured }`: whether the `brave_api_key` secret
//!   is stored. No answer ever holds the key.
//! - `PUT /api/integrations/brave/key` takes `{ key }` and stores it, trimmed, in
//!   `brave_api_key`; an empty key deletes the secret. It answers as `GET` does.
//! - `POST /api/integrations/brave/test` makes one search for "TheVault" with the stored key and
//!   answers 200 `{ ok, count, ms, error: null | { kind, message } }`, `count` being the number
//!   of results Brave returned.
//!
//! A search is `GET <endpoint>/res/v1/web/search?q=<q>&count=<n>` with `X-Subscription-Token`
//! and `Accept: application/json`. The endpoint is the `brave_settings` secret's `endpoint`, the
//! test seam, else Brave's own host. A 429 is the error kind `rate_limit`; 401, 403 and 422 are
//! `auth`; both carry Brave's own message. Each search writes one trail line,
//! `brave: count=<n> results=<m> outcome=ok|error:<kind> ms=<d>`, which never holds the query.
//!
//! The chat offers `web_search { query, count }` only when the key is stored ([`Search::load`]);
//! it answers `[{ title, url, description, age? }]` with the HTML tags stripped from Brave's
//! titles and descriptions.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ai::{AiError, ErrorKind, ToolSpec};
use axum::Json;
use axum::extract::State;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use writer::Writer;

use crate::{ApiError, blocking};

/// The secret holding the Brave Search API key.
pub const KEY: &str = "brave_api_key";

/// The secret holding the Brave settings: `{ endpoint }`, the test seam.
pub const SETTINGS: &str = "brave_settings";

/// Brave's own API host.
const BRAVE_URL: &str = "https://api.search.brave.com";

/// The search path under the endpoint.
const SEARCH_PATH: &str = "/res/v1/web/search";

/// How long one search may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The most results one `web_search` asks for, and the number when the model names none.
pub const MAX_COUNT: u64 = 10;
pub const DEFAULT_COUNT: u64 = 5;

/// The longest query Brave takes.
const MAX_QUERY: usize = 400;

/// The query the Test button searches for.
const TEST_QUERY: &str = "TheVault";

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_default()
    })
}

/// What a search needs: the key and where to send it.
#[derive(Clone)]
pub struct Search {
    key: String,
    endpoint: String,
}

impl std::fmt::Debug for Search {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Search")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl Search {
    /// The stored key and endpoint, or `None` when no key is stored or secrets cannot be read.
    pub fn load(s: &state::State) -> Option<Search> {
        if !s.has_secret_key() {
            return None;
        }
        let key = s
            .get_secret(KEY)
            .ok()
            .flatten()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())?;
        let endpoint = s
            .get_secret(SETTINGS)
            .ok()
            .flatten()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["endpoint"].as_str().map(str::to_owned))
            .map(|e| e.trim().trim_end_matches('/').to_owned())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| BRAVE_URL.to_owned());
        Some(Search { key, endpoint })
    }

    /// One search for `query`, at most `count` results, writing the trail line.
    pub async fn run(&self, query: &str, count: u64) -> Result<Vec<Hit>, AiError> {
        let began = Instant::now();
        let result = self.call(query, count).await;
        eprintln!(
            "{}",
            trail_line(
                count,
                result.as_ref().map(Vec::len).unwrap_or(0),
                result.as_ref().err().map(|e| e.kind),
                began.elapsed(),
            )
        );
        result
    }

    async fn call(&self, query: &str, count: u64) -> Result<Vec<Hit>, AiError> {
        let url = Url::parse_with_params(
            &format!("{}{SEARCH_PATH}", self.endpoint),
            &[("q", query), ("count", &count.to_string())],
        )
        .map_err(|e| AiError::new(ErrorKind::Invalid, format!("bad Brave endpoint: {e}")))?;
        let res = http()
            .get(url)
            .header("X-Subscription-Token", &self.key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                let kind = if e.is_timeout() {
                    ErrorKind::Timeout
                } else {
                    ErrorKind::Unavailable
                };
                AiError::new(kind, format!("Brave Search could not be reached: {e}"))
            })?;
        let status = res.status();
        let body = res.text().await.map_err(|e| {
            AiError::new(
                ErrorKind::Unavailable,
                format!("Brave Search's answer broke off: {e}"),
            )
        })?;
        if !status.is_success() {
            let kind = match status.as_u16() {
                429 => ErrorKind::RateLimit,
                401 | 403 | 422 => ErrorKind::Auth,
                500.. => ErrorKind::Unavailable,
                _ => ErrorKind::Provider,
            };
            return Err(AiError::new(kind, describe_error(status, &body)));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| {
            AiError::new(
                ErrorKind::Provider,
                format!("Brave Search answered something that is not JSON: {e}"),
            )
        })?;
        Ok(hits(&v))
    }
}

/// One search result, as the model gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub description: String,
    pub age: Option<String>,
}

impl Hit {
    fn to_json(&self) -> Value {
        let mut o =
            json!({ "title": self.title, "url": self.url, "description": self.description });
        if let Some(age) = &self.age {
            o["age"] = json!(age);
        }
        o
    }
}

/// The results in Brave's answer `v`: `web.results[]`, the tags stripped.
fn hits(v: &Value) -> Vec<Hit> {
    let text = |r: &Value, k: &str| strip_tags(r[k].as_str().unwrap_or_default());
    v["web"]["results"]
        .as_array()
        .map(|rs| {
            rs.iter()
                .filter(|r| r["url"].is_string())
                .map(|r| Hit {
                    title: text(r, "title"),
                    url: r["url"].as_str().unwrap_or_default().to_owned(),
                    description: text(r, "description"),
                    age: r["age"]
                        .as_str()
                        .map(str::to_owned)
                        .filter(|a| !a.is_empty()),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `Brave answered <status>: <detail>` from Brave's error body, or the body as it came.
fn describe_error(status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        let e = &v["error"];
        e["detail"]
            .as_str()
            .or_else(|| e["message"].as_str())
            .or_else(|| e["code"].as_str())
            .or_else(|| v["message"].as_str())
            .map(str::to_owned)
    });
    let detail = detail.unwrap_or_else(|| body.trim().chars().take(500).collect());
    if detail.is_empty() {
        format!("Brave answered {status}")
    } else {
        format!("Brave answered {status}: {detail}")
    }
}

/// `text` without its HTML tags, the common entities decoded.
pub fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..].find(';').map(|i| i + 1).filter(|&i| i <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `brave: count=<n> results=<m> outcome=ok|error:<kind> ms=<d>`: never the query.
pub fn trail_line(count: u64, results: usize, error: Option<ErrorKind>, took: Duration) -> String {
    let outcome = match error {
        None => "ok".to_owned(),
        Some(k) => format!("error:{}", k.as_str()),
    };
    format!(
        "brave: count={count} results={results} outcome={outcome} ms={}",
        took.as_millis()
    )
}

// ---- the chat tool ---------------------------------------------------------------------------

/// The `web_search` tool, offered only when a key is stored.
pub fn tool_spec() -> ToolSpec {
    crate::ai_chat::tool(
        "web_search",
        "Search the web with Brave Search. Returns up to `count` results, each with title, url, description and sometimes age. Use it for anything the notes do not hold, such as current events or facts from the web.",
        json!({
            "query": { "type": "string", "description": "The search words, at most 400 characters." },
            "count": { "type": "integer", "description": "How many results, 1 to 10; 5 when unsure." },
        }),
    )
}

/// A `web_search` call's answer: the text for the model, whether it failed, and the summary the
/// panel shows (`links`, the titles and URLs; `message` and `kind` on an error).
pub struct Answer {
    pub content: String,
    pub is_error: bool,
    pub summary: Value,
}

impl Answer {
    fn error(e: &AiError) -> Answer {
        Answer {
            content: json!({ "error": e.message, "kind": e.kind.as_str() }).to_string(),
            is_error: true,
            summary: json!({ "notes": [], "message": e.message, "kind": e.kind.as_str() }),
        }
    }
}

/// Run a `web_search` call with input `input`.
pub async fn web_search(search: &Search, input: &Value) -> Answer {
    let query = input["query"].as_str().unwrap_or_default().trim();
    if query.is_empty() {
        return Answer::error(&AiError::new(
            ErrorKind::Invalid,
            "web_search needs a query",
        ));
    }
    if query.chars().count() > MAX_QUERY {
        return Answer::error(&AiError::new(
            ErrorKind::Invalid,
            format!("the query is over {MAX_QUERY} characters: shorten it"),
        ));
    }
    let count = input["count"]
        .as_u64()
        .unwrap_or(DEFAULT_COUNT)
        .clamp(1, MAX_COUNT);
    match search.run(query, count).await {
        Ok(hits) => Answer {
            content: Value::Array(hits.iter().map(Hit::to_json).collect()).to_string(),
            is_error: false,
            summary: json!({
                "notes": [],
                "links": hits.iter().map(|h| json!({ "title": h.title, "url": h.url })).collect::<Vec<_>>(),
            }),
        },
        Err(e) => Answer::error(&e),
    }
}

// ---- the routes ------------------------------------------------------------------------------

type Shared = State<Arc<Writer>>;

fn configured(s: &state::State) -> Result<Value, ApiError> {
    let configured = s.has_secret_key() && s.get_secret(KEY)?.is_some();
    Ok(json!({ "configured": configured }))
}

/// `GET /api/integrations/brave`.
pub async fn status(State(w): Shared) -> Result<Json<Value>, ApiError> {
    blocking(w, |w| configured(w.state())).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBody {
    key: String,
}

/// `PUT /api/integrations/brave/key`.
pub async fn put_key(State(w): Shared, Json(b): Json<KeyBody>) -> Result<Json<Value>, ApiError> {
    let key = b.key.trim().to_owned();
    blocking(w, move |w| {
        let s = w.state();
        if key.is_empty() {
            match s.delete_secret(KEY) {
                Ok(()) | Err(state::Error::UnknownSecret(_)) => {}
                Err(e) => return Err(e.into()),
            }
        } else {
            s.set_secret(KEY, &key)?;
        }
        configured(s)
    })
    .await
    .map(Json)
}

/// `POST /api/integrations/brave/test`.
pub async fn test(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let began = Instant::now();
    let search = blocking(w, |w| Ok::<_, ApiError>(Search::load(w.state()))).await?;
    let result = match search {
        Some(s) => s.run(TEST_QUERY, MAX_COUNT).await,
        None => Err(AiError::new(
            ErrorKind::NotConfigured,
            "no Brave Search API key is stored",
        )),
    };
    let ms = began.elapsed().as_millis() as u64;
    Ok(Json(match result {
        Ok(hits) => json!({ "ok": true, "count": hits.len(), "ms": ms, "error": null }),
        Err(e) => json!({
            "ok": false, "count": null, "ms": ms,
            "error": { "kind": e.kind.as_str(), "message": e.message },
        }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_stripped_and_entities_decoded() {
        assert_eq!(
            strip_tags(
                "The <strong>vault</strong> &amp; its &#x27;notes&#39; &lt;3 &bogus; & more"
            ),
            "The vault & its 'notes' <3 &bogus; & more"
        );
    }

    #[test]
    fn the_trail_line_holds_counts_and_outcome_only() {
        assert_eq!(
            trail_line(5, 3, None, Duration::from_millis(12)),
            "brave: count=5 results=3 outcome=ok ms=12"
        );
        assert_eq!(
            trail_line(5, 0, Some(ErrorKind::RateLimit), Duration::from_millis(7)),
            "brave: count=5 results=0 outcome=error:rate_limit ms=7"
        );
    }

    #[test]
    fn brave_errors_read_the_detail() {
        let body = r#"{"type":"ErrorResponse","error":{"id":"x","status":429,"code":"RATE_LIMITED","detail":"Request rate limit exceeded for plan."}}"#;
        assert_eq!(
            describe_error(reqwest::StatusCode::TOO_MANY_REQUESTS, body),
            "Brave answered 429 Too Many Requests: Request rate limit exceeded for plan."
        );
    }

    #[test]
    fn the_tool_is_a_strict_object() {
        let t = tool_spec();
        assert_eq!(t.input_schema["additionalProperties"], json!(false));
        let required = t.input_schema["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(required.contains(&json!("query")) && required.contains(&json!("count")));
    }
}
