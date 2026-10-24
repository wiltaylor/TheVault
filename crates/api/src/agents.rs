//! Connecting an AI agent that has a shell (D11, D32).
//!
//! An admin issues a one-time install code at `POST /api/agent-codes { name, scope }`. It works
//! once, for ten minutes, like a device pairing code. The agent's machine runs the one-line command
//! the answer carries, which fetches `GET /agent/install.sh` and runs it with the code. The script
//! installs the `vault` CLI from `/api/clients/cli/linux-<arch>`, redeems the code at the
//! unauthenticated `POST /api/agent-codes/redeem { code }` for an `agent-<name>` token, and writes
//! the CLI's config and the agent skill. A used, expired or unknown code answers 410.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use state::Scope;
use writer::Writer;

use crate::devices::{check_server_url, server_url_from};
use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

/// How long an install code works.
const CODE_TTL_MS: i64 = 10 * 60 * 1000;

/// The install script, with `@VAULT_URL@` where the server's URL goes.
const INSTALL_SH: &str = include_str!("install.sh");

/// The longest name an agent may have: its token is `agent-<name>`, and a token name is at most
/// 64 characters.
const MAX_NAME: usize = 64 - "agent-".len();

fn check_name(name: &str) -> Result<(), ApiError> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "agent name {name:?} must be 1 to {MAX_NAME} letters, digits, `-`, `_` or `.`"
        )))
    }
}

/// The server's URL as it goes into a shell script and a command line: an http or https URL of
/// characters that need no quoting.
fn script_url(url: &str) -> Result<String, ApiError> {
    let url = check_server_url(url)?;
    let plain = url
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '/' | '-' | '_' | '[' | ']'));
    if plain {
        Ok(url)
    } else {
        Err(ApiError::bad_request(format!(
            "server_url {url:?} holds characters a shell command would need quoted"
        )))
    }
}

fn request_url(given: Option<String>, headers: &HeaderMap) -> Result<String, ApiError> {
    match given.or_else(|| server_url_from(headers)) {
        Some(url) => script_url(&url),
        None => Err(ApiError::bad_request(
            "the request has no Host header; send server_url",
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    name: String,
    scope: String,
    #[serde(default)]
    server_url: Option<String>,
}

/// `POST /api/agent-codes`: issue an install code for an agent. The answer carries the code, when
/// it expires, the token it makes and the one-line command that installs the agent.
pub async fn create(
    State(w): Shared,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Result<Response, ApiError> {
    check_name(&body.name)?;
    let scope: Scope = body.scope.parse()?;
    let server_url = request_url(body.server_url, &headers)?;
    let name = body.name;
    let issued = blocking(w, move |w| {
        w.state().create_agent_code(&name, scope, CODE_TTL_MS)
    })
    .await?;
    let command = format!(
        "curl -fsSL {server_url}/agent/install.sh | sh -s -- {}",
        issued.code
    );
    let out = json!({
        "code": issued.code,
        "expires": issued.expires_ms,
        "name": issued.token_name,
        "scope": issued.scope.as_str(),
        "server_url": server_url,
        "command": command,
    });
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemBody {
    code: String,
}

/// `POST /api/agent-codes/redeem`: use up an install code for its token. The answer is the only
/// time the token's secret is shown.
pub async fn redeem(
    State(w): Shared,
    headers: HeaderMap,
    Json(body): Json<RedeemBody>,
) -> Result<Response, ApiError> {
    let server_url = request_url(None, &headers)?;
    let redeemed = blocking(w, move |w| {
        let redeemed = w.state().redeem_agent_code(&body.code)?;
        if let Some((info, _)) = &redeemed {
            w.state().record_activity(
                state::NewActivity::new(
                    "auth.token_created",
                    "auth",
                    &state::Author::Token(info.name.clone()),
                    format!(
                        "An install code made the {} token {}",
                        info.scope.as_str(),
                        info.name
                    ),
                )
                .subject("token", &info.name)
                .detail(json!({ "name": info.name, "scope": info.scope.as_str(), "with": "install_code" })),
            );
        }
        Ok::<_, state::Error>(redeemed)
    })
    .await?;
    let Some((info, secret)) = redeemed else {
        return Err(ApiError::new(
            StatusCode::GONE,
            "gone",
            "the install code is wrong, used or expired; create a new one in Settings > API tokens",
        ));
    };
    let out = json!({
        "server_url": server_url,
        "token": secret,
        "name": info.name,
        "scope": info.scope.as_str(),
    });
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

/// `GET /agent/install.sh`: the install script, with the server's URL as the request reached it.
pub async fn install_script(headers: HeaderMap) -> Result<Response, ApiError> {
    let server_url = request_url(None, &headers)?;
    let script = INSTALL_SH.replace("@VAULT_URL@", &server_url);
    let mut res = script.into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/x-shellscript; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_url_needs_no_quoting() {
        assert_eq!(
            script_url("https://vault.example:8443/").unwrap(),
            "https://vault.example:8443"
        );
        assert!(script_url("http://[::1]:8080").is_ok());
        for bad in [
            "https://a'b",
            "https://a$(id)",
            "https://a;b",
            "ftp://x",
            "https://a b",
        ] {
            assert!(script_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_script_names_no_placeholder_but_the_url() {
        assert_eq!(INSTALL_SH.matches("@VAULT_URL@").count(), 1);
        assert!(INSTALL_SH.starts_with("#!/bin/sh\n"));
    }
}
