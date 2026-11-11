//! Pure Login-metadata redaction, shared by indexing, API clients and agent script reads.
use serde_json::{Value, json};


fn print_value(value: &crate::Value) -> String {
    match value {
        crate::Value::Str(text) => serde_json::to_string(text).expect("string serialization"),
        crate::Value::Int(n) => n.to_string(),
        crate::Value::Float(n) => format!("{n:?}"),
        crate::Value::Bool(value) => value.to_string(),
        crate::Value::List(values) => format!("[{}]", values.iter().map(print_value).collect::<Vec<_>>().join(", ")),
        crate::Value::Record(values) => format!("{{ {} }}", values.iter().map(|(key,value)| format!("{key}: {}",print_value(value))).collect::<Vec<_>>().join(", ")),
    }
}

fn print_doc(doc: &crate::Doc) -> String {
    let mut out = String::new();
    for (name,value) in &doc.fields { out.push_str(&format!("{name} = {}\n", print_value(value))); }
    for block in &doc.blocks {
        out.push_str(&block.kind);
        for label in &block.labels { out.push(' '); out.push_str(&serde_json::to_string(label).expect("label serialization")); }
        out.push_str(" {\n"); out.push_str(&print_doc(&block.body)); out.push_str("}\n");
    }
    out
}

/// The copy indexed for filtering and full-text search. The stored note stays raw.
pub fn redacted_agent_doc(type_name: &str, doc: &crate::Doc) -> crate::Doc {
    let mut copy = doc.clone();
    if type_name == "agent_thread" {
        for (name,value) in &mut copy.fields {
            if name == "transcript" {
                let mut transcript = serde_json::from_str::<Value>(value.as_str().unwrap_or("{}")).unwrap_or_else(|_| json!({}));
                if redact_agent_content(&mut transcript).is_err() { transcript = json!({}); }
                *value = crate::Value::Str(transcript.to_string());
            }
        }
    }
    copy
}

fn private_transcript(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .is_ok_and(|value| value.get("messages").is_some() && value.get("proposals").is_some())
}

/// A redacted copy of a note. Stored notes remain unchanged.
pub fn redact_agent_note(type_name: &str, text: &str) -> Result<String, String> {
    if (!type_name.is_empty() && type_name != "agent_thread") || !text.contains("transcript") {
        return Ok(text.to_owned());
    }
    let mut doc = crate::Doc::parse(text, "private-note.wcl")
        .map_err(|_| "The private transcript could not be redacted.".to_owned())?;
    let mut changed = false;
    for (name, field) in &mut doc.fields {
        if name != "transcript" {
            continue;
        }
        let text = field.as_str().unwrap_or("{}");
        if type_name != "agent_thread" && !private_transcript(text) {
            continue;
        }
        let mut value: Value = serde_json::from_str(text)
            .map_err(|_| "The private transcript could not be redacted.".to_owned())?;
        let before = value.clone();
        redact_agent_content(&mut value)?;
        if value != before {
            *field = crate::Value::Str(value.to_string());
            changed = true;
        }
    }
    Ok(if changed {
        print_doc(&doc)
    } else {
        text.to_owned()
    })
}

/// Covers full notes, selected query fields, conflict text and encoded tool results.
/// The caller boundary decides whether redaction is needed before calling this.
pub fn redact_agent_content(value: &mut Value) -> Result<(), String> {
    redact_content_in(value, None)
}

fn redact_content_in(value: &mut Value, inherited_type: Option<&str>) -> Result<(), String> {
    if value["kind"] == "agent" || value["name"] == "propose_agent_change" {
        hide_browser_secrets(value);
    }
    match value {
        Value::Object(fields) => {
            // get_agent returns the same definition twice: parsed JSON and WCL text.
            if fields.contains_key("agent")
                && fields.contains_key("format")
                && let Some(source) = fields.get("wcl").and_then(Value::as_str)
            {
                let name = fields
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("hidden");
                let source = match crate::Agent::parse(name, source) {
                    Ok(mut agent) => {
                        agent.permissions.browser_secrets.clear();
                        Value::String(agent.to_wcl())
                    }
                    Err(_) => Value::Null,
                };
                fields.insert("wcl".to_owned(), source);
            }
            let type_name = fields
                .get("type")
                .and_then(Value::as_str)
                .or(inherited_type)
                .map(str::to_owned);
            for (key, child) in fields {
                if key == "browser_secrets" && child.is_array() {
                    *child = json!([]);
                } else if matches!(key.as_str(), "wcl" | "current")
                    && let Some(text) = child.as_str()
                {
                    *child = Value::String(redact_agent_note(
                        type_name.as_deref().unwrap_or_default(),
                        text,
                    )?);
                } else if key == "transcript"
                    && let Some(text) = child.as_str()
                    && (type_name.as_deref() == Some("agent_thread")
                        || (type_name.is_none() && private_transcript(text)))
                {
                    let mut transcript: Value = serde_json::from_str(text)
                        .map_err(|_| "The private transcript could not be redacted.".to_owned())?;
                    let before = transcript.clone();
                    redact_agent_content(&mut transcript)?;
                    if transcript != before {
                        *child = Value::String(transcript.to_string());
                    }
                } else {
                    let context = if matches!(key.as_str(), "note" | "fields") {
                        type_name.as_deref()
                    } else {
                        None
                    };
                    redact_content_in(child, context)?;
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                redact_agent_content(child)?;
            }
        }
        Value::String(text)
            if text.trim_start().starts_with('{')
                && (text.contains("\"proposals\"")
                    || text.contains("\"transcript\"")
                    || text.contains("\"before\"")
                    || text.contains("\"browser_secrets\"")) =>
        {
            if let Ok(mut decoded) = serde_json::from_str::<Value>(text) {
                let before = decoded.clone();
                redact_agent_content(&mut decoded)?;
                if decoded != before {
                    *text = decoded.to_string();
                }
            } else if text.contains("browser_secrets") {
                *text = "The private transcript could not be redacted.".to_owned();
            }
        }
        _ => {}
    }
    Ok(())
}

/// Remove Login lists from structured agent proposals and proposal tool inputs.
pub fn hide_browser_secrets(value: &mut Value) {
    if value["kind"] == "agent" && (value.get("before").is_some() || value.get("after").is_some()) {
        let name = value["name"].as_str().unwrap_or("hidden").to_owned();
        for field in ["before", "after"] {
            if let Some(source) = value[field].as_str() {
                value[field] = match crate::Agent::parse(&name, source) {
                    Ok(mut agent) => {
                        agent.permissions.browser_secrets.clear();
                        json!(agent.to_wcl())
                    }
                    Err(_) => Value::Null,
                };
            }
        }
        value["browser_secrets_hidden"] = json!(true);
    }
    // A proposal's after text also occurs in the tool-call input in a transcript/event.
    if value["name"] == "propose_agent_change"
        && let Some(source) = value["input"]["wcl"].as_str()
    {
        let name = value["input"]["name"].as_str().unwrap_or("hidden");
        let text = match crate::Agent::parse(name, source) {
            Ok(mut agent) => {
                agent.permissions.browser_secrets.clear();
                json!(agent.to_wcl())
            }
            Err(_) => Value::Null,
        };
        value["input"]["wcl"] = text;
        value["browser_secrets_hidden"] = json!(true);
    }
    match value {
        Value::Object(fields) => {
            for child in fields.values_mut() {
                hide_browser_secrets(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                hide_browser_secrets(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_secrets_get_agent_results_do_not_escape_through_encoded_tool_content() {
        let source = "permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n";
        let result = json!({"name":"helper","wcl":source,"agent":{"name":"helper","permissions":{"browser":true,"browser_secrets":["bank"]}},"format":"agent file vocabulary"});
        let mut transcript = json!({"messages":[{"content":[{"type":"tool_result","content":result.to_string()}]}],"proposals":[]});
        redact_agent_content(&mut transcript).unwrap();
        assert!(!transcript.to_string().contains("bank"));
    }

    #[test]
    fn browser_secrets_redaction_preserves_unrelated_transcript_fields() {
        let source = "title = \"Meeting\"\ntranscript = \"ordinary text\"\n";
        assert_eq!(redact_agent_note("memo", source).unwrap(), source);
        assert_eq!(redact_agent_note("", source).unwrap(), source);
        let mut value = json!({"type":"memo","wcl":source,"note":{"fields":{"transcript":"ordinary text", "example":"{\"before\": incomplete"}},"rows":[{"type":"memo","fields":{"transcript":"ordinary text"}}]});
        let before = value.clone();
        redact_agent_content(&mut value).unwrap();
        assert_eq!(value, before);
        let mut invalid_private =
            json!({"type":"agent_thread","note":{"fields":{"transcript":"invalid private JSON"}}});
        assert!(redact_agent_content(&mut invalid_private).is_err());
    }
}
