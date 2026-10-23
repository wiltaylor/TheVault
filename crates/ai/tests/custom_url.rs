//! Custom API URLs for Anthropic and OpenAI (ai D24): the normalising rules, the ChatGPT plan
//! staying on OpenAI's own host, keyless calls to a custom URL, the compatibility errors and the
//! host in the trail line.

mod common;

use std::time::Duration;

use ai::{Credentials, Endpoints, ErrorKind, Provider, Request, Settings, api_url};
use common::Env;
use state::{SecretKey, State};
use testkit::{AnthropicReply, FakeAnthropic, FakeOpenAi, OpenAiReply};

#[test]
fn api_urls_are_normalised_with_or_without_v1() {
    let mut e = Endpoints::default();
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
        e.set_url(Provider::OpenAi, Some(given)).unwrap();
        assert_eq!(e.url(Provider::OpenAi), Some(stored), "{given}");
        let base = e.api_base(Provider::OpenAi);
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
        e.custom_host(Provider::OpenAi).as_deref(),
        Some("example.com")
    );
    e.set_url(Provider::Anthropic, Some("http://10.0.0.5:11434/"))
        .unwrap();
    assert_eq!(
        e.custom_host(Provider::Anthropic).as_deref(),
        Some("10.0.0.5:11434")
    );

    // Empty or absent clears the URL: the official API again.
    e.set_url(Provider::OpenAi, Some("  ")).unwrap();
    assert_eq!(e.url(Provider::OpenAi), None);
    assert_eq!(e.api_base(Provider::OpenAi), ai::OPENAI_BASE);
    assert_eq!(e.custom_host(Provider::OpenAi), None);
    e.set_url(Provider::Anthropic, None).unwrap();
    assert_eq!(e.api_base(Provider::Anthropic), ai::ANTHROPIC_BASE);
}

#[test]
fn bad_api_urls_are_refused() {
    let mut e = Endpoints::default();
    e.set_url(Provider::OpenAi, Some("https://ok.example/v1"))
        .unwrap();
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
        let err = e.set_url(Provider::OpenAi, Some(bad)).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Invalid, "{bad}");
        assert!(!err.message.contains("pw"), "{}", err.message);
        assert!(ai::check_url(bad).is_err(), "{bad}");
        // A refusal leaves the URL as it was.
        assert_eq!(e.url(Provider::OpenAi), Some("https://ok.example/v1"));
    }
    assert_eq!(
        e.set_url(Provider::ChatGpt, Some("https://x.example"))
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
}

#[test]
fn a_plan_request_never_goes_to_a_custom_openai_url() {
    // Without the test seam, the plan uses OpenAI's own host whatever the OpenAI URL is.
    let mut e = Endpoints::default();
    e.set_url(Provider::OpenAi, Some("https://openrouter.ai/api/v1"))
        .unwrap();
    assert_eq!(e.api_base(Provider::ChatGpt), ai::OPENAI_BASE);
    assert_eq!(e.custom_host(Provider::ChatGpt), None);

    // With it, the plan reaches the `chatgpt` endpoint and the custom OpenAI URL sees nothing.
    let env = Env::new();
    let custom = FakeOpenAi::start();
    let mut settings = Settings::load(&env.state).unwrap();
    settings.endpoints.openai = Some(custom.base_url());
    settings.save(&env.state).unwrap();
    env.sign_in("plan-access", 60 * 60 * 1000);
    let cred = env.cred(Provider::ChatGpt);
    assert_eq!(cred.base_url, env.openai.base_url());
    assert_eq!(cred.custom_host, None);
    let req = Request::new(Provider::ChatGpt, "gpt-6.1-sol", "Hi");
    let reply = ai::complete_blocking(&ai::blocking_client(), &cred, &req, None).unwrap();
    assert_eq!(reply.text, "OK");
    assert_eq!(env.openai.response_requests().len(), 1);
    assert!(custom.requests().is_empty(), "{:?}", custom.requests());
    // The API key does go to the custom URL.
    let cred = env.cred(Provider::OpenAi);
    let req = Request::new(Provider::OpenAi, "gpt-6", "Hi");
    ai::complete_blocking(&ai::blocking_client(), &cred, &req, None).unwrap();
    assert_eq!(custom.response_requests().len(), 1);
}

/// A state with no keys, its Anthropic and OpenAI API URLs at the given fakes.
fn keyless(anthropic: &FakeAnthropic, openai: &FakeOpenAi) -> (tempfile::TempDir, State) {
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(&dir.path().join("state.db"), Some(SecretKey::generate())).unwrap();
    let mut settings = Settings::default();
    settings
        .endpoints
        .set_url(Provider::Anthropic, Some(&anthropic.base_url()))
        .unwrap();
    settings
        .endpoints
        .set_url(Provider::OpenAi, Some(&openai.base_url()))
        .unwrap();
    settings.save(&state).unwrap();
    (dir, state)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_custom_url_with_no_key_is_configured_and_sends_no_auth_header() {
    let anthropic = FakeAnthropic::builder().open().start();
    let openai = FakeOpenAi::builder().open().start();
    let (_dir, state) = keyless(&anthropic, &openai);
    let http = ai::client();
    for (p, model) in [
        (Provider::Anthropic, "claude-opus-5"),
        (Provider::OpenAi, "gpt-6"),
    ] {
        assert!(Credentials::configured(&state, p).unwrap(), "{p}");
        let cred = Credentials::resolve(&state, p).unwrap();
        assert!(cred.secret.is_empty());
        let req = Request::new(p, model, "Hi");
        let began = std::time::Instant::now();
        let result = ai::complete(&http, &cred, &req).await;
        assert_eq!(result.as_ref().unwrap().text, "OK", "{p}");
        let models = ai::list_models(&http, &cred).await.unwrap();
        assert!(!models.is_empty(), "{p}");
        let line = ai::trail_for(
            p,
            model,
            &result,
            began.elapsed(),
            cred.custom_host.as_deref(),
        );
        let base = match p {
            Provider::Anthropic => anthropic.base_url(),
            _ => openai.base_url(),
        };
        let host = base.trim_start_matches("http://");
        assert!(line.ends_with(&format!(" host={host}")), "{line}");
    }
    assert!(!ai::Credentials::configured(&state, Provider::ChatGpt).unwrap());
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
    state
        .set_secret(ai::ANTHROPIC_API_KEY, anthropic.api_key())
        .unwrap();
    let cred = Credentials::resolve(&state, Provider::Anthropic).unwrap();
    let req = Request::new(Provider::Anthropic, "claude-opus-5", "Hi");
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
    let call = |p: Provider, model: &str| {
        let req = Request::new(p, model, "Hi");
        ai::complete_blocking(&http, &env.cred(p), &req, None).unwrap_err()
    };

    for status in [404, 405] {
        env.openai
            .push(OpenAiReply::error(status, "not_found", "Not found"));
        let e = call(Provider::OpenAi, "gpt-6");
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
        let e = call(Provider::Anthropic, "claude-opus-5");
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
    let e = call(Provider::Anthropic, "claude-nope");
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(e.message.contains("model: claude-nope"), "{}", e.message);
}

#[test]
fn the_official_api_leaves_no_host_in_the_trail() {
    let cred = ai::Credential::new(Provider::OpenAi, "http://127.0.0.1:9", ai::Secret::new("k"));
    assert_eq!(cred.custom_host, None);
    let line = ai::trail_line(
        Provider::OpenAi,
        "gpt-6",
        ai::Usage::default(),
        ai::Outcome::Ok,
        Duration::ZERO,
        cred.custom_host.as_deref(),
    );
    assert!(!line.contains("host="), "{line}");
}
