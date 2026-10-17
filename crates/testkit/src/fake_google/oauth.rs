//! The fake OAuth endpoints: the token endpoint at [`super::TOKEN_PATH`] and userinfo at
//! [`super::USERINFO_PATH`], with the consent step a browser would do as
//! [`FakeGoogle::consent`], or over HTTP at [`AUTH_PATH`].
//!
//! The token endpoint takes a form-urlencoded POST, as Google's does, with the client id and
//! secret in the body:
//!
//! - `grant_type=authorization_code` exchanges a code [`FakeGoogle::consent`] issued, once, with
//!   the same `redirect_uri` and, when the authorization URL carried a `code_challenge`, the
//!   `code_verifier` whose S256 hash it was. It answers an access token and the refresh token.
//! - `grant_type=refresh_token` with the refresh token answers a new access token and no refresh
//!   token, since Google does not rotate them.
//!
//! A wrong client id or secret answers 401 `invalid_client`. A wrong code, verifier, redirect or
//! refresh token answers 400 `invalid_grant`, and so does every token request while
//! [`FakeGoogle::set_invalid_grant`] is on. A body that is not a form answers 400
//! `invalid_request`. Userinfo answers `{ sub, email, email_verified }` for an access token the
//! endpoint issued, and 401 otherwise.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{FakeGoogle, FakeGoogleBuilder, Shared, TOKEN_PATH, USERINFO_PATH, lock};
use crate::standin::{Reply, Request, url_pairs};

/// The client id the fake accepts unless [`FakeGoogleBuilder::client`] says otherwise.
pub const DEFAULT_CLIENT_ID: &str = "fake-client-id.apps.googleusercontent.com";

/// The client secret the fake accepts unless [`FakeGoogleBuilder::client`] says otherwise.
pub const DEFAULT_CLIENT_SECRET: &str = "fake-client-secret";

/// The refresh token the fake issues and accepts unless [`FakeGoogleBuilder::refresh_token`]
/// says otherwise.
pub const DEFAULT_REFRESH_TOKEN: &str = "fake-refresh-token";

/// The account address userinfo answers unless [`FakeGoogleBuilder::account`] says otherwise.
pub const DEFAULT_ACCOUNT: &str = "me@example.com";

/// How the fake OAuth starts.
#[derive(Debug, Clone)]
pub(super) struct Config {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    account: String,
    invalid_grant: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            client_id: DEFAULT_CLIENT_ID.to_owned(),
            client_secret: DEFAULT_CLIENT_SECRET.to_owned(),
            refresh_token: DEFAULT_REFRESH_TOKEN.to_owned(),
            account: DEFAULT_ACCOUNT.to_owned(),
            invalid_grant: false,
        }
    }
}

impl FakeGoogleBuilder {
    /// The OAuth client id and secret the token endpoint accepts.
    pub fn client(mut self, id: &str, secret: &str) -> Self {
        self.oauth.client_id = id.to_owned();
        self.oauth.client_secret = secret.to_owned();
        self
    }

    /// The refresh token a code exchange issues and a refresh accepts.
    pub fn refresh_token(mut self, token: &str) -> Self {
        self.oauth.refresh_token = token.to_owned();
        self
    }

    /// The address userinfo answers.
    pub fn account(mut self, email: &str) -> Self {
        self.oauth.account = email.to_owned();
        self
    }

    /// Start with every token request answering `invalid_grant`.
    pub fn invalid_grant(mut self, on: bool) -> Self {
        self.oauth.invalid_grant = on;
        self
    }
}

/// A code [`FakeGoogle::consent`] issued and the token endpoint has not yet exchanged.
#[derive(Debug)]
struct Code {
    code: String,
    redirect_uri: String,
    challenge: Option<String>,
    scope: String,
}

/// The fake OAuth's clients, codes and tokens.
#[derive(Debug)]
pub(super) struct State {
    config: Config,
    codes: Vec<Code>,
    access_tokens: Vec<String>,
    issued: usize,
}

impl State {
    pub(super) fn new(config: Config) -> State {
        State {
            config,
            codes: Vec::new(),
            access_tokens: Vec::new(),
            issued: 0,
        }
    }

    pub(super) fn has_access_token(&self, token: &str) -> bool {
        self.access_tokens.iter().any(|t| t == token)
    }

    fn next(&mut self, what: &str) -> String {
        self.issued += 1;
        format!("fake-{what}-{}", self.issued)
    }
}

impl FakeGoogle {
    /// Turn the `invalid_grant` answer to every token request on or off, as a revoked or expired
    /// refresh token would.
    pub fn set_invalid_grant(&self, on: bool) {
        self.oauth().config.invalid_grant = on;
    }

    /// Change the address userinfo answers.
    pub fn set_account(&self, email: &str) {
        self.oauth().config.account = email.to_owned();
    }

    /// What Google's consent screen does with `auth_url` once the user agrees: issue a code for
    /// its `redirect_uri`, `code_challenge` and `scope`, and return the URL the browser is sent
    /// to, `<redirect_uri>?state=<state>&code=<code>&scope=<scope>`. The authorization URL's host
    /// is not checked, since it is the browser that visits it.
    ///
    /// # Panics
    ///
    /// When `auth_url` has no `redirect_uri`.
    pub fn consent(&self, auth_url: &str) -> String {
        let query = auth_url.split_once('?').map(|(_, q)| q).unwrap_or_default();
        consent(&mut self.oauth(), query).expect("the authorization URL has a redirect_uri")
    }

    /// Every access token the token endpoint has issued, oldest first.
    pub fn access_tokens(&self) -> Vec<String> {
        self.oauth().access_tokens.clone()
    }
}

/// The path of the fake consent page, Google's own authorization path. A browser, or a test
/// standing in for one, that GETs it with an authorization URL's query is answered as Google
/// answers once the user agrees: a 302 to the redirect, with the code [`FakeGoogle::consent`]
/// would issue. A query without a `redirect_uri` answers 400 `invalid_request`.
pub const AUTH_PATH: &str = "/o/oauth2/v2/auth";

/// Issue a code for the authorization URL query `query` and return where the browser goes next,
/// or `None` when the query has no `redirect_uri`.
fn consent(oauth: &mut State, query: &str) -> Option<String> {
    let pairs = url_pairs(query);
    let param = |name: &str| {
        pairs
            .iter()
            .find_map(|(k, v)| (k == name).then(|| v.clone()))
    };
    let redirect_uri = param("redirect_uri")?;
    let code = oauth.next("code");
    let scope = param("scope").unwrap_or_default();
    oauth.codes.push(Code {
        code: code.clone(),
        redirect_uri: redirect_uri.clone(),
        challenge: param("code_challenge"),
        scope: scope.clone(),
    });
    let mut query = String::new();
    if let Some(state) = param("state") {
        query.push_str(&format!("state={}&", encode(&state)));
    }
    query.push_str(&format!("code={}&scope={}", encode(&code), encode(&scope)));
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    Some(format!("{redirect_uri}{sep}{query}"))
}

/// Percent-encode `s` for a query string.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Answer a token or userinfo request, or `None` when `request` is neither.
pub(super) fn handle(shared: &Shared, request: &Request) -> Option<Reply> {
    match (request.method.as_str(), request.route()) {
        ("POST", TOKEN_PATH) => Some(token(&mut lock(&shared.oauth), request)),
        ("GET", AUTH_PATH) => {
            let query = request
                .path
                .split_once('?')
                .map(|(_, q)| q)
                .unwrap_or_default();
            Some(match consent(&mut lock(&shared.oauth), query) {
                Some(to) => Reply::text(302, "").header("location", &to),
                None => oauth_error(
                    400,
                    "invalid_request",
                    "Missing required parameter: redirect_uri",
                ),
            })
        }
        (_, TOKEN_PATH) => Some(oauth_error(405, "invalid_request", "Use POST.")),
        ("GET", USERINFO_PATH) => Some(if super::authorized(shared, request) {
            let account = lock(&shared.oauth).config.account.clone();
            Reply::json(
                200,
                &json!({ "sub": "100000000000000000001", "email": account, "email_verified": true }),
            )
        } else {
            super::unauthorized()
        }),
        _ => None,
    }
}

/// The error body of Google's token endpoint: `{ "error", "error_description" }`.
fn oauth_error(status: u16, error: &str, description: &str) -> Reply {
    Reply::json(
        status,
        &json!({ "error": error, "error_description": description }),
    )
}

fn token(oauth: &mut State, request: &Request) -> Reply {
    let form = request
        .header("content-type")
        .is_some_and(|c| c.starts_with("application/x-www-form-urlencoded"));
    if !form {
        return oauth_error(
            400,
            "invalid_request",
            "The request body must be application/x-www-form-urlencoded.",
        );
    }
    let field = |name: &str| request.form(name).unwrap_or_default();
    if field("client_id") != oauth.config.client_id
        || field("client_secret") != oauth.config.client_secret
    {
        return oauth_error(401, "invalid_client", "The OAuth client was not found.");
    }
    match field("grant_type").as_str() {
        "refresh_token" => {
            if oauth.config.invalid_grant || field("refresh_token") != oauth.config.refresh_token {
                return oauth_error(400, "invalid_grant", "Token has been expired or revoked.");
            }
            let access = oauth.next("access-token");
            oauth.access_tokens.push(access.clone());
            Reply::json(
                200,
                &json!({
                    "access_token": access,
                    "expires_in": 3599,
                    "scope": "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/calendar openid https://www.googleapis.com/auth/userinfo.email",
                    "token_type": "Bearer",
                }),
            )
        }
        "authorization_code" => {
            if oauth.config.invalid_grant {
                return oauth_error(400, "invalid_grant", "Bad Request");
            }
            let code = field("code");
            let Some(k) = oauth.codes.iter().position(|c| c.code == code) else {
                return oauth_error(400, "invalid_grant", "Malformed auth code.");
            };
            // A code is used once, whether or not the exchange succeeds.
            let issued = oauth.codes.remove(k);
            if field("redirect_uri") != issued.redirect_uri {
                return oauth_error(400, "redirect_uri_mismatch", "Bad Request");
            }
            if let Some(challenge) = &issued.challenge {
                let verifier = field("code_verifier");
                if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != *challenge {
                    return oauth_error(400, "invalid_grant", "Invalid code verifier.");
                }
            }
            let access = oauth.next("access-token");
            oauth.access_tokens.push(access.clone());
            Reply::json(
                200,
                &json!({
                    "access_token": access,
                    "expires_in": 3599,
                    "refresh_token": oauth.config.refresh_token,
                    "scope": issued.scope,
                    "token_type": "Bearer",
                }),
            )
        }
        _ => oauth_error(400, "unsupported_grant_type", "Invalid grant_type."),
    }
}
