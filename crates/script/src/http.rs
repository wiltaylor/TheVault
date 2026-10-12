//! The `http` host module: blocking HTTP for `after_*` handlers and scheduled tasks only, checked
//! against the run's domain allowlist (D10, D30). wscript has no HTTP of its own (notes/01).

use std::collections::HashMap;
use std::io::Read;

use reqwest::blocking::Client;
use reqwest::{Method, Url};
use wscript::{Fault, HostError, Module, Script};

use crate::run;

/// What an HTTP call returned. Header names are lower case.
#[derive(Script, Debug, Clone)]
#[script(name = "Response")]
pub struct Response {
    pub status: i64,
    pub headers: HashMap<String, String>,
    pub body: String,
}

type Reply = Fault<Result<Response, String>>;

pub fn module(client: Client) -> Module {
    let mut m = Module::new("http");
    m.doc("HTTP to the hosts on this script's allowlist");
    let c = client.clone();
    m.fn_named(
        "get",
        ["url", "headers"],
        move |url: String, headers: HashMap<String, String>| -> Reply {
            Fault(call(&c, "GET", &url, headers, String::new()))
        },
    );
    let c = client.clone();
    m.fn_named(
        "post",
        ["url", "headers", "body"],
        move |url: String, headers: HashMap<String, String>, body: String| -> Reply {
            Fault(call(&c, "POST", &url, headers, body))
        },
    );
    m.fn_named(
        "request",
        ["method", "url", "headers", "body"],
        move |method: String,
              url: String,
              headers: HashMap<String, String>,
              body: String|
              -> Reply { Fault(call(&client, &method, &url, headers, body)) },
    );
    m
}

/// A client that follows no redirects, so every host a script reaches is one it named and the
/// allowlist checked.
pub fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build the HTTP client")
}

/// `Ok(Err(..))` is a failure the script sees; `Err(..)` faults the run, which only running out
/// of time does.
fn call(
    client: &Client,
    method: &str,
    url: &str,
    headers: HashMap<String, String>,
    body: String,
) -> Result<Result<Response, String>, HostError> {
    let (remaining, allow, max_bytes) = run::with(|r| {
        Ok((
            r.remaining()?,
            r.job.allow_domains.clone(),
            r.limits.max_http_response,
        ))
    })?;
    let url = match Url::parse(url) {
        Ok(u) => u,
        Err(e) => return Ok(Err(format!("{url:?} is not a URL: {e}"))),
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Ok(Err(format!("{url} is not an http or https URL")));
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !allow
        .iter()
        .any(|d| d.trim_end_matches('.').eq_ignore_ascii_case(&host))
    {
        return Ok(Err(format!(
            "host {host:?} is not on this script's allowlist"
        )));
    }
    let method = match Method::from_bytes(method.to_ascii_uppercase().as_bytes()) {
        Ok(m) => m,
        Err(_) => return Ok(Err(format!("{method:?} is not an HTTP method"))),
    };
    let mut req = client.request(method, url.clone()).timeout(remaining);
    for (name, value) in &headers {
        req = req.header(name, value);
    }
    if !body.is_empty() {
        req = req.body(body);
    }
    let res = match req.send() {
        Ok(r) => r,
        Err(e) => return failed(e, &url),
    };
    let status = i64::from(res.status().as_u16());
    let headers = res
        .headers()
        .iter()
        .map(|(n, v)| {
            (
                n.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let mut bytes = Vec::new();
    let read = res
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes);
    if let Err(e) = read {
        run::with(|r| r.remaining())?;
        return Ok(Err(format!("reading the response from {url}: {e}")));
    }
    if bytes.len() > max_bytes {
        return Ok(Err(format!(
            "the response from {url} is larger than the {max_bytes} bytes a script may read"
        )));
    }
    Ok(Ok(Response {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }))
}

fn failed(e: reqwest::Error, url: &Url) -> Result<Result<Response, String>, HostError> {
    // A request cut short by the run's own deadline ends the run.
    run::with(|r| r.remaining())?;
    let why = if e.is_timeout() {
        "timed out".to_owned()
    } else if e.is_connect() {
        "could not connect".to_owned()
    } else {
        e.to_string()
    };
    Ok(Err(format!("{url}: {why}")))
}
