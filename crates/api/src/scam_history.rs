//! Owner-approved historical scam classification. The durable ledger precedes every call.
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use ai::{Accounts, Credentials, decisions};
use axum::{Json, extract::State, http::{HeaderMap, StatusCode}};
use schema::Classifier;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use state::Author;
use writer::{Saved, Writer};

use crate::{ApiError, Caller, blocking};

const PREVIEW: &str = "scam.history.preview";
const LEDGER: &str = "scam.history.ledger";
static LOCK: Mutex<()> = Mutex::new(());

fn stale() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "stale_preview", "Inputs changed or approval consumed. Review a new preview.")
}

fn owner(author: &Author) -> Result<(), ApiError> {
    if matches!(author, Author::Session(_)) { Ok(()) }
    else { Err(ApiError::new(StatusCode::FORBIDDEN, "session_required", "Owner session required")) }
}

fn hash(bytes: impl AsRef<[u8]>) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn session(headers: &HeaderMap) -> Result<String, ApiError> {
    crate::auth::session_id(headers).map(hash).ok_or_else(stale)
}

fn config(w: &Writer) -> Result<(String, Classifier, ai::Account), ApiError> {
    let source = w.vault().read_text("classifiers/scam.wcl").map_err(writer::Error::from)?;
    let classifier = Classifier::parse("scam", &source).map_err(|_| ApiError::bad_request("Invalid scam definition"))?;
    let accounts = Accounts::load(w.state()).map_err(|_| stale())?;
    let account = accounts.pick(classifier.account.as_deref(), None).map_err(|_| stale())?.clone();
    if account.kind != ai::Kind::OpenAiCompatible || !account.url().is_some_and(|url| {
        reqwest::Url::parse(url).is_ok_and(|u| u.path().trim_end_matches('/').ends_with("/api/v1"))
    }) { return Err(ApiError::bad_request("Select a Decisions account in Settings > AI")); }
    Ok((source, classifier, account))
}

fn account_hash(a: &ai::Account) -> String { hash(serde_json::to_vec(a).expect("account")) }

#[derive(Clone, Deserialize, Serialize, PartialEq)]
struct Item { id: String, fingerprint: String }

#[derive(Clone, Deserialize, Serialize)]
struct Preview {
    fingerprint: String,
    session: String,
    expires_ms: i64,
    until_ms: i64,
    days: u32,
    price: f64,
    source_hash: String,
    account_hash: String,
    items: Vec<Item>,
    decisions_per_item: usize,
    estimated_usd: f64,
}

#[derive(Clone, Deserialize, Serialize)]
struct Attempt { item: Item, outcome: String }

#[derive(Clone, Deserialize, Serialize)]
struct Run { preview: Preview, status: String, attempts: Vec<Attempt> }

#[derive(Default, Deserialize, Serialize)]
struct Ledger { runs: Vec<Run> }

fn ledger(w: &Writer) -> Result<Ledger, ApiError> {
    match w.state().meta(LEDGER)? {
        None => Ok(Ledger::default()),
        Some(raw) => serde_json::from_str(&raw).map_err(|_| stale()),
    }
}

fn save(w: &Writer, ledger: &Ledger) -> Result<(), ApiError> {
    w.state().set_meta(LEDGER, &serde_json::to_string(ledger).expect("ledger"))?;
    Ok(())
}

fn snapshot(w: &Writer, id: &str) -> Result<Saved, ApiError> {
    let note = w.read(id)?;
    Ok(Saved { id: note.id, type_name: note.type_name, version: note.version, wcl: note.wcl,
        event: writer::Event::Update, previous: None, restored: false, author: None, also: vec![] })
}

fn selection(w: &Writer, c: &Classifier, days: u32, until: i64) -> Result<Vec<Item>, ApiError> {
    // Substitute only the shipped relative date clause, never discard the incoming predicates.
    let condition = c.condition.as_deref().ok_or_else(stale)?;
    let date = "sent >= now() - days(2)";
    if condition.matches(date).count() != 1 || !condition.ends_with(date) {
        return Err(ApiError::bad_request("Historical scam runs require the standard sent-date filter"));
    }
    let filter = format!("({}) && ((type == \"email\" && !under(labels, \"system/SENT\") && !under(labels, \"system/DRAFT\")) || (type == \"message\" && direction == \"in\"))", condition.replace(date, &format!("(sent >= now() - days({days}) && sent <= now())")));
    let q = query::Query { from: query::From::Types(c.types.iter().filter(|t| matches!(t.as_str(), "email" | "message")).cloned().collect()),
        include_archived: false, filter: Some(query::parse_expr(&filter).map_err(|_| stale())?), search: None,
        sort: vec![], group_by: None, select: vec![], limit: None };
    let mut ids = Vec::new();
    w.read_index(|index, schema| {
        let context = query::Context::new(index, schema).at(until as f64 / 1000.0);
        let mut page = query::PageRequest { cursor: None, size: 1000 };
        loop {
            let result = query::run(&q, &context, &page)?;
            ids.extend(result.rows.into_iter().map(|r| r.id));
            match result.next { Some(next) => page.cursor = Some(next), None => break }
        }
        Ok::<_, query::Error>(())
    }).map_err(|_| stale())?;
    let mut items = Vec::new();
    for id in ids {
        let s = snapshot(w, &id)?;
        let doc = schema::Doc::parse(&s.wcl, "historical scam").map_err(|_| stale())?;
        let fields = writer::doc_to_json(&doc);
        // Missing output, not an invented historical failure count. Never replace a verdict.
        if c.questions.iter().all(|q| fields["fields"][&q.target].is_null()) {
            items.push(Item { id, fingerprint: hash(s.wcl.as_bytes()) });
        }
    }
    items.sort_by(|a,b| a.id.cmp(&b.id));
    Ok(items)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input { days: u32, estimated_usd_per_decision: f64 }

pub async fn preview(State(w): State<Arc<Writer>>, Caller(author): Caller, headers: HeaderMap, Json(input): Json<Input>) -> Result<Json<Value>, ApiError> {
    owner(&author)?;
    let session = session(&headers)?;
    blocking(w, move |w| {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if !(1..=90).contains(&input.days) || !input.estimated_usd_per_decision.is_finite() || input.estimated_usd_per_decision < 0.0 {
            return Err(ApiError::bad_request("Choose 1–90 days and an explicit finite nonnegative estimated USD price"));
        }
        if ledger(w)?.runs.iter().any(|r| r.status == "running") { return Err(stale()); }
        let (source,c,a) = config(w)?;
        let until = w.now_ms();
        let items = selection(w,&c,input.days,until)?;
        let total = items.len() as f64 * c.questions.len() as f64 * input.estimated_usd_per_decision;
        if !total.is_finite() { return Err(ApiError::bad_request("Estimate overflow")); }
        let mut p = Preview { fingerprint: ulid::Ulid::new().to_string(), session, expires_ms: until.saturating_add(600_000), until_ms: until,
            days: input.days, price: input.estimated_usd_per_decision, source_hash: hash(source), account_hash: account_hash(&a),
            items, decisions_per_item: c.questions.len(), estimated_usd: total };
        // Random nonce makes otherwise identical previews single-use. Digest names all inputs.
        p.fingerprint = hash(serde_json::to_vec(&p).expect("preview"));
        w.state().set_meta(PREVIEW, &serde_json::to_string(&p).expect("preview"))?;
        Ok(Json(json!({"fingerprint":p.fingerprint,"days":p.days,"until_ms":until,"count":p.items.len(),"account":a.name,"model":c.model,
            "decisions_per_item":p.decisions_per_item,"unit_price":p.price,"estimated_usd":p.estimated_usd})))
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval { fingerprint: String, days: u32, estimated_usd_per_decision: f64, approve: bool }

pub async fn approve(State(w): State<Arc<Writer>>, Caller(author): Caller, headers: HeaderMap, Json(input): Json<Approval>) -> Result<Json<Value>, ApiError> {
    owner(&author)?;
    let session = session(&headers)?;
    let weak = Arc::downgrade(&w);
    blocking(w, move |w| {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let raw = w.state().meta(PREVIEW)?.ok_or_else(stale)?;
        let p: Preview = serde_json::from_str(&raw).map_err(|_| stale())?;
        let (source,c,a) = config(w)?;
        let mut l = ledger(w)?;
        if !input.approve || p.fingerprint != input.fingerprint || p.days != input.days || p.price != input.estimated_usd_per_decision
            || p.session != session || w.now_ms() >= p.expires_ms || w.now_ms() < p.until_ms
            || hash(source) != p.source_hash || account_hash(&a) != p.account_hash
            || selection(w,&c,p.days,p.until_ms)? != p.items
            || l.runs.iter().any(|r| r.status == "running" || r.preview.fingerprint == p.fingerprint) { return Err(stale()); }
        l.runs.push(Run { preview:p, status:"running".into(), attempts:vec![] });
        save(w,&l)?;
        w.state().record_activity(state::NewActivity::new("scam.history.approved", "notes", &author, "Approved fingerprint-bound historical scam run, no arrival replay"));
        spawn(weak).map_err(|_| ApiError::bad_request("Run admitted but worker unavailable. Pending items resume after restart."))?;
        Ok(Json(summary(l.runs.last().expect("run"))))
    }).await
}

fn summary(run: &Run) -> Value {
    json!({"fingerprint":run.preview.fingerprint,"status":run.status,"count":run.preview.items.len(),
        "estimated_usd":run.preview.estimated_usd,"attempts":run.attempts.iter().map(|a| json!({"id":a.item.id,"outcome":a.outcome})).collect::<Vec<_>>()})
}

pub async fn progress(State(w): State<Arc<Writer>>, Caller(author): Caller) -> Result<Json<Value>, ApiError> {
    owner(&author)?;
    blocking(w, |w| Ok::<_, ApiError>(Json(ledger(w)?.runs.last().map(summary).unwrap_or(json!({"status":"not_started","attempts":[]}))))).await
}

fn spawn(w: Weak<Writer>) -> std::io::Result<()> {
    std::thread::Builder::new().name("scam-history".into()).spawn(move || {
        loop {
            let Some(writer) = w.upgrade() else { break };
            match step(&writer) {
                Ok(true) => {},
                Ok(false) => break,
                Err(_) => { tracing::error!(target:"ai", "historical scam ledger stopped; pending items retained"); break }
            }
            drop(writer);
            std::thread::sleep(Duration::from_secs(1));
        }
    }).map(|_| ())
}

/// Called once on server startup. Uncertain calls are terminal, pending work is retained.
pub(crate) fn recover(w: &Arc<Writer>) -> Result<(), ApiError> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut l = ledger(w)?;
    for run in &mut l.runs {
        for a in &mut run.attempts {
            if a.outcome == "attempting" { a.outcome = "interrupted_no_retry".into(); }
        }
    }
    save(w,&l)?;
    if l.runs.iter().any(|r| r.status == "running") { spawn(Arc::downgrade(w)).map_err(|_| stale())?; }
    Ok(())
}

fn step(w: &Writer) -> Result<bool, ApiError> {
    let (p,item) = {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut l = ledger(w)?;
        let Some(run) = l.runs.last_mut().filter(|r| r.status == "running") else { return Ok(false) };
        let Some(item) = run.preview.items.iter().find(|i| !run.attempts.iter().any(|a| a.item.id == i.id)).cloned() else {
            run.status = "complete".into(); save(w,&l)?; return Ok(false);
        };
        let p = run.preview.clone();
        // Durable before network I/O. A crash after this write never retries this item.
        run.attempts.push(Attempt { item:item.clone(), outcome:"attempting".into() });
        save(w,&l)?;
        (p,item)
    };
    let outcome = decide(w,&p,&item).unwrap_or("failed");
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut l = ledger(w)?;
    let run = l.runs.last_mut().ok_or_else(stale)?;
    let a = run.attempts.iter_mut().find(|a| a.item.id == item.id).ok_or_else(stale)?;
    a.outcome = outcome.into();
    save(w,&l)?;
    Ok(true)
}

fn decide(w: &Writer, p: &Preview, item: &Item) -> Result<&'static str, ApiError> {
    let (source,c,a) = config(w)?;
    if hash(source) != p.source_hash || account_hash(&a) != p.account_hash { return Ok("skipped_configuration_changed"); }
    let saved = snapshot(w,&item.id)?;
    if hash(saved.wcl.as_bytes()) != item.fingerprint { return Ok("skipped_note_changed"); }
    let request = crate::classifiers::request(&saved,&c).map_err(|_| stale())?;
    let credential = Credentials::resolve(w.state(),&a).map_err(|_| stale())?;
    let result = decisions::decide(&credential,&request,Duration::from_secs(30));
    let usage = result.as_ref().map_or_else(|e| e.usage, |r| ai::Usage { input:r.usage.input_tokens, output:r.usage.output_tokens });
    w.state().record_ai_usage(&a.id,&request.model,"scam_history",usage.input,usage.output)?;
    let response = match result { Ok(r) => r, Err(_) => return Ok("provider_failed") };
    crate::classifiers::apply(w,&saved,&c,&response).map_err(|_| stale())?;
    Ok("succeeded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_owner_sessions_can_approve_not_scripts_tasks_tokens_or_agents() {
        for author in [Author::Token("admin".into()), Author::Agent("agent-test".into()),
            Author::Handler("script".into()), Author::Task("scheduled-script".into()), Author::AgentMail("mail".into())] {
            assert!(owner(&author).is_err());
        }
        assert!(owner(&Author::Session("owner".into())).is_ok());
    }

    #[test]
    fn historical_routes_are_human_only() {
        for path in ["/api/classifiers/scam/history", "/api/classifiers/scam/history/preview", "/api/classifiers/scam/history/approve"] {
            assert!(crate::auth::is_human_only(&axum::http::Method::POST, path));
        }
    }
}
