//! A JSON Schema check for tool inputs and schema-constrained replies (ai D5). Eager input
//! streaming means the provider no longer checks a streamed tool input, so every one is checked
//! here before the tool runs.
//!
//! It covers the keywords the providers' strict modes accept and the common constraints:
//! `type` (one or a list), `enum`, `const`, `properties`, `required`, `additionalProperties`
//! (`false` or a schema), `items`, `minItems`, `maxItems`, `minLength`, `maxLength`, `minimum`,
//! `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `anyOf`, `oneOf`, `allOf`, `not`, and
//! `$ref` to `#`, `#/$defs/<name>` or `#/definitions/<name>`. Other keywords, `format` among
//! them, are not checked. `true` accepts anything and `false` nothing.

use serde_json::{Map, Value};

/// Check `value` against `schema`. The error names the first place that fails, as a JSON
/// pointer, and why.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, schema, value, "", 0)
}

/// How deep `$ref`s may nest before the schema counts as recursive without end.
const MAX_DEPTH: usize = 64;

fn check(
    root: &Value,
    schema: &Value,
    value: &Value,
    at: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: the schema nests too deep", place(at)));
    }
    let s = match schema {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return Err(format!("{}: no value is allowed here", place(at))),
        Value::Object(s) => s,
        _ => return Ok(()),
    };
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let target = resolve(root, r)
            .ok_or_else(|| format!("{}: the schema's $ref {r:?} names nothing", place(at)))?;
        check(root, target, value, at, depth + 1)?;
    }
    if let Some(t) = s.get("type") {
        let allowed: Vec<&str> = match t {
            Value::String(one) => vec![one.as_str()],
            Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.iter().any(|t| is_type(value, t)) {
            return Err(format!(
                "{}: expected {}, got {}",
                place(at),
                allowed.join(" or "),
                type_name(value)
            ));
        }
    }
    if let Some(options) = s.get("enum").and_then(Value::as_array)
        && !options.iter().any(|o| same(o, value))
    {
        return Err(format!(
            "{}: {} is not one of {}",
            place(at),
            short(value),
            Value::Array(options.clone())
        ));
    }
    if let Some(c) = s.get("const")
        && !same(c, value)
    {
        return Err(format!("{}: expected {c}, got {}", place(at), short(value)));
    }
    match value {
        Value::Object(obj) => check_object(root, s, obj, at, depth)?,
        Value::Array(items) => {
            if let Some(item) = s.get("items") {
                for (i, v) in items.iter().enumerate() {
                    check(root, item, v, &format!("{at}/{i}"), depth + 1)?;
                }
            }
            bound(
                s,
                "minItems",
                items.len(),
                |n, b| n >= b,
                "at least",
                "items",
                at,
            )?;
            bound(
                s,
                "maxItems",
                items.len(),
                |n, b| n <= b,
                "at most",
                "items",
                at,
            )?;
        }
        Value::String(text) => {
            let n = text.chars().count();
            bound(
                s,
                "minLength",
                n,
                |n, b| n >= b,
                "at least",
                "characters",
                at,
            )?;
            bound(
                s,
                "maxLength",
                n,
                |n, b| n <= b,
                "at most",
                "characters",
                at,
            )?;
        }
        Value::Number(n) => {
            let n = n.as_f64().unwrap_or(0.0);
            let limit = |k: &str| s.get(k).and_then(Value::as_f64);
            let fail = |what: &str, b: f64| Err(format!("{}: {n} is not {what} {b}", place(at)));
            if let Some(b) = limit("minimum")
                && n < b
            {
                return fail("at least", b);
            }
            if let Some(b) = limit("maximum")
                && n > b
            {
                return fail("at most", b);
            }
            if let Some(b) = limit("exclusiveMinimum")
                && n <= b
            {
                return fail("above", b);
            }
            if let Some(b) = limit("exclusiveMaximum")
                && n >= b
            {
                return fail("below", b);
            }
        }
        _ => {}
    }
    if let Some(all) = s.get("allOf").and_then(Value::as_array) {
        for sub in all {
            check(root, sub, value, at, depth + 1)?;
        }
    }
    if let Some(any) = s.get("anyOf").and_then(Value::as_array) {
        let mut first = None;
        let matched = any
            .iter()
            .any(|sub| match check(root, sub, value, at, depth + 1) {
                Ok(()) => true,
                Err(e) => {
                    first.get_or_insert(e);
                    false
                }
            });
        if !matched {
            return Err(first.unwrap_or_else(|| format!("{}: matches no anyOf", place(at))));
        }
    }
    if let Some(one) = s.get("oneOf").and_then(Value::as_array) {
        let n = one
            .iter()
            .filter(|sub| check(root, sub, value, at, depth + 1).is_ok())
            .count();
        if n != 1 {
            return Err(format!(
                "{}: matches {n} of the oneOf schemas, not exactly one",
                place(at)
            ));
        }
    }
    if let Some(not) = s.get("not")
        && check(root, not, value, at, depth + 1).is_ok()
    {
        return Err(format!("{}: matches a schema it must not", place(at)));
    }
    Ok(())
}

fn check_object(
    root: &Value,
    s: &Map<String, Value>,
    obj: &Map<String, Value>,
    at: &str,
    depth: usize,
) -> Result<(), String> {
    if let Some(required) = s.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(name) {
                return Err(format!("{}: {name:?} is required", place(at)));
            }
        }
    }
    let props = s.get("properties").and_then(Value::as_object);
    for (name, v) in obj {
        let here = format!("{at}/{}", name.replace('~', "~0").replace('/', "~1"));
        match props.and_then(|p| p.get(name)) {
            Some(sub) => check(root, sub, v, &here, depth + 1)?,
            None => match s.get("additionalProperties") {
                Some(Value::Bool(false)) => {
                    return Err(format!("{}: {name:?} is not allowed", place(at)));
                }
                Some(extra @ Value::Object(_)) => check(root, extra, v, &here, depth + 1)?,
                _ => {}
            },
        }
    }
    Ok(())
}

fn bound(
    s: &Map<String, Value>,
    key: &str,
    n: usize,
    ok: impl Fn(usize, usize) -> bool,
    what: &str,
    unit: &str,
    at: &str,
) -> Result<(), String> {
    match s.get(key).and_then(Value::as_u64) {
        Some(b) if !ok(n, b as usize) => Err(format!(
            "{}: has {n} {unit}, expected {what} {b}",
            place(at)
        )),
        _ => Ok(()),
    }
}

fn resolve<'a>(root: &'a Value, r: &str) -> Option<&'a Value> {
    if r == "#" {
        return Some(root);
    }
    root.pointer(r.strip_prefix('#')?)
}

fn is_type(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        "integer" => {
            v.as_i64().is_some()
                || v.as_u64().is_some()
                || v.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        _ => true,
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Equal as JSON values, with `1` and `1.0` the same number.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn short(v: &Value) -> String {
    let text = v.to_string();
    if text.chars().count() > 60 {
        format!("{}...", text.chars().take(60).collect::<String>())
    } else {
        text
    }
}

fn place(at: &str) -> String {
    if at.is_empty() {
        "the value".to_owned()
    } else {
        at.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["text", "limit"],
            "properties": {
                "text": {"type": "string", "minLength": 1},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20},
                "tags": {"type": "array", "items": {"$ref": "#/$defs/tag"}},
                "mode": {"enum": ["a", "b"]},
                "maybe": {"anyOf": [{"type": "string"}, {"type": "null"}]}
            },
            "$defs": {"tag": {"type": "string", "maxLength": 3}}
        })
    }

    #[test]
    fn accepts_a_matching_value() {
        let v = json!({"text": "roof", "limit": 5, "tags": ["a"], "mode": "b", "maybe": null});
        assert_eq!(validate(&schema(), &v), Ok(()));
    }

    #[test]
    fn names_the_failing_place() {
        let s = schema();
        let e = |v: Value| validate(&s, &v).unwrap_err();
        assert!(e(json!({"text": "x"})).contains("\"limit\" is required"));
        assert!(e(json!({"text": "x", "limit": 50})).contains("/limit"));
        assert!(e(json!({"text": "x", "limit": 1.5})).contains("expected integer"));
        assert!(
            e(json!({"text": "x", "limit": 1, "other": 1})).contains("\"other\" is not allowed")
        );
        assert!(e(json!({"text": "x", "limit": 1, "tags": ["long"]})).contains("/tags/0"));
        assert!(e(json!({"text": "x", "limit": 1, "mode": "c"})).contains("not one of"));
        assert!(e(json!({"text": "x", "limit": 1, "maybe": 3})).contains("/maybe"));
        assert!(e(json!("text")).contains("expected object"));
    }
}
