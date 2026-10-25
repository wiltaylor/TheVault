//! An account's own base URL (ai D24, accounts A1): the normalising rules, the ChatGPT plan
//! staying on OpenAI's own host, keyless calls to an account's URL, the compatibility errors,
//! the host in the trail line, OpenRouter-style URLs and extra headers (accounts A3).

mod common;

use std::time::Duration;

use ai::{Account, AccountInput, Accounts, Credentials, ErrorKind, Kind, Request, api_url};
use common::{Env, add_account};
use state::{SecretKey, State};
use testkit::{AnthropicReply, FakeAnthropic, FakeOpenAi, OpenAiReply};

/// An unsaved account of `kind` at `url`.
fn account(kind: Kind, url: Option<&str>) -> Result<Account, ai::AiError> {
    let mut all = Accounts::default();
    all.add(AccountInput {
        name: Some("Test".into()),
        kind: Some(kind),
        base_url: Some(url.map(str::to_owned)),
        ..AccountInput::default()
    })
    .cloned()
}

#[test]
fn api_urls_are_normalised_with_or_without_v1() {
    for (given, stored, responses) in [
        (
            "https://openrouter.ai/api/v1",
            "https://openrouter.ai/api/v1",
            "https://openrouter.ai/api/v1/responses",
        ),
        (
            " https://openrouter.ai/api/v1/ ",
            "https://openrouter.ai/api/v1",
            "https://openrouter.ai/api/v1/responses",
        ),
        (
            "https://openrouter.ai/api",
            "https://openrouter.ai/api",
            "https://openrouter.ai/api/v1/responses",
        ),
        (
            "http://10.0.0.5:11434//",
            "http://10.0.0.5:11434",
            "http://10.0.0.5:11434/v1/responses",
        ),
        // Another version segment is the API root as it is.
        (
            "https://api.z.ai/api/coding/paas/v4",
            "https://api.z.ai/api/coding/paas/v4",
            "https://api.z.ai/api/coding/paas/v4/responses",
        ),
        // Not a version segment: /v1 is added.
        (
            "https://example.com/v1beta",
            "https://example.com/v1beta",
            "https://example.com/v1beta/v1/responses",
        ),
    ] {
        let a = account(Kind::OpenAi, Some(given)).unwrap();
        assert_eq!(a.url(), Some(stored), "{given}");
        let base = a.api_base(&ai::Endpoints::default());
        assert_eq!(api_url(&base, "/v1/responses"), responses, "{given}");
    }
    assert_eq!(
        api_url("https://api.z.ai/api/anthropic", "/v1/models?limit=1000"),
        "https://api.z.ai/api/anthropic/v1/models?limit=1000"
    );
    assert_eq!(
        api_url("https://example.com/api/v2", "/v1/models?limit=1000"),
        "https://example.com/api/v2/models?limit=1000"
    );
    assert_eq!(
        api_url("https://api.z.ai/api/paas/v4", "/v1/chat/completions"),
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
    let a = account(Kind::OpenAi, Some("https://example.com/v1beta")).unwrap();
    assert_eq!(a.custom_host().as_deref(), Some("example.com"));
    let a = account(Kind::Anthropic, Some("http://10.0.0.5:11434/")).unwrap();
    assert_eq!(a.custom_host().as_deref(), Some("10.0.0.5:11434"));

    // None is the kind's own API.
    let a = account(Kind::OpenAi, Some("  ")).unwrap();
    assert_eq!(a.url(), None);
    assert_eq!(a.api_base(&ai::Endpoints::default()), ai::OPENAI_BASE);
    assert_eq!(a.custom_host(), None);
    let a = account(Kind::Anthropic, None).unwrap();
    assert_eq!(a.api_base(&ai::Endpoints::default()), ai::ANTHROPIC_BASE);
}

#[test]
fn bad_api_urls_are_refused() {
    for bad in [
        "ftp://example.com/v1",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "example.com/v1",
        "not a url",
        "https://user:pw@example.com/v1",
        "https://user@example.com/v1",
        "https://example.com/v1?key=1",
        "https://example.com/v1#x",
    ] {
        let err = account(Kind::OpenAi, Some(bad)).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Invalid, "{bad}");
        assert!(!err.message.contains("pw"), "{}", err.message);
        assert!(ai::check_url(bad).is_err(), "{bad}");
    }
    assert_eq!(
        account(Kind::ChatGpt, Some("https://x.example"))
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
    assert_eq!(
        account(Kind::OpenAiCompatible, None).unwrap_err().kind,
        ErrorKind::Invalid,
        "an OpenAI-compatible account needs its URL"
    );
}

#[test]
fn a_plan_request_never_goes_to_another_accounts_url() {
    // Without the test seam, the plan uses OpenAI's own host.
    let a = account(Kind::ChatGpt, None).unwrap();
    assert_eq!(a.api_base(&ai::Endpoints::default()), ai::OPENAI_BASE);
    assert_eq!(a.custom_host(), None);

    // With it, the plan reaches the `chatgpt` endpoint and the OpenAI account's URL sees nothing.
    let env = Env::new();
    let custom = FakeOpenAi::start();
    let url = custom.base_url();
    add_account(
        &env.state,
        "Other",
        Kind::OpenAi,
        Some(&url),
        Some(custom.api_key()),
    );
    env.sign_in("plan-access", 60 * 60 * 1000);
    let cred = env.cred(Kind::ChatGpt);
    assert_eq!(cred.base_url, env.openai.base_url());
    assert_eq!(cred.custom_host, None);
    let req = Request::new("gpt-6.1-sol", "Hi");
    let reply = ai::complete_blocking(&ai::blocking_client(), &cred, &req, None).unwrap();
    assert_eq!(reply.text, "OK");
    assert_eq!(env.openai.response_requests().len(), 1);
    assert!(custom.requests().is_empty(), "{:?}", custom.requests());
    // The other account does go to its own URL.
    let all = Accounts::load(&env.state).unwrap();
    let cred = Credentials::resolve(&env.state, all.find("Other").unwrap()).unwrap();
    let req = Request::new("gpt-6", "Hi");
    ai::complete_blocking(&ai::blocking_client(), &cred, &req, None).unwrap();
    assert_eq!(custom.response_requests().len(), 1);
}

/// A state with no keys, an Anthropic and an OpenAI account at the given fakes.
fn keyless(anthropic: &FakeAnthropic, openai: &FakeOpenAi) -> (tempfile::TempDir, State) {
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(&dir.path().join("state.db"), Some(SecretKey::generate())).unwrap();
    add_account(
        &state,
        "Anthropic",
        Kind::Anthropic,
        Some(&anthropic.base_url()),
        None,
    );
    add_account(
        &state,
        "OpenAI",
        Kind::OpenAi,
        Some(&openai.base_url()),
        None,
    );
    add_account(&state, "ChatGPT", Kind::ChatGpt, None, None);
    (dir, state)
}

fn of(state: &State, kind: Kind) -> Account {
    Accounts::load(state)
        .unwrap()
        .accounts
        .into_iter()
        .find(|a| a.kind == kind)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_custom_url_with_no_key_is_configured_and_sends_no_auth_header() {
    let anthropic = FakeAnthropic::builder().open().start();
    let openai = FakeOpenAi::builder().open().start();
    let (_dir, state) = keyless(&anthropic, &openai);
    let http = ai::client();
    for (p, model) in [(Kind::Anthropic, "claude-opus-5"), (Kind::OpenAi, "gpt-6")] {
        let a = of(&state, p);
        assert!(Credentials::configured(&state, &a).unwrap(), "{p}");
        let cred = Credentials::resolve(&state, &a).unwrap();
        assert!(cred.secret.is_empty());
        let req = Request::new(model, "Hi");
        let began = std::time::Instant::now();
        let result = ai::complete(&http, &cred, &req).await;
        assert_eq!(result.as_ref().unwrap().text, "OK", "{p}");
        let models = ai::list_models(&http, &cred).await.unwrap();
        assert!(!models.is_empty(), "{p}");
        let line = ai::trail_for(&cred, model, &result, began.elapsed());
        let base = match p {
            Kind::Anthropic => anthropic.base_url(),
            _ => openai.base_url(),
        };
        let host = base.trim_start_matches("http://");
        assert!(line.ends_with(&format!(" host={host}")), "{line}");
    }
    assert!(!ai::Credentials::configured(&state, &of(&state, Kind::ChatGpt)).unwrap());
    for r in anthropic.requests() {
        assert_eq!(r.header("x-api-key"), None);
        assert!(r.header("anthropic-version").is_some());
    }
    for r in openai.requests() {
        assert_eq!(r.header("authorization"), None);
    }
    assert_eq!(anthropic.message_requests().len(), 1);
    assert_eq!(openai.response_requests().len(), 1);

    // A key set as well is sent as before.
    let a = of(&state, Kind::Anthropic);
    state
        .set_secret(&ai::key_secret(&a.id), anthropic.api_key())
        .unwrap();
    let cred = Credentials::resolve(&state, &a).unwrap();
    let req = Request::new("claude-opus-5", "Hi");
    ai::complete(&http, &cred, &req).await.unwrap();
    assert_eq!(
        anthropic
            .message_requests()
            .last()
            .unwrap()
            .header("x-api-key"),
        Some(anthropic.api_key())
    );
}

#[test]
fn a_service_without_the_api_gives_the_named_invalid_error() {
    let env = Env::new();
    let http = ai::blocking_client();
    let call = |p: Kind, model: &str| {
        let req = Request::new(model, "Hi");
        ai::complete_blocking(&http, &env.cred(p), &req, None).unwrap_err()
    };

    for status in [404, 405] {
        env.openai
            .push(OpenAiReply::error(status, "not_found", "Not found"));
        let e = call(Kind::OpenAi, "gpt-6");
        assert_eq!(e.kind, ErrorKind::Invalid);
        assert_eq!(
            e.message,
            "This service does not support the OpenAI Responses API (POST /v1/responses)."
        );
        assert_eq!(e.status, Some(status));

        env.anthropic.push(AnthropicReply::error(
            status,
            "not_found_error",
            "Not Found",
        ));
        let e = call(Kind::Anthropic, "claude-opus-5");
        assert_eq!(e.kind, ErrorKind::Invalid);
        assert_eq!(
            e.message,
            "This service does not support the Anthropic Messages API (POST /v1/messages)."
        );
    }

    // A 404 that names the model is the provider's own message.
    env.anthropic.push(AnthropicReply::error(
        404,
        "not_found_error",
        "model: claude-nope",
    ));
    let e = call(Kind::Anthropic, "claude-nope");
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(e.message.contains("model: claude-nope"), "{}", e.message);
}

#[test]
fn the_official_api_leaves_no_host_in_the_trail() {
    let cred = ai::Credential::new(Kind::OpenAi, "http://127.0.0.1:9", ai::Secret::new("k"));
    assert_eq!(cred.custom_host, None);
    let line = ai::trail_line(
        &cred.name,
        Kind::OpenAi,
        "gpt-6",
        ai::Usage::default(),
        ai::Outcome::Ok,
        Duration::ZERO,
        cred.custom_host.as_deref(),
    );
    assert!(!line.contains("host="), "{line}");
}

#[test]
fn an_openrouter_style_account_joins_its_api_v1_url_and_sends_its_headers() {
    let env = Env::new();
    let fake = FakeOpenAi::builder().chat_completions().start();
    // A base URL ending in /api/v1, as OpenRouter's does: the calls go to /api/v1/..., with no
    // second /v1.
    let url = format!("{}/api/v1", fake.base_url());
    let mut all = Accounts::load(&env.state).unwrap();
    let a = all
        .add(AccountInput {
            name: Some("OpenRouter".into()),
            kind: Some(Kind::OpenAiCompatible),
            base_url: Some(Some(url)),
            headers: Some(vec![
                ("HTTP-Referer".into(), "https://vault.example".into()),
                ("X-Title".into(), "TheVault".into()),
            ]),
            ..AccountInput::default()
        })
        .unwrap()
        .clone();
    all.save(&env.state).unwrap();
    env.state
        .set_secret(&ai::key_secret(&a.id), fake.api_key())
        .unwrap();
    let cred = Credentials::resolve(&env.state, &a).unwrap();
    assert!(cred.chat_completions());
    let http = ai::blocking_client();
    let reply = ai::complete_blocking(
        &http,
        &cred,
        &Request::new("anthropic/claude-sonnet-5", "Hi"),
        None,
    )
    .unwrap();
    assert_eq!(reply.text, "OK");
    assert_eq!(reply.kind, Kind::OpenAiCompatible);
    assert_eq!(reply.name, "OpenRouter");
    ai::list_models_blocking(&http, &cred, None).unwrap();
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/chat/completions");
    assert_eq!(requests[1].path, "/api/v1/models");
    for r in &requests {
        assert_eq!(r.header("http-referer"), Some("https://vault.example"));
        assert_eq!(r.header("x-title"), Some("TheVault"));
        assert_eq!(
            r.header("authorization"),
            Some(format!("Bearer {}", fake.api_key()).as_str())
        );
    }
}

#[test]
fn a_keyless_openai_compatible_account_sends_no_auth_header() {
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(&dir.path().join("state.db"), Some(SecretKey::generate())).unwrap();
    let fake = FakeOpenAi::builder().chat_completions().open().start();
    let a = add_account(
        &state,
        "Ollama",
        Kind::OpenAiCompatible,
        Some(&fake.base_url()),
        None,
    );
    assert!(Credentials::configured(&state, &a).unwrap());
    let cred = Credentials::resolve(&state, &a).unwrap();
    assert!(cred.secret.is_empty());
    let reply = ai::complete_blocking(
        &ai::blocking_client(),
        &cred,
        &Request::new("llama4", "Hi"),
        None,
    )
    .unwrap();
    assert_eq!(reply.text, "OK");
    assert_eq!(fake.chat_requests()[0].header("authorization"), None);
}
