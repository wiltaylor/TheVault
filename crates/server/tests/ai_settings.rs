//! S1 with the AI fakes: Settings > AI's routes, the provider keys, Sign in with ChatGPT, the
//! Test route and the model lists (ai D6, D7, D8, D15).

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

    /// A server whose `ai_settings` point at the fakes, the ChatGPT sign-in at `auth_base` when
    /// given and at the fake sign-in server otherwise.
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
            "anthropic": api.anthropic.base_url(),
            "openai": api.openai.base_url(),
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

    fn start_chatgpt(&self) -> String {
        let r = self.post(
            "/api/integrations/ai/chatgpt/start",
            json!({}),
            StatusCode::OK,
        );
        r["auth_url"].as_str().unwrap().to_owned()
    }

    fn finish(&self, url: &str, status: StatusCode) -> Value {
        self.post(
            "/api/integrations/ai/chatgpt/finish",
            json!({ "url": url }),
            status,
        )
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
    let chatgpt = &status["providers"]["chatgpt"];
    assert_eq!(chatgpt["configured"], true);
    assert_eq!(chatgpt["account"], "me@example.com");
    assert_eq!(status["default_provider"], "chatgpt");
    let stored = api.server.secret_json("ai_chatgpt");
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
    assert_eq!(api.server.secret_json("ai_chatgpt")["host_id"], host);

    // Models through the plan token, then Test against the plan.
    let models = api.get("/api/ai/models?provider=chatgpt");
    let ids: Vec<&str> = models["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["gpt-6.1-sol", "gpt-6-luna"]);
    api.put(
        "/api/integrations/ai/settings",
        json!({ "models": { "chatgpt": "gpt-6-luna" } }),
    );
    let t = api.post(
        "/api/integrations/ai/chatgpt/test",
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["model"], "gpt-6-luna");

    let refresh = api.siwc.refresh_token();
    let creds = api.credentials();
    let out = api.post(
        "/api/integrations/ai/chatgpt/disconnect",
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(out["providers"]["chatgpt"]["configured"], false);
    assert_eq!(api.siwc.revoked(), [refresh]);
    assert_eq!(api.server.secret("ai_chatgpt"), None);
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
    assert_eq!(
        api.get("/api/integrations/ai")["providers"]["chatgpt"]["configured"],
        false
    );
    assert!(api.server.secret_json("ai_chatgpt")["access_token"].is_null());
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
    assert_eq!(r["providers"]["chatgpt"]["account"], "me@example.com");
    for body in api.bodies.borrow().iter() {
        assert!(!body.contains("standin-access") && !body.contains("standin-refresh"));
    }
}

#[test]
fn keys_are_stored_and_deleted_and_never_answered() {
    let api = Api::start();
    let empty = api.get("/api/integrations/ai");
    assert_eq!(empty["secrets_key"], true);
    assert_eq!(empty["providers"]["anthropic"]["configured"], false);
    assert_eq!(empty["default_provider"], Value::Null);

    let s = api.put(
        "/api/integrations/ai/anthropic/key",
        json!({ "key": format!(" {} ", api.anthropic.api_key()) }),
    );
    assert_eq!(s["providers"]["anthropic"]["configured"], true);
    assert_eq!(s["default_provider"], "anthropic");
    assert_eq!(
        api.server.secret("ai_anthropic_api_key").as_deref(),
        Some(api.anthropic.api_key())
    );
    api.put(
        "/api/integrations/ai/openai/key",
        json!({ "key": api.openai.api_key() }),
    );
    let s = api.get("/api/integrations/ai");
    assert_eq!(s["providers"]["openai"]["configured"], true);
    assert_eq!(s["default_provider"], "anthropic");

    let s = api.put(
        "/api/integrations/ai/settings",
        json!({ "default_provider": "openai", "models": { "openai": "gpt-6-mini" } }),
    );
    assert_eq!(s["default_provider"], "openai");
    assert_eq!(s["providers"]["openai"]["model"], "gpt-6-mini");
    // The test seam survives a save.
    assert_eq!(
        api.server.secret_json("ai_settings")["endpoints"]["openai"],
        api.openai.base_url()
    );
    assert!(
        api.get("/api/integrations/ai")
            .to_string()
            .find("endpoints")
            .is_none()
    );

    let s = api.put("/api/integrations/ai/openai/key", json!({ "key": "  " }));
    assert_eq!(s["providers"]["openai"]["configured"], false);
    assert_eq!(api.server.secret("ai_openai_api_key"), None);
    // Deleting a key that is not there is fine.
    api.put("/api/integrations/ai/openai/key", json!({ "key": "" }));
    api.send(
        api.client
            .put(api.server.url("/api/integrations/ai/chatgpt/key"))
            .json(&json!({ "key": "x" })),
        StatusCode::BAD_REQUEST,
    );
    api.send(
        api.client
            .put(api.server.url("/api/integrations/ai/settings"))
            .json(&json!({ "default_provider": "gemini" })),
        StatusCode::BAD_REQUEST,
    );
    api.assert_never_answered(&api.credentials());
}

#[test]
fn test_reports_ok_from_each_provider_and_the_error_kind_on_failure() {
    let api = Api::start();
    // No key yet.
    let t = api.post(
        "/api/integrations/ai/anthropic/test",
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], false);
    assert_eq!(t["error"]["kind"], "not_configured");

    api.put(
        "/api/integrations/ai/anthropic/key",
        json!({ "key": api.anthropic.api_key() }),
    );
    // No model chosen: the preselected claude-opus-5 is tested and kept.
    let t = api.send(
        api.client
            .post(api.server.url("/api/integrations/ai/anthropic/test")),
        StatusCode::OK,
    );
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["model"], "claude-opus-5");
    assert_eq!(t["text"], "OK");
    assert!(t["ms"].is_u64());
    assert_eq!(
        api.get("/api/integrations/ai")["providers"]["anthropic"]["model"],
        "claude-opus-5"
    );
    let sent = api.anthropic.message_requests();
    assert_eq!(sent.last().unwrap().json()["max_tokens"], 16);

    api.anthropic.push(AnthropicReply::error(
        401,
        "authentication_error",
        "invalid x-api-key",
    ));
    let t = api.post(
        "/api/integrations/ai/anthropic/test",
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

    api.put(
        "/api/integrations/ai/openai/key",
        json!({ "key": api.openai.api_key() }),
    );
    // OpenAI has no preselected model: Test asks for one.
    let t = api.post(
        "/api/integrations/ai/openai/test",
        json!({}),
        StatusCode::OK,
    );
    assert_eq!(t["error"]["kind"], "not_configured", "{t}");
    let t = api.post(
        "/api/integrations/ai/openai/test",
        json!({ "model": "gpt-6" }),
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
        "/api/integrations/ai/openai/test",
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
fn a_read_token_lists_models_but_cannot_reach_settings() {
    let api = Api::start();
    api.put(
        "/api/integrations/ai/anthropic/key",
        json!({ "key": api.anthropic.api_key() }),
    );
    let t = api.post(
        "/api/tokens",
        json!({ "name": "reader", "scope": "read" }),
        StatusCode::CREATED,
    );
    let reader = client(t["secret"].as_str().unwrap());

    let res = reader
        .get(api.server.url("/api/ai/models?provider=anthropic"))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let m: Value = res.json().unwrap();
    assert_eq!(m["provider"], "anthropic");
    assert_eq!(m["model"], "claude-opus-5");
    assert_eq!(m["models"][0]["id"], "claude-opus-5");
    assert_eq!(m["models"][0]["display_name"], "Claude Opus 5");
    // With no provider named, the default one.
    let res = reader.get(api.server.url("/api/ai/models")).send().unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    for (method, path) in [
        ("GET", "/api/integrations/ai"),
        ("PUT", "/api/integrations/ai/anthropic/key"),
        ("POST", "/api/integrations/ai/chatgpt/start"),
        ("POST", "/api/integrations/ai/anthropic/test"),
    ] {
        let res = reader
            .request(method.parse().unwrap(), api.server.url(path))
            .json(&json!({ "key": "x" }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }

    // A provider with no key is a clear 400.
    let res = reader
        .get(api.server.url("/api/ai/models?provider=openai"))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let e: Value = res.json().unwrap();
    assert_eq!(e["error"], "not_configured");
}
