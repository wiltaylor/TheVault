//! Owner-only local setup. Never resolve credentials, refresh sign-ins or call a provider.
use std::sync::Arc;

use ai::{Account, Accounts, Credentials, Kind};
use axum::{Json, extract::{Path, State}, http::StatusCode};
use schema::Classifier;
use serde::Deserialize;
use serde_json::{Value, json};
use state::Author;
use writer::Writer;

use crate::{ApiError, Caller, blocking};

fn session(author: &Author) -> Result<(), ApiError> {
    if matches!(author, Author::Session(_)) { Ok(()) } else {
        Err(ApiError::new(StatusCode::FORBIDDEN, "session_required", "A signed-in owner session is required"))
    }
}

fn compatible(account: &Account) -> bool {
    account.kind == Kind::OpenAiCompatible && account.url().is_some_and(|base| {
        ai::check_url(base).ok().and_then(|url| reqwest::Url::parse(&url).ok())
            .is_some_and(|url| url.path().trim_end_matches('/').ends_with("/api/v1"))
    })
}

fn local(w: &Writer) -> Result<Value, ApiError> {
    let accounts = Accounts::load(w.state()).map_err(|_| ApiError::bad_request("Cannot read AI accounts"))?;
    let choices: Vec<_> = accounts.accounts.iter().filter(|a| compatible(a))
        .map(|a| json!({"id": a.id, "name": a.name})).collect();
    let mut names: Vec<_> = std::fs::read_dir(w.vault().root().join("classifiers"))
        .map_err(|_| ApiError::bad_request("Cannot read classifier definitions"))?
        .flatten().filter_map(|entry| entry.file_name().to_str()?.strip_suffix(".wcl")
            .filter(|name| schema::is_name(name)).map(str::to_owned)).collect();
    names.sort();
    let mut rows = Vec::new();
    for name in names {
        let config = w.vault().read_text(&format!("classifiers/{name}.wcl")).ok()
            .and_then(|source| Classifier::parse(&name, &source).ok());
        let mut row = json!({"name": name, "valid": config.is_some(), "reason": "invalid_definition",
            "last_outcome": w.state().meta(&format!("classifier_last_outcome:{name}"))?
                .filter(|s| ["attempting", "succeeded", "setup_failed", "provider_failed", "write_failed", "held_too_large"].contains(&s.as_str()))
                .unwrap_or_else(|| "not_recorded".into())});
        if let Some(config) = config {
            row["model"] = json!(config.model);
            row["account"] = json!(config.account);
            row["fields"] = json!(config.fields);
            row["types"] = json!(config.types);
            row["filter"] = json!(config.condition);
            let reason = match accounts.pick(config.account.as_deref(), None) {
                Err(_) => "missing_account",
                Ok(account) => {
                    row["resolved_account"] = json!(account.name);
                    row["resolved_account_id"] = json!(account.id);
                    if account.kind != Kind::OpenAiCompatible { "incompatible_kind" }
                    else if !compatible(account) { "invalid_endpoint" }
                    else if !Credentials::key_set(w.state(), &account.id).unwrap_or(false) { "no_stored_key" }
                    else { "locally_ready" }
                }
            };
            row["reason"] = json!(reason);
        }
        rows.push(row);
    }
    Ok(json!({"accounts": choices, "classifiers": rows}))
}

pub async fn read(State(w): State<Arc<Writer>>, Caller(author): Caller) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    blocking(w, |w| local(w).map(Json)).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection { account: String }

pub async fn select(State(w): State<Arc<Writer>>, Caller(author): Caller,
    Path(name): Path<String>, Json(input): Json<Selection>) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    blocking(w, move |w| {
        let accounts = Accounts::load(w.state()).map_err(|_| ApiError::bad_request("Cannot read AI accounts"))?;
        let account = accounts.get(&input.account).filter(|a| compatible(a))
            .ok_or_else(|| ApiError::bad_request("Select an OpenAI-compatible /api/v1 account in Settings > AI"))?;
        w.set_classifier_account(&name, &account.name)?;
        local(w).map(Json)
    }).await
}
