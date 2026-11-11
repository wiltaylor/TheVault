//! Pure Login-metadata redaction, shared by API clients and agent script reads.
use serde_json::{Value, json};

/// A redacted copy of a note. Stored notes remain unchanged.
pub fn redact_agent_note(type_name: &str, text: &str) -> Result<String, String> {
    if type_name != "agent_thread" && !text.contains("transcript") {
        return Ok(text.to_owned());
    }
    let mut doc = crate::Input::Wcl(text.to_owned())
        .parse("private-note.wcl")
        .map_err(|_| "The private transcript could not be redacted.".to_owned())?;
    let mut changed = false;
    for (name, field) in &mut doc.fields {
        if name == "transcript" {
            let mut value: Value = serde_json::from_str(field.as_str().unwrap_or("{}"))
                .map_err(|_| "The private transcript could not be redacted.".to_owned())?;
            redact_agent_content(&mut value)?;
            *field = schema::Value::Str(value.to_string());
            changed = true;
        }
    }
    Ok(if changed {
        crate::print_doc(&doc)
    } else {
        text.to_owned()
    })
}

/// Covers full notes, selected query fields, conflict text and encoded tool results.
/// The caller boundary decides whether redaction is needed before calling this.
pub fn redact_agent_content(value: &mut Value) -> Result<(), String> {
    if value["kind"] == "agent" || value["name"] == "propose_agent_change" {
        hide_browser_secrets(value);
    }
    match value {
        Value::Object(fields) => {
            let type_name = fields
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            for (key, child) in fields {
                if matches!(key.as_str(), "wcl" | "current")
                    && let Some(text) = child.as_str()
                {
                    *child = Value::String(redact_agent_note(&type_name, text)?);
                } else if key == "transcript"
                    && let Some(text) = child.as_str()
                {
                    let mut transcript: Value = serde_json::from_str(text)
                        .map_err(|_| "The private transcript could not be redacted.".to_owned())?;
                    redact_agent_content(&mut transcript)?;
                    *child = Value::String(transcript.to_string());
                } else {
                    redact_agent_content(child)?;
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
                    || text.contains("\"before\"")) =>
        {
            if let Ok(mut decoded) = serde_json::from_str::<Value>(text) {
                redact_agent_content(&mut decoded)?;
                *text = decoded.to_string();
            } else {
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
                value[field] = match schema::Agent::parse(&name, source) {
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
        let text = match schema::Agent::parse(name, source) {
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
