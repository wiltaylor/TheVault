//! The server's wscript host: the engine from `script-core` with the server's modules, and the
//! `before_*` and `after_*` handlers (D6, D9, D10, D30, notes/01).
//!
//! What a script can reach depends on the [`Kind`] of run:
//!
//! | Module | `before_*` handlers, migrations | `after_*` handlers | tasks |
//! |--------|---------------------------------|--------------------|-------|
//! | `vault` | yes | yes | yes |
//! | `http` | no | yes | yes |
//! | `ai` | no | yes | yes |
//! | `secrets` (read and write) | no | yes | yes |
//! | `fs` | no | no | yes |
//! | `value`, `json`, `math`, `regex` | yes | yes | yes |
//!
//! Device scripts compile against the `vault` module and a `device` module that refuses every
//! call: they run on a paired phone, never here (D6).
//!
//! `docs/scripts.md` lists every host function. wscript's own `fs`, `process` and `time` modules
//! are never registered: this host's `fs` replaces wscript's, checked against the task's
//! `read_paths` and `write_paths`. A module a kind of run lacks is a compile error, not a runtime
//! one: HTTP in a `before_*` handler does not compile (D30).
//!
//! Changes go through `writer`, whose [`Vault`] implementation is the write path (D13, D30). HTTP
//! calls reach only the hosts on the job's allowlist, or any host when it holds `"*"`, and never
//! follow redirects (D10). A script reads only the secrets its job names, and cannot list them
//! (D10). File access reaches only the job's directories, never the vault itself. The `ai`
//! module reads the AI credentials itself, so a script names neither them nor the providers'
//! hosts, and can never read a key (ai D13).

mod ai_module;
mod fs;
mod handlers;
mod http;

use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use wscript::std_modules::DynValue;
use wscript::{Fault, HostError, Module};

pub use ai_module::{Ai, AiError, ChatMessage, Reply as AiReply};
pub use handlers::{Handlers, MAX_DEPTH};
pub use http::Response;
pub use script_core::*;

/// The wscript host of one vault, with the server's modules.
pub struct Host(script_core::Host);

impl Host {
    /// A host whose scripts reach `vault`, run on `workers` threads, bounded by `limits`. Its
    /// `ai` module reads its credentials through `ai`.
    ///
    /// Build it outside an async runtime: its HTTP clients block.
    pub fn new(vault: Arc<dyn Vault>, workers: usize, limits: Limits, ai: Ai) -> Host {
        Host(script_core::Host::with_extensions(
            vault,
            workers,
            limits,
            &Server {
                http: http::client(),
                ai,
            },
        ))
    }
}

impl Deref for Host {
    type Target = script_core::Host;

    fn deref(&self) -> &script_core::Host {
        &self.0
    }
}

/// The server's modules: `http`, `ai` and `secrets` for `after_*` handlers and tasks, `fs` for
/// tasks.
struct Server {
    http: reqwest::blocking::Client,
    ai: Ai,
}

impl Extensions for Server {
    fn vault(&self, m: &mut Module, v: &Arc<dyn Vault>) {
        let vv = v.clone();
        m.fn_named(
            "upload_asset_file",
            ["path"],
            move |path: String| -> Reply<DynValue> {
                let bytes = match fs::read_allowed(&path, &vv.protected()) {
                    Ok(Ok(b)) => b,
                    Ok(Err(e)) => return Fault(Ok(Err(e))),
                    Err(e) => return Fault(Err(e)),
                };
                Fault(host_call(|_, _| {
                    let name = Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .ok_or_else(|| format!("{path:?} names no file"))?;
                    vv.upload_asset(&name, bytes).map(convert::to_dyn)
                }))
            },
        );
    }

    fn modules(&self, kind: Kind, vault: &Arc<dyn Vault>) -> Vec<Module> {
        let side_effects = || {
            vec![
                http::module(self.http.clone()),
                ai_module::module(self.ai.clone()),
                secrets_module(vault.clone()),
            ]
        };
        match kind {
            Kind::After => side_effects(),
            Kind::Task => {
                let mut m = side_effects();
                m.push(fs::module(vault.clone()));
                m
            }
            // Device scripts run on a phone; the server only checks that they compile.
            Kind::Device => vec![script_core::device_module(Arc::new(DeviceAccess::new(
                "",
                Arc::new(NoDevice),
                Switches::default(),
            )))],
            Kind::Before | Kind::Migration => Vec::new(),
        }
    }
}

/// The largest value `secrets::set` stores (google-sync D21).
pub const MAX_SECRET_VALUE: usize = 64 * 1024;

/// Whether the running job names secret `name`.
fn granted(name: &str) -> Result<bool, HostError> {
    run::with(|r| {
        r.remaining()?;
        Ok(r.job.secrets.iter().any(|s| s == name))
    })
}

fn secrets_module(v: Arc<dyn Vault>) -> Module {
    let mut m = Module::new("secrets");
    m.doc("Read and write the secrets this script was granted, by name (D10, google-sync D21)");
    let w = v.clone();
    m.fn_named(
        "set",
        ["name", "value"],
        move |name: String, value: String| -> Reply<()> {
            Fault(granted(&name).map(|granted| {
                if !granted {
                    return Err(format!("secret {name:?} is not granted to this script"));
                }
                if value.len() > MAX_SECRET_VALUE {
                    return Err(format!(
                        "secret {name:?} is {} bytes: a secret holds at most 64 KiB",
                        value.len()
                    ));
                }
                w.set_secret(&name, &value)
            }))
        },
    );
    m.fn_named("get", ["name"], move |name: String| -> Reply<String> {
        Fault(granted(&name).map(|granted| {
            if !granted {
                return Err(format!("secret {name:?} is not granted to this script"));
            }
            v.secret(&name)?
                .ok_or_else(|| format!("there is no secret named {name:?}"))
        }))
    });
    m
}
