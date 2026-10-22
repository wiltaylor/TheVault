//! A fake of OpenAI's Sign in with ChatGPT server (`auth.openai.com`): the token endpoint, the
//! JWKS, the OpenID configuration and revocation. The authorize page is never called: a
//! browser would visit it, so [`FakeSiwc::consent`] does what it does.
//!
//! ```no_run
//! use testkit::FakeSiwc;
//!
//! let auth = FakeSiwc::start();
//! // ai_settings.endpoints.chatgpt_auth = auth.base_url().
//! // A sign-in: the start route builds the authorize URL, the user consents...
//! # let auth_url = String::new();
//! let callback = auth.consent(&auth_url); // http://127.0.0.1:53683/auth/callback?code=..&state=..&client_id=oaiapp_..
//! // ... and the finish route exchanges the code and checks the ID token against the JWKS.
//! // A stored session to refresh, without signing in:
//! let refresh = auth.refresh_token(); // the refresh token it accepts now
//! auth.set_invalid_grant(true); // every token request now answers invalid_grant
//! ```
//!
//! The token endpoint takes form POSTs, as OpenAI's does:
//!
//! - `grant_type=authorization_code` with the issued `client_id`, a `code` [`FakeSiwc::consent`]
//!   issued (once), the same `redirect_uri`, the `code_verifier` whose S256 hash was the
//!   `code_challenge`, and `resource=https://api.openai.com/v1`. It answers `access_token`,
//!   `refresh_token`, an `id_token` signed RS256 with the test key (issuer [`FakeSiwc::issuer`],
//!   audience the issued client id, the authorize URL's `nonce`, `email` the account),
//!   `expires_in`, `scope` and `token_type`.
//! - `grant_type=refresh_token` with the issued `client_id`, the current refresh token and the
//!   `resource`, and no `scope`. It answers a new access token and a new refresh token: the old
//!   one stops working, so a second use of it answers `invalid_grant`, as rotation does.
//!
//! Both carry `earliest_refresh_at` (Unix seconds) when [`FakeSiwcBuilder::earliest_refresh_in`]
//! is set. A wrong client id answers 401 `invalid_client`; a wrong code, verifier, redirect or
//! refresh token answers 400 `invalid_grant`, as does every request while
//! [`FakeSiwc::set_invalid_grant`] is on. Revocation takes `token`, `token_type_hint` and
//! `client_id` and answers 200.
//!
//! It binds `127.0.0.1` on a free port. Every request is recorded, whatever it was answered.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::rand::SystemRandom;
use ring::rsa::{KeyPair, PublicKeyComponents};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::fake_http::{Answer, Listener, listen, lock};
use crate::standin::{Reply, Request, url_pairs};

/// The test signing key, an RSA 2048 key made for these tests and nothing else.
const TEST_KEY_PEM: &str = include_str!("fake_siwc/test_key.pem");

/// The `kid` of the test key.
pub const SIWC_KEY_ID: &str = "fake-siwc-key-1";

/// The client id a consent issues unless [`FakeSiwcBuilder::client_id`] says otherwise.
pub const DEFAULT_SIWC_CLIENT_ID: &str = "oaiapp_fake_thevault";

/// The refresh token the fake accepts at the start unless [`FakeSiwcBuilder::refresh_token`]
/// says otherwise.
pub const DEFAULT_SIWC_REFRESH_TOKEN: &str = "fake-siwc-refresh-0";

/// The account address the ID tokens carry unless [`FakeSiwcBuilder::account`] says otherwise.
pub const DEFAULT_SIWC_ACCOUNT: &str = "me@example.com";

/// The scopes granted unless [`FakeSiwcBuilder::scope`] says otherwise.
pub const DEFAULT_SIWC_SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

/// The resource every token request must name.
pub const SIWC_RESOURCE: &str = "https://api.openai.com/v1";

pub const SIWC_TOKEN_PATH: &str = "/api/accounts/oauth/token";
pub const SIWC_REVOKE_PATH: &str = "/api/accounts/oauth/revoke";
pub const SIWC_JWKS_PATH: &str = "/.well-known/jwks.json";
pub const SIWC_OPENID_PATH: &str = "/.well-known/openid-configuration";

/// How a [`FakeSiwc`] starts.
#[derive(Debug, Clone)]
pub struct FakeSiwcBuilder {
    client_id: String,
    refresh_token: String,
    account: String,
    scope: String,
    expires_in: u64,
    earliest_refresh_in: Option<u64>,
    invalid_grant: bool,
    refresh_delay: Duration,
}

impl Default for FakeSiwcBuilder {
    fn default() -> Self {
        FakeSiwcBuilder {
            client_id: DEFAULT_SIWC_CLIENT_ID.to_owned(),
            refresh_token: DEFAULT_SIWC_REFRESH_TOKEN.to_owned(),
            account: DEFAULT_SIWC_ACCOUNT.to_owned(),
            scope: DEFAULT_SIWC_SCOPE.to_owned(),
            expires_in: 3600,
            earliest_refresh_in: None,
            invalid_grant: false,
            refresh_delay: Duration::ZERO,
        }
    }
}

impl FakeSiwcBuilder {
    /// The `oaiapp_...` client id a consent issues and the token endpoint accepts.
    pub fn client_id(mut self, id: &str) -> Self {
        self.client_id = id.to_owned();
        self
    }

    /// The refresh token accepted before any exchange or refresh.
    pub fn refresh_token(mut self, token: &str) -> Self {
        self.refresh_token = token.to_owned();
        self
    }

    pub fn account(mut self, email: &str) -> Self {
        self.account = email.to_owned();
        self
    }

    /// The scopes a token answer grants; leave out `chatgpt.tokens.use.direct` to test an
    /// account that cannot use its plan.
    pub fn scope(mut self, scope: &str) -> Self {
        self.scope = scope.to_owned();
        self
    }

    /// The access tokens' `expires_in`, in seconds.
    pub fn expires_in(mut self, secs: u64) -> Self {
        self.expires_in = secs;
        self
    }

    /// Answer `earliest_refresh_at` this many seconds from now.
    pub fn earliest_refresh_in(mut self, secs: u64) -> Self {
        self.earliest_refresh_in = Some(secs);
        self
    }

    pub fn invalid_grant(mut self, on: bool) -> Self {
        self.invalid_grant = on;
        self
    }

    /// Hold each refresh this long before answering, so concurrent refreshes would overlap.
    pub fn refresh_delay(mut self, d: Duration) -> Self {
        self.refresh_delay = d;
        self
    }

    pub fn start(self) -> FakeSiwc {
        let der = STANDARD
            .decode(
                TEST_KEY_PEM
                    .lines()
                    .filter(|l| !l.starts_with("-----"))
                    .collect::<String>(),
            )
            .expect("the test key is base64");
        let key = KeyPair::from_pkcs8(&der).expect("the test key is an RSA PKCS#8 key");
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                current_refresh: self.refresh_token.clone(),
                config: self,
                codes: Vec::new(),
                access_tokens: Vec::new(),
                revoked: Vec::new(),
                refreshes: 0,
                issued: 0,
            }),
            requests: Mutex::default(),
            key,
            issuer: Mutex::new(String::new()),
        });
        let s = shared.clone();
        let listener = listen("the fake ChatGPT sign-in server", move |r| handle(&s, r));
        *lock(&shared.issuer) = format!("http://{}", listener.addr);
        FakeSiwc { listener, shared }
    }
}

#[derive(Debug)]
struct Code {
    code: String,
    redirect_uri: String,
    challenge: Option<String>,
    nonce: Option<String>,
}

struct State {
    config: FakeSiwcBuilder,
    codes: Vec<Code>,
    current_refresh: String,
    access_tokens: Vec<String>,
    revoked: Vec<String>,
    refreshes: usize,
    issued: usize,
}

impl State {
    fn next(&mut self, what: &str) -> String {
        self.issued += 1;
        format!("fake-siwc-{what}-{}", self.issued)
    }
}

struct Shared {
    state: Mutex<State>,
    requests: Mutex<Vec<Request>>,
    key: KeyPair,
    issuer: Mutex<String>,
}

/// A running fake sign-in server. Dropping it stops the server.
pub struct FakeSiwc {
    listener: Listener,
    shared: Arc<Shared>,
}

impl FakeSiwc {
    pub fn builder() -> FakeSiwcBuilder {
        FakeSiwcBuilder::default()
    }

    pub fn start() -> FakeSiwc {
        Self::builder().start()
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener.addr
    }

    /// `http://127.0.0.1:<port>`: the value for `ai_settings.endpoints.chatgpt_auth`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.listener.addr)
    }

    /// The `iss` of its ID tokens and the OpenID configuration: the base URL.
    pub fn issuer(&self) -> String {
        self.base_url()
    }

    /// The client id a consent issues.
    pub fn client_id(&self) -> String {
        lock(&self.shared.state).config.client_id.clone()
    }

    /// The refresh token it accepts now.
    pub fn refresh_token(&self) -> String {
        lock(&self.shared.state).current_refresh.clone()
    }

    /// Every access token issued so far, in order.
    pub fn access_tokens(&self) -> Vec<String> {
        lock(&self.shared.state).access_tokens.clone()
    }

    /// How many refreshes it answered with tokens.
    pub fn refresh_count(&self) -> usize {
        lock(&self.shared.state).refreshes
    }

    /// The tokens revoked so far.
    pub fn revoked(&self) -> Vec<String> {
        lock(&self.shared.state).revoked.clone()
    }

    pub fn set_invalid_grant(&self, on: bool) {
        lock(&self.shared.state).config.invalid_grant = on;
    }

    pub fn set_refresh_delay(&self, d: Duration) {
        lock(&self.shared.state).config.refresh_delay = d;
    }

    pub fn set_scope(&self, scope: &str) {
        lock(&self.shared.state).config.scope = scope.to_owned();
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }

    /// The token endpoint requests whose `grant_type` is `grant`.
    pub fn token_requests(&self, grant: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| {
                r.route() == SIWC_TOKEN_PATH && r.form("grant_type").as_deref() == Some(grant)
            })
            .collect()
    }

    /// The key set its ID tokens verify against, as `jwks_uri` serves it.
    pub fn jwks(&self) -> Value {
        jwks(&self.shared.key)
    }

    /// Sign `claims` as an RS256 JWT with the test key, for tests of ID-token checks.
    pub fn sign(&self, claims: &Value) -> String {
        sign(&self.shared.key, claims)
    }

    /// What the consent page does once the user agrees: issue a code for the authorize URL's
    /// `redirect_uri`, `code_challenge` and `nonce`, and return the address the browser is sent
    /// to, `<redirect_uri>?code=<code>&state=<state>&client_id=<issued client id>`.
    ///
    /// # Panics
    ///
    /// When `auth_url` has no `redirect_uri`.
    pub fn consent(&self, auth_url: &str) -> String {
        let query = auth_url.split_once('?').map(|(_, q)| q).unwrap_or_default();
        let pairs = url_pairs(query);
        let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let redirect = get("redirect_uri").expect("the authorize URL has a redirect_uri");
        let mut st = lock(&self.shared.state);
        let code = st.next("code");
        st.codes.push(Code {
            code: code.clone(),
            redirect_uri: redirect.clone(),
            challenge: get("code_challenge"),
            nonce: get("nonce"),
        });
        let mut out = format!(
            "{redirect}?code={}&client_id={}",
            encode(&code),
            encode(&st.config.client_id)
        );
        if let Some(state) = get("state") {
            out.push_str(&format!("&state={}", encode(&state)));
        }
        out
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn jwks(key: &KeyPair) -> Value {
    let c: PublicKeyComponents<Vec<u8>> = key.public().into();
    json!({ "keys": [{
        "kty": "RSA", "use": "sig", "alg": "RS256", "kid": SIWC_KEY_ID,
        "n": URL_SAFE_NO_PAD.encode(&c.n), "e": URL_SAFE_NO_PAD.encode(&c.e),
    }]})
}

fn sign(key: &KeyPair, claims: &Value) -> String {
    let header = json!({ "alg": "RS256", "typ": "JWT", "kid": SIWC_KEY_ID });
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut sig,
    )
    .expect("signing with the test key");
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig))
}

fn oauth_error(status: u16, error: &str, description: &str) -> Answer {
    Answer::Whole(Reply::json(
        status,
        &json!({ "error": error, "error_description": description }),
    ))
}

fn handle(shared: &Shared, request: &Request) -> Answer {
    lock(&shared.requests).push(request.clone());
    let issuer = lock(&shared.issuer).clone();
    match (request.method.as_str(), request.route()) {
        ("GET", SIWC_OPENID_PATH) => Answer::Whole(Reply::json(
            200,
            &json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/api/accounts/authorize"),
                "token_endpoint": format!("{issuer}{SIWC_TOKEN_PATH}"),
                "jwks_uri": format!("{issuer}{SIWC_JWKS_PATH}"),
                "revocation_endpoint": format!("{issuer}{SIWC_REVOKE_PATH}"),
                "response_types_supported": ["code"],
                "id_token_signing_alg_values_supported": ["RS256"],
            }),
        )),
        ("GET", SIWC_JWKS_PATH) => Answer::Whole(Reply::json(200, &jwks(&shared.key))),
        ("POST", SIWC_TOKEN_PATH) => token(shared, request, &issuer),
        ("POST", SIWC_REVOKE_PATH) => {
            if let Some(t) = request.form("token") {
                lock(&shared.state).revoked.push(t);
            }
            Answer::Whole(Reply::json(200, &json!({})))
        }
        _ => oauth_error(404, "not_found", "Not found"),
    }
}

fn token(shared: &Shared, request: &Request, issuer: &str) -> Answer {
    let form = |k: &str| request.form(k);
    let delay = {
        let st = lock(&shared.state);
        if st.config.invalid_grant {
            return oauth_error(
                400,
                "invalid_grant",
                "The refresh token is no longer valid.",
            );
        }
        if form("client_id").as_deref() != Some(st.config.client_id.as_str()) {
            return oauth_error(401, "invalid_client", "Unknown client.");
        }
        if form("resource").as_deref() != Some(SIWC_RESOURCE) {
            return oauth_error(
                400,
                "invalid_request",
                "resource must be https://api.openai.com/v1",
            );
        }
        st.config.refresh_delay
    };
    match form("grant_type").as_deref() {
        Some("authorization_code") => {
            let mut st = lock(&shared.state);
            let Some(at) = st
                .codes
                .iter()
                .position(|c| Some(&c.code) == form("code").as_ref())
            else {
                return oauth_error(400, "invalid_grant", "Unknown or used code.");
            };
            let code = st.codes.remove(at);
            if form("redirect_uri").as_deref() != Some(code.redirect_uri.as_str()) {
                return oauth_error(400, "invalid_grant", "redirect_uri does not match.");
            }
            if let Some(challenge) = &code.challenge {
                let verifier = form("code_verifier").unwrap_or_default();
                if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != *challenge {
                    return oauth_error(400, "invalid_grant", "code_verifier does not match.");
                }
            }
            let access = st.next("access");
            let refresh = st.next("refresh");
            st.access_tokens.push(access.clone());
            st.current_refresh = refresh.clone();
            let now = now_secs();
            let mut claims = json!({
                "iss": issuer, "aud": st.config.client_id, "sub": "user-fake-1",
                "email": st.config.account, "iat": now, "exp": now + 3600,
            });
            if let Some(n) = code.nonce {
                claims["nonce"] = json!(n);
            }
            let id_token = sign(&shared.key, &claims);
            Answer::Whole(Reply::json(
                200,
                &tokens(&st, &access, &refresh, Some(&id_token)),
            ))
        }
        Some("refresh_token") => {
            if form("scope").is_some() {
                return oauth_error(400, "invalid_request", "Leave scope out of a refresh.");
            }
            if !delay.is_zero() {
                thread::sleep(delay);
            }
            let mut st = lock(&shared.state);
            if form("refresh_token").as_deref() != Some(st.current_refresh.as_str()) {
                return oauth_error(400, "invalid_grant", "The refresh token was already used.");
            }
            let access = st.next("access");
            let refresh = st.next("refresh");
            st.access_tokens.push(access.clone());
            st.current_refresh = refresh.clone();
            st.refreshes += 1;
            Answer::Whole(Reply::json(200, &tokens(&st, &access, &refresh, None)))
        }
        _ => oauth_error(400, "unsupported_grant_type", "Unsupported grant_type."),
    }
}

fn tokens(st: &State, access: &str, refresh: &str, id_token: Option<&str>) -> Value {
    let mut t = json!({
        "access_token": access, "refresh_token": refresh, "token_type": "Bearer",
        "expires_in": st.config.expires_in, "scope": st.config.scope,
    });
    if let Some(id) = id_token {
        t["id_token"] = json!(id);
    }
    if let Some(secs) = st.config.earliest_refresh_in {
        t["earliest_refresh_at"] = json!(now_secs() + secs as i64);
    }
    t
}
