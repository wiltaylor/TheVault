//! The `ai` module against the testkit's fake providers (ai D11, D12, D13).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ai::{Endpoints, Models, Provider, Settings};
use script::{Ai, Error, Host, Job, Kind, Limits, Outcome, Vault};
use serde_json::{Value, json};
use state::{SecretKey, State};
use tempfile::TempDir;
use testkit::{AnthropicMessage, AnthropicReply, FakeAnthropic, FakeOpenAi, OpenAiResponse, Reply};
use writer::Writer;

struct Fixture {
    _dir: TempDir,
    writer: Arc<Writer>,
    host: Host,
    anthropic: FakeAnthropic,
    openai: FakeOpenAi,
}

/// A vault whose `ai_settings` point at the fakes, with Anthropic's `claude-test` the default
/// and both API keys stored.
fn fixture() -> Fixture {
    let f = bare();
    Settings {
        default_provider: Some(Provider::Anthropic),
        models: Models {
            anthropic: Some("claude-test".into()),
            openai: Some("gpt-test".into()),
            chatgpt: None,
        },
        endpoints: Endpoints {
            anthropic: Some(f.anthropic.base_url()),
            openai: Some(f.openai.base_url()),
            chatgpt_auth: None,
        },
        ..Settings::default()
    }
    .save(f.writer.state())
    .unwrap();
    let state = f.writer.state();
    state
        .set_secret(ai::ANTHROPIC_API_KEY, f.anthropic.api_key())
        .unwrap();
    state
        .set_secret(ai::OPENAI_API_KEY, f.openai.api_key())
        .unwrap();
    f
}

/// A vault with no AI settings or keys at all.
fn bare() -> Fixture {
    with_ai(Ai::of)
}

fn with_ai(ai: impl FnOnce(&Arc<Writer>) -> Ai) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    let state = State::open(
        &vault.server_dir().join("state.db"),
        Some(SecretKey::generate()),
    )
    .unwrap();
    let writer = Arc::new(Writer::open(vault, state).unwrap());
    let host = Host::new(
        writer.clone() as Arc<dyn Vault>,
        2,
        Limits::default(),
        ai(&writer),
    );
    Fixture {
        _dir: dir,
        writer,
        host,
        anthropic: FakeAnthropic::start(),
        openai: FakeOpenAi::start(),
    }
}

impl Fixture {
    fn run(&self, kind: Kind, source: &str, timeout: Option<Duration>) -> Outcome {
        let mut j = Job::new(kind, "summarise", source, "run");
        j.timeout = timeout;
        self.host.run(j)
    }

    fn task(&self, source: &str) -> Outcome {
        self.run(Kind::Task, source, None)
    }
}

/// The run's value, which must have succeeded.
fn ok(out: &Outcome) -> Value {
    match &out.result {
        Ok(v) => v.clone(),
        Err(e) => panic!("the script failed: {e}\nlog: {}", out.log),
    }
}

/// The run log's trail lines, which must be its only lines.
fn trail(out: &Outcome) -> Vec<String> {
    let lines: Vec<String> = out.log.lines().map(str::to_owned).collect();
    for l in &lines {
        assert!(l.starts_with("ai: provider="), "not a trail line: {l}");
    }
    lines
}

/// A script returning the `AiError` of a call that must fail.
fn failing(call: &str) -> String {
    format!(
        "use ai\nfn run() -> Result[AiError, string] {{\n    match {call} {{\n        Ok(r) => Err(\"answered: \" + r.text),\n        Err(e) => Ok(e),\n    }}\n}}\n"
    )
}

#[test]
fn complete_returns_the_text_and_token_counts_and_leaves_one_trail_line() {
    let f = fixture();
    f.anthropic
        .push(AnthropicMessage::text("The roof needs new tiles.").usage(42, 7));
    let out = f.task(
        "use ai\nfn run() -> Result[Reply, AiError] {\n    ai::complete(\"Summarise: the secret roof plan\", #{})\n}\n",
    );
    let reply = ok(&out);
    assert_eq!(reply["text"], json!("The roof needs new tiles."));
    assert_eq!(reply["provider"], json!("anthropic"));
    assert_eq!(reply["model"], json!("claude-test"));
    assert_eq!(reply["input_tokens"], json!(42));
    assert_eq!(reply["output_tokens"], json!(7));
    assert_eq!(reply["stop"], json!("end_turn"));
    assert_eq!(reply["json"], Value::Null);

    let lines = trail(&out);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("ai: provider=anthropic model=claude-test in=42 out=7 outcome=ok ms="),
        "{}",
        lines[0]
    );
    assert!(!out.log.contains("roof"), "{}", out.log);

    let sent = f.anthropic.message_requests();
    assert_eq!(sent.len(), 1);
    let body = sent[0].json();
    assert_eq!(body["model"], json!("claude-test"));
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        json!("Summarise: the secret roof plan")
    );
    // The script named no secret and no host: the module used the stored key itself.
    assert_eq!(sent[0].header("x-api-key"), Some(f.anthropic.api_key()));
}

#[test]
fn with_a_schema_the_reply_json_is_the_parsed_object() {
    let f = fixture();
    f.anthropic
        .push(AnthropicMessage::text(r#"{"title":"Roof","urgent":true}"#));
    let schema = json!({
        "type": "object",
        "properties": { "title": { "type": "string" }, "urgent": { "type": "boolean" } },
        "required": ["title", "urgent"],
        "additionalProperties": false,
    });
    let src = format!(
        "use ai\nfn run() -> Result[Value, string] {{\n    match ai::complete(\"Triage the note\", #{{ \"schema\": {schema} }}) {{\n        Ok(r) => Ok(r.json),\n        Err(e) => Err(e.kind + \": \" + e.message),\n    }}\n}}\n",
        // A wscript string writes a literal brace twice: one alone opens an interpolation.
        schema = serde_json::to_string(&schema.to_string())
            .unwrap()
            .replace('{', "{{")
            .replace('}', "}}")
    );
    let out = f.task(&src);
    assert_eq!(ok(&out), json!({ "title": "Roof", "urgent": true }));
    let body = f.anthropic.message_requests()[0].json();
    assert_eq!(
        body["output_config"]["format"],
        json!({ "type": "json_schema", "schema": schema })
    );
    assert_eq!(trail(&out).len(), 1);
}

#[test]
fn chat_sends_the_history_in_order() {
    let f = fixture();
    f.anthropic.push(AnthropicMessage::text("Blue."));
    let out = f.task(
        r#"use ai
fn run() -> Result[string, string] {
    let history = [
        ChatMessage { role: "user", text: "Pick a colour." },
        ChatMessage { role: "assistant", text: "Red or blue?" },
        ChatMessage { role: "user", text: "The second one." },
    ]
    match ai::chat(history, #{ "system": "Answer in one word." }) {
        Ok(r) => Ok(r.text),
        Err(e) => Err(e.message),
    }
}
"#,
    );
    assert_eq!(ok(&out), json!("Blue."));
    let body = f.anthropic.message_requests()[0].json();
    assert_eq!(body["system"], json!("Answer in one word."));
    let turns: Vec<(Value, Value)> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["role"].clone(), m["content"][0]["text"].clone()))
        .collect();
    assert_eq!(
        turns,
        vec![
            (json!("user"), json!("Pick a colour.")),
            (json!("assistant"), json!("Red or blue?")),
            (json!("user"), json!("The second one.")),
        ]
    );
    let lines = trail(&out);
    assert_eq!(lines.len(), 1);
    assert!(!out.log.contains("colour"), "{}", out.log);
}

#[test]
fn a_chat_message_with_an_unknown_role_is_invalid() {
    let f = fixture();
    let out = f.task(&failing(
        "ai::chat([ChatMessage { role: \"system\", text: \"x\" }], #{})",
    ));
    let e = ok(&out);
    assert_eq!(e["kind"], json!("invalid"), "{e}");
    assert!(f.anthropic.message_requests().is_empty());
    assert!(trail(&out)[0].contains("outcome=error:invalid"));
}

#[test]
fn options_override_the_settings_defaults() {
    let f = fixture();
    f.openai.push(OpenAiResponse::text("Done.").usage(10, 3));
    f.anthropic.push(AnthropicMessage::text("Fine."));
    let out = f.task(
        r#"use ai
fn run() -> Result[List[Reply], AiError] {
    let a = ai::complete("one", #{
        "provider": "openai",
        "model": "gpt-other",
        "system": "Be brief.",
        "effort": "high",
        "max_tokens": "300",
    })?
    let b = ai::complete("two", #{ "model": "claude-other", "max_tokens": "200", "effort": "low" })?
    Ok([a, b])
}
"#,
    );
    let replies = ok(&out);
    assert_eq!(replies[0]["provider"], json!("openai"));
    assert_eq!(replies[0]["model"], json!("gpt-other"));
    assert_eq!(replies[1]["provider"], json!("anthropic"));
    assert_eq!(replies[1]["model"], json!("claude-other"));

    let o = f.openai.response_requests()[0].json();
    assert_eq!(o["model"], json!("gpt-other"));
    assert_eq!(o["instructions"], json!("Be brief."));
    assert_eq!(o["reasoning"]["effort"], json!("high"));
    assert_eq!(o["max_output_tokens"], json!(300));
    let a = f.anthropic.message_requests()[0].json();
    assert_eq!(a["model"], json!("claude-other"));
    assert_eq!(a["max_tokens"], json!(200));
    assert_eq!(a["output_config"]["effort"], json!("low"));

    let lines = trail(&out);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("ai: provider=openai model=gpt-other in=10 out=3 outcome=ok"));
    assert!(lines[1].starts_with("ai: provider=anthropic model=claude-other "));
}

#[test]
fn a_bad_option_is_invalid_and_never_reaches_the_provider() {
    let f = fixture();
    for (options, wants) in [
        (r#"#{ "temperature": "0.2" }"#, "unknown option temperature"),
        (r#"#{ "max_tokens": "lots" }"#, "max_tokens"),
        (r#"#{ "effort": "extreme" }"#, "effort"),
        (r#"#{ "schema": "not json" }"#, "schema"),
        (r#"#{ "provider": "gemini" }"#, "gemini"),
    ] {
        let out = f.task(&failing(&format!("ai::complete(\"x\", {options})")));
        let e = ok(&out);
        assert_eq!(e["kind"], json!("invalid"), "{options}: {e}");
        assert!(
            e["message"].as_str().unwrap().contains(wants),
            "{options}: {e}"
        );
        assert_eq!(trail(&out).len(), 1);
    }
    assert!(f.anthropic.message_requests().is_empty());
}

#[test]
fn a_refusal_is_an_err_with_its_kind_and_category() {
    let f = fixture();
    f.anthropic.push(
        AnthropicMessage::new()
            .refusal(Some("cyber"), Some("Not this one."))
            .usage(20, 0),
    );
    let out = f.task(&failing("ai::complete(\"Write malware\", #{})"));
    let e = ok(&out);
    assert_eq!(e["kind"], json!("refusal"));
    assert_eq!(e["category"], json!("cyber"));
    assert_eq!(e["provider"], json!("anthropic"));
    assert_eq!(e["model"], json!("claude-test"));
    assert!(e["message"].as_str().unwrap().contains("refused"), "{e}");
    let lines = trail(&out);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with(
            "ai: provider=anthropic model=claude-test in=20 out=0 outcome=refused ms="
        ),
        "{}",
        lines[0]
    );
    assert!(!out.log.contains("malware"));
}

#[test]
fn an_entry_function_returning_an_ai_error_fails_with_its_message() {
    let f = fixture();
    f.anthropic
        .push(AnthropicMessage::new().refusal(Some("cyber"), None));
    let out = f.task(
        "use ai\nfn run() -> Result[string, AiError] {\n    Ok(ai::complete(\"x\", #{})?.text)\n}\n",
    );
    match out.result {
        Err(Error::Returned { message, .. }) => {
            assert!(message.contains("refused the request"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn with_nothing_configured_a_call_is_not_configured() {
    let f = bare();
    let out = f.task(&failing("ai::complete(\"x\", #{})"));
    let e = ok(&out);
    assert_eq!(e["kind"], json!("not_configured"));
    assert_eq!(e["provider"], json!(""));
    assert!(
        e["message"].as_str().unwrap().contains("Settings > AI"),
        "{e}"
    );
    let lines = trail(&out);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("ai: provider=- model=- in=0 out=0 outcome=error:not_configured ms="),
        "{}",
        lines[0]
    );

    // A provider and model named, but no key stored for it.
    let out = f.task(&failing(
        "ai::complete(\"x\", #{ \"provider\": \"openai\", \"model\": \"gpt-test\" })",
    ));
    let e = ok(&out);
    assert_eq!(e["kind"], json!("not_configured"));
    assert_eq!(e["provider"], json!("openai"));
    assert_eq!(e["model"], json!("gpt-test"));
    assert!(trail(&out)[0].starts_with("ai: provider=openai model=gpt-test "));
    assert!(f.openai.response_requests().is_empty());
}

#[test]
fn a_call_outlasting_the_run_deadline_ends_the_run() {
    let f = fixture();
    f.anthropic.push(AnthropicReply::Http(
        Reply::json(200, &json!({})).delay(Duration::from_secs(30)),
    ));
    let src = "use ai\nfn run() -> string {\n    match ai::complete(\"x\", #{}) {\n        Ok(r) => r.text,\n        Err(e) => \"caught: \" + e.kind,\n    }\n}\n";
    let started = Instant::now();
    let out = f.run(Kind::Task, src, Some(Duration::from_millis(700)));
    // The timeout ends the run: the script cannot catch it as an `Err` and carry on.
    assert_eq!(
        out.result.clone().unwrap_err(),
        Error::Timeout {
            script: "summarise".into(),
            limit: Duration::from_millis(700)
        }
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    let lines = trail(&out);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("outcome=error:timeout"), "{}", lines[0]);
}

#[test]
fn ai_does_not_compile_in_a_before_handler_or_a_migration() {
    let f = fixture();
    let src = "use ai\nfn run() -> Result[string, string] {\n    match ai::complete(\"x\", #{}) {\n        Ok(r) => Ok(r.text),\n        Err(e) => Err(e.message),\n    }\n}\n";
    for kind in [Kind::Before, Kind::Migration] {
        assert!(
            matches!(f.host.check(kind, "s", src), Err(Error::Compile { .. })),
            "{kind:?}"
        );
    }
    f.host.check(Kind::After, "s", src).unwrap();
    f.host.check(Kind::Task, "s", src).unwrap();
}

#[test]
fn an_after_handler_reaches_ai() {
    let f = fixture();
    f.anthropic.push(AnthropicMessage::text("Tagged."));
    let out = f.run(
        Kind::After,
        "use ai\nfn run() -> Result[string, string] {\n    match ai::complete(\"x\", #{}) {\n        Ok(r) => Ok(r.text),\n        Err(e) => Err(e.message),\n    }\n}\n",
        None,
    );
    assert_eq!(ok(&out), json!("Tagged."));
    assert_eq!(trail(&out).len(), 1);
}

#[test]
fn on_a_device_every_call_is_unavailable() {
    let f = with_ai(|_| Ai::unavailable());
    for call in [
        "ai::complete(\"x\", #{})",
        "ai::chat([ChatMessage { role: \"user\", text: \"x\" }], #{ \"provider\": \"anthropic\" })",
    ] {
        let out = f.task(&failing(call));
        let e = ok(&out);
        assert_eq!(e["kind"], json!("unavailable"), "{call}: {e}");
        assert!(
            trail(&out)[0].contains("outcome=error:unavailable"),
            "{}",
            out.log
        );
    }
}

#[test]
fn the_docs_example_compiles_for_a_task() {
    let docs = include_str!("../../../docs/scripts.md");
    let section = &docs[docs.find("## `ai`").expect("an ai section")..];
    let start = section.find("```rust\n").expect("an example") + "```rust\n".len();
    let example = &section[start..start + section[start..].find("```").unwrap()];
    let f = fixture();
    f.host.check(Kind::Task, "example", example).unwrap();
}
