//! Data-only classifier definitions. The provider's alpha wire contract belongs to `ai`.

use std::collections::HashSet;

use serde_json::{Value as Json, json};

use crate::{Doc, Invalid, Issue, Value, is_name};

#[derive(Debug, Clone)]
pub struct Classifier {
    pub name: String,
    pub types: Vec<String>,
    pub condition: Option<String>,
    pub account: Option<String>,
    pub model: String,
    pub fields: Vec<String>,
    pub questions: Vec<ClassifierQuestion>,
    pub notify: Option<ClassifierNotify>,
}

#[derive(Debug, Clone)]
pub struct ClassifierQuestion {
    pub name: String,
    pub kind: String,
    pub instructions: Json,
    pub criteria: Option<Json>,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct ClassifierNotify {
    pub question: String,
    pub option: Option<String>,
    pub threshold: f64,
    pub title: String,
}

impl Classifier {
    pub fn parse(name: &str, source: &str) -> Result<Self, Invalid> {
        let file = format!("classifiers/{name}.wcl");
        let doc = Doc::parse(source, &file)?;
        Self::from_doc(name, &doc).map_err(|message| Invalid {
            file,
            issues: vec![Issue::new("classifier", message)],
        })
    }

    fn from_doc(name: &str, doc: &Doc) -> Result<Self, String> {
        if !is_name(name) {
            return Err("Invalid classifier name".into());
        }
        check_fields(doc, &["types", "where", "account", "model", "fields"])?;
        let types = strings(doc, "types")?;
        let fields = strings(doc, "fields")?;
        let condition = optional_text(doc, "where")?;
        if let Some(condition) = &condition {
            query_syntax::parse_expr(condition).map_err(|_| "Invalid where condition")?;
        }
        let account = optional_text(doc, "account")?;
        let model = text(doc, "model")?;
        let mut questions = Vec::new();
        let mut names = HashSet::new();
        let mut targets = HashSet::new();
        let mut notify = None;
        for block in &doc.blocks {
            if !block.body.blocks.is_empty() {
                return Err("Nested classifier blocks are not supported".into());
            }
            match block.kind.as_str() {
                "question" => {
                    if block.labels.len() != 1
                        || !is_name(&block.labels[0])
                        || !names.insert(block.labels[0].clone())
                    {
                        return Err("Questions need unique valid names".into());
                    }
                    check_fields(&block.body, &["type", "instructions", "criteria", "target"])?;
                    let kind = text(&block.body, "type")?;
                    let instructions = context(&block.body, "instructions")?;
                    let criteria = block.body.field("criteria").map(to_json);
                    let valid = match kind.as_str() {
                        "noul" => criteria.as_ref().is_none_or(|c| {
                            c.as_object().is_some_and(|c| {
                                c.len() == 2
                                    && c.contains_key("true")
                                    && c.contains_key("false")
                                    && c.values().all(is_context)
                            })
                        }),
                        "choice" => criteria
                            .as_ref()
                            .and_then(Json::as_object)
                            .is_some_and(|c| {
                                c.len() >= 2
                                    && c.keys().all(|k| !k.trim().is_empty())
                                    && c.values().all(is_context)
                            }),
                        "score" => criteria
                            .as_ref()
                            .and_then(Json::as_array)
                            .is_some_and(|c| c.len() >= 2 && c.iter().all(is_context)),
                        _ => false,
                    };
                    if !valid {
                        return Err("Invalid question type or criteria".into());
                    }
                    let target = text(&block.body, "target")?;
                    if !is_name(&target)
                        || crate::RESERVED_FIELDS.contains(&target.as_str())
                        || !targets.insert(target.clone())
                    {
                        return Err("Targets must be unique nonreserved field names".into());
                    }
                    questions.push(ClassifierQuestion {
                        name: block.labels[0].clone(),
                        kind,
                        instructions,
                        criteria,
                        target,
                    });
                }
                "notify" => {
                    if notify.is_some() || !block.labels.is_empty() {
                        return Err("Use at most one unlabelled notify block".into());
                    }
                    check_fields(&block.body, &["question", "option", "threshold", "title"])?;
                    let threshold = block
                        .body
                        .field("threshold")
                        .and_then(Value::as_f64)
                        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
                        .ok_or("Notify threshold must be from 0 to 1")?;
                    let title = text(&block.body, "title")?;
                    if title.chars().count() > 200 {
                        return Err("Notify title exceeds 200 characters".into());
                    }
                    notify = Some(ClassifierNotify {
                        question: text(&block.body, "question")?,
                        option: optional_text(&block.body, "option")?,
                        threshold,
                        title,
                    });
                }
                _ => return Err("Unknown classifier block".into()),
            }
        }
        if questions.is_empty() || questions.len() > 64 {
            return Err("Use 1 to 64 questions".into());
        }
        if let Some(rule) = &notify {
            let question = questions
                .iter()
                .find(|q| q.name == rule.question)
                .ok_or("Notify names an unknown question")?;
            let valid = match question.kind.as_str() {
                "noul" => rule.option.is_none(),
                "choice" => rule.option.as_ref().is_some_and(|o| {
                    question
                        .criteria
                        .as_ref()
                        .and_then(Json::as_object)
                        .is_some_and(|c| c.contains_key(o))
                }),
                _ => false,
            };
            if !valid {
                return Err("Notify requires noul without option, or choice with a valid option; score is not supported".into());
            }
        }
        Ok(Self {
            name: name.into(),
            types,
            condition,
            account,
            model,
            fields,
            questions,
            notify,
        })
    }
}

fn check_fields(doc: &Doc, allowed: &[&str]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (name, _) in &doc.fields {
        if !allowed.contains(&name.as_str()) || !seen.insert(name) {
            return Err(format!("Unknown or duplicate field {name}"));
        }
    }
    Ok(())
}

fn text(doc: &Doc, name: &str) -> Result<String, String> {
    doc.field(name)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{name} must be a nonempty string"))
}

fn optional_text(doc: &Doc, name: &str) -> Result<Option<String>, String> {
    doc.field(name).map(|_| text(doc, name)).transpose()
}

fn strings(doc: &Doc, name: &str) -> Result<Vec<String>, String> {
    let Some(Value::List(values)) = doc.field(name) else {
        return Err(format!("{name} must be a list"));
    };
    let mut seen = HashSet::new();
    values
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| is_name(s) && seen.insert(*s))
                .map(str::to_owned)
                .ok_or_else(|| format!("{name} needs unique valid names"))
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|v| {
            if v.is_empty() {
                Err(format!("{name} must not be empty"))
            } else {
                Ok(v)
            }
        })
}

fn is_context(v: &Json) -> bool {
    v.is_string() || v.is_object() || v.is_array()
}
fn context(doc: &Doc, name: &str) -> Result<Json, String> {
    doc.field(name)
        .map(to_json)
        .filter(is_context)
        .ok_or_else(|| format!("{name} must be text, a record or a list"))
}

fn to_json(v: &Value) -> Json {
    match v {
        Value::Str(v) => json!(v),
        Value::Int(v) => json!(v),
        Value::Float(v) => json!(v),
        Value::Bool(v) => json!(v),
        Value::List(v) => Json::Array(v.iter().map(to_json).collect()),
        Value::Record(v) => Json::Object(v.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}
