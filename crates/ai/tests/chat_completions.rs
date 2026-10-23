//! The OpenAI Chat Completions client (ai D25) against the testkit fake in Chat Completions
//! mode: streamed text, tool calls whose arguments arrive in pieces, schemas and the
//! `json_object` fallback, finish reasons, usage, images after tool messages, exact replay, and
//! the official API and the ChatGPT plan staying on Responses.

mod common;

use std::sync::Arc;

use ai::{
    Content, Credential, Credentials, Endpoints, ErrorKind, Event, ImageData, Message, OpenAiApi,
    Part, Provider, Request, Role, Settings, Stop,
};
use common::{Env, events, get_note_tool, text_of};
use serde_json::{Value, json};
use state::{SecretKey, State};
use tempfile::TempDir;
use testkit::{FakeOpenAi, FakeOpenAiBuilder, OpenAiReply, OpenAiResponse};

/// A one-pixel PNG.
const PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A `state.db` whose OpenAI card points at a Chat Completions fake, saved as such.
struct Chat {
    _dir: TempDir,
    state: Arc<State>,
    fake: FakeOpenAi,
}

impl Chat {
    fn new() -> Chat {
        Chat::with(FakeOpenAi::builder().chat_completions())
    }

    fn with(builder: FakeOpenAiBuilder) -> Chat {
        Chat::saved(builder.start(), Some(OpenAiApi::ChatCompletions))
    }

    fn saved(fake: FakeOpenAi, api: Option<OpenAiApi>) -> Chat {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(&dir.path().join("state.db"), Some(SecretKey::generate())).unwrap();
        Settings {
            endpoints: Endpoints {
                openai: Some(fake.base_url()),
                openai_api: api,
                ..Endpoints::default()
            },
            ..Settings::default()
        }
        .save(&state)
        .unwrap();
        state
            .set_secret(ai::OPENAI_API_KEY, fake.api_key())
            .unwrap();
        Chat {
            _dir: dir,
            state: Arc::new(state),
            fake,
        }
    }

    fn cred(&self) -> Credential {
        Credentials::resolve(&self.state, Provider::OpenAi).unwrap()
    }

    fn body(&self, n: usize) -> Value {
        self.fake.chat_requests()[n].json()
    }
}

fn request(prompt: &str) -> Request {
    Request::new(Provider::OpenAi, "glm-5.3-flash", prompt)
}

/// No sampling parameter (ai D3) and no `strict` on a tool, which many services reject.
fn assert_plain(body: &Value) {
    for f in [
        "temperature",
        "top_p",
        "top_k",
        "store",
        "input",
        "instructions",
    ] {
        assert!(body.get(f).is_none(), "{f} was sent: {body}");
    }
    for t in body["tools"].as_array().into_iter().flatten() {
        assert!(t.get("strict").is_none(), "{t}");
        assert!(t["function"].get("strict").is_none(), "{t}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_text_and_counts_usage() {
    let env = Chat::new();
    let cred = env.cred();
    assert!(cred.chat_completions());
    env.fake.push(
        OpenAiResponse::text("Slate, in March.")
            .chunk(3)
            .usage(40, 6),
    );
    let mut req = request("roof?");
    req.system = Some("Answer from the notes.".to_owned());
    let mut s = ai::stream(&ai::client(), &cred, &req).await.unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text_of(&evs), "Slate, in March.");
    assert!(evs.len() > 4, "streamed in pieces: {evs:?}");
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

    assert!(env.fake.response_requests().is_empty());
    let body = env.body(0);
    assert_plain(&body);
    assert_eq!(body["model"], json!("glm-5.3-flash"));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "Answer from the notes."},
            {"role": "user", "content": "roof?"},
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reasoning_content_is_not_the_answer() {
    let env = Chat::new();
    env.fake.push(
        OpenAiResponse::new()
            .reasoning("Thinking about roofs", "enc")
            .with_text("Slate."),
    );
    let reply = ai::complete(&ai::client(), &env.cred(), &request("roof?"))
        .await
        .unwrap();
    assert_eq!(reply.text, "Slate.");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_call_in_pieces_is_one_checked_call_and_replays_exactly() {
    let env = Chat::new();
    let args = json!({"id": "01JROOF", "fields": ["title", "body"]});
    env.fake.push(
        OpenAiResponse::new()
            .with_text("Looking.")
            .function_call("call_7", "get_note", args.clone())
            .chunk(5),
    );
    let mut req = request("roof?");
    req.tools = vec![get_note_tool()];
    let http = ai::client();
    let cred = env.cred();
    let mut s = ai::stream(&http, &cred, &req).await.unwrap();
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
    assert_eq!(calls[0].id, "call_7");
    assert_eq!(calls[0].name, "get_note");
    assert_eq!(calls[0].input, args);
    assert_eq!(calls[0].error, None);
    let Some(Event::Done {
        stop: Stop::ToolUse,
        provider_state: Some(state),
    }) = evs.last().cloned()
    else {
        panic!("{evs:?}");
    };
    let sent = json!({
        "role": "assistant",
        "content": "Looking.",
        "tool_calls": [{"id": "call_7", "type": "function",
                        "function": {"name": "get_note", "arguments": args.to_string()}}],
    });
    assert_eq!(state.output, json!([sent.clone()]));

    let body = env.body(0);
    assert_plain(&body);
    assert_eq!(
        body["tools"],
        json!([{"type": "function", "function": {
            "name": "get_note", "description": "Read one note by id.",
            "parameters": get_note_tool().input_schema,
        }}])
    );

    // The next turn replays the assistant message as it was, then answers the call.
    let assistant = Message {
        role: Role::Assistant,
        content: vec![
            Content::text("Looking."),
            Content::ToolCall {
                id: "call_7".into(),
                name: "get_note".into(),
                input: args.clone(),
            },
        ],
        provider_state: Some(state),
    };
    let mut next = request("");
    next.tools = vec![get_note_tool()];
    next.messages = vec![
        Message::user("roof?"),
        assistant.clone(),
        Message::tool_results(vec![Content::tool_result("call_7", "Slate roof", false)]),
    ];
    env.fake.push(OpenAiResponse::text("Slate."));
    let reply = ai::complete(&http, &cred, &next).await.unwrap();
    assert_eq!(reply.text, "Slate.");
    let messages = env.body(1)["messages"].clone();
    assert_eq!(messages[1], sent, "replayed exactly");
    assert_eq!(
        messages[2],
        json!({"role": "tool", "tool_call_id": "call_7", "content": "Slate roof"})
    );

    // Another model gets the turn rebuilt from the transcript.
    let mut other = next.clone();
    other.model = "glm-5".into();
    let mut a = assistant;
    a.content = vec![Content::ToolCall {
        id: "call_7".into(),
        name: "get_note".into(),
        input: args.clone(),
    }];
    other.messages[1] = a;
    ai::complete(&http, &cred, &other).await.unwrap();
    assert_eq!(
        env.body(2)["messages"][1],
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": "call_7", "type": "function",
               "function": {"name": "get_note", "arguments": args.to_string()}}]})
    );
}

#[test]
fn a_schema_reply_is_parsed_and_checked() {
    let env = Chat::new();
    env.fake.push(OpenAiResponse::text(r#"{"answer": "yes"}"#));
    let schema = json!({"type": "object", "additionalProperties": false, "required": ["answer"],
                        "properties": {"answer": {"type": "string"}}});
    let mut req = request("is it slate?");
    req.schema = Some(schema.clone());
    let reply = ai::complete_blocking(&ai::blocking_client(), &env.cred(), &req, None).unwrap();
    assert_eq!(reply.json, Some(json!({"answer": "yes"})));
    let body = env.body(0);
    assert_plain(&body);
    assert!(body.get("stream").is_none(), "{body}");
    assert_eq!(
        body["response_format"],
        json!({"type": "json_schema", "json_schema": {"name": "reply", "schema": schema, "strict": true}})
    );
}

#[test]
fn a_service_without_json_schema_gets_json_object_and_the_reply_is_still_checked() {
    let env = Chat::with(
        FakeOpenAi::builder()
            .chat_completions()
            .reject_json_schema(),
    );
    let schema = json!({"type": "object", "additionalProperties": false, "required": ["answer"],
                        "properties": {"answer": {"type": "string"}}});
    let mut req = request("is it slate?");
    req.system = Some("Be brief.".into());
    req.schema = Some(schema.clone());
    let http = ai::blocking_client();
    env.fake.push(OpenAiResponse::text(r#"{"answer": "yes"}"#));
    let reply = ai::complete_blocking(&http, &env.cred(), &req, None).unwrap();
    assert_eq!(reply.json, Some(json!({"answer": "yes"})));
    let sent = env.fake.chat_requests();
    assert_eq!(sent.len(), 2, "one retry");
    assert_eq!(
        sent[0].json()["response_format"]["type"],
        json!("json_schema")
    );
    let body = sent[1].json();
    assert_eq!(body["response_format"], json!({"type": "json_object"}));
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert_eq!(body["messages"][0]["role"], json!("system"));
    assert!(system.starts_with("Be brief."), "{system}");
    assert!(system.contains(&schema.to_string()), "{system}");

    // A reply breaking the schema is still refused.
    env.fake.push(OpenAiResponse::text(r#"{"answer": 3}"#));
    let e = ai::complete_blocking(&http, &env.cred(), &req, None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(e.message.contains("schema"), "{e}");

    // Streamed, the same.
    env.fake.push(OpenAiResponse::text(r#"{"answer": "no"}"#));
    let reply = ai::stream_blocking(&http, &env.cred(), &req, None, |_| {}).unwrap();
    assert_eq!(reply.json, Some(json!({"answer": "no"})));
}

#[tokio::test(flavor = "multi_thread")]
async fn finish_reasons_length_and_content_filter_and_a_refusal() {
    let env = Chat::new();
    let http = ai::client();
    let cred = env.cred();

    env.fake
        .push(OpenAiResponse::text("Slate, in").incomplete("max_output_tokens"));
    let reply = ai::complete(&http, &cred, &request("roof?")).await.unwrap();
    assert_eq!(reply.stop, Stop::MaxTokens);
    assert_eq!(reply.text, "Slate, in");
    assert_eq!(reply.provider_state, None);

    env.fake
        .push(OpenAiResponse::text("Part").incomplete("content_filter"));
    let mut s = ai::stream(&http, &cred, &request("x")).await.unwrap();
    let (evs, err) = events(&mut s).await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text_of(&evs), "Part");
    assert!(matches!(
        evs.last(),
        Some(Event::Done { stop: Stop::Refusal { category: Some(c), .. }, provider_state: None })
            if c == "content_filter"
    ));
    let e = {
        env.fake
            .push(OpenAiResponse::text("Part").incomplete("content_filter"));
        ai::complete(&http, &cred, &request("x")).await.unwrap_err()
    };
    assert_eq!(e.kind, ErrorKind::Refusal);
    assert_eq!(e.category.as_deref(), Some("content_filter"));

    env.fake
        .push(OpenAiResponse::new().refusal("I can't help with that."));
    let e = ai::complete(&http, &cred, &request("x")).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Refusal);
    assert_eq!(e.explanation.as_deref(), Some("I can't help with that."));
}

#[test]
fn a_blocking_whole_answer_maps_length_and_sends_max_tokens() {
    let env = Chat::new();
    let cred = env.cred();
    env.fake.push(
        OpenAiResponse::text("Slate")
            .incomplete("max_output_tokens")
            .usage(9, 16),
    );
    let mut req = request("roof?");
    req.max_tokens = Some(16);
    let reply = ai::complete_blocking(&ai::blocking_client(), &cred, &req, None).unwrap();
    assert_eq!(reply.stop, Stop::MaxTokens);
    assert_eq!(
        reply.usage,
        ai::Usage {
            input: 9,
            output: 16
        }
    );
    assert_eq!(
        env.fake.chat_requests().last().unwrap().json()["max_tokens"],
        json!(16)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_stream_is_an_error() {
    let env = Chat::new();
    env.fake
        .push(OpenAiResponse::text("partial").failed("rate_limit_exceeded", "Slow down"));
    let mut s = ai::stream(&ai::client(), &env.cred(), &request("x"))
        .await
        .unwrap();
    let (evs, err) = events(&mut s).await;
    assert_eq!(text_of(&evs), "partial");
    let e = err.unwrap();
    assert_eq!(e.kind, ErrorKind::RateLimit);
    assert!(e.message.contains("Slow down"), "{e}");
}

#[test]
fn a_blocking_tool_call_is_answered_whole() {
    let env = Chat::new();
    env.fake.push(
        OpenAiResponse::new()
            .function_call("call_1", "get_note", json!({"id": "01J", "fields": []}))
            .usage(30, 4),
    );
    let mut req = request("roof?");
    req.tools = vec![get_note_tool()];
    let reply = ai::complete_blocking(&ai::blocking_client(), &env.cred(), &req, None).unwrap();
    assert_eq!(reply.stop, Stop::ToolUse);
    assert_eq!(reply.tool_calls.len(), 1);
    assert_eq!(
        reply.tool_calls[0].input,
        json!({"id": "01J", "fields": []})
    );
    assert_eq!(
        reply.usage,
        ai::Usage {
            input: 30,
            output: 4
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_result_is_a_tool_message_then_a_user_message_of_image_urls() {
    let env = Chat::new();
    let mut req = request("");
    req.messages = vec![
        Message::user("what is in the picture?"),
        Message {
            role: Role::Assistant,
            content: vec![Content::ToolCall {
                id: "call_1".into(),
                name: "read_asset".into(),
                input: json!({"id": "01JPIC"}),
            }],
            provider_state: None,
        },
        Message::tool_results(vec![Content::ToolResult {
            id: "call_1".into(),
            content: "{\"id\":\"01JPIC\"}".into(),
            is_error: false,
            parts: vec![
                Part::Image {
                    asset_id: "01JPIC".into(),
                    media_type: "image/png".into(),
                    data: Some(ImageData {
                        media_type: "image/png".into(),
                        base64: PIXEL_PNG.into(),
                    }),
                },
                Part::image("01JGONE", "image/png"),
            ],
        }]),
    ];
    ai::complete(&ai::client(), &env.cred(), &req)
        .await
        .unwrap();
    let messages = env.body(0)["messages"].clone();
    assert_eq!(messages.as_array().unwrap().len(), 4, "{messages}");
    assert_eq!(messages[2]["role"], json!("tool"));
    assert_eq!(messages[2]["tool_call_id"], json!("call_1"));
    let text = messages[2]["content"].as_str().unwrap();
    assert!(text.starts_with("{\"id\":\"01JPIC\"}"), "{text}");
    assert!(
        text.contains("[image asset 01JGONE was not loaded]"),
        "{text}"
    );
    assert_eq!(messages[3]["role"], json!("user"));
    let parts = messages[3]["content"].as_array().unwrap();
    assert_eq!(parts[0]["type"], json!("text"));
    assert_eq!(
        parts[1],
        json!({"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{PIXEL_PNG}")}})
    );
    let images = env.fake.images();
    assert_eq!(images[0].len(), 1);
    assert_eq!(&images[0][0].bytes[1..4], b"PNG");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_saved_type_is_used_without_probing() {
    // Saved Responses on a Chat Completions service: the named error, and no second try.
    let env = Chat::saved(FakeOpenAi::builder().chat_completions().start(), None);
    let cred = env.cred();
    assert!(!cred.chat_completions());
    let e = ai::complete(&ai::client(), &cred, &request("x"))
        .await
        .unwrap_err();
    assert!(e.is_unsupported_api(), "{e:?}");
    assert!(e.message.contains("Responses API"), "{e}");
    assert!(env.fake.chat_requests().is_empty());

    // Saved Chat Completions on a Responses service.
    let env = Chat::saved(FakeOpenAi::start(), Some(OpenAiApi::ChatCompletions));
    let e = ai::complete(&ai::client(), &env.cred(), &request("x"))
        .await
        .unwrap_err();
    assert!(e.is_unsupported_api(), "{e:?}");
    assert_eq!(
        e.message,
        "This service does not support the OpenAI Chat Completions API (POST /v1/chat/completions)."
    );
    assert!(env.fake.response_requests().is_empty());
}

#[test]
fn a_versioned_root_reaches_chat_completions_under_it() {
    assert_eq!(
        ai::api_url(
            "https://api.z.ai/api/coding/paas/v4",
            "/v1/chat/completions"
        ),
        "https://api.z.ai/api/coding/paas/v4/chat/completions"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_chatgpt_plan_and_the_official_api_always_use_responses() {
    // The plan, with the OpenAI card saved as Chat Completions.
    let env = Env::new();
    let mut settings = Settings::load(&env.state).unwrap();
    settings.endpoints.openai_api = Some(OpenAiApi::ChatCompletions);
    settings.save(&env.state).unwrap();
    env.sign_in("fake-plan-access-token", 60 * 60 * 1000);
    let plan = env.cred(Provider::ChatGpt);
    assert!(!plan.chat_completions());
    let req = Request::new(Provider::ChatGpt, "gpt-6.1-sol", "x");
    ai::complete(&ai::client(), &plan, &req).await.unwrap();
    assert_eq!(env.openai.response_requests().len(), 1);
    // The API key on the same custom URL does use Chat Completions.
    assert!(env.cred(Provider::OpenAi).chat_completions());

    // The official API: no URL, so the saved type is not read.
    let e = Endpoints {
        openai_api: Some(OpenAiApi::ChatCompletions),
        ..Endpoints::default()
    };
    assert_eq!(e.openai_api(), OpenAiApi::Responses);
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(&dir.path().join("state.db"), Some(SecretKey::generate())).unwrap();
    Settings {
        endpoints: e,
        ..Settings::default()
    }
    .save(&state)
    .unwrap();
    state.set_secret(ai::OPENAI_API_KEY, "sk-x").unwrap();
    let cred = Credentials::resolve(&state, Provider::OpenAi).unwrap();
    assert_eq!(cred.base_url, ai::OPENAI_BASE);
    assert!(!cred.chat_completions());
    // A credential made without a custom host never speaks Chat Completions either.
    let mut bare = Credential::new(Provider::OpenAi, ai::OPENAI_BASE, ai::Secret::new("k"));
    bare.openai_api = OpenAiApi::ChatCompletions;
    assert!(!bare.chat_completions());
}

#[test]
fn a_bad_request_is_not_an_unsupported_api() {
    let env = Chat::new();
    env.fake
        .push(OpenAiReply::error(400, "invalid_request", "model: unknown"));
    let e = ai::complete_blocking(&ai::blocking_client(), &env.cred(), &request("x"), None)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(!e.is_unsupported_api());
}
