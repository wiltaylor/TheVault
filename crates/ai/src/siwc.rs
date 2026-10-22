//! Sign in with ChatGPT, OpenAI's official route for a ChatGPT plan (ai D2, D8, D9): the pieces
//! the Settings > AI routes put together, and the refresh [`crate::Credentials::resolve`] runs.
//!
//! Nothing here uses the Codex CLI's client id, `chatgpt.com/backend-api`, an `originator`
//! header or `~/.codex/auth.json` (ai D2).
//!
//! Every function takes the sign-in server's base URL, `ai_settings.endpoints.chatgpt_auth` or
//! [`crate::settings::CHATGPT_AUTH_BASE`], and the HTTP ones block: from async code call them
//! inside `tokio::task::spawn_blocking`.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::credentials::Secret;
use crate::error::{AiError, ErrorKind, scrub};
use crate::types::Provider;

/// The authorize page, under the sign-in base.
pub const AUTHORIZE_PATH: &str = "/api/accounts/authorize";

/// The token endpoint, under the sign-in base: code exchange and refresh.
pub const TOKEN_PATH: &str = "/api/accounts/oauth/token";

/// The OpenID configuration, under the sign-in base, which names the JWKS and the revocation
/// endpoint.
pub const OPENID_CONFIGURATION_PATH: &str = "/.well-known/openid-configuration";

/// The client id of a first sign-in, before OpenAI issues one.
pub const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";

/// The `agent_name_hint` of a first sign-in.
pub const AGENT_NAME: &str = "TheVault";

/// Where the browser is sent back to: a loopback port where nothing listens, so the user pastes
/// the address the browser failed to load.
pub const REDIRECT_URI: &str = "http://127.0.0.1:53683/auth/callback";

/// The scopes asked for.
pub const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

/// The scope without which the account is signed in but cannot use its plan.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";

/// The resource every token is for.
pub const RESOURCE: &str = "https://api.openai.com/v1";

/// The tokens of a code exchange or a refresh. `Debug` shows none of them.
#[derive(Clone)]
pub struct TokenSet {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub id_token: Option<Secret>,
    /// Seconds the access token lasts.
    pub expires_in: Option<u64>,
    /// The earliest a refresh is allowed, in Unix milliseconds, from `earliest_refresh_at`.
    pub earliest_refresh_ms: Option<i64>,
    /// The scopes granted, space-separated.
    pub scope: Option<String>,
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &self.access_token)
            .field("refresh_token", &self.refresh_token)
            .field("id_token", &self.id_token)
            .field("expires_in", &self.expires_in)
            .field("earliest_refresh_ms", &self.earliest_refresh_ms)
            .field("scope", &self.scope)
            .finish()
    }
}

impl TokenSet {
    /// Whether the plan scope was granted.
    pub fn has_plan_scope(&self) -> bool {
        self.scope
            .as_deref()
            .is_some_and(|s| s.split_whitespace().any(|s| s == PLAN_SCOPE))
    }

    fn parse(body: &Value) -> Option<TokenSet> {
        let s = |k: &str| {
            body.get(k)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        Some(TokenSet {
            access_token: Secret::new(s("access_token")?),
            refresh_token: s("refresh_token").map(Secret::new),
            id_token: s("id_token").map(Secret::new),
            expires_in: body.get("expires_in").and_then(number).map(|n| n as u64),
            earliest_refresh_ms: body
                .get("earliest_refresh_at")
                .and_then(number)
                .map(|n| if n < 1e12 { n * 1000.0 } else { n } as i64),
            scope: s("scope"),
        })
    }
}

/// A number, or a string holding one.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Why a refresh failed.
#[derive(Debug)]
pub enum RefreshError {
    /// The refresh token is dead: revoked, expired, reused or rotated away. The user must sign
    /// in again.
    InvalidGrant,
    Other(AiError),
}

/// The OAuth error codes that mean the refresh token is dead.
const DEAD_GRANT: [&str; 4] = [
    "invalid_grant",
    "refresh_token_expired",
    "refresh_token_reused",
    "refresh_token_invalidated",
];

fn token_call(
    http: &reqwest::blocking::Client,
    auth_base: &str,
    form: &[(&str, &str)],
    secrets: &[&str],
) -> Result<TokenSet, RefreshError> {
    let url = format!("{}{TOKEN_PATH}", auth_base.trim_end_matches('/'));
    let fail = |kind: ErrorKind, message: String| {
        let mut message = message;
        for s in secrets {
            message = scrub(&message, s);
        }
        RefreshError::Other(AiError::new(kind, message).on(Provider::ChatGpt, ""))
    };
    let resp = http
        .post(&url)
        .header("accept", "application/json")
        .form(form)
        .send()
        .map_err(|e| {
            let kind = if e.is_timeout() {
                ErrorKind::Timeout
            } else {
                ErrorKind::Unavailable
            };
            fail(
                kind,
                format!("the ChatGPT sign-in server could not be reached: {e}"),
            )
        })?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return TokenSet::parse(&body).ok_or_else(|| {
            fail(
                ErrorKind::Provider,
                "the ChatGPT sign-in server answered no access token".to_owned(),
            )
        });
    }
    let code = body
        .get("error")
        .and_then(|e| e.as_str().or_else(|| e.get("code").and_then(Value::as_str)))
        .unwrap_or_default();
    if DEAD_GRANT.contains(&code) {
        return Err(RefreshError::InvalidGrant);
    }
    let description = body
        .get("error_description")
        .and_then(Value::as_str)
        .or_else(|| body.pointer("/error/message").and_then(Value::as_str))
        .unwrap_or(code);
    let kind = match status {
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimit,
        500..=599 => ErrorKind::Unavailable,
        _ => ErrorKind::Invalid,
    };
    Err(fail(
        kind,
        format!("the ChatGPT sign-in server answered {status}: {description}"),
    ))
}

/// Refresh the token pair: a form POST of `grant_type=refresh_token`, the issued `client_id`,
/// the `refresh_token` and the `resource`, without `scope`. The answer carries the rotated
/// refresh token, which the caller must store before anything else uses the old one.
pub fn refresh(
    http: &reqwest::blocking::Client,
    auth_base: &str,
    client_id: &str,
    refresh_token: &Secret,
) -> Result<TokenSet, RefreshError> {
    token_call(
        http,
        auth_base,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token.expose()),
            ("resource", RESOURCE),
        ],
        &[refresh_token.expose()],
    )
}

/// Exchange the authorization code of the pasted callback address: a form POST of
/// `grant_type=authorization_code` with the issued `client_id`, the `code`, the PKCE
/// `code_verifier`, [`REDIRECT_URI`] and [`RESOURCE`]. No client secret. Check the ID token
/// ([`verify_id_token`]) and [`TokenSet::has_plan_scope`] before storing the tokens.
pub fn exchange_code(
    http: &reqwest::blocking::Client,
    auth_base: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
) -> Result<TokenSet, AiError> {
    token_call(
        http,
        auth_base,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", REDIRECT_URI),
            ("resource", RESOURCE),
        ],
        &[code, code_verifier],
    )
    .map_err(|e| match e {
        RefreshError::InvalidGrant => AiError::new(
            ErrorKind::Auth,
            "the ChatGPT sign-in code was refused: start the sign-in again",
        )
        .on(Provider::ChatGpt, ""),
        RefreshError::Other(e) => e,
    })
}

/// What the authorize URL carries.
#[derive(Debug, Clone)]
pub struct Authorize<'a> {
    /// The issued client id of an earlier sign-in, or `None` for a first one, which sends
    /// [`DYNAMIC_CLIENT_ID`] and [`AGENT_NAME`].
    pub client_id: Option<&'a str>,
    /// `ext_agent_host_id`, kept across sign-ins.
    pub host_id: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    /// The PKCE challenge, `S256` of the verifier ([`pkce`]).
    pub code_challenge: &'a str,
    /// `id_token_hint` and `login_hint`, sent on a later sign-in when known.
    pub id_token_hint: Option<&'a str>,
    pub login_hint: Option<&'a str>,
}

/// The authorize page address to open in the browser (ai D8).
pub fn authorize_url(auth_base: &str, a: &Authorize<'_>) -> String {
    let base = format!("{}{AUTHORIZE_PATH}", auth_base.trim_end_matches('/'));
    let mut url = Url::parse(&base).unwrap_or_else(|_| {
        Url::parse(&format!(
            "{}{AUTHORIZE_PATH}",
            crate::settings::CHATGPT_AUTH_BASE
        ))
        .expect("the real authorize URL parses")
    });
    {
        let mut q = url.query_pairs_mut();
        match a.client_id {
            Some(id) => {
                q.append_pair("client_id", id);
                if let Some(h) = a.id_token_hint {
                    q.append_pair("id_token_hint", h);
                }
                if let Some(h) = a.login_hint {
                    q.append_pair("login_hint", h);
                }
            }
            None => {
                q.append_pair("client_id", DYNAMIC_CLIENT_ID);
                q.append_pair("agent_name_hint", AGENT_NAME);
            }
        }
        q.append_pair("ext_agent_host_id", a.host_id);
        q.append_pair("response_type", "code");
        q.append_pair("redirect_uri", REDIRECT_URI);
        q.append_pair("scope", SCOPES);
        q.append_pair("resource", RESOURCE);
        q.append_pair("state", a.state);
        q.append_pair("nonce", a.nonce);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("code_challenge", a.code_challenge);
    }
    url.into()
}

/// A PKCE pair: a random verifier, and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = URL_SAFE_NO_PAD.encode(random::<32>());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// A random value for `state` or `nonce`.
pub fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(random::<24>())
}

/// A new host id, `urn:uuid:<v4>`. Make it once and keep it in [`crate::ChatGptAuth`].
pub fn new_host_id() -> String {
    let mut b = random::<16>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn random<const N: usize>() -> [u8; N] {
    let hex = state::random_hex(N);
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    out
}

/// The parts of the OpenID configuration the sign-in uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenIdConfiguration {
    pub issuer: String,
    pub jwks_uri: String,
    pub revocation_endpoint: Option<String>,
}

fn get_json(http: &reqwest::blocking::Client, url: &str, what: &str) -> Result<Value, AiError> {
    let fail = |m: String| AiError::new(ErrorKind::Unavailable, m).on(Provider::ChatGpt, "");
    let resp = http
        .get(url)
        .header("accept", "application/json")
        .send()
        .map_err(|e| fail(format!("the ChatGPT {what} could not be read: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(fail(format!("the ChatGPT {what} answered {status}")));
    }
    resp.json()
        .map_err(|e| fail(format!("the ChatGPT {what} is not JSON: {e}")))
}

/// Read `<auth_base>/.well-known/openid-configuration`.
pub fn openid_configuration(
    http: &reqwest::blocking::Client,
    auth_base: &str,
) -> Result<OpenIdConfiguration, AiError> {
    let url = format!(
        "{}{OPENID_CONFIGURATION_PATH}",
        auth_base.trim_end_matches('/')
    );
    let v = get_json(http, &url, "OpenID configuration")?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(OpenIdConfiguration {
        issuer: s("issuer")
            .ok_or_else(|| AiError::invalid("the ChatGPT OpenID configuration names no issuer"))?,
        jwks_uri: s("jwks_uri").ok_or_else(|| {
            AiError::invalid("the ChatGPT OpenID configuration names no jwks_uri")
        })?,
        revocation_endpoint: s("revocation_endpoint"),
    })
}

/// Read the issuer's key set.
pub fn jwks(http: &reqwest::blocking::Client, jwks_uri: &str) -> Result<Value, AiError> {
    get_json(http, jwks_uri, "key set")
}

/// What a verified ID token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdClaims {
    pub subject: Option<String>,
    pub email: Option<String>,
    /// Expiry, in Unix seconds.
    pub expires: i64,
}

/// Check an RS256 ID token against `jwks`: its signature by the key its `kid` names, the issuer,
/// the audience (the issued client id), that it has not expired at `now_ms`, and the nonce.
pub fn verify_id_token(
    id_token: &str,
    jwks: &Value,
    issuer: &str,
    audience: &str,
    nonce: &str,
    now_ms: i64,
) -> Result<IdClaims, AiError> {
    let bad = |why: &str| {
        AiError::new(ErrorKind::Auth, format!("the ChatGPT ID token {why}"))
            .on(Provider::ChatGpt, "")
    };
    let mut parts = id_token.split('.');
    let (Some(h), Some(p), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad("is not a JWT"));
    };
    let decode = |s: &str| URL_SAFE_NO_PAD.decode(s.trim_end_matches('='));
    let header: Value = decode(h)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| bad("has an unreadable header"))?;
    let claims: Value = decode(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| bad("has unreadable claims"))?;
    let signature = decode(sig).map_err(|_| bad("has an unreadable signature"))?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(bad("is not signed with RS256"));
    }
    let kid = header.get("kid").and_then(Value::as_str);
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let key = keys
        .iter()
        .filter(|k| k.get("kty").and_then(Value::as_str) == Some("RSA"))
        .find(|k| kid.is_none() || k.get("kid").and_then(Value::as_str) == kid)
        .ok_or_else(|| bad("names a key the issuer does not publish"))?;
    let component = |k: &str| {
        key.get(k)
            .and_then(Value::as_str)
            .and_then(|v| decode(v).ok())
            .ok_or_else(|| bad("names a malformed key"))
    };
    let public = ring::signature::RsaPublicKeyComponents {
        n: component("n")?,
        e: component("e")?,
    };
    let signed = format!("{h}.{p}");
    public
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            signed.as_bytes(),
            &signature,
        )
        .map_err(|_| bad("has a bad signature"))?;
    let s = |k: &str| claims.get(k).and_then(Value::as_str);
    if s("iss").map(|i| i.trim_end_matches('/')) != Some(issuer.trim_end_matches('/')) {
        return Err(bad("is from another issuer"));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == audience,
        Some(Value::Array(a)) => a.iter().any(|v| v.as_str() == Some(audience)),
        _ => false,
    };
    if !aud_ok {
        return Err(bad("is for another client"));
    }
    let expires = claims
        .get("exp")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("has no expiry"))?;
    if expires.saturating_mul(1000) <= now_ms {
        return Err(bad("has expired"));
    }
    if s("nonce") != Some(nonce) {
        return Err(bad("carries the wrong nonce"));
    }
    Ok(IdClaims {
        subject: s("sub").map(str::to_owned),
        email: s("email").map(str::to_owned),
        expires,
    })
}

/// Revoke `refresh_token` at the revocation endpoint: a form POST of `token`,
/// `token_type_hint=refresh_token` and `client_id`. Best effort on disconnect.
pub fn revoke(
    http: &reqwest::blocking::Client,
    revocation_endpoint: &str,
    client_id: &str,
    refresh_token: &Secret,
) -> Result<(), AiError> {
    let fail = |m: String| {
        AiError::new(ErrorKind::Unavailable, scrub(&m, refresh_token.expose()))
            .on(Provider::ChatGpt, "")
    };
    let resp = http
        .post(revocation_endpoint)
        .form(&[
            ("token", refresh_token.expose()),
            ("token_type_hint", "refresh_token"),
            ("client_id", client_id),
        ])
        .send()
        .map_err(|e| {
            fail(format!(
                "the ChatGPT revocation endpoint could not be reached: {e}"
            ))
        })?;
    if !resp.status().is_success() {
        return Err(fail(format!(
            "the ChatGPT revocation endpoint answered {}",
            resp.status()
        )));
    }
    Ok(())
}
