//! S1 with the AI fakes: Settings > AI's routes, the table of AI accounts, their keys, Sign in
//! with ChatGPT per account, the Test route and the model lists (ai D6, D7, D8, D15; accounts A1
//! to A7).

use std::cell::RefCell;

use reqwest::StatusCode;
use reqwest::Url;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::{
    AnthropicReply, FakeAnthropic, FakeOpenAi, FakeSiwc, OpenAiReply, Reply, StandIn, TestServer,
};

const REDIRECT: &str = "http://127.0.0.1:53683/auth/callback";

struct Api {
    server: TestServer,
    client: Client,
    anthropic: FakeAnthropic,
    openai: FakeOpenAi,
    siwc: FakeSiwc,
    /// Every response body, to check none holds a key or token.
    bodies: RefCell<Vec<String>>,
}

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

fn param(url: &str, name: &str) -> Option<String> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// `url` with query parameter `name` set to `value`.
fn with_param(url: &str, name: &str, value: &str) -> String {
    let mut u = Url::parse(url).unwrap();
    let pairs: Vec<(String, String)> = u
        .query_pairs()
        .map(|(k, v)| {
            let v = if k == name {
                value.to_owned()
            } else {
                v.into_owned()
            };
            (k.into_owned(), v)
        })
        .collect();
    u.query_pairs_mut().clear().extend_pairs(pairs);
    u.into()
}

impl Api {
    fn start() -> Api {
        Api::with_auth_base(None)
    }

    /// A server with no accounts yet, whose ChatGPT plan seam points at the fake and whose
    /// ChatGPT sign-in goes to `auth_base` when given and to the fake sign-in server otherwise.
    fn with_auth_base(auth_base: Option<String>) -> Api {
        let server = TestServer::start();
        let client = client(server.admin_token());
        let api = Api {
            client,
            anthropic: FakeAnthropic::start(),
            openai: FakeOpenAi::start(),
            siwc: FakeSiwc::start(),
            server,
            bodies: RefCell::default(),
        };
        let settings = json!({ "endpoints": {
            "chatgpt": api.openai.base_url(),
            "chatgpt_auth": auth_base.unwrap_or_else(|| api.siwc.base_url()),
        }});
        api.send(
            api.client
                .put(api.server.url("/api/secrets/ai_settings"))
                .json(&json!({ "value": settings.to_string() })),
            StatusCode::OK,
        );
        api
    }

    fn send(&self, req: RequestBuilder, status: StatusCode) -> Value {
        let res = req.send().unwrap();
        let got = res.status();
        let text = res.text().unwrap();
        assert_eq!(got, status, "{text}");
        self.bodies.borrow_mut().push(text.clone());
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        self.send(self.client.get(self.server.url(path)), StatusCode::OK)
    }

    fn post(&self, path: &str, body: Value, status: StatusCode) -> Value {
        self.send(self.client.post(self.server.url(path)).json(&body), status)
    }

    fn put(&self, path: &str, body: Value) -> Value {
        self.send(
            self.client.put(self.server.url(path)).json(&body),
            StatusCode::OK,
        )
    }

    fn delete(&self, path: &str) -> Value {
        self.send(self.client.delete(self.server.url(path)), StatusCode::OK)
    }

    /// Add an account: `body` with `name` and `kind`. Answers its status.
    fn add(&self, body: Value) -> Value {
        self.post("/api/integrations/ai/accounts", body, StatusCode::OK)
    }

    /// Add the ChatGPT subscription account `ChatGPT` when there is none. Answers its id.
    fn plan(&self) -> String {
        let all = self.get("/api/integrations/ai");
        if let Some(a) = all["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["kind"] == "chatgpt")
        {
            return a["id"].as_str().unwrap().to_owned();
        }
        let a = self.add(json!({ "name": "ChatGPT", "kind": "chatgpt" }));
        a["id"].as_str().unwrap().to_owned()
    }

    /// The account named `name` in `GET /api/integrations/ai`.
    fn account(&self, name: &str) -> Value {
        let all = self.get("/api/integrations/ai");
        all["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no account {name}: {all}"))
    }

    fn start_chatgpt(&self) -> String {
        let id = self.plan();
        let r = self.post(
            &format!("/api/integrations/ai/accounts/{id}/chatgpt/start"),
            json!({}),
            StatusCode::OK,
        );
        r["auth_url"].as_str().unwrap().to_owned()
    }

    fn finish(&self, url: &str, status: StatusCode) -> Value {
        let id = self.plan();
        self.post(
            &format!("/api/integrations/ai/accounts/{id}/chatgpt/finish"),
            json!({ "url": url }),
            status,
        )
    }

    /// The plan account's sign-in secret.
    fn plan_secret(&self) -> Value {
        self.server.secret_json(&ai::chatgpt_secret(&self.plan()))
    }

    /// Assert no response so far holds any of `secrets`.
    fn assert_never_answered(&self, secrets: &[String]) {
        for body in self.bodies.borrow().iter() {
            for s in secrets {
                assert!(!s.is_empty());
                assert!(!body.contains(s.as_str()), "a response holds {s:?}: {body}");
            }
        }
    }

    /// Every key and token the fakes know, but the ID tokens, which a later sign-in's authorize
    /// URL carries as `id_token_hint` (ai D8).
    fn credentials(&self) -> Vec<String> {
        let mut out = vec![
            self.anthropic.api_key().to_owned(),
            self.openai.api_key().to_owned(),
            self.siwc.refresh_token(),
        ];
        out.extend(self.siwc.access_tokens());
        out.extend(self.siwc.revoked());
        out
    }
}

#[test]
fn chatgpt_sign_in_start_finish_restart_and_disconnect() {
    let api = Api::start();
    let first = api.start_chatgpt();
    let id = api.plan();
    let u = Url::parse(&first).unwrap();
    assert_eq!(
        format!("{}/api/accounts/authorize", api.siwc.base_url()),
        format!("{}://{}{}", u.scheme(), u.authority(), u.path())
    );
    let p = |k: &str| param(&first, k);
    assert_eq!(p("client_id").as_deref(), Some("dynamic_agent_client"));
    assert_eq!(p("agent_name_hint").as_deref(), Some("TheVault"));
    assert_eq!(p("response_type").as_deref(), Some("code"));
    assert_eq!(p("redirect_uri").as_deref(), Some(REDIRECT));
    assert_eq!(
        p("scope").as_deref(),
        Some("openid profile email offline_access resource.invoke chatgpt.tokens.use.direct")
    );
    assert_eq!(p("resource").as_deref(), Some("https://api.openai.com/v1"));
    assert_eq!(p("code_challenge_method").as_deref(), Some("S256"));
    for k in ["state", "nonce", "code_challenge"] {
        assert!(p(k).is_some_and(|v| v.len() >= 16), "{k}");
    }
    let host = p("ext_agent_host_id").unwrap();
    assert!(host.starts_with("urn:uuid:"), "{host}");
    assert_eq!(p("id_token_hint"), None);
    assert_eq!(p("login_hint"), None);

    let callback = api.siwc.consent(&first);
    let status = api.finish(&callback, StatusCode::OK);
    let plan = &status["accounts"][0];
    assert_eq!(plan["kind"], "chatgpt");
    assert_eq!(plan["kind_label"], "ChatGPT subscription");
    assert_eq!(plan["configured"], true);
    assert_eq!(plan["key_set"], false);
    assert_eq!(
        plan["signed_in"],
        json!({ "account": "me@example.com", "expired": false })
    );
    assert_eq!(plan["default_window"], 272000);
    assert_eq!(status["default"], json!(id));
    let stored = api.plan_secret();
    assert_eq!(stored["client_id"], api.siwc.client_id());
    assert_eq!(stored["host_id"], host);
    assert_eq!(stored["account"], "me@example.com");
    assert_eq!(stored["refresh_token"], api.siwc.refresh_token());
    assert!(stored["access_token"].is_string());
    assert!(stored["id_token"].is_string());
    assert!(stored["expires_ms"].is_i64());
    // The exchange used the issued client id, the verifier and the redirect.
    let exchange = &api.siwc.token_requests("authorization_code")[0];
    assert_eq!(exchange.form("client_id"), Some(api.siwc.client_id()));
    assert_eq!(exchange.form("redirect_uri").as_deref(), Some(REDIRECT));

    // The same pasted address again is refused: its state was used.
    let again = api.finish(&callback, StatusCode::BAD_REQUEST);
    assert!(again["message"].as_str().unwrap().contains("already used"));

    // A later sign-in names the issued client and the account, with the same host id.
    let second = api.start_chatgpt();
    let q = |k: &str| param(&second, k);
    assert_eq!(q("client_id"), Some(api.siwc.client_id()));
    assert_eq!(q("agent_name_hint"), None);
    assert_eq!(q("ext_agent_host_id").as_deref(), Some(host.as_str()));
    assert_eq!(
        q("id_token_hint").as_ref(),
        stored["id_token"].as_str().map(str::to_owned).as_ref()
    );
    assert_eq!(q("login_hint").as_deref(), Some("me@example.com"));
    assert_ne!(q("state"), p("state"));
    api.finish(&api.siwc.consent(&second), StatusCode::OK);
    assert_eq!(api.plan_secret()["host_id"], host);

    // Models through the plan token, then Test against the plan.
    let models = api.get(&format!("/api/ai/models?account={id}"));
    let ids: Vec<&str> = models["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["gpt-6.1-sol", "gpt-6-luna"]);
    api.put(
        &format!("/api/integrations/ai/accounts/{id}"),
        json!({ "model": "gpt-6-luna" }),
    );
    let t = api.post(
        &format!("/api/integrations/ai/accounts/{id}/test"),
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["model"], "gpt-6-luna");
    assert_eq!(t["kind"], "chatgpt");
    // The Test's plan answer left the plan's usage (accounts A13).
    let usage = &api.account("ChatGPT")["usage"];
    assert_eq!(usage["windows"][0]["used_percent"], 12.5, "{usage}");
    assert_eq!(usage["windows"][1]["label"], "Weekly");

    let refresh = api.siwc.refresh_token();
    let creds = api.credentials();
    let out = api.post(
        &format!("/api/integrations/ai/accounts/{id}/chatgpt/disconnect"),
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(out["accounts"][0]["configured"], false);
    assert_eq!(out["accounts"][0]["signed_in"], Value::Null);
    assert_eq!(out["accounts"][0]["usage"], Value::Null);
    assert_eq!(api.siwc.revoked(), [refresh]);
    assert_eq!(api.server.secret(&ai::chatgpt_secret(&id)), None);
    api.assert_never_answered(&creds);
}

#[test]
fn a_wrong_state_and_a_missing_plan_scope_are_refused() {
    let api = Api::start();
    let url = api.start_chatgpt();
    let callback = api.siwc.consent(&url);
    let wrong = with_param(&callback, "state", "not-the-state");
    let r = api.finish(&wrong, StatusCode::BAD_REQUEST);
    assert!(
        r["message"].as_str().unwrap().contains("not started here"),
        "{r}"
    );
    assert!(api.siwc.token_requests("authorization_code").is_empty());

    // A sign-in started for one account cannot finish on another.
    let other = api.add(json!({ "name": "Work plan", "kind": "chatgpt" }));
    let r = api.post(
        &format!(
            "/api/integrations/ai/accounts/{}/chatgpt/finish",
            other["id"].as_str().unwrap()
        ),
        json!({ "url": callback }),
        StatusCode::BAD_REQUEST,
    );
    assert!(
        r["message"].as_str().unwrap().contains("another account"),
        "{r}"
    );

    // A sign-in that did not grant the plan scope is refused, and its tokens revoked.
    api.siwc.set_scope("openid profile email offline_access");
    let url = api.start_chatgpt();
    let r = api.finish(&api.siwc.consent(&url), StatusCode::BAD_REQUEST);
    assert!(
        r["message"]
            .as_str()
            .unwrap()
            .contains("chatgpt.tokens.use.direct"),
        "{r}"
    );
    assert_eq!(api.siwc.revoked().len(), 1);
    assert_eq!(api.account("ChatGPT")["configured"], false);
    assert!(api.plan_secret()["access_token"].is_null());
    // A refusal from the consent page itself.
    let url = api.start_chatgpt();
    let state = param(&url, "state").unwrap();
    let r = api.finish(
        &format!("{REDIRECT}?error=access_denied&state={state}"),
        StatusCode::BAD_REQUEST,
    );
    assert!(
        r["message"].as_str().unwrap().contains("access_denied"),
        "{r}"
    );
    // The sign-in routes are a chatgpt account's only.
    let key = api.add(json!({ "name": "Claude", "kind": "anthropic", "key": "sk-ant-x" }));
    api.post(
        &format!(
            "/api/integrations/ai/accounts/{}/chatgpt/start",
            key["id"].as_str().unwrap()
        ),
        json!({}),
        StatusCode::BAD_REQUEST,
    );
}

/// A sign-in server that answers a token of the test's making: the issuer's configuration,
/// key set and token endpoint, on a stand-in.
fn stand_in_auth(siwc: &FakeSiwc, auth: &StandIn, jwks: Value, id_token: &str) {
    auth.route(
        "/.well-known/openid-configuration",
        Reply::json(
            200,
            &json!({
                "issuer": siwc.issuer(),
                "jwks_uri": auth.url("/jwks"),
                "revocation_endpoint": auth.url("/revoke"),
            }),
        ),
    );
    auth.route("/jwks", Reply::json(200, &jwks));
    auth.route("/revoke", Reply::json(200, &json!({})));
    auth.route(
        "/api/accounts/oauth/token",
        Reply::json(
            200,
            &json!({
                "access_token": "standin-access", "refresh_token": "standin-refresh",
                "id_token": id_token, "expires_in": 3600, "token_type": "Bearer",
                "scope": "openid email offline_access chatgpt.tokens.use.direct",
            }),
        ),
    );
}

fn id_claims(siwc: &FakeSiwc, nonce: &str) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    json!({
        "iss": siwc.issuer(), "aud": "oaiapp_standin", "sub": "u1",
        "email": "me@example.com", "nonce": nonce, "iat": now, "exp": now + 3600,
    })
}

#[test]
fn a_bad_id_token_signature_and_a_wrong_nonce_are_refused() {
    let auth = StandIn::start();
    let api = Api::with_auth_base(Some(auth.url("")));
    let callback = |url: &str| {
        format!(
            "{REDIRECT}?code=c1&client_id=oaiapp_standin&state={}",
            param(url, "state").unwrap()
        )
    };

    // Signed by the test key, checked against a key set whose modulus differs.
    let url = api.start_chatgpt();
    let nonce = param(&url, "nonce").unwrap();
    let mut jwks = api.siwc.jwks();
    let n = jwks["keys"][0]["n"].as_str().unwrap().to_owned();
    let swap = if &n[10..11] == "A" { "B" } else { "A" };
    jwks["keys"][0]["n"] = json!(format!("{}{swap}{}", &n[..10], &n[11..]));
    stand_in_auth(
        &api.siwc,
        &auth,
        jwks,
        &api.siwc.sign(&id_claims(&api.siwc, &nonce)),
    );
    let r = api.finish(&callback(&url), StatusCode::BAD_REQUEST);
    assert!(
        r["message"].as_str().unwrap().contains("bad signature"),
        "{r}"
    );

    // Signed properly, for another sign-in's nonce.
    let url = api.start_chatgpt();
    let token = api.siwc.sign(&id_claims(&api.siwc, "some-other-nonce"));
    stand_in_auth(&api.siwc, &auth, api.siwc.jwks(), &token);
    let r = api.finish(&callback(&url), StatusCode::BAD_REQUEST);
    assert!(
        r["message"].as_str().unwrap().contains("wrong nonce"),
        "{r}"
    );

    // The right nonce and key: stored.
    let url = api.start_chatgpt();
    let nonce = param(&url, "nonce").unwrap();
    let token = api.siwc.sign(&id_claims(&api.siwc, &nonce));
    stand_in_auth(&api.siwc, &auth, api.siwc.jwks(), &token);
    let r = api.finish(&callback(&url), StatusCode::OK);
    assert_eq!(r["accounts"][0]["signed_in"]["account"], "me@example.com");
    for body in api.bodies.borrow().iter() {
        assert!(!body.contains("standin-access") && !body.contains("standin-refresh"));
    }
}

#[test]
fn accounts_are_added_answered_reordered_and_deleted_and_keys_never_answered() {
    let api = Api::start();
    let empty = api.get("/api/integrations/ai");
    assert_eq!(empty["secrets_key"], true);
    assert_eq!(empty["accounts"], json!([]));
    assert_eq!(empty["default"], Value::Null);
    assert_eq!(
        empty["context"],
        json!({ "auto_compact": true, "compact_at": 80 })
    );
    assert_eq!(empty["fetch_private"], false);
    assert_eq!(
        empty["kinds"],
        json!([
            { "kind": "anthropic", "label": "Anthropic API", "default_url": "https://api.anthropic.com", "default_window": 200000 },
            { "kind": "openai", "label": "OpenAI API", "default_url": "https://api.openai.com", "default_window": 400000 },
            { "kind": "openai_compatible", "label": "OpenAI-compatible", "default_url": null, "default_window": 128000 },
            { "kind": "chatgpt", "label": "ChatGPT subscription", "default_url": "https://api.openai.com", "default_window": 272000 },
        ])
    );
    assert_eq!(api.get("/api/ai/accounts")["kinds"], empty["kinds"]);

    // An Anthropic API account with its key, on the official API.
    let a = api.add(json!({
        "name": " Claude ", "kind": "anthropic", "key": format!(" {} ", api.anthropic.api_key()),
    }));
    let id = a["id"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 26, "a ULID: {a}");
    assert_eq!(
        a,
        json!({
            "id": id, "name": "Claude", "kind": "anthropic", "kind_label": "Anthropic API",
            "base_url": null, "default_base_url": "https://api.anthropic.com", "model": null,
            "headers": [], "context": null, "default_window": 200000, "window": 200000,
            "configured": true, "key_set": true, "signed_in": null, "agents": [], "usage": null,
        })
    );
    assert_eq!(
        api.server.secret(&ai::key_secret(&id)).as_deref(),
        Some(api.anthropic.api_key())
    );
    // The first account is the default.
    assert_eq!(api.get("/api/integrations/ai")["default"], json!(id));

    // OpenRouter over Chat Completions, with its headers and window.
    let r = api.add(json!({
        "name": "OpenRouter", "kind": "openai_compatible",
        "base_url": "https://openrouter.ai/api/v1/", "model": "anthropic/claude-sonnet-5",
        "headers": [["HTTP-Referer", "https://vault.example"], ["X-Title", "TheVault"]],
        "context": { "window": 1000000 },
    }));
    let router = r["id"].as_str().unwrap().to_owned();
    assert_eq!(r["base_url"], "https://openrouter.ai/api/v1");
    assert_eq!(r["default_base_url"], Value::Null);
    assert_eq!(r["kind_label"], "OpenAI-compatible");
    assert_eq!(
        r["headers"],
        json!([
            ["HTTP-Referer", "https://vault.example"],
            ["X-Title", "TheVault"]
        ])
    );
    assert_eq!(r["window"], 1000000);
    assert_eq!(r["default_window"], 128000);
    // An account at its own URL is configured with no key (ai D24).
    assert_eq!(r["configured"], true);
    assert_eq!(r["key_set"], false);

    // Refusals, each a 400 that changes nothing.
    for (body, error) in [
        (json!({ "name": "claude", "kind": "openai" }), "invalid"),
        (json!({ "name": "", "kind": "openai" }), "invalid"),
        (
            json!({ "name": "x".repeat(61), "kind": "openai" }),
            "invalid",
        ),
        (json!({ "name": "Gem", "kind": "gemini" }), "bad_request"),
        (
            json!({ "name": "Nowhere", "kind": "openai_compatible" }),
            "invalid",
        ),
        (
            json!({ "name": "Ftp", "kind": "openai", "base_url": "ftp://x/v1" }),
            "invalid_url",
        ),
        (
            json!({ "name": "Pw", "kind": "openai", "base_url": "https://me:hunter2@x.example/v1" }),
            "invalid_url",
        ),
        (
            json!({ "name": "Auth", "kind": "openai_compatible", "base_url": "https://x.example",
                    "headers": [["Authorization", "Bearer sk-x"]] }),
            "invalid",
        ),
        (
            json!({ "name": "Hdr", "kind": "anthropic", "headers": [["X-Title", "T"]] }),
            "invalid",
        ),
        (
            json!({ "name": "Plan", "kind": "chatgpt", "key": "sk-x" }),
            "bad_request",
        ),
        (
            json!({ "name": "Plan", "kind": "chatgpt", "base_url": "https://x.example" }),
            "invalid",
        ),
        (
            json!({ "name": "Tiny", "kind": "openai", "context": { "window": 10 } }),
            "invalid",
        ),
    ] {
        let e = api.post(
            "/api/integrations/ai/accounts",
            body.clone(),
            StatusCode::BAD_REQUEST,
        );
        assert_eq!(e["error"], error, "{body} {e}");
        assert!(!e.to_string().contains("hunter2"), "{e}");
    }
    assert_eq!(
        api.get("/api/integrations/ai")["accounts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Keys: set, replaced, cleared; a chatgpt account takes none.
    let s = api.put(
        &format!("/api/integrations/ai/accounts/{router}/key"),
        json!({ "key": "sk-or-1" }),
    );
    assert_eq!(s["accounts"][1]["key_set"], true);
    assert_eq!(
        api.server.secret(&ai::key_secret(&router)).as_deref(),
        Some("sk-or-1")
    );
    let s = api.put(
        &format!("/api/integrations/ai/accounts/{router}/key"),
        json!({ "key": "  " }),
    );
    assert_eq!(s["accounts"][1]["key_set"], false);
    assert_eq!(api.server.secret(&ai::key_secret(&router)), None);
    let plan = api.plan();
    api.send(
        api.client
            .put(
                api.server
                    .url(&format!("/api/integrations/ai/accounts/{plan}/key")),
            )
            .json(&json!({ "key": "x" })),
        StatusCode::BAD_REQUEST,
    );

    // The default and the order.
    let s = api.put("/api/integrations/ai/default", json!({ "id": router }));
    assert_eq!(s["default"], json!(router));
    let s = api.put(
        "/api/integrations/ai/accounts/order",
        json!({ "ids": [plan, router, id] }),
    );
    let names: Vec<&str> = s["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["ChatGPT", "OpenRouter", "Claude"]);
    api.send(
        api.client
            .put(api.server.url("/api/integrations/ai/accounts/order"))
            .json(&json!({ "ids": [plan] })),
        StatusCode::BAD_REQUEST,
    );

    // A change keeps what it does not name; null clears; the kind cannot change.
    let s = api.put(
        &format!("/api/integrations/ai/accounts/{router}"),
        json!({ "model": null, "headers": [], "context": null }),
    );
    let r = &s["accounts"][1];
    assert_eq!(r["model"], Value::Null);
    assert_eq!(r["headers"], json!([]));
    assert_eq!(r["window"], 128000);
    assert_eq!(r["base_url"], "https://openrouter.ai/api/v1");
    api.send(
        api.client
            .put(
                api.server
                    .url(&format!("/api/integrations/ai/accounts/{router}")),
            )
            .json(&json!({ "kind": "openai" })),
        StatusCode::BAD_REQUEST,
    );
    api.send(
        api.client
            .put(api.server.url("/api/integrations/ai/accounts/nope"))
            .json(&json!({ "name": "x" })),
        StatusCode::NOT_FOUND,
    );

    // Deleting the default moves it to the first account left, and takes the secrets.
    api.put(
        &format!("/api/integrations/ai/accounts/{router}/key"),
        json!({ "key": "sk-or-2" }),
    );
    let s = api.delete(&format!("/api/integrations/ai/accounts/{router}"));
    assert_eq!(s["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(s["default"], json!(plan));
    assert_eq!(api.server.secret(&ai::key_secret(&router)), None);

    // Answers never hold a key, and the old routes are gone.
    let mut creds = api.credentials();
    creds.extend(["sk-or-1".to_owned(), "sk-or-2".to_owned()]);
    api.assert_never_answered(&creds);
    for (method, path) in [
        ("PUT", "/api/integrations/ai/anthropic/key"),
        ("POST", "/api/integrations/ai/chatgpt/start"),
        ("GET", "/api/ai/providers"),
    ] {
        let res = api
            .client
            .request(method.parse().unwrap(), api.server.url(path))
            .json(&json!({ "key": "x" }))
            .send()
            .unwrap();
        assert!(
            matches!(
                res.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{method} {path}: {}",
            res.status()
        );
    }
}

#[test]
fn the_global_settings_are_their_own_and_the_old_keys_are_refused() {
    let api = Api::start();
    let s = api.put(
        "/api/integrations/ai/settings",
        json!({ "fetch_private": true, "context": { "auto_compact": false, "compact_at": 70 } }),
    );
    assert_eq!(s["fetch_private"], true);
    assert_eq!(
        s["context"],
        json!({ "auto_compact": false, "compact_at": 70 })
    );
    // The test seam survives a save, and no answer shows it.
    assert_eq!(
        api.server.secret_json("ai_settings")["endpoints"]["chatgpt_auth"],
        api.siwc.base_url()
    );
    assert!(
        !api.get("/api/integrations/ai")
            .to_string()
            .contains("endpoints")
    );
    // The provider-keyed settings of before accounts are unknown fields now.
    for bad in [
        json!({ "default_provider": "anthropic" }),
        json!({ "models": { "anthropic": "m" } }),
        json!({ "urls": { "openai": "https://x.example" } }),
        json!({ "openai_api": "chat_completions" }),
        json!({ "context": { "windows": { "anthropic": 100000 } } }),
    ] {
        api.send(
            api.client
                .put(api.server.url("/api/integrations/ai/settings"))
                .json(&bad),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }
    api.send(
        api.client
            .put(api.server.url("/api/integrations/ai/settings"))
            .json(&json!({ "context": { "compact_at": 99 } })),
        StatusCode::BAD_REQUEST,
    );
}

#[test]
fn test_reports_ok_from_each_kind_and_the_error_kind_on_failure() {
    let api = Api::start();
    // No key, and the official API.
    let claude = api.add(json!({ "name": "Claude", "kind": "anthropic" }));
    let id = claude["id"].as_str().unwrap().to_owned();
    assert_eq!(claude["configured"], false);
    let t = api.post(
        &format!("/api/integrations/ai/accounts/{id}/test"),
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], false);
    assert_eq!(t["error"]["kind"], "not_configured");
    assert!(
        t["error"]["message"].as_str().unwrap().contains("Claude"),
        "{t}"
    );

    api.put(
        &format!("/api/integrations/ai/accounts/{id}"),
        json!({ "base_url": api.anthropic.base_url() }),
    );
    api.put(
        &format!("/api/integrations/ai/accounts/{id}/key"),
        json!({ "key": api.anthropic.api_key() }),
    );
    // No model chosen: the preselected claude-opus-5 is tested and kept.
    let t = api.send(
        api.client.post(
            api.server
                .url(&format!("/api/integrations/ai/accounts/{id}/test")),
        ),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["account"], json!(id));
    assert_eq!(t["name"], "Claude");
    assert_eq!(t["kind"], "anthropic");
    assert_eq!(t["model"], "claude-opus-5");
    assert_eq!(t["text"], "OK");
    assert!(t["ms"].is_u64());
    assert_eq!(api.account("Claude")["model"], "claude-opus-5");
    let sent = api.anthropic.message_requests();
    assert_eq!(sent.last().unwrap().json()["max_tokens"], 16);

    api.anthropic.push(AnthropicReply::error(
        401,
        "authentication_error",
        "invalid x-api-key",
    ));
    let t = api.post(
        &format!("/api/integrations/ai/accounts/{id}/test"),
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], false);
    assert_eq!(t["model"], "claude-opus-5");
    assert_eq!(t["error"]["kind"], "auth");
    assert!(
        t["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid x-api-key"),
        "{t}"
    );

    // OpenAI with no model: the first model listed is tested and kept.
    let o = api.add(json!({
        "name": "OpenAI", "kind": "openai", "base_url": api.openai.base_url(),
        "key": api.openai.api_key(),
    }));
    let oid = o["id"].as_str().unwrap().to_owned();
    let t = api.post(
        &format!("/api/integrations/ai/accounts/{oid}/test"),
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["model"], "gpt-6");
    api.openai.push(OpenAiReply::error(
        401,
        "invalid_api_key",
        "Incorrect API key provided",
    ));
    let t = api.post(
        &format!("/api/integrations/ai/accounts/{oid}/test"),
        json!({ "model": "gpt-6" }),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], false, "{t}");
    assert_eq!(t["error"]["kind"], "auth");
    assert!(
        t["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Incorrect API key"),
        "{t}"
    );
    api.assert_never_answered(&api.credentials());
}

#[test]
fn a_read_token_lists_models_and_accounts_but_cannot_reach_settings() {
    let api = Api::start();
    let a = api.add(json!({
        "name": "Claude", "kind": "anthropic", "base_url": api.anthropic.base_url(),
        "key": api.anthropic.api_key(),
    }));
    let id = a["id"].as_str().unwrap().to_owned();
    let t = api.post(
        "/api/tokens",
        json!({ "name": "reader", "scope": "read" }),
        StatusCode::CREATED,
    );
    let reader = client(t["secret"].as_str().unwrap());

    for path in [
        format!("/api/ai/models?account={id}"),
        "/api/ai/models?account=claude".into(),
    ] {
        let res = reader.get(api.server.url(&path)).send().unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let m: Value = res.json().unwrap();
        assert_eq!(m["account"], json!(id));
        assert_eq!(m["model"], "claude-opus-5");
        assert_eq!(m["models"][0]["id"], "claude-opus-5");
        assert_eq!(m["models"][0]["display_name"], "Claude Opus 5");
    }
    // With no account named, the default one.
    let res = reader.get(api.server.url("/api/ai/models")).send().unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = reader
        .get(api.server.url("/api/ai/models?account=nope"))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // The picker's list holds no URL.
    let res = reader
        .get(api.server.url("/api/ai/accounts"))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().unwrap();
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert!(!text.contains("base_url"), "{text}");

    for (method, path) in [
        ("GET", "/api/integrations/ai".to_owned()),
        ("POST", "/api/integrations/ai/accounts".to_owned()),
        ("PUT", format!("/api/integrations/ai/accounts/{id}/key")),
        ("POST", format!("/api/integrations/ai/accounts/{id}/test")),
        ("DELETE", format!("/api/integrations/ai/accounts/{id}")),
        ("PUT", "/api/integrations/ai/default".to_owned()),
    ] {
        let res = reader
            .request(method.parse().unwrap(), api.server.url(&path))
            .json(&json!({ "key": "x" }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }

    // An account with no key on the official API is a clear 400.
    let o = api.add(json!({ "name": "OpenAI", "kind": "openai" }));
    let res = reader
        .get(api.server.url(&format!(
            "/api/ai/models?account={}",
            o["id"].as_str().unwrap()
        )))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let e: Value = res.json().unwrap();
    assert_eq!(e["error"], "not_configured");
}

#[test]
fn test_works_against_own_urls_with_no_key_and_names_a_missing_api() {
    let api = Api::start();
    let anthropic = FakeAnthropic::builder().open().start();
    let openai = FakeOpenAi::builder().open().start();
    let a = api.add(json!({
        "name": "Local Claude", "kind": "anthropic",
        "base_url": format!("{}/v1", anthropic.base_url()),
    }));
    let o =
        api.add(json!({ "name": "Local GPT", "kind": "openai", "base_url": openai.base_url() }));
    assert_eq!(a["configured"], true);
    assert_eq!(o["configured"], true);
    assert_eq!(a["key_set"], false);
    let test = |id: &Value, body: Value| {
        api.post(
            &format!(
                "/api/integrations/ai/accounts/{}/test",
                id.as_str().unwrap()
            ),
            body,
            StatusCode::OK,
        )
    };

    // Anthropic with no model chosen: the live list's preselected model is tested.
    let t = test(&a["id"], json!({}));
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["model"], "claude-opus-5");
    let t = test(&o["id"], json!({ "model": "gpt-6" }));
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["text"], "OK");
    for r in anthropic.requests() {
        assert_eq!(r.header("x-api-key"), None);
    }
    for r in openai.requests() {
        assert_eq!(r.header("authorization"), None);
    }

    // An OpenAI API account on a service without the Responses API is named.
    openai.push(OpenAiReply::error(404, "not_found", "Not found"));
    let t = test(&o["id"], json!({ "model": "gpt-6" }));
    assert_eq!(t["ok"], false, "{t}");
    assert_eq!(t["error"]["kind"], "invalid");
    assert_eq!(
        t["error"]["message"],
        "This service does not support the OpenAI Responses API (POST /v1/responses)."
    );

    // An OpenAI-compatible account speaks Chat Completions, with its headers, and never tries
    // Responses.
    let zai = FakeOpenAi::builder().open().chat_completions().start();
    let z = api.add(json!({
        "name": "Z.ai", "kind": "openai_compatible", "base_url": format!("{}/api/paas/v4", zai.base_url()),
        "headers": [["X-Title", "TheVault"]],
    }));
    let t = test(&z["id"], json!({ "model": "glm-5.3-flash" }));
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(zai.response_requests().len(), 0);
    let chat = &zai.chat_requests()[0];
    assert_eq!(chat.path, "/api/paas/v4/chat/completions");
    assert_eq!(chat.header("x-title"), Some("TheVault"));
    // Its model list sends the headers too.
    api.get(&format!(
        "/api/ai/models?account={}",
        z["id"].as_str().unwrap()
    ));
    let listed = zai
        .requests()
        .into_iter()
        .find(|r| r.method == "GET")
        .expect("a models request");
    assert_eq!(listed.path, "/api/paas/v4/models");
    assert_eq!(listed.header("x-title"), Some("TheVault"));

    // On a Responses-only service it is the named Chat Completions error.
    let responses = FakeOpenAi::builder().open().start();
    let r = api.add(json!({
        "name": "Responses only", "kind": "openai_compatible", "base_url": responses.base_url(),
    }));
    let t = test(&r["id"], json!({ "model": "gpt-6" }));
    assert_eq!(t["ok"], false, "{t}");
    assert_eq!(
        t["error"]["message"],
        "This service does not support the OpenAI Chat Completions API (POST /v1/chat/completions)."
    );
}

const HELPER: &str = "label = \"Helper\"\naccount = \"Claude\"\n";

#[test]
fn renaming_an_account_rewrites_the_agents_naming_it_and_deleting_lets_them_fall_back() {
    let api = Api::start();
    let a = api.add(json!({
        "name": "Claude", "kind": "anthropic", "base_url": api.anthropic.base_url(),
        "key": api.anthropic.api_key(), "model": "claude-opus-5",
    }));
    let id = a["id"].as_str().unwrap().to_owned();
    api.add(json!({
        "name": "Other", "kind": "openai", "base_url": api.openai.base_url(),
        "key": api.openai.api_key(),
    }));
    let put_agent = |name: &str, wcl: &str| {
        api.send(
            api.client
                .put(api.server.url(&format!("/api/agents/{name}")))
                .header("content-type", "text/plain")
                .body(wcl.to_owned()),
            StatusCode::OK,
        )
    };
    let helper = put_agent("helper", HELPER);
    assert_eq!(helper["account"], "Claude");
    put_agent("lower", "account = \"claude\"\n");
    put_agent("elsewhere", "account = \"Other\"\n");
    assert_eq!(api.account("Claude")["agents"], json!(["helper", "lower"]));

    let s = api.put(
        &format!("/api/integrations/ai/accounts/{id}"),
        json!({ "name": "Claude work" }),
    );
    assert_eq!(s["accounts"][0]["name"], "Claude work");
    assert_eq!(s["accounts"][0]["agents"], json!(["helper", "lower"]));
    let read = |name: &str| {
        api.get(&format!("/api/agents/{name}"))["wcl"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(
        read("helper").contains("account = \"Claude work\""),
        "{}",
        read("helper")
    );
    assert!(
        read("helper").contains("label   = \"Helper\"")
            || read("helper").contains("label = \"Helper\"")
    );
    assert!(read("lower").contains("account = \"Claude work\""));
    assert!(read("elsewhere").contains("account = \"Other\""));
    let agents = api.get("/api/agents");
    let helper = agents["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "helper")
        .unwrap()
        .clone();
    assert_eq!(helper["account"], "Claude work");

    // Deleting it leaves the agents naming it, which run on the default account.
    api.delete(&format!("/api/integrations/ai/accounts/{id}"));
    assert!(read("helper").contains("account = \"Claude work\""));
    assert_eq!(
        api.get("/api/integrations/ai")["accounts"][0]["name"],
        "Other"
    );
}

#[test]
fn accounts_added_and_changed_at_once_are_all_kept() {
    let api = Api::start();
    let token = api.server.admin_token().to_owned();
    let base = api.server.url("");
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let token = token.clone();
            let base = base.clone();
            std::thread::spawn(move || {
                let c = client(&token);
                let url = |p: &str| format!("{}{p}", base.trim_end_matches('/'));
                let res = c
                    .post(url("/api/integrations/ai/accounts"))
                    .json(&json!({ "name": format!("Account {n}"), "kind": "openai" }))
                    .send()
                    .unwrap();
                assert_eq!(res.status(), StatusCode::OK);
                let id = res.json::<Value>().unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                for (path, body) in [
                    (
                        format!("/api/integrations/ai/accounts/{id}/key"),
                        json!({ "key": format!("sk-{n}") }),
                    ),
                    (
                        format!("/api/integrations/ai/accounts/{id}"),
                        json!({ "model": format!("model-{n}") }),
                    ),
                ] {
                    let res = c.put(url(&path)).json(&body).send().unwrap();
                    assert_eq!(
                        res.status(),
                        StatusCode::OK,
                        "{path}: {}",
                        res.text().unwrap()
                    );
                }
                id
            })
        })
        .collect();
    let ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let all = api.get("/api/integrations/ai");
    let accounts = all["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 8, "{all}");
    for (n, id) in ids.iter().enumerate() {
        let a = accounts.iter().find(|a| a["id"] == json!(id)).unwrap();
        assert_eq!(a["name"], format!("Account {n}"));
        assert_eq!(a["model"], format!("model-{n}"));
        assert_eq!(a["key_set"], true);
    }
}

#[test]
fn an_accounts_secrets_take_names_the_secrets_route_accepts() {
    let api = Api::start();
    let id = api.plan();
    assert!(id.chars().any(|c| c.is_ascii_uppercase()), "a ULID: {id}");
    // The e2e seam: a signed-in plan stored through the secrets route.
    let name = ai::chatgpt_secret(&id);
    assert_eq!(
        name,
        format!("ai_account_chatgpt_{}", id.to_ascii_lowercase())
    );
    let auth = json!({
        "client_id": "oaiapp_test", "access_token": "plan-access", "refresh_token": "plan-refresh",
        "expires_ms": 4_102_444_800_000_i64, "account": "me@example.com",
    });
    api.put(
        &format!("/api/secrets/{name}"),
        json!({ "value": auth.to_string() }),
    );
    let plan = api.account("ChatGPT");
    assert_eq!(plan["configured"], true, "{plan}");
    assert_eq!(plan["signed_in"]["account"], "me@example.com");
    let key = api.add(json!({ "name": "Claude", "kind": "anthropic" }));
    let kid = key["id"].as_str().unwrap();
    api.put(
        &format!("/api/secrets/{}", ai::key_secret(kid)),
        json!({ "value": "sk-ant-x" }),
    );
    assert_eq!(api.account("Claude")["key_set"], true);
}
