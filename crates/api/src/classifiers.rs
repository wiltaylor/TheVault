//! Home-server-only, one-shot classification after a committed create. Never called by an app.
//! The hook reads type selectors and queues snapshots. HTTP runs on the worker outside locks.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, Weak, mpsc};
use std::time::{Duration, Instant};

use ai::{Accounts, AiError, Credentials, ErrorKind, decisions};
use query::{BinOp, Expr, From, Lit, PageRequest, Query};
use schema::{Classifier, Doc, Schema};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use state::Author;
use writer::{Event, Hooks, Migration, NoteChange, Saved, ScriptRole, Writer};

struct ClassifierHooks {
    inner: Box<dyn Hooks>,
    sender: mpsc::SyncSender<()>,
    writer: Arc<OnceLock<Weak<Writer>>>,
}

pub struct Binding(mpsc::Receiver<()>, Arc<OnceLock<Weak<Writer>>>);

pub fn wrap(inner: Box<dyn Hooks>) -> (Box<dyn Hooks>, Binding) {
    let (sender, receiver) = mpsc::sync_channel(1);
    let writer = Arc::new(OnceLock::new());
    (
        Box::new(ClassifierHooks {
            inner,
            sender,
            writer: writer.clone(),
        }),
        Binding(receiver, writer),
    )
}

impl Binding {
    /// Holds only a weak writer while idle, so dropping the server releases the worker.
    pub fn start(self, writer: &Arc<Writer>) -> std::io::Result<()> {
        crate::safety_backfill::recover(writer).map_err(std::io::Error::other)?;
        crate::scam_history::recover(writer)
            .map_err(|_| std::io::Error::other("historical scam recovery failed"))?;
        writer
            .activate_agent_safety()
            .map_err(std::io::Error::other)?;
        let writer = Arc::downgrade(writer);
        let _ = self.1.set(writer.clone());
        std::thread::Builder::new()
            .name("classifiers".into())
            .spawn(move || {
                let mut invalid = HashMap::new();
                loop {
                    let Some(writer) = writer.upgrade() else {
                        break;
                    };
                    match writer.state().take_classification() {
                        Ok(Some(payload)) => {
                            match serde_json::from_str::<Pending>(&payload) {
                                Ok(pending) => {
                                    let safety_only = pending.safety_only;
                                    classify(&writer, &pending.saved(), safety_only, &mut invalid)
                                },
                                Err(_) => tracing::error!(target: "ai", "classifier: outcome=error:pending_payload"),
                            }
                            continue;
                        }
                        Err(_) => tracing::error!(target: "ai", "classifier: outcome=error:pending_read"),
                        Ok(None) => {}
                    }
                    drop(writer);
                    if self.0.recv().is_err() {
                        break;
                    }
                }
            })?;
        Ok(())
    }
}

impl Hooks for ClassifierHooks {
    fn before(&self, change: &mut NoteChange, author: &Author) -> Result<(), String> {
        self.inner.before(change, author)
    }
    fn side_effects(&self, changes: &[NoteChange]) -> writer::Result<Vec<NoteChange>> {
        self.inner.side_effects(changes)
    }
    fn after(&self, saved: &[Saved], author: &Author, schema: &Schema) {
        let Some(writer) = self.writer.get().and_then(Weak::upgrade) else {
            self.inner.after(saved, author, schema);
            return;
        };
        self.inner.after(saved, author, schema);
        let types = classifier_types(&writer);
        let payloads: Vec<_> = saved
            .iter()
            .flat_map(|s| std::iter::once((s, false)).chain(s.also.iter().map(|s| (s, true))))
            .filter(|(s, _)| {
                s.event == Event::Create && !s.restored && types.contains(&s.type_name)
            })
            .map(|(s, safety_only)| {
                serde_json::to_string(&Pending {
                    id: s.id.clone(),
                    type_name: s.type_name.clone(),
                    version: s.version.clone(),
                    wcl: s.wcl.clone(),
                    safety_only,
                })
                .expect("pending snapshot")
            })
            .collect();
        if payloads.is_empty() {
            return;
        }
        let queued = writer.state().queue_classifications(&payloads).is_ok();
        if queued {
            let safety_types = writer
                .vault()
                .read_text("classifiers/agent_safety.wcl")
                .ok()
                .and_then(|s| Classifier::parse("agent_safety", &s).ok())
                .map(|c| c.types)
                .unwrap_or_default();
            for s in saved.iter().flat_map(|s| std::iter::once(s).chain(&s.also)) {
                if s.event == Event::Create
                    && !s.restored
                    && safety_types.contains(&s.type_name)
                    && let Ok(doc) = Doc::parse(&s.wcl, "incoming note")
                    && writer
                        .register_agent_safety(&s.id, &s.type_name, &doc)
                        .is_err()
                {
                    tracing::error!(target: "ai", "agent safety registration failed note={}", s.id);
                }
            }
            // The bounded channel is just a coalescing wakeup, never the work queue.
            let _ = self.sender.try_send(());
        } else {
            for note in saved
                .iter()
                .filter(|s| s.event == Event::Create && !s.restored && types.contains(&s.type_name))
            {
                tracing::error!(target: "ai", "classifier: note={} outcome=error:pending_write", note.id);
            }
        }
    }
    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> Result<Option<Doc>, String> {
        self.inner.migrate(type_name, migration, id, doc)
    }
    fn asset_known(&self, id: &str) -> bool {
        self.inner.asset_known(id)
    }
    fn check_script(&self, role: ScriptRole, name: &str, source: &str) -> Result<(), String> {
        self.inner.check_script(role, name, source)
    }
}

fn classifier_types(writer: &Writer) -> HashSet<String> {
    std::fs::read_dir(writer.vault().root().join("classifiers"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name();
            let name = file.to_str()?.strip_suffix(".wcl")?;
            let source = writer
                .vault()
                .read_text(&format!("classifiers/{name}.wcl"))
                .ok()?;
            Classifier::parse(name, &source).ok()
        })
        .flat_map(|config| config.types)
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    #[serde(default)]
    safety_only: bool,
    id: String,
    type_name: String,
    version: String,
    wcl: String,
}

impl Pending {
    fn saved(self) -> Saved {
        Saved {
            id: self.id,
            type_name: self.type_name,
            version: self.version,
            wcl: self.wcl,
            event: Event::Create,
            restored: false,
            previous: None,
            author: None,
            also: vec![],
        }
    }
}

fn classify(
    writer: &Writer,
    saved: &Saved,
    safety_only: bool,
    invalid: &mut HashMap<String, Option<[u8; 32]>>,
) {
    let Ok(entries) = std::fs::read_dir(writer.vault().root().join("classifiers")) else {
        eprintln!("classifier: note={} outcome=error:definitions", saved.id);
        return;
    };
    let mut names: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".wcl")
                .filter(|n| schema::is_name(n))
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    invalid.retain(|name, _| names.contains(name));
    for name in names {
        if safety_only && name != "agent_safety" {
            continue;
        }
        let source = writer
            .vault()
            .read_text(&format!("classifiers/{name}.wcl"))
            .ok();
        let fingerprint = source.as_ref().map(|s| Sha256::digest(s.as_bytes()).into());
        let config = source
            .as_ref()
            .and_then(|s| Classifier::parse(&name, s).ok());
        let Some(config) = config else {
            if invalid.get(&name) != Some(&fingerprint) {
                invalid.insert(name.clone(), fingerprint);
                tracing::warn!(target: "ai",
                    "classifier: name={name} note={} outcome=error:definition",
                    saved.id
                );
            }
            continue;
        };
        invalid.remove(&name);
        if !config.types.contains(&saved.type_name) {
            continue;
        }
        if config.name == "agent_safety"
            && !Doc::parse(&saved.wcl, "safety scope")
                .is_ok_and(|doc| writer.incoming_agent_note(&saved.type_name, &doc))
        {
            continue;
        }
        match matches(writer, saved, &config) {
            Ok(false) => continue,
            Err(_) => {
                eprintln!(
                    "classifier: name={name} note={} outcome=error:filter",
                    saved.id
                );
                continue;
            }
            Ok(true) => {}
        }
        run(writer, saved, &config);
    }
}

pub(crate) fn matches(
    writer: &Writer,
    saved: &Saved,
    config: &Classifier,
) -> Result<bool, AiError> {
    let Some(condition) = &config.condition else {
        return Ok(true);
    };
    let filter =
        query::parse_expr(condition).map_err(|_| AiError::invalid("Invalid classifier filter"))?;
    let q = Query {
        from: From::Types(config.types.clone()),
        include_archived: true,
        filter: Some(Expr::Binary(
            BinOp::And,
            Box::new(filter),
            Box::new(Expr::Binary(
                BinOp::Eq,
                Box::new(Expr::Name("id".into())),
                Box::new(Expr::Lit(Lit::Str(saved.id.clone()))),
            )),
        )),
        search: None,
        sort: vec![],
        group_by: None,
        select: vec![],
        limit: Some(1),
    };
    writer
        .read_index(|index, schema| {
            query::run(
                &q,
                &query::Context::new(index, schema)
                    .with_this(&saved.id)
                    .at(writer.now_ms() as f64 / 1000.0),
                &PageRequest {
                    cursor: None,
                    size: 1,
                },
            )
        })
        .map(|p| !p.rows.is_empty())
        .map_err(|_| AiError::invalid("Classifier filter failed"))
}

pub(crate) fn request(saved: &Saved, config: &Classifier) -> Result<decisions::Request, AiError> {
    let doc = Doc::parse(&saved.wcl, "classifier input")
        .map_err(|_| AiError::invalid("Invalid classifier input"))?;
    let note = writer::doc_to_json(&doc);
    let mut state = Map::new();
    for field in &config.fields {
        // Named content blocks are available alongside top-level note fields.
        let value = note["fields"].get(field).cloned().unwrap_or_else(|| {
            Value::Array(
                note["blocks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["labels"][0].as_str() == Some(field))
                    .cloned()
                    .collect(),
            )
        });
        state.insert(field.clone(), value);
    }
    let questions = config
        .questions
        .iter()
        .map(|q| {
            let mut value = json!({"type": q.kind, "instructions": q.instructions});
            if let Some(criteria) = &q.criteria {
                value["criteria"] = criteria.clone();
            }
            serde_json::from_value(value)
                .map(|value| (q.name.clone(), value))
                .map_err(|_| AiError::invalid("Invalid classifier question"))
        })
        .collect::<Result<_, _>>()?;
    Ok(decisions::Request {
        model: config.model.clone(),
        state: Value::Object(state),
        questions,
    })
}

fn run(writer: &Writer, saved: &Saved, config: &Classifier) -> Option<String> {
    let safety_envelope = (config.name == "agent_safety")
        .then(|| {
            Doc::parse(&saved.wcl, "safety snapshot")
                .ok()
                .map(|doc| writer.agent_safety_envelope(&saved.type_name, &doc))
        })
        .flatten();
    let safety_hash = safety_envelope
        .as_ref()
        .map(writer::agent_safety::envelope_fingerprint);
    if let (Some(envelope), Some(hash)) = (&safety_envelope, &safety_hash)
        && envelope.to_string().len() > writer::agent_safety::ENVELOPE_LIMIT
    {
        let _ = writer.finish_agent_safety(&saved.id, hash, "held", "too_large");
        last_outcome(writer, config, "held_too_large");
        return None;
    }
    let prepared = (|| {
        let accounts = Accounts::load(writer.state())?;
        let account = accounts.pick(config.account.as_deref(), None)?;
        // Reject incompatible accounts before resolving credentials (ChatGPT may refresh).
        if account.kind != ai::Kind::OpenAiCompatible {
            return Err(AiError::invalid(
                "Decisions require an OpenAI-compatible account",
            ));
        }
        let credential = Credentials::resolve(writer.state(), account)?;
        decisions::validate_credential(&credential)?;
        let mut request = request(saved, config)?;
        if let Some(envelope) = &safety_envelope {
            // Exactly the projection used by the clearance fingerprint.
            request.state = envelope.clone();
        }
        Ok::<_, AiError>((credential, request))
    })();
    let (credential, request) = match prepared {
        Ok(p) => p,
        Err(e) => {
            last_outcome(writer, config, "setup_failed");
            if let Some(hash) = &safety_hash {
                let _ = writer.finish_agent_safety(&saved.id, hash, "held", "decision_failed");
            }
            configuration_warning(writer, config, &e);
            eprintln!(
                "classifier: name={} note={} outcome=error:{}",
                config.name, saved.id, e.kind
            );
            return None;
        }
    };
    let began = Instant::now();
    // Persist before sending. An interrupted attempt is never represented as a success.
    last_outcome(writer, config, "attempting");
    let mut result = decisions::decide(&credential, &request, Duration::from_secs(30));
    let mut outcome = if result.is_ok() {
        "succeeded"
    } else {
        "provider_failed"
    };
    let usage = result.as_ref().map_or_else(
        |e| e.usage,
        |r| ai::Usage {
            input: r.usage.input_tokens,
            output: r.usage.output_tokens,
        },
    );
    if let Err(e) = writer.state().record_ai_usage(
        &credential.account,
        &request.model,
        "classifier",
        usage.input,
        usage.output,
    ) {
        eprintln!("ai: recording classifier usage: {e}");
    }
    match &result {
        Ok(_) => {
            let _ = writer
                .state()
                .set_meta(&format!("classifier_config_warning:{}", config.name), "");
        }
        Err(error) if matches!(error.kind, ErrorKind::Auth | ErrorKind::NotConfigured) => {
            configuration_warning(writer, config, error)
        }
        Err(_) => {}
    }
    let mut version = None;
    if let Ok(response) = &result {
        match apply(writer, saved, config, response) {
            Ok(saved) => version = Some(saved.version),
            Err(_) => {
                outcome = "write_failed";
                let mut error = AiError::new(
                    ErrorKind::Invalid,
                    "Classifier write failed or note changed",
                );
                error.usage = ai::Usage {
                    input: response.usage.input_tokens,
                    output: response.usage.output_tokens,
                };
                result = Err(error);
            }
        }
    }
    if let Some(hash) = &safety_hash {
        let (status, reason) = match &result {
            Ok(response) => match response.answers.get("reason") {
                Some(decisions::Answer::Choice { choice, .. }) => (
                    if config.withhold.contains(choice) {
                        "held"
                    } else {
                        "allowed"
                    },
                    choice.as_str(),
                ),
                _ => ("held", "invalid_decision"),
            },
            Err(_) => ("held", "decision_failed"),
        };
        let _ = writer.finish_agent_safety(&saved.id, hash, status, reason);
    }
    last_outcome(writer, config, outcome);
    tracing::info!(target: "ai",
        "{} classifier={} note={}",
        decisions::trail(&credential, &request, &result, began.elapsed()),
        config.name,
        saved.id
    );
    if let (Ok(response), Some(rule)) = (&result, &config.notify)
        && response.answers[&rule.question]
            .notify_probability(rule.option.as_deref())
            .is_some_and(|p| p >= rule.threshold)
    {
        let notification = state::NewNotification {
            title: rule.title.clone(),
            body: None,
            note_id: Some(saved.id.clone()),
            url: None,
            source: format!("classifier:{}", config.name),
        };
        if writer.notify(&notification).is_err() {
            eprintln!(
                "classifier: name={} note={} outcome=error:notification",
                config.name, saved.id
            );
        }
    }
    version
}

fn last_outcome(writer: &Writer, config: &Classifier, outcome: &str) {
    if writer
        .state()
        .set_meta(&format!("classifier_last_outcome:{}", config.name), outcome)
        .is_err()
    {
        tracing::error!(target: "ai", "classifier: outcome=error:diagnostic_write");
    }
}

fn configuration_warning(writer: &Writer, config: &Classifier, error: &AiError) {
    let key = format!("classifier_config_warning:{}", config.name);
    let kind = error.kind.to_string();
    if writer.state().meta(&key).ok().flatten().as_deref() == Some(&kind) {
        return;
    }
    let warning = state::NewNotification {
        title: "Classifier account needs attention".into(),
        body: Some(if config.name == "agent_safety" {
            "Agent safety could not start. Select an OpenRouter account in Settings > AI, or name that account in classifiers/agent_safety.wcl. Incoming notes remain held from agents until cleared or explicitly released.".into()
        } else {
            format!(
                "Classifier {} could not start. Select an OpenRouter account in Settings > AI, or name that account in the classifier file. Unclassified notes must still be triaged as before.",
                config.name
            )
        }),
        note_id: None,
        url: Some("/settings/ai".into()),
        source: format!("classifier_config:{}", config.name),
    };
    if writer.notify(&warning).is_ok() {
        let _ = writer.state().set_meta(&key, &kind);
    }
}

pub(crate) fn apply(
    writer: &Writer,
    saved: &Saved,
    config: &Classifier,
    response: &decisions::Response,
) -> writer::Result<Saved> {
    let answers = config
        .questions
        .iter()
        .map(|question| {
            (
                question.target.clone(),
                response.answers[&question.name].value(),
            )
        })
        .collect::<Vec<_>>();
    let original = writer::doc_to_json(
        &Doc::parse(&saved.wcl, "classifier base").map_err(writer::Error::Invalid)?,
    );
    let base = answers
        .iter()
        .map(|(field, _)| (field.clone(), original["fields"][field].clone()))
        .collect::<Vec<_>>();
    writer.classify(
        &saved.id,
        &answers,
        &base,
        &Author::Agent(format!("classifier_{}", config.name)),
    )
}
