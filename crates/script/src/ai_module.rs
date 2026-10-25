//! The `ai` host module: `ai::complete` and `ai::chat` for `after_*` handlers and scheduled tasks
//! (ai D11, D12, D13).
//!
//! The module reads the AI accounts and their credentials from `state.db` itself, through
//! `crates/ai`: a script lists neither the `ai_*` secrets in `secrets` nor the providers' hosts in
//! `allow_domains`, and no function hands it a key. The `account` option names the account, by
//! id or name; without it a call uses the default account (accounts A5). The `provider` option
//! of before accounts (`anthropic`, `openai` or `chatgpt`) still works: it names the account the
//! migration made for that provider. Calls block and end at the run's deadline,
//! as `http` calls do. Every call, whatever its outcome, appends one trail line to the run log
//! (ai D12), which never holds the prompt.
//!
//! On the embedded host of a device, [`Ai::unavailable`], the module is registered so device
//! vaults' scripts compile, and every call returns `Err` with kind `unavailable`: the device holds
//! no AI credentials.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use ::ai::{Accounts, Credentials, Effort, ErrorKind, Kind, Message, Outcome, Request, Usage};
use reqwest::blocking::Client;
use serde_json::Value as Json;
use writer::Writer;
use wscript::std_modules::DynValue;
use wscript::{Fault, HostError, Module, Script};

use crate::{convert, run};

/// A finished `ai` call.
#[derive(Script, Debug, Clone)]
#[script(name = "Reply")]
pub struct Reply {
    /// The model's whole answer.
    pub text: String,
    /// With `schema` set: the answer parsed as JSON and checked against the schema. `null`
    /// otherwise.
    pub json: DynValue,
    /// The name of the account that answered.
    pub account: String,
    /// The account's kind: `anthropic`, `openai`, `openai_compatible` or `chatgpt`.
    pub kind: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// `end_turn`, or `max_tokens` when the output cap cut the answer off.
    pub stop: String,
}

/// Why an `ai` call failed. Fields the failure does not know are empty.
#[derive(Script, Debug, Clone, PartialEq)]
#[script(name = "AiError")]
pub struct AiError {
    /// `not_configured`, `auth`, `rate_limit`, `refusal`, `invalid`, `timeout`, `unavailable` or
    /// `provider`.
    pub kind: String,
    pub message: String,
    /// The name of the account the call went to, when one was chosen.
    pub account: String,
    pub model: String,
    /// A refusal's category, such as `cyber`, when the provider named one.
    pub category: String,
}

/// One turn of the conversation `ai::chat` sends.
#[derive(Script, Debug, Clone)]
#[script(name = "ChatMessage")]
pub struct ChatMessage {
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
}

type Answer = Fault<Result<Reply, AiError>>;

/// The writer a host's `ai` module reads credentials from, set once it exists.
type Binding = Arc<OnceLock<Weak<Writer>>>;

/// Where the `ai` module reads its credentials and settings (ai D13).
#[derive(Clone)]
pub struct Ai(Option<(Binding, Client)>);

impl Ai {
    /// Credentials from `writer`'s `state.db`. Build it outside an async runtime: its client
    /// blocks.
    pub fn of(writer: &Arc<Writer>) -> Ai {
        let binding: Binding = Arc::new(OnceLock::new());
        let _ = binding.set(Arc::downgrade(writer));
        Ai::bound(binding)
    }

    /// Credentials from the writer `binding` is set to, once it is.
    pub(crate) fn bound(binding: Binding) -> Ai {
        Ai(Some((binding, ::ai::blocking_client())))
    }

    /// No AI at all: every call returns `Err` with kind `unavailable`. The embedded host of a
    /// device, which holds no AI credentials.
    pub fn unavailable() -> Ai {
        Ai(None)
    }
}

pub fn module(access: Ai) -> Module {
    let mut m = Module::new("ai");
    m.doc(
        "Large language models with the credentials of Settings > AI (ai D13). Every call adds a \
         trail line to the run log",
    );
    let a = access.clone();
    m.fn_named(
        "complete",
        ["prompt", "options"],
        move |prompt: String, options: HashMap<String, String>| -> Answer {
            Fault(call(&a, vec![Message::user(prompt)], &options))
        },
    );
    m.fn_named(
        "chat",
        ["messages", "options"],
        move |messages: Vec<ChatMessage>, options: HashMap<String, String>| -> Answer {
            Fault(match history(messages) {
                Ok(h) => call(&access, h, &options),
                Err(e) => refused_early(&options, e),
            })
        },
    );
    m
}

/// `messages` as the transcript a request sends, in order.
fn history(messages: Vec<ChatMessage>) -> Result<Vec<Message>, ::ai::AiError> {
    if messages.is_empty() {
        return Err(::ai::AiError::invalid(
            "ai::chat takes at least one message",
        ));
    }
    messages
        .into_iter()
        .enumerate()
        .map(|(i, m)| match m.role.trim() {
            "user" => Ok(Message::user(m.text)),
            "assistant" => Ok(Message::assistant(m.text)),
            other => Err(::ai::AiError::invalid(format!(
                "message {} has role {other:?}: a role is user or assistant",
                i + 1
            ))),
        })
        .collect()
}

/// What a call's `options` ask for. Unset means the default account, or its default model.
#[derive(Debug, Default)]
struct Options {
    account: Option<String>,
    model: Option<String>,
    system: Option<String>,
    schema: Option<Json>,
    effort: Option<Effort>,
    max_tokens: Option<u32>,
}

/// The option names `ai::complete` and `ai::chat` take.
const OPTIONS: [&str; 7] = [
    "account",
    "provider",
    "model",
    "system",
    "schema",
    "effort",
    "max_tokens",
];

impl Options {
    /// Read `options`, where an empty value counts as unset. The account is read first, so an
    /// error in a later option still names it. A legacy `provider` must be one of the three
    /// names of before accounts.
    fn parse(options: &HashMap<String, String>) -> (Option<String>, Result<Options, String>) {
        let get = |name: &str| {
            options
                .get(name)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
        };
        let account = match (get("account"), get("provider")) {
            (Some(a), _) => Some(a.to_owned()),
            (None, Some(p)) if ["anthropic", "openai", "chatgpt"].contains(&p) => {
                Some(p.to_owned())
            }
            (None, Some(p)) => {
                return (
                    None,
                    Err(format!(
                        "unknown AI provider {p:?}: expected anthropic, openai or chatgpt, or name \
                         an account with the account option"
                    )),
                );
            }
            (None, None) => None,
        };
        let parsed = (|| {
            let mut unknown: Vec<&str> = options
                .keys()
                .map(String::as_str)
                .filter(|k| !OPTIONS.contains(k))
                .collect();
            if !unknown.is_empty() {
                unknown.sort_unstable();
                return Err(format!(
                    "unknown option {}: the options are {}",
                    unknown.join(", "),
                    OPTIONS.join(", ")
                ));
            }
            let schema = match get("schema") {
                None => None,
                Some(text) => match serde_json::from_str::<Json>(text) {
                    Ok(s @ Json::Object(_)) => Some(s),
                    Ok(_) => return Err("schema must be a JSON Schema object".to_owned()),
                    Err(e) => return Err(format!("schema is not JSON: {e}")),
                },
            };
            let max_tokens = match get("max_tokens") {
                None => None,
                Some(text) => match text.parse::<u32>() {
                    Ok(n) if n > 0 => Some(n),
                    _ => {
                        return Err(format!(
                            "max_tokens is {text:?}: it must be a whole number above 0"
                        ));
                    }
                },
            };
            Ok(Options {
                account: account.clone(),
                model: get("model").map(str::to_owned),
                // The system prompt is kept as written, spaces and all.
                system: options
                    .get("system")
                    .filter(|s| !s.trim().is_empty())
                    .cloned(),
                schema,
                effort: get("effort").map(Effort::from_str).transpose()?,
                max_tokens,
            })
        })();
        (account, parsed)
    }
}

/// A call that failed before reaching a provider, because of its arguments.
fn refused_early(
    options: &HashMap<String, String>,
    e: ::ai::AiError,
) -> Result<Result<Reply, AiError>, HostError> {
    run::with(|r| r.remaining())?;
    let model = options.get("model").map(|m| m.trim()).unwrap_or_default();
    finish(None, model, Err(e), Duration::ZERO)
}

/// Make one call. `Ok(Err(..))` is a failure the script sees; `Err(..)` faults the run, which
/// only running out of time does.
fn call(
    access: &Ai,
    messages: Vec<Message>,
    options: &HashMap<String, String>,
) -> Result<Result<Reply, AiError>, HostError> {
    let deadline = run::with(|r| {
        r.remaining()?;
        Ok(r.deadline)
    })?;
    let began = Instant::now();
    let (_, parsed) = Options::parse(options);
    let model = options.get("model").map(|m| m.trim()).unwrap_or_default();
    let Some((binding, client)) = &access.0 else {
        let e = ::ai::AiError::new(
            ErrorKind::Unavailable,
            "AI is not available on this device: a script that calls ai runs on the home server",
        );
        return finish(None, model, Err(e), began.elapsed());
    };
    let opts = match parsed {
        Ok(o) => o,
        Err(message) => {
            let e = ::ai::AiError::invalid(message);
            return finish(None, model, Err(e), began.elapsed());
        }
    };
    let Some(writer) = binding.get().and_then(Weak::upgrade) else {
        let e = ::ai::AiError::new(ErrorKind::Unavailable, "the vault is not open");
        return finish(None, model, Err(e), began.elapsed());
    };
    let state = writer.state();
    let account =
        Accounts::load(state).and_then(|all| all.pick(opts.account.as_deref(), None).cloned());
    let account = match account {
        Ok(a) => a,
        Err(e) => return finish(None, model, Err(e), began.elapsed()),
    };
    let cred = match Credentials::resolve(state, &account) {
        Ok(c) => c,
        Err(e) => return finish(Some((&account, None)), model, Err(e), began.elapsed()),
    };
    let model = match opts
        .model
        .clone()
        .or_else(|| account.model().map(str::to_owned))
    {
        Some(m) => m,
        None => match ::ai::default_model_blocking(client, &cred, Some(deadline)) {
            Ok(m) => m,
            Err(e) => return finish(Some((&account, Some(&cred))), "", Err(e), began.elapsed()),
        },
    };
    let req = Request {
        model: model.clone(),
        system: opts.system,
        messages,
        tools: Vec::new(),
        schema: opts.schema,
        effort: opts.effort,
        max_tokens: opts.max_tokens,
    };
    let result = ::ai::complete_blocking(client, &cred, &req, Some(deadline));
    if let Ok(reply) = &result
        && let Some(usage) = &reply.plan_usage
        && let Err(e) = usage.save(state, &account.id)
    {
        eprintln!("ai: keeping the plan usage of {}: {e}", account.name);
    }
    finish(
        Some((&account, Some(&cred))),
        &model,
        result,
        began.elapsed(),
    )
}

/// Log the call's trail line, end the run when the call outlasted its deadline, and hand the
/// script its answer. `to` is the account the call went to, with its credential once resolved,
/// whose custom host the trail line names (ai D24).
fn finish(
    to: Option<(&::ai::Account, Option<&::ai::Credential>)>,
    model: &str,
    result: Result<::ai::Reply, ::ai::AiError>,
    took: Duration,
) -> Result<Result<Reply, AiError>, HostError> {
    let line = match to {
        Some((_, Some(cred))) => ::ai::trail_for(cred, model, &result, took),
        Some((a, None)) => {
            let usage = result.as_ref().map_or_else(|e| e.usage, |r| r.usage);
            ::ai::trail_line(
                &a.name,
                a.kind,
                model,
                usage,
                Outcome::of(&result),
                took,
                None,
            )
        }
        None => no_account_trail(model, &result, took),
    };
    run::with(|r| {
        r.log(&line, true);
        Ok(())
    })?;
    // A call cut short by the run's own deadline ends the run, as an `http` call does.
    run::with(|r| r.remaining())?;
    let name = to.map(|(a, _)| a.name.clone()).unwrap_or_default();
    Ok(match result {
        Ok(reply) => Ok(Reply {
            text: reply.text,
            json: reply.json.map_or(DynValue::Null, convert::to_dyn),
            account: reply.name,
            kind: reply.kind.as_str().to_owned(),
            model: reply.model,
            input_tokens: tokens(reply.usage.input),
            output_tokens: tokens(reply.usage.output),
            stop: reply.stop.as_str().to_owned(),
        }),
        Err(e) => Err(AiError {
            kind: e.kind.as_str().to_owned(),
            message: e.message,
            account: e.account.map_or(name, String::from),
            model: e.model.map_or_else(|| model.to_owned(), String::from),
            category: e.category.map(String::from).unwrap_or_default(),
        }),
    })
}

/// The trail line of a call that failed before an account was chosen: `account=- kind=-`.
fn no_account_trail(
    model: &str,
    result: &Result<::ai::Reply, ::ai::AiError>,
    took: Duration,
) -> String {
    let line = ::ai::trail_line(
        "",
        Kind::Anthropic,
        model,
        Usage::default(),
        Outcome::of(result),
        took,
        None,
    );
    line.replacen("account=- kind=anthropic", "account=- kind=-", 1)
}

fn tokens(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}
