//! Values across the host boundary: JSON in, the script's dynamic `Value` (`DynValue`) between,
//! and whatever the entry function returns back out as JSON.

use serde_json::{Number, Value as Json};
use wscript::core::defs::{DEF_OPTION, DEF_RESULT, DefTable, TAG_ERR, TAG_OK, TAG_SOME};
use wscript::std_modules::DynValue;
use wscript::{FromValue, Value};

pub fn to_dyn(v: Json) -> DynValue {
    match v {
        Json::Null => DynValue::Null,
        Json::Bool(b) => DynValue::Bool(b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => DynValue::Int(i),
            None => DynValue::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(s) => DynValue::String(s),
        Json::Array(items) => DynValue::List(items.into_iter().map(to_dyn).collect()),
        Json::Object(entries) => {
            DynValue::Map(entries.into_iter().map(|(k, v)| (k, to_dyn(v))).collect())
        }
    }
}

pub fn from_dyn(v: DynValue) -> Json {
    match v {
        DynValue::Null => Json::Null,
        DynValue::Bool(b) => Json::Bool(b),
        DynValue::Int(i) => Json::Number(i.into()),
        DynValue::Float(f) => Number::from_f64(f).map_or(Json::Null, Json::Number),
        DynValue::String(s) => Json::String(s),
        DynValue::List(items) => Json::Array(items.into_iter().map(from_dyn).collect()),
        DynValue::Map(entries) => {
            let mut keys: Vec<_> = entries.into_iter().collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            Json::Object(keys.into_iter().map(|(k, v)| (k, from_dyn(v))).collect())
        }
    }
}

/// What an entry function returned. `Err(message)` when it returned a `Result` holding an error.
///
/// Unit is `null`; `bool`, `int`, `float` and `string` are themselves; `Option` is its value or
/// `null`; lists, string-keyed maps, structs and `Value`s convert element by element. An error
/// that is a struct with a string `message` field is that message.
pub fn returned(v: Value, defs: &DefTable) -> Result<Result<Json, String>, String> {
    match &v {
        Value::Enum(e) if e.def == DEF_RESULT => {
            let inner = e.fields.borrow().first().cloned().unwrap_or(Value::Unit);
            if e.tag == TAG_OK {
                returned(inner, defs)
            } else if e.tag == TAG_ERR {
                Ok(Err(match returned(inner, defs)? {
                    Ok(Json::String(s)) => s,
                    // An error struct, such as `ai`'s `AiError`, reads as its message.
                    Ok(Json::Object(o)) if o.get("message").is_some_and(Json::is_string) => {
                        o["message"].as_str().unwrap_or_default().to_owned()
                    }
                    Ok(other) => other.to_string(),
                    Err(s) => s,
                }))
            } else {
                Err("the script returned a malformed Result".into())
            }
        }
        Value::Enum(e) if e.def == DEF_OPTION => {
            if e.tag == TAG_SOME {
                let inner = e.fields.borrow().first().cloned().unwrap_or(Value::Unit);
                returned(inner, defs)
            } else {
                Ok(Ok(Json::Null))
            }
        }
        _ => plain(v, defs).map(Ok),
    }
}

fn plain(v: Value, defs: &DefTable) -> Result<Json, String> {
    Ok(match v {
        Value::Unit => Json::Null,
        Value::Bool(b) => Json::Bool(b),
        Value::Int(i) => Json::Number(i.into()),
        Value::Float(f) => Number::from_f64(f).map_or(Json::Null, Json::Number),
        Value::Char(c) => Json::String(c.to_string()),
        Value::Str(s) => Json::String(s.to_string()),
        Value::List(items) => Json::Array(
            items
                .borrow()
                .iter()
                .map(|i| plain(i.clone(), defs))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(entries) => {
            let mut out = serde_json::Map::new();
            for (k, v) in entries.borrow().iter() {
                let key = match k {
                    wscript::core::value::Key::Str(s) => s.to_string(),
                    wscript::core::value::Key::Int(i) => i.to_string(),
                    wscript::core::value::Key::Bool(b) => b.to_string(),
                    wscript::core::value::Key::Char(c) => c.to_string(),
                };
                out.insert(key, plain(v.clone(), defs)?);
            }
            Json::Object(out)
        }
        // A struct a host function returned, such as `http`'s `Response`, is an object of its
        // fields. An opaque handle has no JSON form.
        Value::Struct(st) if defs.as_struct(st.def).is_some_and(|sd| !sd.opaque) => {
            let names: Vec<String> = defs
                .as_struct(st.def)
                .map(|sd| sd.fields.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default();
            let mut out = serde_json::Map::new();
            for (name, v) in names.into_iter().zip(st.fields.borrow().iter()) {
                out.insert(name, plain(v.clone(), defs)?);
            }
            Json::Object(out)
        }
        other => {
            let kind = other.kind_name();
            DynValue::from_value(other, defs)
                .map(from_dyn)
                .map_err(|_| format!("the script returned a {kind}, which has no JSON form"))?
        }
    })
}
