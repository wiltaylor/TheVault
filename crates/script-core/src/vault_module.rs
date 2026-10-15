//! The `vault` host module: every function a script calls to read and change notes. Each one
//! goes through a [`Vault`], so changes take the write path (D30). `docs/scripts.md` documents
//! them for script authors.

use std::str::FromStr;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::TimeZone;
use serde_json::{Value as Json, json};
use wscript::std_modules::DynValue;
use wscript::{Fault, Module};

use crate::{Reply, Vault, convert, host_call, note_body, run};

fn to_dyn_list(rows: Vec<Json>) -> Vec<DynValue> {
    rows.into_iter().map(convert::to_dyn).collect()
}

fn size(n: i64) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// The date today in the run's time zone, as `YYYY-MM-DD`.
fn today(now_ms: i64) -> String {
    let tz = run::peek(|r| r.job.timezone.clone())
        .flatten()
        .and_then(|z| chrono_tz::Tz::from_str(&z).ok())
        .unwrap_or(chrono_tz::Tz::UTC);
    tz.timestamp_millis_opt(now_ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Who a notification from the run in progress is from: `task:<name>` for a scheduled script,
/// `handler:<type>.<event>` for a handler, `script:<name>` otherwise.
fn notify_source() -> String {
    crate::current_job(|j| {
        if let Some(vocab::Author::Task(task)) = &j.author {
            return format!("task:{task}");
        }
        let note = j.args.first();
        let text = |k: &str| note.and_then(|n| n.get(k)).and_then(Json::as_str);
        match (j.kind, text("type"), text("event")) {
            (crate::Kind::Task, _, _) => format!("task:{}", j.name),
            (_, Some(t), Some(e)) => format!("handler:{t}.{e}"),
            _ => format!("script:{}", j.name),
        }
    })
    .unwrap_or_else(|| "script".to_owned())
}

/// Paragraph blocks for `text`: one per run of text between blank lines.
fn paragraphs(text: &str) -> Vec<Json> {
    text.replace("\r\n", "\n")
        .split("\n\n")
        .map(|p| p.trim_matches('\n'))
        .filter(|p| !p.trim().is_empty())
        .map(|p| json!({ "kind": "paragraph", "fields": { "text": p } }))
        .collect()
}

pub fn vault_module(v: Arc<dyn Vault>) -> Module {
    let mut m = Module::new("vault");
    m.doc(
        "Read, query, search and change notes, assets and taxonomies through the write path (D30)",
    );

    let vv = v.clone();
    m.fn_named("get", ["id"], move |id: String| -> Reply<DynValue> {
        Fault(host_call(|_, _| vv.get(&id).map(convert::to_dyn)))
    });
    let vv = v.clone();
    m.fn_named(
        "query",
        ["text"],
        move |text: String| -> Reply<Vec<DynValue>> {
            Fault(host_call(|_, l| {
                vv.query(&text, l.max_query_rows).map(to_dyn_list)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "query_page",
        ["text", "size", "cursor"],
        move |text: String, page_size: i64, cursor: String| -> Reply<DynValue> {
            Fault(host_call(|_, _| {
                let cursor = (!cursor.is_empty()).then_some(cursor);
                vv.query_page(&text, size(page_size), cursor)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "search",
        ["text", "limit"],
        move |text: String, limit: i64| -> Reply<Vec<DynValue>> {
            Fault(host_call(|_, _| {
                vv.search(&text, size(limit), vocab::Archived::Exclude)
                    .map(to_dyn_list)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "search_archived",
        ["text", "limit", "archived"],
        move |text: String, limit: i64, archived: String| -> Reply<Vec<DynValue>> {
            Fault(host_call(|_, _| {
                let mode = vocab::Archived::parse(&archived).ok_or_else(|| {
                    format!("archived is {archived:?}: it must be exclude, include or only")
                })?;
                vv.search(&text, size(limit), mode).map(to_dyn_list)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "create",
        ["type_name", "note"],
        move |type_name: String, note: DynValue| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.create(&type_name, note_body(note), a)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "update",
        ["id", "note", "version"],
        move |id: String, note: DynValue, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.update(&id, note_body(note), &version, a)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "patch",
        ["id", "fields", "version"],
        move |id: String, fields: DynValue, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.patch(&id, convert::from_dyn(fields), &version, a)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "append_text",
        ["id", "field", "text", "version"],
        move |id: String, field: String, text: String, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                let blocks = paragraphs(&text);
                if blocks.is_empty() {
                    return Err("the text to append is empty".into());
                }
                vv.append_blocks(&id, &field, blocks, &version, a)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "append_blocks",
        ["id", "field", "blocks", "version"],
        move |id: String, field: String, blocks: DynValue, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                let blocks = match convert::from_dyn(blocks) {
                    Json::Array(b) => b,
                    b @ Json::Object(_) => vec![b],
                    _ => return Err("append_blocks takes a block or a list of blocks".into()),
                };
                vv.append_blocks(&id, &field, blocks, &version, a)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named(
        "delete",
        ["id", "version"],
        move |id: String, version: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.delete(&id, &version, a).map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named("restore", ["id"], move |id: String| -> Reply<DynValue> {
        Fault(host_call(|a, _| vv.restore(&id, a).map(convert::to_dyn)))
    });
    let vv = v.clone();
    m.fn_named("archive", ["id"], move |id: String| -> Reply<DynValue> {
        Fault(host_call(|a, _| {
            vv.set_archived(&id, true, a).map(convert::to_dyn)
        }))
    });
    let vv = v.clone();
    m.fn_named("unarchive", ["id"], move |id: String| -> Reply<DynValue> {
        Fault(host_call(|a, _| {
            vv.set_archived(&id, false, a).map(convert::to_dyn)
        }))
    });
    let vv = v.clone();
    m.fn_named("links", ["id"], move |id: String| -> Reply<Vec<DynValue>> {
        Fault(host_call(|_, _| vv.links(&id).map(to_dyn_list)))
    });
    let vv = v.clone();
    m.fn_named(
        "backlinks",
        ["id"],
        move |id: String| -> Reply<Vec<DynValue>> {
            Fault(host_call(|_, _| vv.backlinks(&id).map(to_dyn_list)))
        },
    );
    let vv = v.clone();
    m.fn_("assets", move || -> Reply<Vec<DynValue>> {
        Fault(host_call(|_, _| vv.assets().map(to_dyn_list)))
    });
    let vv = v.clone();
    m.fn_named("read_asset", ["id"], move |id: String| -> Reply<DynValue> {
        Fault(host_call(|_, l| {
            let (mut info, bytes) = vv.asset(&id)?;
            if bytes.len() > l.max_file {
                return Err(format!(
                    "asset {id:?} is larger than the {} bytes a script may read",
                    l.max_file
                ));
            }
            info["base64"] = json!(BASE64.encode(bytes));
            Ok(convert::to_dyn(info))
        }))
    });
    let vv = v.clone();
    m.fn_named(
        "upload_asset",
        ["filename", "base64"],
        move |filename: String, data: String| -> Reply<DynValue> {
            Fault(host_call(|_, _| {
                let bytes = BASE64
                    .decode(data.trim())
                    .map_err(|e| format!("the data is not base64: {e}"))?;
                vv.upload_asset(&filename, bytes).map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_("types", move || -> Reply<Vec<DynValue>> {
        Fault(host_call(|_, _| vv.types().map(to_dyn_list)))
    });
    let vv = v.clone();
    m.fn_("taxonomies", move || -> Reply<Vec<DynValue>> {
        Fault(host_call(|_, _| vv.taxonomies().map(to_dyn_list)))
    });
    let vv = v.clone();
    m.fn_named(
        "add_term",
        ["taxonomy", "path"],
        move |taxonomy: String, path: String| -> Reply<DynValue> {
            Fault(host_call(|a, _| {
                vv.add_term(&taxonomy, &path, a).map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_named("daily", ["date"], move |date: String| -> Reply<DynValue> {
        Fault(host_call(|a, _| {
            let date = if date.is_empty() || date == "today" {
                today(vv.now_ms())
            } else {
                date
            };
            vv.daily(&date, a).map(convert::to_dyn)
        }))
    });
    let vv = v.clone();
    m.fn_named(
        "notify",
        ["title", "body", "note"],
        move |title: String, body: String, note: String| -> Reply<DynValue> {
            Fault(host_call(|_, _| {
                vv.notify(&notify_source(), &title, &body, &note)
                    .map(convert::to_dyn)
            }))
        },
    );
    let vv = v.clone();
    m.fn_("today", move || -> String { today(vv.now_ms()) });
    m.fn_("now", move || -> i64 { v.now_ms() });
    m.fn_named("log", ["message"], |message: String| -> Fault<()> {
        Fault(run::with(|r| {
            r.remaining()?;
            r.log(&message, true);
            Ok(())
        }))
    });
    m
}
