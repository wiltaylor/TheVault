//! Explicit owner approval for historical safety decisions. No provider work in previews.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai::{Accounts, Credentials, decisions};
use axum::{Json, extract::State, http::StatusCode};
use schema::Classifier;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use state::Author;
use writer::Writer;

use crate::{ApiError, Caller, blocking};

const PRICE: &str = "safety_estimated_usd_per_decision";
const PREVIEW: &str = "agent_safety.backfill.preview";
const RUN: &str = "agent_safety.backfill.run";
const RELEASE: &str = "agent_safety.bulk.preview";
// Serializes approvals, not provider calls. Durable RUN prevents replay after restart.
static APPROVAL: Mutex<()> = Mutex::new(());

fn session(author: &Author) -> Result<(), ApiError> {
    if matches!(author, Author::Session(_)) {
        Ok(())
    } else {
        Err(ApiError::new(StatusCode::FORBIDDEN, "session_required", "A signed-in owner session is required"))
    }
}

fn stale() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "stale_preview", "Preview changed or was consumed. Review a new preview before approving.")
}

fn ai_error(e: ai::AiError) -> ApiError {
    ApiError::bad_request(e.to_string())
}

fn config(w: &Writer) -> Result<(String, Classifier, ai::Account), ApiError> {
    let source = w.vault().read_text("classifiers/agent_safety.wcl").map_err(writer::Error::from)?;
    let classifier = Classifier::parse("agent_safety", &source).map_err(|_| ApiError::bad_request("Invalid agent_safety classifier"))?;
    let accounts = Accounts::load(w.state()).map_err(ai_error)?;
    let account = accounts.pick(classifier.account.as_deref(), None).map_err(ai_error)?.clone();
    if account.kind != ai::Kind::OpenAiCompatible {
        return Err(ApiError::bad_request("An OpenAI-compatible decisions account is required"));
    }
    Ok((source, classifier, account))
}

fn account_hash(account: &ai::Account) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(account).expect("account")))
}

pub(crate) fn recover(w: &Writer) -> Result<(), state::Error> {
    if let Some(raw) = w.state().meta(RUN)?
        && let Ok(mut summary) = serde_json::from_str::<Value>(&raw)
        && summary["status"] == "running" {
        summary["status"] = json!("interrupted_no_retry");
        w.state().set_meta(RUN, &summary.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
struct Item {
    id: String,
    fingerprint: String,
}

fn items(w: &Writer, filter: &Filter) -> Result<Vec<Item>, ApiError> {
    let mut notes = Vec::new();
    for note in w.held_agent_notes(filter.kind.as_deref(), filter.since.as_deref(), filter.until.as_deref(), filter.reason.as_deref())? {
        let id = note["id"].as_str().unwrap_or_default();
        if filter.ids.as_ref().is_some_and(|ids| !ids.iter().any(|v| v == id)) {
            continue;
        }
        notes.push(Item { id: id.into(), fingerprint: w.agent_safety(id)?.fingerprint });
    }
    notes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(notes)
}

fn eligible(w: &Writer, since: &str, until: &str, classifier: &Classifier) -> Result<Vec<Item>, ApiError> {
    let mut result = Vec::new();
    for item in items(w, &Filter { since: Some(since.into()), until: Some(until.into()), ..Default::default() })? {
        let note = w.read(&item.id)?;
        // Pre-existing notes and their parents already have durable delivered markers.
        // Never classify arrivals through this path or enqueue a clearance trigger.
        if matches!(note.type_name.as_str(), "email" | "message" | "call" | "event")
            && classifier.types.contains(&note.type_name)
            && w.state().meta(&format!("agent_safety.legacy.{}", item.id))?.is_some()
            && w.state().meta(&format!("agent_safety.delivered.{}", item.id))?.is_some()
            && w.incoming_agent_note(&note.type_name, &note.doc)
            && w.state().meta(&format!("agent_safety.backfill.attempt.{}", item.id))?.is_none()
            && crate::classifiers::matches(w, &snapshot(w, &item.id)?, classifier).map_err(ai_error)?
        {
            result.push(item);
        }
    }
    Ok(result)
}

fn snapshot(w: &Writer, id: &str) -> Result<writer::Saved, ApiError> {
    let note = w.read(id)?;
    Ok(writer::Saved { id: note.id, type_name: note.type_name, version: note.version, wcl: note.wcl, event: writer::Event::Update, previous: None, restored: false, author: None, also: vec![] })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimateInput {
    /// Explicit owner entry only. Absent means use the previously entered account estimate.
    estimated_usd_per_decision: Option<f64>,
}

#[derive(Deserialize, Serialize)]
struct Preview {
    id: String,
    since: String,
    until: String,
    source: String,
    account: String,
    notes: Vec<Item>,
    decisions_per_note: usize,
    unit_price: f64,
    estimated_usd: f64,
}

pub async fn preview(State(w): State<Arc<Writer>>, Caller(author): Caller, Json(input): Json<EstimateInput>) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    blocking(w, move |w| {
        let _guard = APPROVAL.lock().unwrap_or_else(|e| e.into_inner());
        if w.state().meta(RUN)?.is_some() { return Err(stale()); }
        let (source, classifier, mut account) = config(w)?;
        if let Some(price) = input.estimated_usd_per_decision {
            if !price.is_finite() || price < 0.0 { return Err(ApiError::bad_request("Enter a finite nonnegative estimated USD cost per classifier decision")); }
            Accounts::change(w.state(), ai_error, |accounts| {
                let selected = accounts.accounts.iter_mut().find(|a| a.id == account.id).ok_or_else(stale)?;
                selected.other.insert(PRICE.into(), json!(price));
                account = selected.clone();
                Ok(())
            })?;
        }
        let price = account.other.get(PRICE).and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| ApiError::bad_request("Explicit owner-entered estimated USD cost per classifier decision required. No default price."))?;
        let now = chrono::DateTime::from_timestamp_millis(w.now_ms()).ok_or_else(|| ApiError::bad_request("Invalid clock"))?;
        let since = (now - chrono::Duration::days(90)).to_rfc3339();
        let until = now.to_rfc3339();
        let notes = eligible(w, &since, &until, &classifier)?;
        let estimate = notes.len() as f64 * classifier.questions.len() as f64 * price;
        if !estimate.is_finite() { return Err(ApiError::bad_request("Estimate overflow")); }
        let p = Preview { id: ulid::Ulid::new().to_string(), since, until, source, account: account_hash(&account), notes, decisions_per_note: classifier.questions.len(), unit_price: price, estimated_usd: estimate };
        w.state().set_meta(PREVIEW, &serde_json::to_string(&p).expect("preview"))?;
        Ok(Json(json!({"preview":p.id,"count":p.notes.len(),"account":account.name,"model":classifier.model,"decisions_per_note":p.decisions_per_note,"unit_price":price,"estimated_usd":estimate,"disclaimer":"Owner-entered estimate, not verified provider pricing or a billing guarantee. Backfill clears do not start agents."})))
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    preview: String,
    approve: bool,
}

pub async fn start(State(w): State<Arc<Writer>>, Caller(author): Caller, Json(input): Json<Approval>) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    let worker = w.clone();
    blocking(w, move |w| {
        let _guard = APPROVAL.lock().unwrap_or_else(|e| e.into_inner());
        let raw = w.state().meta(PREVIEW)?.ok_or_else(stale)?;
        let p: Preview = serde_json::from_str(&raw).map_err(|_| stale())?;
        let (source, classifier, account) = config(w)?;
        if !input.approve || input.preview != p.id || w.state().meta(RUN)?.is_some()
            || source != p.source || account_hash(&account) != p.account
            || eligible(w, &p.since, &p.until, &classifier)? != p.notes { return Err(stale()); }
        let summary = json!({"status":"running","count":p.notes.len(),"attempted":0,"allowed":0,"held":{},"failed":0,"estimated_usd":p.estimated_usd});
        w.state().set_meta(RUN, &summary.to_string())?;
        w.state().record_activity(state::NewActivity::new("agent_safety.backfill", "notes", &author, "Approved one-time safety backfill without agent triggers"));
        std::thread::Builder::new().name("safety-backfill".into()).spawn(move || {
            if run(&worker, &p, &classifier).is_err() {
                tracing::error!(target: "ai", "safety backfill stopped without retry");
                if let Ok(Some(raw)) = worker.state().meta(RUN)
                    && let Ok(mut summary) = serde_json::from_str::<Value>(&raw) {
                    summary["status"] = json!("interrupted_no_retry");
                    let _ = worker.state().set_meta(RUN, &summary.to_string());
                }
            }
        }).map_err(|_| ApiError::bad_request("Could not start backfill. Approval consumed, no automatic retry."))?;
        Ok(Json(summary))
    }).await
}

fn run(w: &Writer, p: &Preview, classifier: &Classifier) -> Result<(), ApiError> {
    let mut summary: Value = serde_json::from_str(&w.state().meta(RUN)?.ok_or_else(stale)?).map_err(|_| stale())?;
    for item in &p.notes {
        // One sequential request, at most one start per second, never retried.
        let (source, _, account) = config(w)?;
        if source != p.source || account_hash(&account) != p.account { return Err(stale()); }
        w.state().set_meta(&format!("agent_safety.backfill.attempt.{}", item.id), "1")?;
        let outcome = decide_one(w, item, classifier, &account);
        summary["attempted"] = json!(summary["attempted"].as_u64().unwrap_or(0) + 1);
        match outcome {
            Ok((true, _)) => summary["allowed"] = json!(summary["allowed"].as_u64().unwrap_or(0) + 1),
            Ok((false, reason)) => {
                let count = summary["held"][&reason].as_u64().unwrap_or(0) + 1;
                summary["held"][reason] = json!(count);
            }
            Err(_) => summary["failed"] = json!(summary["failed"].as_u64().unwrap_or(0) + 1),
        }
        w.state().set_meta(RUN, &summary.to_string())?;
        std::thread::sleep(Duration::from_secs(1));
    }
    summary["status"] = json!("complete");
    w.state().set_meta(RUN, &summary.to_string())?;
    w.state().record_activity(state::NewActivity::new("agent_safety.backfill.complete", "notes", &Author::Session("owner".into()), summary.to_string()));
    Ok(())
}

fn decide_one(w: &Writer, item: &Item, classifier: &Classifier, account: &ai::Account) -> Result<(bool, String), ApiError> {
    let note = w.read(&item.id)?;
    let envelope = w.agent_safety_envelope(&note.type_name, &note.doc);
    if writer::agent_safety::envelope_fingerprint(&envelope) != item.fingerprint || w.agent_safety(&item.id)?.allowed() { return Err(stale()); }
    if envelope.to_string().len() > writer::agent_safety::ENVELOPE_LIMIT {
        if !w.finish_agent_safety_backfill(&item.id, &item.fingerprint, "held", "too_large")? { return Err(stale()); }
        return Ok((false, "too_large".into()));
    }
    let saved = snapshot(w, &item.id)?;
    if !crate::classifiers::matches(w, &saved, classifier).map_err(ai_error)? { return Err(stale()); }
    let mut request = crate::classifiers::request(&saved, classifier).map_err(ai_error)?;
    request.state = envelope;
    let credential = Credentials::resolve(w.state(), account).map_err(ai_error)?;
    let result = decisions::decide(&credential, &request, Duration::from_secs(30));
    let usage = result.as_ref().map_or_else(|e| e.usage, |r| ai::Usage { input: r.usage.input_tokens, output: r.usage.output_tokens });
    w.state().record_ai_usage(&account.id, &request.model, "safety_backfill", usage.input, usage.output)?;
    let (allowed, reason) = match result {
        Ok(response) => match response.answers.get("reason") {
            Some(decisions::Answer::Choice { choice, .. }) => (!classifier.withhold.contains(choice), choice.clone()),
            _ => return Err(ApiError::bad_request("Invalid safety decision")),
        },
        Err(_) => {
            w.finish_agent_safety_backfill(&item.id, &item.fingerprint, "held", "decision_failed")?;
            return Err(ApiError::bad_request("Decision failed, no retry"));
        }
    };
    if !w.finish_agent_safety_backfill(&item.id, &item.fingerprint, if allowed { "allowed" } else { "held" }, &reason)? { return Err(stale()); }
    Ok((allowed, reason))
}

pub async fn progress(State(w): State<Arc<Writer>>, Caller(author): Caller) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    blocking(w, |w| {
        let value = w.state().meta(RUN)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!({"status":"not_started"}));
        Ok::<_, ApiError>(Json(value))
    }).await
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    #[serde(rename = "type")]
    kind: Option<String>,
    since: Option<String>,
    until: Option<String>,
    reason: Option<String>,
    ids: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
struct ReleasePreview { id: String, filter: Filter, notes: Vec<Item> }

pub async fn release_preview(State(w): State<Arc<Writer>>, Caller(author): Caller, Json(filter): Json<Filter>) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    blocking(w, move |w| {
        let _guard = APPROVAL.lock().unwrap_or_else(|e| e.into_inner());
        let notes = items(w, &filter)?;
        let p = ReleasePreview { id: ulid::Ulid::new().to_string(), filter, notes };
        w.state().set_meta(RELEASE, &serde_json::to_string(&p).expect("preview"))?;
        Ok::<_, ApiError>(Json(json!({"preview":p.id,"count":p.notes.len()})))
    }).await
}

pub async fn release(State(w): State<Arc<Writer>>, Caller(author): Caller, Json(input): Json<Approval>) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    blocking(w, move |w| {
        let _guard = APPROVAL.lock().unwrap_or_else(|e| e.into_inner());
        let raw = w.state().meta(RELEASE)?.ok_or_else(stale)?;
        let p: ReleasePreview = serde_json::from_str(&raw).map_err(|_| stale())?;
        if !input.approve || input.preview != p.id || items(w, &p.filter)? != p.notes { return Err(stale()); }
        w.state().set_meta(RELEASE, "consumed")?;
        let mut released = 0;
        let mut released_ids = Vec::new();
        let mut failed = 0;
        for item in p.notes {
            match w.release_agent_safety(&item.id, &item.fingerprint, &author) {
                Ok(true) => { released += 1; released_ids.push(item.id); },
                _ => failed += 1,
            }
        }
        let summary = json!({"released":released,"failed":failed,"released_ids":released_ids});
        w.state().record_activity(state::NewActivity::new("agent_safety.bulk_release", "notes", &author, summary.to_string()));
        Ok(Json(summary))
    }).await
}
