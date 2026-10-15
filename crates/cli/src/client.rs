//! The HTTP client every API command goes through. It adds no behaviour of its own (D32): it sends
//! the request, and hands back the JSON the server answered with.

use reqwest::Method;
use reqwest::Url;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::Fail;

/// One server, reached with one token.
pub struct Api {
    base: Url,
    token: Option<String>,
    http: Client,
}

/// A request body.
pub enum Body {
    None,
    Json(Value),
    Bytes(Vec<u8>),
}

impl Api {
    pub fn new(base: &str, token: Option<String>) -> Result<Api, Fail> {
        let base = Url::parse(base)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && !u.cannot_be_a_base())
            .ok_or_else(|| {
                Fail::new(
                    "usage",
                    format!("the server URL {base:?} is not an http or https URL"),
                )
            })?;
        let http = Client::builder()
            .timeout(None)
            .build()
            .map_err(|e| Fail::new("internal", e.to_string()))?;
        Ok(Api {
            base,
            token: token.filter(|t| !t.is_empty()),
            http,
        })
    }

    /// The URL of `segments` under the base URL, each one percent-encoded, with `query` appended.
    fn url(&self, segments: &[&str], query: &[(&str, String)]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (k, v.as_str())));
        }
        url
    }

    fn request(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> RequestBuilder {
        let mut rb = self
            .http
            .request(method, self.url(segments, query))
            .header(ACCEPT, "application/json");
        if let Some(token) = &self.token {
            rb = rb.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        rb
    }

    /// Send a request and return the JSON the server answered. A status other than 2xx fails
    /// with the server's error body and the status.
    pub fn call(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
        body: Body,
    ) -> Result<Value, Fail> {
        let res = self.send(method, segments, query, body)?;
        let status = res.status();
        let text = res.text().map_err(connect)?;
        let value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text }))
        };
        if status.is_success() {
            Ok(value)
        } else {
            Err(Fail::api(status.as_u16(), value))
        }
    }

    /// Send a request and return the raw response bytes, for downloads. A status other than 2xx
    /// fails as [`Api::call`] does.
    pub fn download(&self, segments: &[&str]) -> Result<Vec<u8>, Fail> {
        let res = self.send(Method::GET, segments, &[], Body::None)?;
        let status = res.status();
        let bytes = res.bytes().map_err(connect)?;
        if status.is_success() {
            return Ok(bytes.to_vec());
        }
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "text": String::from_utf8_lossy(&bytes) }));
        Err(Fail::api(status.as_u16(), value))
    }

    fn send(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
        body: Body,
    ) -> Result<Response, Fail> {
        let rb = self.request(method, segments, query);
        let rb = match body {
            Body::None => rb,
            Body::Json(v) => rb.json(&v),
            Body::Bytes(b) => rb.header(CONTENT_TYPE, "application/octet-stream").body(b),
        };
        rb.send().map_err(connect)
    }

    pub fn get(&self, segments: &[&str]) -> Result<Value, Fail> {
        self.call(Method::GET, segments, &[], Body::None)
    }

    pub fn post(&self, segments: &[&str], body: Value) -> Result<Value, Fail> {
        self.call(Method::POST, segments, &[], Body::Json(body))
    }

    pub fn put(&self, segments: &[&str], body: Value) -> Result<Value, Fail> {
        self.call(Method::PUT, segments, &[], Body::Json(body))
    }

    pub fn patch(&self, segments: &[&str], body: Value) -> Result<Value, Fail> {
        self.call(Method::PATCH, segments, &[], Body::Json(body))
    }

    pub fn delete(&self, segments: &[&str]) -> Result<Value, Fail> {
        self.call(Method::DELETE, segments, &[], Body::None)
    }
}

fn connect(e: reqwest::Error) -> Fail {
    Fail::new("connect", e.to_string())
}
