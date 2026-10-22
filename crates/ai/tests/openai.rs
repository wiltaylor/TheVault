//! The OpenAI Responses client against the testkit fake, with an API key and with a ChatGPT
//! plan token (ai D1, D3, D4, D6, D10, D11).

mod common;

use ai::{Content, Credential, ErrorKind, Event, Message, Provider, Request, Secret, Stop};
use common::{Env, events, get_note_tool, text_of};
use serde_json::{Value, json};
use testkit::{OpenAiReply, OpenAiResponse, fake_openai::PLAN_FORBIDDEN};

fn request(prompt: &str) -> Request {
    Request::new(Provider::OpenAi, "gpt-6", prompt)
}

fn plan_request(prompt: &str) -> Request {
    Request::new(Provider::ChatGpt, "gpt-6.1-sol", prompt)
}

/// A plan credential with a fixed access token, so no refresh runs.
fn plan_cred(env: &Env) -> Credential {
    Credential::new(
        Provider::ChatGpt,
        env.openai.base_url(),
        Secret::new("fake-plan-access-token"),
    )
}

fn assert_plan_body(body: &Value) {
    assert_eq!(body["store"], json!(false), "{body}");
    assert_eq!(body["stream"], json!(true), "{body}");
    for field in PLAN_FORBIDDEN {
        assert!(body.get(field).is_none(), "{field} was sent: {body}");
    }
    assert!(body.get("top_k").is_none());
    let system_items = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["role"] == json!("system"))
        .count();
    assert_eq!(system_items, 0, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_text_with_an_api_key() {
    let env = Env::new();
    env.openai.push(
        OpenAiResponse::text("Slate, in March.")
            .chunk(3)
            .usage(40, 6),
    );
    let mut req = request("roof?");
    req.system = Some("Answer from the notes.".to_owned());
    let mut s = ai::stream(&ai::client(), &env.cred(Provider::OpenAi), &req)
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text_of(&evs), "Slate, in March.");
    assert!(evs.contains(&Event::Usage(ai::Usage {
        input: 40,
        output: 6
    })));
    assert!(matches!(
        evs.last(),
        Some(Event::Done {
            stop: Stop::EndTurn,
            provider_state: Some(_)
        })
    ));

    let r = &env.openai.response_requests()[0];
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {}", env.openai.api_key()).as_str())
    );
    let body = r.json();
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["instructions"], json!("Answer from the notes."));
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(
        body["input"],
        json!([{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "roof?"}]}])
    );
    for field in [
        "temperature",
        "top_p",
        "max_output_tokens",
        "previous_response_id",
    ] {
        assert!(body.get(field).is_none(), "{field}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_function_call_is_a_checked_tool_call() {
    let env = Env::new();
    let args = json!({"id": "01JROOF", "fields": ["title"]});
    env.openai.push(
        OpenAiResponse::new()
            .reasoning("Look it up", "enc-1")
            .function_call("call_1", "get_note", args.clone())
            .chunk(4),
    );
    let mut req = request("roof?");
    req.tools = vec![get_note_tool()];
    let reply = ai::complete(&ai::client(), &env.cred(Provider::OpenAi), &req)
        .await
        .unwrap();
    assert_eq!(reply.stop, Stop::ToolUse);
    assert_eq!(reply.tool_calls.len(), 1);
    let call = &reply.tool_calls[0];
    assert_eq!(
        (call.id.as_str(), call.name.as_str()),
        ("call_1", "get_note")
    );
    assert_eq!(call.input, args);
    assert_eq!(call.error, None);
    let body = env.openai.response_requests()[0].json();
    assert_eq!(
        body["tools"],
        json!([{
            "type": "function", "name": "get_note", "description": "Read one note by id.",
            "parameters": get_note_tool().input_schema, "strict": true,
        }])
    );

    // The next turn sends the call and its result back.
    let mut next = request("");
    next.messages = vec![
        Message::user("roof?"),
        Message {
            provider_state: None,
            ..reply.message()
        },
        Message::tool_results(vec![Content::ToolResult {
            id: "call_1".into(),
            content: "no such note".into(),
            is_error: true,
        }]),
    ];
    ai::complete(&ai::client(), &env.cred(Provider::OpenAi), &next)
        .await
        .unwrap();
    let input = env.openai.response_requests()[1].json()["input"].clone();
    assert_eq!(input[1]["type"], json!("function_call"));
    assert_eq!(input[1]["call_id"], json!("call_1"));
    assert_eq!(
        serde_json::from_str::<Value>(input[1]["arguments"].as_str().unwrap()).unwrap(),
        args
    );
    assert_eq!(
        input[2],
        json!({"type": "function_call_output", "call_id": "call_1", "output": "Error: no such note"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn refusal_content_is_a_refusal() {
    let env = Env::new();
    env.openai
        .push(OpenAiResponse::new().refusal("I can't help with that."));
    let mut s = ai::stream(&ai::client(), &env.cred(Provider::OpenAi), &request("x"))
        .await
        .unwrap();
    let (evs, _) = events(&mut s).await;
    assert_eq!(
        evs.last(),
        Some(&Event::Done {
            stop: Stop::Refusal {
                category: None,
                explanation: Some("I can't help with that.".into())
            },
            provider_state: None,
        })
    );
    env.openai.push(OpenAiResponse::new().refusal("No."));
    let e = ai::complete(&ai::client(), &env.cred(Provider::OpenAi), &request("x"))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Refusal);
    assert_eq!(e.explanation.as_deref(), Some("No."));
}

#[tokio::test(flavor = "multi_thread")]
async fn response_failed_is_an_error_after_the_text_so_far() {
    let env = Env::new();
    env.openai.push(
        OpenAiResponse::text("half an ans")
            .failed("rate_limit_exceeded", "Slow down")
            .usage(9, 3),
    );
    let mut s = ai::stream(&ai::client(), &env.cred(Provider::OpenAi), &request("x"))
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert_eq!(text_of(&evs), "half an ans");
    let e = err.expect("failed");
    assert_eq!(e.kind, ErrorKind::RateLimit, "{e}");
    assert!(e.message.contains("Slow down"));
    assert_eq!(e.provider, Some(Provider::OpenAi));
    assert_eq!(
        env.openai.response_requests().len(),
        1,
        "never retried mid-stream"
    );
}

#[test]
fn a_blocking_api_key_call_is_answered_whole_and_may_cap_output() {
    let env = Env::new();
    env.openai.push(OpenAiResponse::text("Done."));
    let mut req = request("x");
    req.max_tokens = Some(300);
    let reply = ai::complete_blocking(
        &ai::blocking_client(),
        &env.cred(Provider::OpenAi),
        &req,
        None,
    )
    .unwrap();
    assert_eq!(reply.text, "Done.");
    let body = env.openai.response_requests()[0].json();
    assert_eq!(body["stream"], json!(false));
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["max_output_tokens"], json!(300));
}

#[test]
fn plan_requests_follow_the_plan_rules() {
    let env = Env::new();
    env.openai.push(OpenAiResponse::text("Planned.").chunk(2));
    let mut req = plan_request("x");
    req.system = Some("System prompt".to_owned());
    req.max_tokens = Some(100);
    req.effort = Some(ai::Effort::Max);
    req.tools = vec![get_note_tool()];
    let reply =
        ai::complete_blocking(&ai::blocking_client(), &plan_cred(&env), &req, None).unwrap();
    assert_eq!(reply.text, "Planned.");
    let r = &env.openai.response_requests()[0];
    assert_eq!(
        r.header("authorization"),
        Some("Bearer fake-plan-access-token")
    );
    assert!(r.header("originator").is_none());
    let body = r.json();
    assert_plan_body(&body);
    assert_eq!(body["instructions"], json!("System prompt"));
    assert_eq!(body["reasoning"], json!({"effort": "xhigh"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_streams_follow_the_plan_rules_too() {
    let env = Env::new();
    let mut req = plan_request("x");
    req.max_tokens = Some(16);
    let reply = ai::complete(&ai::client(), &plan_cred(&env), &req)
        .await
        .unwrap();
    assert_eq!(reply.text, "OK");
    assert_plan_body(&env.openai.response_requests()[0].json());
}

#[test]
fn a_plan_answer_without_response_completed_is_an_error() {
    let env = Env::new();
    env.openai
        .push(OpenAiResponse::text("Almost all of it").cut_after(4));
    let e = ai::complete_blocking(
        &ai::blocking_client(),
        &plan_cred(&env),
        &plan_request("x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Provider, "{e}");
    assert!(e.message.contains("response.completed"), "{e}");

    env.openai
        .push(OpenAiResponse::text("cut").incomplete("max_output_tokens"));
    let e = ai::complete_blocking(
        &ai::blocking_client(),
        &plan_cred(&env),
        &plan_request("x"),
        None,
    )
    .unwrap_err();
    assert!(e.message.contains("response.completed"), "{e}");
}

#[test]
fn plan_usage_limits_are_rate_limits() {
    let env = Env::new();
    env.openai.push(OpenAiReply::error(
        429,
        "subscription_sharing_usage_limit_exceeded",
        "You have reached your ChatGPT usage limit.",
    ));
    let e = ai::complete_blocking(
        &ai::blocking_client(),
        &plan_cred(&env),
        &plan_request("x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::RateLimit);
    assert!(e.message.contains("chatgpt.com/settings/usage"), "{e}");
    assert_eq!(
        env.openai.response_requests().len(),
        1,
        "a usage limit is not retried"
    );

    env.openai.push(OpenAiResponse::text("partial").failed(
        "subscription_sharing_usage_limit_exceeded",
        "Limit reached mid-way",
    ));
    let e = ai::complete_blocking(
        &ai::blocking_client(),
        &plan_cred(&env),
        &plan_request("x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::RateLimit, "{e}");

    env.openai
        .push(OpenAiReply::detail(403, "This account is not eligible."));
    let e = ai::complete_blocking(
        &ai::blocking_client(),
        &plan_cred(&env),
        &plan_request("x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert!(e.message.contains("not eligible"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn reasoning_is_replayed_to_the_same_model_only() {
    let env = Env::new();
    env.openai.push(
        OpenAiResponse::new()
            .reasoning("Recall the roof note", "enc-secret-reasoning")
            .with_text("Slate."),
    );
    let http = ai::client();
    let cred = env.cred(Provider::OpenAi);
    let first = request("roof?");
    let reply = ai::complete(&http, &cred, &first).await.unwrap();
    let assistant = reply.message();
    assert!(assistant.provider_state.is_some());
    let history = vec![first.messages[0].clone(), assistant, Message::user("why?")];

    let mut same = request("");
    same.messages = history.clone();
    ai::complete(&http, &cred, &same).await.unwrap();
    let input = env.openai.response_requests()[1].json()["input"].clone();
    assert_eq!(
        input[1],
        json!({
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": "Recall the roof note"}],
            "encrypted_content": "enc-secret-reasoning",
        }),
        "replayed verbatim but for its id"
    );
    assert_eq!(input[2]["type"], json!("message"));
    assert_eq!(input[2]["content"][0]["text"], json!("Slate."));
    assert!(input[2].get("id").is_none());

    let mut other = Request::new(Provider::OpenAi, "gpt-6-mini", "");
    other.messages = history;
    ai::complete(&http, &cred, &other).await.unwrap();
    let input = env.openai.response_requests()[2].json()["input"].clone();
    assert_eq!(input.as_array().unwrap().len(), 3);
    assert_eq!(
        input[1],
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Slate."}]})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_state_does_not_cross_to_the_api_key() {
    let env = Env::new();
    env.openai.push(
        OpenAiResponse::new()
            .reasoning("r", "enc-plan")
            .with_text("Hi"),
    );
    let reply = ai::complete(&ai::client(), &plan_cred(&env), &plan_request("x"))
        .await
        .unwrap();
    let mut req = Request::new(Provider::OpenAi, "gpt-6.1-sol", "");
    req.messages = vec![Message::user("x"), reply.message(), Message::user("y")];
    ai::complete(&ai::client(), &env.cred(Provider::OpenAi), &req)
        .await
        .unwrap();
    let input = env.openai.response_requests()[1].json()["input"].clone();
    assert!(!input.to_string().contains("enc-plan"), "{input}");
    assert_eq!(input[1]["role"], json!("assistant"));
}

#[tokio::test(flavor = "multi_thread")]
async fn model_lists_have_both_shapes() {
    let env = Env::new();
    let api = ai::list_models(&ai::client(), &env.cred(Provider::OpenAi))
        .await
        .unwrap();
    let ids: Vec<&str> = api.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-6", "gpt-6-mini", "text-embedding-4"]);
    assert!(api.iter().all(|m| m.display_name.is_none()));
    assert!(
        ai::preselect(Provider::OpenAi, &api).is_none(),
        "no OpenAI id is hard-coded"
    );

    let plan = ai::list_models(&ai::client(), &plan_cred(&env))
        .await
        .unwrap();
    assert_eq!(
        plan,
        vec![
            ai::Model {
                id: "gpt-6.1-sol".into(),
                display_name: Some("GPT-6.1 Sol".into())
            },
            ai::Model {
                id: "gpt-6-luna".into(),
                display_name: Some("GPT-6 Luna".into())
            },
        ],
        "hidden models are left out"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_for_another_provider_is_refused_before_sending() {
    let env = Env::new();
    let e = ai::complete(
        &ai::client(),
        &env.cred(Provider::OpenAi),
        &plan_request("x"),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    let mut req = request("x");
    req.model = String::new();
    let e = ai::complete(&ai::client(), &env.cred(Provider::OpenAi), &req)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotConfigured);
    assert!(env.openai.response_requests().is_empty());
}
