//! The Anthropic Messages client against the testkit fake (ai D1, D3, D5, D10, D11).

mod common;

use std::time::Duration;

use ai::{
    Content, Effort, ErrorKind, Event, Message, Outcome, Provider, ProviderState, Request, Role,
    Stop,
};
use common::{Env, events, get_note_tool, text_of};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, AnthropicReply};

fn request(prompt: &str) -> Request {
    Request::new(Provider::Anthropic, "claude-opus-5", prompt)
}

fn assert_no_sampling(body: &Value) {
    for field in ["temperature", "top_p", "top_k", "fallbacks"] {
        assert!(body.get(field).is_none(), "{field} was sent: {body}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_text_with_the_pinned_version() {
    let env = Env::new();
    env.anthropic.push(
        AnthropicMessage::text("Hello there, roof person")
            .chunk(3)
            .usage(21, 7),
    );
    let http = ai::client();
    let mut req = request("hi");
    req.system = Some("Be brief.".to_owned());
    let mut s = ai::stream(&http, &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none(), "{err:?}");
    let deltas = evs
        .iter()
        .filter(|e| matches!(e, Event::TextDelta(_)))
        .count();
    assert!(deltas > 3, "text arrived in pieces: {evs:?}");
    assert_eq!(text_of(&evs), "Hello there, roof person");
    assert!(evs.contains(&Event::Usage(ai::Usage {
        input: 21,
        output: 7
    })));
    assert!(matches!(
        evs.last(),
        Some(Event::Done {
            stop: Stop::EndTurn,
            provider_state: Some(_)
        })
    ));

    let sent = env.anthropic.message_requests();
    assert_eq!(sent.len(), 1);
    let r = &sent[0];
    assert_eq!(r.header("anthropic-version"), Some("2026-10-21"));
    assert_eq!(r.header("x-api-key"), Some(env.anthropic.api_key()));
    let body = r.json();
    assert_no_sampling(&body);
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["max_tokens"], json!(64000));
    assert_eq!(body["system"], json!("Be brief."));
    assert!(
        body.get("thinking").is_none(),
        "no effort, no thinking: {body}"
    );
    assert!(body.get("output_config").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_call_arrives_whole_from_several_json_chunks() {
    let env = Env::new();
    let input = json!({"id": "01JROOF", "fields": ["title", "status", "decided"]});
    env.anthropic.push(
        AnthropicMessage::new()
            .with_text("Let me look.")
            .tool_use("toolu_1", "get_note", input.clone())
            .chunk(5),
    );
    let mut req = request("what about the roof?");
    req.tools = vec![get_note_tool()];
    let http = ai::client();
    let mut s = ai::stream(&http, &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none(), "{err:?}");
    let calls: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            Event::ToolCall(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "toolu_1");
    assert_eq!(calls[0].name, "get_note");
    assert_eq!(calls[0].input, input);
    assert_eq!(calls[0].error, None);
    let Some(Event::Done {
        stop,
        provider_state: Some(state),
    }) = evs.last()
    else {
        panic!("no Done: {evs:?}");
    };
    assert_eq!(*stop, Stop::ToolUse);
    assert_eq!(
        state.output[1]["input"], input,
        "the block keeps the parsed input"
    );

    let body = env.anthropic.message_requests()[0].json();
    let tool = &body["tools"][0];
    assert_eq!(tool["strict"], json!(true));
    assert_eq!(tool["eager_input_streaming"], json!(true));
    assert_eq!(tool["input_schema"], get_note_tool().input_schema);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_tool_inputs_are_flagged_not_run() {
    let env = Env::new();
    env.anthropic.push(
        AnthropicMessage::new()
            .tool_use("toolu_1", "get_note", json!({"id": 7, "fields": []}))
            .tool_use_raw("toolu_2", "get_note", "{\"id\": \"x\", \"fields\": [")
            .tool_use("toolu_3", "delete_everything", json!({})),
    );
    let mut req = request("go");
    req.tools = vec![get_note_tool()];
    let reply = ai::complete(&ai::client(), &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    assert_eq!(reply.stop, Stop::ToolUse);
    let errors: Vec<_> = reply
        .tool_calls
        .iter()
        .map(|c| c.error.clone().unwrap_or_default())
        .collect();
    assert!(
        errors[0].contains("schema") && errors[0].contains("/id"),
        "{errors:?}"
    );
    assert!(errors[1].contains("not valid JSON"), "{errors:?}");
    assert_eq!(
        reply.tool_calls[1].input,
        json!("{\"id\": \"x\", \"fields\": [")
    );
    assert!(errors[2].contains("no tool named"), "{errors:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_tool_runs_on_a_turn_cut_off_by_max_tokens() {
    let env = Env::new();
    env.anthropic.push(
        AnthropicMessage::new()
            .tool_use("toolu_1", "get_note", json!({"id": "a", "fields": []}))
            .stop("max_tokens"),
    );
    let mut req = request("go");
    req.tools = vec![get_note_tool()];
    let mut s = ai::stream(&ai::client(), &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none());
    assert!(
        !evs.iter().any(|e| matches!(e, Event::ToolCall(_))),
        "{evs:?}"
    );
    assert_eq!(
        evs.last(),
        Some(&Event::Done {
            stop: Stop::MaxTokens,
            provider_state: None
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_schema_constrains_the_reply_and_effort_turns_on_adaptive_thinking() {
    let env = Env::new();
    env.anthropic
        .push(AnthropicMessage::text(r#"{"title": "Roof", "done": true}"#));
    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["title", "done"],
        "properties": {"title": {"type": "string"}, "done": {"type": "boolean"}}
    });
    let mut req = request("summarise");
    req.schema = Some(schema.clone());
    req.effort = Some(Effort::High);
    let reply = ai::complete(&ai::client(), &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    assert_eq!(reply.json, Some(json!({"title": "Roof", "done": true})));
    let body = env.anthropic.message_requests()[0].json();
    assert_eq!(
        body["output_config"],
        json!({"effort": "high", "format": {"type": "json_schema", "schema": schema}})
    );
    assert_eq!(body["thinking"], json!({"type": "adaptive"}));
    assert_no_sampling(&body);

    // A reply that breaks the schema is an invalid error.
    env.anthropic
        .push(AnthropicMessage::text(r#"{"title": 3}"#));
    let e = ai::complete(&ai::client(), &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid, "{e}");
}

#[test]
fn a_blocking_call_is_answered_whole() {
    let env = Env::new();
    env.anthropic
        .push(AnthropicMessage::text(r#"{"n": 2}"#).usage(3, 4));
    let mut req = request("count");
    req.schema = Some(json!({"type": "object", "properties": {"n": {"type": "integer"}}}));
    req.tools = vec![get_note_tool()];
    let reply = ai::complete_blocking(
        &ai::blocking_client(),
        &env.cred(Provider::Anthropic),
        &req,
        None,
    )
    .unwrap();
    assert_eq!(reply.json, Some(json!({"n": 2})));
    assert_eq!(
        reply.usage,
        ai::Usage {
            input: 3,
            output: 4
        }
    );
    let body = env.anthropic.message_requests()[0].json();
    assert!(body.get("stream").is_none());
    assert_eq!(body["max_tokens"], json!(16000));
    assert!(body["tools"][0].get("eager_input_streaming").is_none());
    assert_eq!(body["tools"][0]["strict"], json!(true));
    assert_no_sampling(&body);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_ends_the_stream_and_is_an_error_when_collected() {
    let env = Env::new();
    let refusal = || {
        AnthropicMessage::new()
            .with_text("I started to")
            .refusal(Some("cyber"), Some("This touches a restricted area."))
    };
    env.anthropic.push(refusal());
    let req = request("do the thing");
    let mut s = ai::stream(&ai::client(), &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none());
    assert_eq!(text_of(&evs), "I started to", "the partial answer stays");
    assert_eq!(
        evs.last(),
        Some(&Event::Done {
            stop: Stop::Refusal {
                category: Some("cyber".to_owned()),
                explanation: Some("This touches a restricted area.".to_owned()),
            },
            provider_state: None,
        })
    );

    env.anthropic.push(refusal());
    let result = ai::complete(&ai::client(), &env.cred(Provider::Anthropic), &req).await;
    let e = result.as_ref().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Refusal);
    assert_eq!(e.category.as_deref(), Some("cyber"));
    assert_eq!(e.provider, Some(Provider::Anthropic));
    assert_eq!(e.model.as_deref(), Some("claude-opus-5"));
    assert!(e.message.contains("cyber"), "{}", e.message);
    assert_eq!(Outcome::of(&result), Outcome::Refused);
    let line = ai::trail_for(
        Provider::Anthropic,
        &req.model,
        &result,
        Duration::from_millis(5),
    );
    assert!(line.contains("outcome=refused"), "{line}");
    assert_eq!(
        env.anthropic.message_requests().len(),
        2,
        "a refusal is never retried"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_429_is_retried_after_retry_after() {
    let env = Env::new();
    env.anthropic.push(AnthropicReply::rate_limited(1));
    env.anthropic.push(AnthropicMessage::text("Fine now"));
    let began = std::time::Instant::now();
    let reply = ai::complete(
        &ai::client(),
        &env.cred(Provider::Anthropic),
        &request("hi"),
    )
    .await
    .unwrap();
    assert_eq!(reply.text, "Fine now");
    assert!(
        began.elapsed() >= Duration::from_millis(900),
        "waited for retry-after"
    );
    let sent = env.anthropic.message_requests();
    assert_eq!(sent.len(), 2);
    for r in &sent {
        assert_eq!(r.header("anthropic-version"), Some("2026-10-21"));
        assert_no_sampling(&r.json());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_stop_after_two() {
    let env = Env::new();
    for _ in 0..3 {
        env.anthropic.push(AnthropicReply::rate_limited(0));
    }
    let e = ai::complete(
        &ai::client(),
        &env.cred(Provider::Anthropic),
        &request("hi"),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::RateLimit);
    assert_eq!(e.status, Some(429));
    assert_eq!(env.anthropic.message_requests().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_broken_off_is_an_error_and_not_retried() {
    let env = Env::new();
    env.anthropic.push(
        AnthropicMessage::text("Hello there, partial")
            .chunk(2)
            .cut_after(5),
    );
    let mut s = ai::stream(
        &ai::client(),
        &env.cred(Provider::Anthropic),
        &request("hi"),
    )
    .await
    .unwrap();
    let (evs, err) = events(&mut s).await;
    let e = err.expect("an error");
    assert_eq!(e.kind, ErrorKind::Provider, "{e}");
    assert!(e.message.contains("message_stop"), "{e}");
    assert!(!evs.iter().any(|e| matches!(e, Event::Done { .. })));
    assert_eq!(env.anthropic.message_requests().len(), 1);

    env.anthropic
        .push(AnthropicMessage::text("Hello there").error_after(
            4,
            "overloaded_error",
            "Overloaded",
        ));
    let e = ai::complete(
        &ai::client(),
        &env.cred(Provider::Anthropic),
        &request("hi"),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unavailable, "{e}");
    assert_eq!(env.anthropic.message_requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn thinking_is_replayed_to_the_same_model_only() {
    let env = Env::new();
    env.anthropic.push(
        AnthropicMessage::new()
            .thinking("The roof was decided in March.", "sig-abc123")
            .with_text("You chose slate."),
    );
    let http = ai::client();
    let cred = env.cred(Provider::Anthropic);
    let first = request("roof?");
    let reply = ai::complete(&http, &cred, &first).await.unwrap();
    let state: ProviderState = reply.provider_state.clone().expect("provider state");
    assert_eq!(state.provider, Provider::Anthropic);
    assert_eq!(state.model, "claude-opus-5");
    let assistant = reply.message();
    assert_eq!(assistant.role, Role::Assistant);
    assert_eq!(assistant.content, vec![Content::text("You chose slate.")]);

    let history = vec![first.messages[0].clone(), assistant, Message::user("why?")];
    let mut second = request("");
    second.messages = history.clone();
    ai::complete(&http, &cred, &second).await.unwrap();
    let body = env.anthropic.message_requests()[1].json();
    let replayed = &body["messages"][1];
    assert_eq!(replayed["role"], json!("assistant"));
    assert_eq!(
        replayed["content"],
        json!([
            {"type": "thinking", "thinking": "The roof was decided in March.", "signature": "sig-abc123"},
            {"type": "text", "text": "You chose slate."}
        ])
    );

    let mut other = Request::new(Provider::Anthropic, "claude-sonnet-5", "");
    other.messages = history;
    ai::complete(&http, &cred, &other).await.unwrap();
    let body = env.anthropic.message_requests()[2].json();
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type": "text", "text": "You chose slate."}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_results_go_first_in_the_user_turn() {
    let env = Env::new();
    let mut req = request("");
    req.messages = vec![
        Message::user("look it up"),
        Message {
            role: Role::Assistant,
            content: vec![Content::ToolCall {
                id: "toolu_1".into(),
                name: "get_note".into(),
                input: json!({"id": "a", "fields": []}),
            }],
            provider_state: None,
        },
        Message::tool_results(vec![
            Content::text("and then?"),
            Content::ToolResult {
                id: "toolu_1".into(),
                content: "{}".into(),
                is_error: true,
                parts: vec![],
            },
            Content::Notice {
                kind: "cut_off".into(),
                text: "never sent".into(),
            },
        ]),
    ];
    ai::complete(&ai::client(), &env.cred(Provider::Anthropic), &req)
        .await
        .unwrap();
    let body = env.anthropic.message_requests()[0].json();
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type": "tool_use", "id": "toolu_1", "name": "get_note", "input": {"id": "a", "fields": []}}])
    );
    assert_eq!(
        body["messages"][2]["content"],
        json!([
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "{}", "is_error": true},
            {"type": "text", "text": "and then?"}
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn models_are_listed_across_pages() {
    let env = Env::new();
    let fake = testkit::FakeAnthropic::builder().page_size(1).start();
    let cred = ai::Credential::new(
        Provider::Anthropic,
        fake.base_url(),
        ai::Secret::new(fake.api_key()),
    );
    let models = ai::list_models(&ai::client(), &cred).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"]
    );
    assert_eq!(models[0].display_name.as_deref(), Some("Claude Opus 5"));
    assert_eq!(fake.requests().len(), 3, "one request per page");
    assert_eq!(
        ai::preselect(Provider::Anthropic, &models).map(|m| m.id.as_str()),
        Some("claude-opus-5")
    );
    drop(env);
}

#[test]
fn models_are_listed_blocking_too() {
    let env = Env::new();
    let models =
        ai::list_models_blocking(&ai::blocking_client(), &env.cred(Provider::Anthropic), None)
            .unwrap();
    assert_eq!(models.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_key_is_an_auth_error_without_the_key() {
    let env = Env::new();
    let cred = ai::Credential::new(
        Provider::Anthropic,
        env.anthropic.base_url(),
        ai::Secret::new("sk-ant-wrong-key-9876543210"),
    );
    let e = ai::complete(&ai::client(), &cred, &request("hi"))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert_eq!(e.status, Some(401));
    assert!(!format!("{e} {e:?}").contains("wrong-key"));
}

/// The chat route spawns these on the runtime, so they must be `Send`.
#[allow(dead_code)]
fn futures_are_send(http: &reqwest::Client, cred: &ai::Credential, req: &Request) {
    fn send<T: Send>(_: T) {}
    send(ai::stream(http, cred, req));
    send(ai::complete(http, cred, req));
    send(ai::list_models(http, cred));
    send(async {
        let s = ai::stream(http, cred, req).await.unwrap();
        let _ = s.into_stream();
    });
}
