//! Batch writes with a dry run, and upserts by a key field (D32).
//!
//! `POST /api/batch` takes `{ "dry_run"?, "ops": [op] }`. Each op is one of:
//!
//! - `{ "op": "create", "type", "note" | "wcl" }`
//! - `{ "op": "update", "id", "base_version", "note" | "wcl" }`
//! - `{ "op": "delete", "id", "base_version" }`
//! - `{ "op": "upsert", "type", "key", "note" | "wcl", "base_version"? }`
//!
//! A batch commits all of its items or none of them. The response has a result for every item,
//! and a dry run answers exactly as the real run would, with `version` null. The status is 200
//! when every item succeeded and 422 otherwise, dry run or not.
//!
//! `POST /api/upsert` takes one upsert, `{ "type", "key", "note" | "wcl", "base_version"?,
//! "dry_run"? }`, and answers as a batch of one.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{Event, Input, Op, Report, Writer};

use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchBody {
    #[serde(default)]
    dry_run: bool,
    ops: Vec<OpBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpBody {
    op: String,
    #[serde(rename = "type")]
    type_name: Option<String>,
    id: Option<String>,
    key: Option<String>,
    base_version: Option<String>,
    note: Option<Value>,
    wcl: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertBody {
    #[serde(rename = "type")]
    type_name: String,
    key: String,
    base_version: Option<String>,
    note: Option<Value>,
    wcl: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

fn input(at: &str, note: Option<Value>, wcl: Option<String>) -> Result<Input, ApiError> {
    match (note, wcl) {
        (Some(note), None) => Ok(Input::Json(note)),
        (None, Some(wcl)) => Ok(Input::Wcl(wcl)),
        _ => Err(ApiError::bad_request(format!(
            "{at}: send exactly one of `note` (JSON) and `wcl` (text)"
        ))),
    }
}

impl OpBody {
    fn into_op(self, n: usize) -> Result<Op, ApiError> {
        let at = format!("ops[{n}]");
        let need = |v: Option<String>, name: &str| {
            v.ok_or_else(|| ApiError::bad_request(format!("{at}: a {} op needs `{name}`", self.op)))
        };
        let refuse = |present: bool, name: &str| {
            if present {
                Err(ApiError::bad_request(format!(
                    "{at}: a {} op takes no `{name}`",
                    self.op
                )))
            } else {
                Ok(())
            }
        };
        match self.op.as_str() {
            "create" => {
                refuse(self.id.is_some(), "id")?;
                refuse(self.key.is_some(), "key")?;
                refuse(self.base_version.is_some(), "base_version")?;
                Ok(Op::Create {
                    type_name: need(self.type_name.clone(), "type")?,
                    input: input(&at, self.note, self.wcl)?,
                })
            }
            "update" => {
                refuse(self.type_name.is_some(), "type")?;
                refuse(self.key.is_some(), "key")?;
                Ok(Op::Update {
                    id: need(self.id.clone(), "id")?,
                    base: self.base_version,
                    input: input(&at, self.note, self.wcl)?,
                })
            }
            "delete" => {
                refuse(self.type_name.is_some(), "type")?;
                refuse(self.key.is_some(), "key")?;
                refuse(self.note.is_some() || self.wcl.is_some(), "note")?;
                Ok(Op::Delete {
                    id: need(self.id.clone(), "id")?,
                    base: self.base_version,
                })
            }
            "upsert" => {
                refuse(self.id.is_some(), "id")?;
                Ok(Op::Upsert {
                    type_name: need(self.type_name.clone(), "type")?,
                    key: need(self.key.clone(), "key")?,
                    base: self.base_version,
                    input: input(&at, self.note, self.wcl)?,
                })
            }
            other => Err(ApiError::bad_request(format!(
                "{at}: unknown op {other:?}; use create, update, delete or upsert"
            ))),
        }
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Create { .. } => "create",
        Op::Update { .. } => "update",
        Op::Delete { .. } => "delete",
        Op::Upsert { .. } => "upsert",
    }
}

fn error_json(e: writer::Error) -> Value {
    let e = ApiError::from(e);
    let mut body = e.body;
    body["status"] = json!(e.status.as_u16());
    body
}

fn event_name(e: Event) -> &'static str {
    match e {
        Event::Create => "create",
        Event::Update => "update",
        Event::Delete => "delete",
    }
}

fn report_response(names: &[&'static str], report: Report) -> Response {
    let items: Vec<Value> = report
        .items
        .into_iter()
        .enumerate()
        .map(|(n, item)| match item {
            Ok(i) => json!({
                "index": n,
                "op": names[n],
                "result": i.outcome.as_str(),
                "id": i.id,
                "type": i.type_name,
                "version": i.version,
                "wcl": i.wcl,
            }),
            Err(e) => json!({
                "index": n,
                "op": names[n],
                "result": "error",
                "error": error_json(e),
            }),
        })
        .collect();
    let also: Vec<Value> = report
        .also
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "type": s.type_name,
                "event": event_name(s.event),
                "version": (!s.version.is_empty()).then_some(&s.version),
            })
        })
        .collect();
    let status = if report.ok {
        StatusCode::OK
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    let body = json!({
        "dry_run": report.dry_run,
        "ok": report.ok,
        "committed": report.ok && !report.dry_run,
        "items": items,
        "also": also,
        "error": report.error.map(error_json),
    });
    (status, Json(body)).into_response()
}

/// `POST /api/batch`.
pub async fn run(
    State(w): Shared,
    Caller(author): Caller,
    Json(body): Json<BatchBody>,
) -> Result<Response, ApiError> {
    let ops = body
        .ops
        .into_iter()
        .enumerate()
        .map(|(n, op)| op.into_op(n))
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<&'static str> = ops.iter().map(op_name).collect();
    let dry_run = body.dry_run;
    let report = blocking(w, move |w| w.batch(ops, dry_run, &author)).await?;
    Ok(report_response(&names, report))
}

/// `POST /api/upsert`.
pub async fn upsert(
    State(w): Shared,
    Caller(author): Caller,
    Json(body): Json<UpsertBody>,
) -> Result<Response, ApiError> {
    let input = input("upsert", body.note, body.wcl)?;
    let report = blocking(w, move |w| {
        w.upsert(
            &body.type_name,
            &body.key,
            &input,
            body.base_version.as_deref(),
            body.dry_run,
            &author,
        )
    })
    .await?;
    Ok(report_response(&["upsert"], report))
}
