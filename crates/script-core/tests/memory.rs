//! The engine on its own: scripts run against an in-memory `Vault`, with no server crate behind
//! it, as a device runs them (D6).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use script_core::{Archived, Author, Error, Extensions, Host, Job, Kind, Limits, Vault, host_call};
use serde_json::{Value as Json, json};
use wscript::{Fault, Module};

/// Notes kept in a map. Each write bumps the note's version and records who made it.
#[derive(Default)]
struct Memory {
    notes: Mutex<BTreeMap<String, Stored>>,
}

struct Stored {
    type_name: String,
    version: u64,
    note: Json,
    author: Author,
}

fn unsupported<T>() -> Result<T, String> {
    Err("the in-memory vault does not do that".into())
}

impl Memory {
    fn note(&self, id: &str) -> Result<Json, String> {
        let notes = self.notes.lock().unwrap();
        let s = notes
            .get(id)
            .ok_or_else(|| format!("note {id} does not exist"))?;
        Ok(json!({
            "id": id,
            "type": s.type_name,
            "version": s.version.to_string(),
            "note": s.note,
        }))
    }

    fn author(&self, id: &str) -> Author {
        self.notes.lock().unwrap()[id].author.clone()
    }
}

impl Vault for Memory {
    fn get(&self, id: &str) -> Result<Json, String> {
        self.note(id)
    }

    fn query(&self, _text: &str, max_rows: usize) -> Result<Vec<Json>, String> {
        let ids: Vec<String> = self.notes.lock().unwrap().keys().cloned().collect();
        if ids.len() > max_rows {
            return Err("too many rows".into());
        }
        ids.iter().map(|id| self.note(id)).collect()
    }

    fn query_page(&self, _: &str, _: usize, _: Option<String>) -> Result<Json, String> {
        unsupported()
    }

    fn search(&self, _: &str, _: usize, _: Archived) -> Result<Vec<Json>, String> {
        unsupported()
    }

    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String> {
        let id = {
            let mut notes = self.notes.lock().unwrap();
            let id = format!("n{}", notes.len() + 1);
            notes.insert(
                id.clone(),
                Stored {
                    type_name: type_name.to_owned(),
                    version: 1,
                    note,
                    author: author.clone(),
                },
            );
            id
        };
        self.note(&id)
    }

    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String> {
        {
            let mut notes = self.notes.lock().unwrap();
            let s = notes
                .get_mut(id)
                .ok_or_else(|| format!("note {id} does not exist"))?;
            if s.version.to_string() != version {
                return Err(format!("note {id} changed since version {version}"));
            }
            s.version += 1;
            s.note = note;
            s.author = author.clone();
        }
        self.note(id)
    }

    fn delete(&self, _: &str, _: &str, _: &Author) -> Result<Json, String> {
        unsupported()
    }

    fn restore(&self, _: &str, _: &Author) -> Result<Json, String> {
        unsupported()
    }

    fn set_archived(&self, _: &str, _: bool, _: &Author) -> Result<Json, String> {
        unsupported()
    }

    fn links(&self, _: &str) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn backlinks(&self, _: &str) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn assets(&self) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn asset(&self, _: &str) -> Result<(Json, Vec<u8>), String> {
        unsupported()
    }

    fn upload_asset(&self, _: &str, _: Vec<u8>) -> Result<Json, String> {
        unsupported()
    }

    fn types(&self) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn taxonomies(&self) -> Result<Vec<Json>, String> {
        Ok(Vec::new())
    }

    fn add_term(&self, _: &str, _: &str, _: &Author) -> Result<Json, String> {
        unsupported()
    }

    fn daily(&self, _: &str, _: &Author) -> Result<Json, String> {
        unsupported()
    }

    fn secret(&self, _: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(600),
        ..Limits::default()
    }
}

#[test]
fn a_script_creates_patches_and_queries_notes_in_an_in_memory_vault() {
    let memory = Arc::new(Memory::default());
    let host = Host::new(memory.clone() as Arc<dyn Vault>, 1, limits());
    let src = r#"use vault
fn run() -> Result[Value, string] {
    let body = Value::Map(#{
        "fields": Value::Map(#{ "title": Value::String("From the phone") }),
    })
    let saved = vault::create("message", body)?
    let id = saved.get("id").unwrap().as_string().unwrap()
    let patched = vault::patch(id, Value::Map(#{ "read": Value::Bool(true) }), "")?
    let rows = vault::query("from message")?
    vault::log("saved " + id)
    Ok(Value::Map(#{ "patched": patched, "rows": Value::Int(rows.len()) }))
}
"#;
    let out = host.run(Job::new(Kind::Device, "sms", src, "run"));
    let result = out.result.unwrap();

    assert_eq!(out.log, "saved n1\n");
    assert_eq!(result["rows"], 1);
    assert_eq!(result["patched"]["version"], "2");
    assert_eq!(
        memory.note("n1").unwrap()["note"]["fields"],
        json!({ "title": "From the phone", "read": true })
    );
    assert_eq!(memory.author("n1"), Author::Handler("sms".into()));
}

#[test]
fn an_error_from_the_vault_reaches_the_script_as_an_err() {
    let host = Host::new(Arc::new(Memory::default()), 1, limits());
    let src = r#"use vault
fn run() -> Result[string, string] {
    match vault::get("missing") {
        Ok(_) => Ok("found"),
        Err(e) => Err(e),
    }
}
"#;
    let err = host
        .run(Job::new(Kind::Device, "lookup", src, "run"))
        .result
        .unwrap_err();
    assert_eq!(
        err,
        Error::Returned {
            script: "lookup".into(),
            message: "note missing does not exist".into()
        }
    );
}

#[test]
fn the_engine_alone_registers_no_http_secrets_or_fs() {
    let host = Host::new(Arc::new(Memory::default()), 1, limits());
    for module in ["http", "secrets", "fs"] {
        let src = format!("use {module}\nfn run() {{}}\n");
        for kind in [Kind::Before, Kind::After, Kind::Task, Kind::Device] {
            let err = host.check(kind, "probe", &src).unwrap_err();
            assert!(
                matches!(err, Error::Compile { .. }),
                "{module} {kind:?}: {err:?}"
            );
        }
    }
}

/// A stand-in for the `device` module: one function, for device runs only.
struct Device;

impl Extensions for Device {
    fn modules(&self, kind: Kind, _: &Arc<dyn Vault>) -> Vec<Module> {
        if kind != Kind::Device {
            return Vec::new();
        }
        let mut m = Module::new("device");
        m.fn_("name", || -> Fault<Result<String, String>> {
            Fault(host_call(|_, _| Ok("pixel".to_owned())))
        });
        vec![m]
    }
}

#[test]
fn extensions_add_modules_only_to_the_kind_they_name() {
    let host = Host::with_extensions(Arc::new(Memory::default()), 1, limits(), &Device);
    let src = "use device\nfn run() -> Result[string, string] { device::name() }\n";

    let out = host.run(Job::new(Kind::Device, "probe", src, "run"));
    assert_eq!(out.result.unwrap(), json!("pixel"));
    for kind in [Kind::Before, Kind::Migration, Kind::After, Kind::Task] {
        let err = host.check(kind, "probe", src).unwrap_err();
        assert!(matches!(err, Error::Compile { .. }), "{kind:?}: {err:?}");
    }
}
