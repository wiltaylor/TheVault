//! Credentials from the secrets, the ChatGPT plan's refresh (ai D7, D9), Sign in with ChatGPT's
//! pieces against the fake auth server (ai D8), settings, and that no key or token leaks.

mod common;

use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use ai::{
    ChatGptAuth, Credentials, ErrorKind, Outcome, Provider, Request, Secret, Settings, Usage, siwc,
};
use common::{Env, now_ms};
use serde_json::json;
use testkit::{FakeSiwc, OpenAiReply, Reply, StandIn};

#[test]
fn keys_resolve_from_the_secrets() {
    let env = Env::new();
    let cred = env.cred(Provider::Anthropic);
    assert_eq!(cred.secret.expose(), env.anthropic.api_key());
    assert_eq!(cred.base_url, env.anthropic.base_url());
    let cred = env.cred(Provider::OpenAi);
    assert_eq!(cred.base_url, env.openai.base_url());
    assert!(Credentials::configured(&env.state, Provider::OpenAi).unwrap());
    assert!(!Credentials::configured(&env.state, Provider::ChatGpt).unwrap());

    env.state.delete_secret(ai::ANTHROPIC_API_KEY).unwrap();
    let e = Credentials::resolve(&env.state, Provider::Anthropic).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotConfigured);
    let e = Credentials::resolve(&env.state, Provider::ChatGpt).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotConfigured);
}

#[test]
fn a_fresh_plan_token_is_used_as_it_is() {
    let env = Env::new();
    env.sign_in("fresh-access", 60 * 60 * 1000);
    let cred = env.cred(Provider::ChatGpt);
    assert_eq!(cred.secret.expose(), "fresh-access");
    assert_eq!(cred.base_url, env.openai.base_url());
    assert_eq!(cred.account.as_deref(), Some("me@example.com"));
    assert!(env.siwc.requests().is_empty());
}

#[test]
fn a_token_near_expiry_is_refreshed_once_for_eight_threads() {
    let env = Env::with_siwc(
        FakeSiwc::builder()
            .refresh_delay(Duration::from_millis(300))
            .expires_in(3600)
            .start(),
    );
    let old_refresh = env.siwc.refresh_token();
    env.sign_in("old-access", 2 * 60 * 1000);
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let state = env.state.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                Credentials::resolve(&state, Provider::ChatGpt)
                    .map(|c| c.secret.expose().to_owned())
            })
        })
        .collect();
    let tokens: Vec<String> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .collect();
    assert_eq!(env.siwc.refresh_count(), 1);
    assert_eq!(env.siwc.token_requests("refresh_token").len(), 1);
    let issued = env.siwc.access_tokens();
    assert_eq!(issued.len(), 1);
    assert!(tokens.iter().all(|t| *t == issued[0]), "{tokens:?}");

    let r = &env.siwc.token_requests("refresh_token")[0];
    assert_eq!(
        r.form("client_id").as_deref(),
        Some(env.siwc.client_id().as_str())
    );
    assert_eq!(
        r.form("refresh_token").as_deref(),
        Some(old_refresh.as_str())
    );
    assert_eq!(r.form("resource").as_deref(), Some(siwc::RESOURCE));
    assert!(r.form("scope").is_none());

    let stored = env.chatgpt_secret();
    assert_eq!(
        stored["refresh_token"],
        json!(env.siwc.refresh_token()),
        "the rotated token is kept"
    );
    assert_ne!(stored["refresh_token"], json!(old_refresh));
    assert_eq!(stored["access_token"], json!(issued[0]));
    let expires = stored["expires_ms"].as_i64().unwrap();
    assert!(
        (expires - (now_ms() + 3_600_000)).abs() < 60_000,
        "{expires}"
    );
    assert_eq!(
        stored["host_id"],
        json!("urn:uuid:00000000-0000-4000-8000-000000000000")
    );

    // The new token is fresh, so the next call refreshes nothing.
    Credentials::resolve(&env.state, Provider::ChatGpt).unwrap();
    assert_eq!(env.siwc.refresh_count(), 1);
}

#[test]
fn no_refresh_before_earliest_refresh() {
    let env = Env::new();
    env.sign_in("old-access", 2 * 60 * 1000);
    let mut auth = ChatGptAuth::load(&env.state).unwrap();
    auth.earliest_refresh_ms = Some(now_ms() + 60 * 1000);
    auth.save(&env.state).unwrap();
    assert_eq!(env.cred(Provider::ChatGpt).secret.expose(), "old-access");
    assert_eq!(env.siwc.refresh_count(), 0);
}

#[test]
fn invalid_grant_signs_out_and_notifies_once() {
    let env = Env::new();
    env.sign_in("old-access", 60 * 1000);
    env.siwc.set_invalid_grant(true);
    let e = Credentials::resolve(&env.state, Provider::ChatGpt).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth, "{e}");
    assert!(e.message.contains("Settings > AI"));

    let stored = env.chatgpt_secret();
    assert_eq!(stored["client_id"], json!(env.siwc.client_id()));
    assert_eq!(
        stored["host_id"],
        json!("urn:uuid:00000000-0000-4000-8000-000000000000")
    );
    for gone in ["access_token", "refresh_token", "expires_ms", "id_token"] {
        assert!(stored.get(gone).is_none(), "{gone} kept: {stored}");
    }

    let e = Credentials::resolve(&env.state, Provider::ChatGpt).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotConfigured);
    let page = env.state.notifications(false, 50, None).unwrap();
    assert_eq!(page.items.len(), 1, "one notification");
    let n = &page.items[0];
    assert_eq!(n.title, ai::EXPIRED_TITLE);
    assert_eq!(n.url.as_deref(), Some(ai::SETTINGS_URL));
    assert_eq!(n.source, "system");
}

#[test]
fn a_failed_refresh_keeps_a_token_that_still_works() {
    let env = Env::new();
    env.sign_in("still-good", 60 * 1000);
    let down = StandIn::start();
    down.route(siwc::TOKEN_PATH, Reply::text(503, "down for maintenance"));
    let mut settings = Settings::load(&env.state).unwrap();
    settings.endpoints.chatgpt_auth = Some(down.url(""));
    settings.save(&env.state).unwrap();
    assert_eq!(env.cred(Provider::ChatGpt).secret.expose(), "still-good");
    assert_eq!(down.requests().len(), 1, "it tried");
    assert_eq!(env.chatgpt_secret()["access_token"], json!("still-good"));
}

#[test]
fn sign_in_pieces_work_against_the_fake() {
    let env = Env::new();
    let http = ai::blocking_client();
    let base = env.siwc.base_url();
    let (verifier, challenge) = siwc::pkce();
    let state = siwc::random_token();
    let nonce = siwc::random_token();
    let host = siwc::new_host_id();
    assert!(host.starts_with("urn:uuid:") && host.len() == 45, "{host}");
    assert_eq!(&host[23..24], "4", "a v4 uuid: {host}");
    let url = siwc::authorize_url(
        &base,
        &siwc::Authorize {
            client_id: None,
            host_id: &host,
            state: &state,
            nonce: &nonce,
            code_challenge: &challenge,
            id_token_hint: None,
            login_hint: None,
        },
    );
    assert!(
        url.starts_with(&format!(
            "{base}/api/accounts/authorize?client_id=dynamic_agent_client&agent_name_hint=TheVault"
        )),
        "{url}"
    );
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53683%2Fauth%2Fcallback"),
        "{url}"
    );
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("chatgpt.tokens.use.direct"));
    assert!(url.contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"));

    let callback = env.siwc.consent(&url);
    let query = reqwest::Url::parse(&callback).unwrap();
    let get = |k: &str| {
        query
            .query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
    };
    assert_eq!(get("state").as_deref(), Some(state.as_str()));
    let client_id = get("client_id").unwrap();
    assert!(client_id.starts_with("oaiapp_"));

    let tokens =
        siwc::exchange_code(&http, &base, &client_id, &get("code").unwrap(), &verifier).unwrap();
    assert!(tokens.has_plan_scope());
    let config = siwc::openid_configuration(&http, &base).unwrap();
    assert_eq!(config.issuer, env.siwc.issuer());
    let jwks = siwc::jwks(&http, &config.jwks_uri).unwrap();
    let id = tokens.id_token.clone().unwrap();
    let claims = siwc::verify_id_token(
        id.expose(),
        &jwks,
        &config.issuer,
        &client_id,
        &nonce,
        now_ms(),
    )
    .unwrap();
    assert_eq!(claims.email.as_deref(), Some("me@example.com"));

    // Every check bites.
    let check = |token: &str, issuer: &str, aud: &str, nonce: &str, now: i64| {
        siwc::verify_id_token(token, &jwks, issuer, aud, nonce, now)
            .unwrap_err()
            .kind
    };
    assert_eq!(
        check(
            id.expose(),
            &config.issuer,
            &client_id,
            "other-nonce",
            now_ms()
        ),
        ErrorKind::Auth
    );
    assert_eq!(
        check(
            id.expose(),
            "https://evil.example",
            &client_id,
            &nonce,
            now_ms()
        ),
        ErrorKind::Auth
    );
    assert_eq!(
        check(
            id.expose(),
            &config.issuer,
            "oaiapp_other",
            &nonce,
            now_ms()
        ),
        ErrorKind::Auth
    );
    assert_eq!(
        check(
            id.expose(),
            &config.issuer,
            &client_id,
            &nonce,
            now_ms() + 2 * 3_600_000
        ),
        ErrorKind::Auth
    );
    let mut parts: Vec<String> = id.expose().split('.').map(str::to_owned).collect();
    parts[1] = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        json!({"iss": config.issuer, "aud": client_id, "exp": 9_999_999_999i64, "nonce": nonce})
            .to_string(),
    );
    assert_eq!(
        check(
            &parts.join("."),
            &config.issuer,
            &client_id,
            &nonce,
            now_ms()
        ),
        ErrorKind::Auth
    );

    // The code was used once.
    let e = siwc::exchange_code(&http, &base, &client_id, &get("code").unwrap(), &verifier)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);

    // Stored, refreshed and revoked.
    let mut auth = ChatGptAuth {
        client_id: Some(client_id.clone()),
        host_id: Some(host),
        ..Default::default()
    };
    auth.store_tokens(tokens, now_ms());
    auth.account = claims.email;
    assert!(auth.signed_in());
    let refresh = auth.refresh_token.clone().unwrap();
    siwc::revoke(
        &http,
        config.revocation_endpoint.as_deref().unwrap(),
        &client_id,
        &refresh,
    )
    .unwrap();
    assert_eq!(env.siwc.revoked(), vec![refresh.expose().to_owned()]);

    // A later sign-in reuses the issued id and sends the hints.
    let url = siwc::authorize_url(
        &base,
        &siwc::Authorize {
            client_id: Some(&client_id),
            host_id: auth.host_id.as_deref().unwrap(),
            state: "s",
            nonce: "n",
            code_challenge: "c",
            id_token_hint: auth.id_token.as_ref().map(Secret::expose),
            login_hint: auth.account.as_deref(),
        },
    );
    assert!(url.contains(&format!("client_id={client_id}")));
    assert!(url.contains("id_token_hint=") && url.contains("login_hint=me%40example.com"));
    assert!(!url.contains("agent_name_hint"));
}

#[test]
fn a_grant_without_the_plan_scope_is_noticed() {
    let env = Env::with_siwc(
        FakeSiwc::builder()
            .scope("openid email offline_access")
            .start(),
    );
    let (verifier, challenge) = siwc::pkce();
    let url = siwc::authorize_url(
        &env.siwc.base_url(),
        &siwc::Authorize {
            client_id: None,
            host_id: "urn:uuid:x",
            state: "s",
            nonce: "n",
            code_challenge: &challenge,
            id_token_hint: None,
            login_hint: None,
        },
    );
    let callback = reqwest::Url::parse(&env.siwc.consent(&url)).unwrap();
    let code = callback
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let tokens = siwc::exchange_code(
        &ai::blocking_client(),
        &env.siwc.base_url(),
        &env.siwc.client_id(),
        &code,
        &verifier,
    )
    .unwrap();
    assert!(!tokens.has_plan_scope());
}

#[test]
fn settings_round_trip_and_pick_defaults() {
    let env = Env::new();
    let mut s = Settings::load(&env.state).unwrap();
    assert_eq!(
        s.pick(None, None).unwrap_err().kind,
        ErrorKind::NotConfigured
    );
    s.default_provider = Some(Provider::Anthropic);
    s.set_model(Provider::Anthropic, Some("claude-opus-5".into()));
    s.other.insert("future".into(), json!(1));
    s.save(&env.state).unwrap();
    let back = Settings::load(&env.state).unwrap();
    assert_eq!(back, s);
    assert_eq!(
        back.pick(None, None).unwrap(),
        (Provider::Anthropic, "claude-opus-5".into())
    );
    assert_eq!(
        back.pick(Some(Provider::OpenAi), Some("gpt-6")).unwrap(),
        (Provider::OpenAi, "gpt-6".into())
    );
    assert_eq!(
        back.pick(Some(Provider::ChatGpt), None).unwrap_err().kind,
        ErrorKind::NotConfigured
    );
    let raw: serde_json::Value =
        serde_json::from_str(&env.state.get_secret(ai::SETTINGS).unwrap().unwrap()).unwrap();
    assert_eq!(raw["default_provider"], json!("anthropic"));
    assert_eq!(raw["models"]["anthropic"], json!("claude-opus-5"));
    assert_eq!(
        raw["endpoints"]["anthropic"],
        json!(env.anthropic.base_url())
    );
    assert_eq!(raw["future"], json!(1));
    assert_eq!(
        Settings::default().endpoints.api_base(Provider::ChatGpt),
        ai::OPENAI_BASE
    );
    assert_eq!(
        Settings::default().endpoints.chatgpt_auth_base(),
        ai::CHATGPT_AUTH_BASE
    );
}

#[test]
fn no_key_or_token_shows_in_debug_errors_or_trail_lines() {
    let env = Env::new();
    env.sign_in("eyJhbGciOiJSUzI1NiJ9.access-token-value", 60 * 60 * 1000);
    let secrets = [
        env.anthropic.api_key().to_owned(),
        env.openai.api_key().to_owned(),
        "access-token-value".to_owned(),
        env.siwc.refresh_token(),
    ];
    let leaks = |text: &str| {
        for s in &secrets {
            assert!(!text.contains(s.as_str()), "{s} leaked in {text}");
        }
    };
    for p in Provider::ALL {
        leaks(&format!("{:?}", env.cred(p)));
    }
    leaks(&format!("{:?}", ChatGptAuth::load(&env.state).unwrap()));
    leaks(&format!(
        "{:?} {}",
        Secret::new(env.openai.api_key()),
        Secret::new(env.openai.api_key())
    ));

    let tokens = siwc::refresh(
        &ai::blocking_client(),
        &env.siwc.base_url(),
        &env.siwc.client_id(),
        &Secret::new(env.siwc.refresh_token()),
    )
    .unwrap();
    let text = format!("{tokens:?}");
    assert!(!text.contains(tokens.access_token.expose()), "{text}");
    assert!(
        !text.contains(tokens.refresh_token.as_ref().unwrap().expose()),
        "{text}"
    );

    // A provider error quoting the key comes back without it.
    let key = env.openai.api_key().to_owned();
    env.openai.push(OpenAiReply::error(
        401,
        "invalid_api_key",
        &format!("Incorrect API key provided: {key}. Also sk-test-openai-01***789."),
    ));
    let req = Request::new(Provider::OpenAi, "gpt-6", "x");
    let began = std::time::Instant::now();
    let result = ai::complete_blocking(
        &ai::blocking_client(),
        &env.cred(Provider::OpenAi),
        &req,
        None,
    );
    let e = result.as_ref().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    leaks(&format!("{e} {e:?}"));
    assert!(!e.message.contains("sk-test-openai-01"), "{}", e.message);
    let line = ai::trail_for(Provider::OpenAi, &req.model, &result, began.elapsed());
    leaks(&line);
    assert!(
        line.starts_with("ai: provider=openai model=gpt-6 in=0 out=0 outcome=error:auth ms="),
        "{line}"
    );
    let line = ai::trail_line(
        Provider::ChatGpt,
        "gpt-6.1-sol",
        Usage {
            input: 1,
            output: 2,
        },
        Outcome::Ok,
        Duration::from_millis(3),
    );
    assert_eq!(
        line,
        "ai: provider=chatgpt model=gpt-6.1-sol in=1 out=2 outcome=ok ms=3"
    );
}
