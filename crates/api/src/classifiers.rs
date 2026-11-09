//! Home-server-only, one-shot classification after a committed create. Never called by an app.
//! The hook only queues snapshots. The worker reads definitions and calls HTTP outside locks.

use std::collections::HashMap;
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
                                Ok(pending) => classify(&writer, &pending.saved(), &mut invalid),
                                Err(_) => tracing::error!(target: "ai", "classifier: outcome=error:pending_payload"),
                            }
                            continue;
                        }
                        Err(_) => tracing::error!(target: "ai", "classifier: outcome=error:pending_read"),
                        Ok(None) => {}
                    }
                    drop(writer);
                    if self.0.recv_timeout(Duration::from_secs(1)) == Err(mpsc::RecvTimeoutError::Disconnected) {
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
        self.inner.after(saved, author, schema);
        let payloads: Vec<_> = saved
            .iter()
            .filter(|s| s.event == Event::Create && !s.restored)
            .map(|s| {
                serde_json::to_string(&Pending {
                    id: s.id.clone(),
                    type_name: s.type_name.clone(),
                    version: s.version.clone(),
                    wcl: s.wcl.clone(),
                })
                .expect("pending snapshot")
            })
            .collect();
        if payloads.is_empty() {
            return;
        }
        let queued = self
            .writer
            .get()
            .and_then(Weak::upgrade)
            .is_some_and(|writer| writer.state().queue_classifications(&payloads).is_ok());
        if queued {
            // The bounded channel is just a coalescing wakeup, never the work queue.
            let _ = self.sender.try_send(());
        } else {
            for note in saved
                .iter()
                .filter(|s| s.event == Event::Create && !s.restored)
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

#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
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

fn classify(writer: &Writer, saved: &Saved, invalid: &mut HashMap<String, Option<[u8; 32]>>) {
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

fn matches(writer: &Writer, saved: &Saved, config: &Classifier) -> Result<bool, AiError> {
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

fn request(saved: &Saved, config: &Classifier) -> Result<decisions::Request, AiError> {
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
    let prepared = (|| {
        let accounts = Accounts::load(writer.state())?;
        let account = accounts.pick(config.account.as_deref(), None)?;
        let credential = Credentials::resolve(writer.state(), account)?;
        Ok::<_, AiError>((credential, request(saved, config)?))
    })();
    let (credential, request) = match prepared {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "classifier: name={} note={} outcome=error:{}",
                config.name, saved.id, e.kind
            );
            return None;
        }
    };
    let began = Instant::now();
    let mut result = decisions::decide(&credential, &request, Duration::from_secs(30));
    let mut version = None;
    if let Ok(response) = &result {
        match apply(writer, saved, config, response) {
            Ok(saved) => version = Some(saved.version),
            Err(_) => {
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

fn apply(
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
