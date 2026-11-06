//! The home server's notifications, as the phone mirrors them (D4, D9).
//!
//! A push message names an event and a notification id, nothing more, so the phone reads the
//! rest here: [`unread`] lists what it should be showing, [`mark_read`] is a swipe or a tap, and
//! [`set_push`] registers the UnifiedPush endpoint and WebPush keys the server pushes to. The
//! Kotlin side shows and cancels the Android notifications; these calls are its way to the
//! server, through the core's link.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::Link;

/// How long one request waits for the server.
const TIMEOUT: Duration = Duration::from_secs(30);

/// How long opening a connection to the server may take. A server that cannot be reached (the
/// phone off its network, the server down) fails here, not after [`TIMEOUT`], so a round knows at
/// once (TV-59).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A request that did not reach the server, or got no answer: no connection, a timeout, a broken
/// connection. An answer the server refused is not one.
#[derive(Debug)]
pub struct Unreachable(pub String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

/// The [`Unreachable`] message of `e`, if it is one.
pub fn unreachable(e: &anyhow::Error) -> Option<&str> {
    e.downcast_ref::<Unreachable>().map(|u| u.0.as_str())
}

/// How many unread notifications one request asks for: the server's largest page.
const PAGE: usize = 500;

/// How many pages [`unread`] follows before it stops.
const MAX_PAGES: usize = 10;

/// A UnifiedPush endpoint and the WebPush keys (RFC 8291) the server encrypts to, base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTarget {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

pub(crate) fn client() -> anyhow::Result<Client> {
    Ok(Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT)
        .build()?)
}

pub(crate) fn send(link: &Link, rb: RequestBuilder) -> anyhow::Result<Response> {
    rb.bearer_auth(&link.token)
        .send()
        .map_err(|e| Unreachable(format!("cannot reach {}: {e}", link.server_url)).into())
}

/// The error for an answer that is not a success: the server's message, else its status.
pub(crate) fn refused(res: Response) -> anyhow::Error {
    let status = res.status();
    let answer: Value = res.json().unwrap_or(Value::Null);
    let message = answer["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("the server answered {status}"));
    anyhow::anyhow!("{message} ({status})")
}

/// Every unread notification on the home server, newest first, as the API sends them:
/// `{id, created, title, body, note, note_type, note_title, url, source, read}`.
pub fn unread(link: &Link) -> anyhow::Result<Vec<Value>> {
    let client = client()?;
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut query = vec![("unread", "true".to_owned()), ("limit", PAGE.to_string())];
        if let Some(c) = &cursor {
            query.push(("cursor", c.clone()));
        }
        let rb = client
            .get(format!("{}/api/notifications", link.server_url))
            .query(&query);
        let res = send(link, rb)?;
        if !res.status().is_success() {
            return Err(refused(res));
        }
        let page: Value = res.json()?;
        if let Some(found) = page["items"].as_array() {
            items.extend(found.iter().cloned());
        }
        match page["next"].as_str() {
            Some(next) if !next.is_empty() => cursor = Some(next.to_owned()),
            _ => break,
        }
    }
    Ok(items)
}

/// Mark notification `id` read on the home server. A notification the server no longer has
/// (dismissed or cleared meanwhile) needs nothing more, so that is not an error: the answer says
/// whether the server had it.
pub fn mark_read(link: &Link, id: &str) -> anyhow::Result<bool> {
    let rb = client()?.post(format!("{}/api/notifications/{id}/read", link.server_url));
    let res = send(link, rb)?;
    match res.status() {
        s if s.is_success() => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        _ => Err(refused(res)),
    }
}

/// Register this device's UnifiedPush endpoint with the home server, or clear it with `None`.
pub fn set_push(link: &Link, target: Option<&PushTarget>) -> anyhow::Result<()> {
    let body = match target {
        Some(t) => json!({ "endpoint": t.endpoint, "p256dh": t.p256dh, "auth": t.auth }),
        None => json!({ "endpoint": null }),
    };
    let rb = client()?
        .put(format!(
            "{}/api/devices/{}/push",
            link.server_url, link.device_id
        ))
        .json(&body);
    let res = send(link, rb)?;
    if !res.status().is_success() {
        return Err(refused(res));
    }
    Ok(())
}
