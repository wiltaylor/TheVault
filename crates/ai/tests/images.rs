//! Image parts in tool results (ai D19): Anthropic's `image` block in the `tool_result`, the
//! Responses API's `input_image` in the `function_call_output`, and the transcript form.

mod common;

use ai::{Content, ImageData, Kind, Message, Part, Request, Role};
use common::Env;
use serde_json::json;

/// A one-pixel PNG.
const PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn loaded(asset_id: &str) -> Part {
    Part::Image {
        asset_id: asset_id.into(),
        media_type: "image/png".into(),
        data: Some(ImageData {
            media_type: "image/png".into(),
            base64: PIXEL_PNG.into(),
        }),
    }
}

/// A request whose history read asset `01JPIC` through tool call `call`.
fn request(provider: Kind, model: &str, call: &str, parts: Vec<Part>) -> Request {
    let _ = provider;
    let mut req = Request::new(model, "");
    req.messages = vec![
        Message::user("what is in the picture?"),
        Message {
            role: Role::Assistant,
            content: vec![Content::ToolCall {
                id: call.into(),
                name: "read_asset".into(),
                input: json!({"id": "01JPIC"}),
            }],
            provider_state: None,
        },
        Message::tool_results(vec![Content::ToolResult {
            id: call.into(),
            content: "{\"id\":\"01JPIC\"}".into(),
            is_error: false,
            parts,
        }]),
    ];
    req
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_part_becomes_an_image_block_in_the_anthropic_tool_result() {
    let env = Env::new();
    let req = request(
        Kind::Anthropic,
        "claude-opus-5",
        "toolu_1",
        vec![loaded("01JPIC"), Part::image("01JGONE", "image/png")],
    );
    ai::complete(&ai::client(), &env.cred(Kind::Anthropic), &req)
        .await
        .unwrap();
    let body = env.anthropic.message_requests()[0].json();
    assert_eq!(
        body["messages"][2]["content"][0]["content"],
        json!([
            {"type": "text", "text": "{\"id\":\"01JPIC\"}"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": PIXEL_PNG}},
            {"type": "text", "text": "[image asset 01JGONE was not loaded]"},
        ])
    );
    let images = env.anthropic.images();
    assert_eq!(images[0].len(), 1);
    assert_eq!(images[0][0].tool_call_id, "toolu_1");
    assert_eq!(&images[0][0].bytes[1..4], b"PNG");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_part_becomes_an_input_image_in_the_function_call_output() {
    let env = Env::new();
    let req = request(Kind::OpenAi, "gpt-6", "call_1", vec![loaded("01JPIC")]);
    ai::complete(&ai::client(), &env.cred(Kind::OpenAi), &req)
        .await
        .unwrap();
    let input = env.openai.response_requests()[0].json()["input"].clone();
    assert_eq!(
        input[2],
        json!({"type": "function_call_output", "call_id": "call_1", "output": [
            {"type": "input_text", "text": "{\"id\":\"01JPIC\"}"},
            {"type": "input_image", "image_url": format!("data:image/png;base64,{PIXEL_PNG}")},
        ]})
    );
    // The image rides in the tool output: no extra user message.
    assert_eq!(input.as_array().unwrap().len(), 3);
    let images = env.openai.images();
    assert_eq!(images[0].len(), 1);
    assert_eq!(images[0][0].tool_call_id, "call_1");
    assert_eq!(images[0][0].media_type, "image/png");
}

#[test]
fn a_transcript_without_parts_still_deserializes_and_parts_never_carry_bytes() {
    let old: Message = serde_json::from_value(json!({
        "role": "user",
        "content": [{"type": "tool_result", "id": "t1", "content": "x", "is_error": false}],
    }))
    .unwrap();
    assert_eq!(old.content, vec![Content::tool_result("t1", "x", false)]);
    // A result with no parts serializes as before.
    let v = serde_json::to_value(&old).unwrap();
    assert!(v["content"][0].get("parts").is_none(), "{v}");

    let with = Content::ToolResult {
        id: "t2".into(),
        content: "y".into(),
        is_error: false,
        parts: vec![loaded("01JPIC")],
    };
    let v = serde_json::to_value(&with).unwrap();
    assert_eq!(
        v["parts"],
        json!([{"type": "image", "asset_id": "01JPIC", "media_type": "image/png"}])
    );
    assert!(!v.to_string().contains(PIXEL_PNG));
    let back: Content = serde_json::from_value(v).unwrap();
    assert_eq!(
        back,
        Content::ToolResult {
            id: "t2".into(),
            content: "y".into(),
            is_error: false,
            parts: vec![Part::image("01JPIC", "image/png")],
        }
    );
}
