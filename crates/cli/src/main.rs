//! `vault`: the command-line client of the TheVault HTTP API (D32).
//!
//! Every command prints one JSON document on stdout. A command that fails prints the error as JSON
//! too, with the HTTP status when the server refused it, and exits 1. The API commands send
//! exactly what the API takes and print exactly what it answers; only `init`, `passwd`,
//! `reindex`, `backup` and `restore-backup` work on a vault folder directly (D25, D28). `skill`
//! prints the agent skill document bundled with the binary, as JSON or with `--raw` as Markdown
//! (D32).

mod backup;
mod client;
mod local;
mod starter;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, CommandFactory, Parser, Subcommand};
use reqwest::Method;
use serde_json::{Map, Value, json};

use client::{Api, Body};

/// A failed command: the JSON to print, and exit status 1.
#[derive(Debug)]
pub struct Fail {
    body: Value,
}

impl Fail {
    pub fn new(code: &str, message: impl Into<String>) -> Fail {
        Fail {
            body: json!({ "error": code, "message": message.into() }),
        }
    }

    /// The server refused: its error body, with the status added.
    pub fn api(status: u16, body: Value) -> Fail {
        let mut body = match body {
            Value::Object(o) => Value::Object(o),
            other => json!({ "error": "http", "body": other }),
        };
        body["status"] = json!(status);
        Fail { body }
    }

    /// A report that is itself the failure, such as a reindex that found broken files.
    pub fn body(body: Value) -> Fail {
        Fail { body }
    }

    pub fn from_display<E: std::fmt::Display>(code: &'static str) -> impl Fn(E) -> Fail {
        move |e| Fail::new(code, e.to_string())
    }
}

#[derive(Parser)]
#[command(
    name = "vault",
    version,
    about = "Command-line client for TheVault. Every command prints JSON."
)]
struct Cli {
    /// The server's base URL.
    #[arg(
        long,
        global = true,
        env = "VAULT_URL",
        default_value = "http://127.0.0.1:8080"
    )]
    url: String,

    /// The API token to send as a bearer token.
    #[arg(long, global = true, env = "VAULT_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// The vault folder, for the commands that work on it directly.
    #[arg(long, global = true, env = "VAULT_DIR", default_value = ".")]
    vault: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Every command this binary has, with what it does.
    Commands,
    /// The agent skill document: how to drive a vault with this CLI. Start here.
    Skill {
        /// Print the document as Markdown rather than inside JSON.
        #[arg(long)]
        raw: bool,
    },
    /// Check that the server is up.
    Health,
    /// Every type with its fields, kinds, relationships, templates and handlers, and every
    /// taxonomy.
    Schema,
    /// List notes, optionally of one type.
    List {
        #[arg(long = "type")]
        type_name: Option<String>,
    },
    /// Read a note: its canonical WCL, its JSON form and its version.
    Get {
        id: String,
        /// Read this earlier version instead of the current one.
        #[arg(long)]
        version: Option<String>,
    },
    /// Write a note from WCL (or JSON with --json). With an id it replaces that note; with
    /// --type it creates one.
    Put {
        /// The note to replace. Leave it out to create a note.
        id: Option<String>,
        /// The type of the note to create.
        #[arg(long = "type")]
        type_name: Option<String>,
        /// The version the change is based on. A stale one is refused with 409.
        #[arg(long)]
        base: Option<String>,
        #[command(flatten)]
        input: NoteInput,
    },
    /// Delete a note, moving it to the trash.
    Delete {
        id: String,
        #[arg(long)]
        base: Option<String>,
    },
    /// Every version of a note.
    History { id: String },
    /// Write an earlier version of a note back as a new version.
    Restore {
        id: String,
        version: String,
        #[arg(long)]
        base: Option<String>,
    },
    /// Apply a batch of operations in one commit. The input is a JSON array of operations, or
    /// an object with `ops`.
    Batch {
        /// Report what the batch would do without changing anything.
        #[arg(long)]
        dry_run: bool,
        /// Read the batch from this file; `-` or nothing reads stdin.
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
    /// Create or update the note whose key field matches.
    Upsert {
        #[arg(long = "type")]
        type_name: String,
        /// The field that identifies the note.
        #[arg(long)]
        key: String,
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        input: NoteInput,
    },
    /// Run a query, a page at a time.
    Query {
        query: String,
        /// The note bound to `this`.
        #[arg(long)]
        this: Option<String>,
        #[command(flatten)]
        page: Paging,
    },
    /// Turn query text into the visual builder's model.
    QueryParse { query: String },
    /// Turn a visual builder model into query text. The input is the model as JSON, or the
    /// whole answer of `query-parse`.
    QueryText {
        /// Read the model from this file; `-` or nothing reads stdin.
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
    /// Full-text search, a page at a time.
    Search {
        text: String,
        #[command(flatten)]
        page: Paging,
    },
    /// Notes holding data their type no longer describes.
    Flagged,
    /// The trash.
    #[command(subcommand)]
    Trash(Trash),
    /// Notes pinned to the web sidebar.
    #[command(subcommand)]
    Pin(Pin),
    /// Uploaded files.
    #[command(subcommand)]
    Asset(Asset),
    /// Type definitions.
    #[command(subcommand)]
    Type(Type),
    /// Templates of a type.
    #[command(subcommand)]
    Template(Template),
    /// Taxonomies.
    #[command(subcommand)]
    Taxonomy(Taxonomy),
    /// wscript files in scripts/.
    #[command(subcommand)]
    Script(Script),
    /// Scheduled tasks and their runs.
    #[command(subcommand)]
    Task(Task),
    /// API tokens.
    #[command(subcommand)]
    Token(Token),
    /// Secrets that tasks read. Values are never shown.
    #[command(subcommand)]
    Secret(Secret),
    /// The canvases inside a note.
    #[command(subcommand)]
    Canvas(Canvas),
    /// Create a vault folder, its secret key file and its login password.
    Init {
        /// The folder to create.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Add the starter types, the Status taxonomy and the dashboard note.
        #[arg(long)]
        starter: bool,
        /// Read the password from stdin instead of asking for it.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Change the login password of the vault folder.
    Passwd {
        #[arg(long)]
        password_stdin: bool,
    },
    /// Rebuild the index from the vault files and report every file that fails validation.
    /// Stop the server first.
    Reindex,
    /// Write the vault files and state.db into one archive.
    Backup {
        /// The archive to write. It must not exist.
        out: PathBuf,
        /// Put the secret key in the archive too.
        #[arg(long)]
        include_key: bool,
    },
    /// Unpack a backup archive into a new vault folder.
    RestoreBackup {
        archive: PathBuf,
        /// The folder to restore into. It must be missing or empty.
        dir: PathBuf,
    },
}

#[derive(Args)]
struct NoteInput {
    /// Read the note from this file; `-` or nothing reads stdin.
    #[arg(long, short)]
    file: Option<PathBuf>,
    /// The input is the note's JSON form rather than WCL.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct Paging {
    /// The cursor of the page to fetch.
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    page_size: Option<usize>,
    /// Fetch every page and print them as one.
    #[arg(long, conflicts_with = "cursor")]
    all: bool,
}

#[derive(Subcommand)]
enum Trash {
    /// Every note in the trash.
    List,
    /// Bring a note back from the trash.
    Restore { id: String },
}

#[derive(Subcommand)]
enum Pin {
    /// The pinned notes, in sidebar order, with type and title.
    List,
    /// Pin a note to the end of the sidebar list.
    Add { id: String },
    /// Unpin a note.
    Remove { id: String },
}

#[derive(Subcommand)]
enum Asset {
    /// Every uploaded asset.
    List,
    /// Upload a file as a new asset.
    Upload {
        path: PathBuf,
        /// The file name to store; defaults to the file's own name.
        #[arg(long)]
        filename: Option<String>,
    },
    /// Download an asset's bytes into a file.
    Get {
        id: String,
        #[arg(long, short)]
        out: PathBuf,
    },
    /// Delete an asset. Refused while notes name it, unless --unlink is given.
    Delete {
        id: String,
        /// Take the asset out of every note that names it, in the same commit.
        #[arg(long)]
        unlink: bool,
    },
}

#[derive(Subcommand)]
enum Type {
    /// Every type with its fields and templates.
    List,
    /// Read a type definition.
    Get { name: String },
    /// Create a type, or replace one that has no notes.
    Put {
        name: String,
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
    /// Delete a type that has no notes.
    Delete { name: String },
    /// Plan a change to a type that has notes, or apply a plan with --apply.
    Change {
        name: String,
        #[arg(long, short)]
        file: Option<PathBuf>,
        /// A field rename, as old=new.
        #[arg(long = "rename", value_name = "OLD=NEW")]
        renames: Vec<String>,
        /// A script in scripts/ to migrate every note with.
        #[arg(long)]
        migration: Option<String>,
        /// The migration's function, `migrate` by default.
        #[arg(long, requires = "migration")]
        function: Option<String>,
        /// Apply the plan with this token.
        #[arg(long)]
        apply: Option<String>,
    },
}

#[derive(Subcommand)]
enum Template {
    /// Every template of a type.
    List { type_name: String },
    /// Read one template block.
    Get { type_name: String, name: String },
    /// Create or replace one template from its `template <name> { ... }` block.
    Put {
        type_name: String,
        name: String,
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
    /// Delete a template other than default.
    Delete { type_name: String, name: String },
}

#[derive(Subcommand)]
enum Taxonomy {
    /// Every taxonomy with its term tree.
    List,
    /// Read a taxonomy.
    Get { name: String },
    /// Create or replace a taxonomy, renaming or moving terms in every note.
    Put {
        name: String,
        #[arg(long, short)]
        file: Option<PathBuf>,
        /// A term rename or move, as old=new.
        #[arg(long = "rename", value_name = "OLD=NEW")]
        renames: Vec<String>,
    },
    /// Delete a taxonomy no field uses.
    Delete { name: String },
}

#[derive(Subcommand)]
enum Script {
    /// Every script name.
    List,
    /// Read a script's source.
    Get { name: String },
    /// Create or replace a script.
    Put {
        name: String,
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
    /// Delete a script no handler or task names.
    Delete { name: String },
}

#[derive(Subcommand)]
enum Task {
    /// Every scheduled task with its next run and latest run.
    List,
    /// Read a scheduled task's file and status.
    Get { name: String },
    /// Create or replace a scheduled task.
    Put {
        name: String,
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
    /// Delete a scheduled task. Its run history stays.
    Delete { name: String },
    /// Run a task now and wait for the run to end.
    Run { name: String },
    /// A task's runs, newest first, with their logs.
    History {
        name: String,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// One run of a task.
    ShowRun { name: String, id: String },
}

#[derive(Subcommand)]
enum Token {
    /// Every API token, without secrets.
    List,
    /// Create a token. The output is the only time its secret is shown.
    Create {
        name: String,
        /// read, write or admin.
        #[arg(long)]
        scope: String,
    },
    /// Revoke an API token.
    Revoke { name: String },
}

#[derive(Subcommand)]
enum Secret {
    /// Every secret's name and times, never a value.
    List,
    /// A secret's name and times, never its value.
    Get { name: String },
    /// Store or replace a secret. The value is read from stdin.
    Set { name: String },
    /// Delete a secret.
    Delete { name: String },
}

#[derive(Subcommand)]
enum Canvas {
    /// Read one canvas of a note.
    Get { id: String, name: String },
    /// Replace one canvas of a note at its base version.
    Put {
        id: String,
        name: String,
        #[arg(long)]
        base: Option<String>,
        #[arg(long, short)]
        file: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // The one output that is not JSON, and only when asked for.
    if let Command::Skill { raw: true } = cli.command {
        print!("{SKILL}");
        return ExitCode::SUCCESS;
    }
    let (out, code) = match run(cli) {
        Ok(v) => (v, ExitCode::SUCCESS),
        Err(f) => (f.body, ExitCode::FAILURE),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&out).unwrap_or_else(|_| "null".to_owned())
    );
    code
}

fn run(cli: Cli) -> Result<Value, Fail> {
    let vault = cli.vault.as_path();
    match cli.command {
        Command::Commands => Ok(commands()),
        Command::Skill { .. } => Ok(json!({ "name": "thevault", "document": SKILL })),
        Command::Init {
            dir,
            starter,
            password_stdin,
        } => local::init(&dir, starter, password_stdin),
        Command::Passwd { password_stdin } => local::passwd(vault, password_stdin),
        Command::Reindex => local::reindex(vault),
        Command::Backup { out, include_key } => backup::backup(vault, &out, include_key),
        Command::RestoreBackup { archive, dir } => backup::restore(&archive, &dir),
        command => {
            let api = Api::new(&cli.url, cli.token)?;
            remote(&api, command)
        }
    }
}

fn remote(api: &Api, command: Command) -> Result<Value, Fail> {
    match command {
        Command::Health => api.get(&["api", "health"]),
        Command::Schema => api.get(&["api", "schema"]),
        Command::List { type_name } => {
            let query: Vec<_> = type_name.map(|t| ("type", t)).into_iter().collect();
            api.call(Method::GET, &["api", "notes"], &query, Body::None)
        }
        Command::Get { id, version } => match version {
            Some(v) => api.get(&["api", "notes", &id, "history", &v]),
            None => api.get(&["api", "notes", &id]),
        },
        Command::Put {
            id,
            type_name,
            base,
            input,
        } => {
            let mut body = input.read()?;
            match (id, type_name) {
                (Some(id), None) => {
                    if let Some(base) = base {
                        body.insert("base_version".into(), json!(base));
                    }
                    api.put(&["api", "notes", &id], Value::Object(body))
                }
                (None, Some(t)) => {
                    if base.is_some() {
                        return Err(Fail::new("usage", "--base applies only to an update"));
                    }
                    body.insert("type".into(), json!(t));
                    api.post(&["api", "notes"], Value::Object(body))
                }
                _ => Err(Fail::new(
                    "usage",
                    "name the note's id to replace it, or --type to create one, not both",
                )),
            }
        }
        Command::Delete { id, base } => {
            let query: Vec<_> = base.map(|b| ("base_version", b)).into_iter().collect();
            api.call(Method::DELETE, &["api", "notes", &id], &query, Body::None)
        }
        Command::History { id } => api.get(&["api", "notes", &id, "history"]),
        Command::Restore { id, version, base } => api.post(
            &["api", "notes", &id, "restore"],
            json!({ "version": version, "base_version": base }),
        ),
        Command::Batch { dry_run, file } => {
            let input: Value = serde_json::from_str(&read_text(file.as_deref())?)
                .map_err(|e| Fail::new("usage", format!("the batch is not JSON: {e}")))?;
            let mut body = match input {
                Value::Array(ops) => json!({ "ops": ops }),
                Value::Object(o) if o.contains_key("ops") => Value::Object(o),
                _ => {
                    return Err(Fail::new(
                        "usage",
                        "the batch is a JSON array of operations, or an object with `ops`",
                    ));
                }
            };
            if dry_run {
                body["dry_run"] = json!(true);
            }
            api.post(&["api", "batch"], body)
        }
        Command::Upsert {
            type_name,
            key,
            base,
            dry_run,
            input,
        } => {
            let mut body = input.read()?;
            body.insert("type".into(), json!(type_name));
            body.insert("key".into(), json!(key));
            body.insert("dry_run".into(), json!(dry_run));
            if let Some(base) = base {
                body.insert("base_version".into(), json!(base));
            }
            api.post(&["api", "upsert"], Value::Object(body))
        }
        Command::Query { query, this, page } => page.fetch(|cursor, size| {
            api.post(
                &["api", "query"],
                json!({ "query": query, "this": this, "cursor": cursor, "page_size": size }),
            )
        }),
        Command::QueryParse { query } => {
            api.post(&["api", "query", "parse"], json!({ "query": query }))
        }
        Command::QueryText { file } => {
            let input: Value = serde_json::from_str(&read_text(file.as_deref())?)
                .map_err(|e| Fail::new("usage", format!("the model is not JSON: {e}")))?;
            let model = match input {
                Value::Object(mut o) if o.contains_key("model") => o.remove("model").unwrap(),
                other => other,
            };
            api.post(&["api", "query", "text"], json!({ "model": model }))
        }
        Command::Search { text, page } => page.fetch(|cursor, size| {
            let mut query = vec![("q", text.clone())];
            query.extend(cursor.map(|c| ("cursor", c)));
            query.extend(size.map(|s| ("page_size", s.to_string())));
            api.call(Method::GET, &["api", "search"], &query, Body::None)
        }),
        Command::Flagged => api.get(&["api", "flagged"]),
        Command::Trash(Trash::List) => api.get(&["api", "trash"]),
        Command::Trash(Trash::Restore { id }) => api.call(
            Method::POST,
            &["api", "trash", &id, "restore"],
            &[],
            Body::None,
        ),
        Command::Pin(Pin::List) => api.get(&["api", "pins"]),
        Command::Pin(Pin::Add { id }) => {
            api.call(Method::PUT, &["api", "pins", &id], &[], Body::None)
        }
        Command::Pin(Pin::Remove { id }) => {
            api.call(Method::DELETE, &["api", "pins", &id], &[], Body::None)
        }
        Command::Asset(a) => asset(api, a),
        Command::Type(t) => type_cmd(api, t),
        Command::Template(t) => match t {
            Template::List { type_name } => api.get(&["api", "types", &type_name, "templates"]),
            Template::Get { type_name, name } => {
                api.get(&["api", "types", &type_name, "templates", &name])
            }
            Template::Put {
                type_name,
                name,
                file,
            } => api.put(
                &["api", "types", &type_name, "templates", &name],
                json!({ "wcl": read_text(file.as_deref())? }),
            ),
            Template::Delete { type_name, name } => {
                api.delete(&["api", "types", &type_name, "templates", &name])
            }
        },
        Command::Taxonomy(t) => match t {
            Taxonomy::List => api.get(&["api", "taxonomies"]),
            Taxonomy::Get { name } => api.get(&["api", "taxonomies", &name]),
            Taxonomy::Put {
                name,
                file,
                renames,
            } => api.put(
                &["api", "taxonomies", &name],
                json!({ "wcl": read_text(file.as_deref())?, "renames": pairs(&renames)? }),
            ),
            Taxonomy::Delete { name } => api.delete(&["api", "taxonomies", &name]),
        },
        Command::Script(s) => match s {
            Script::List => api.get(&["api", "scripts"]),
            Script::Get { name } => api.get(&["api", "scripts", &name]),
            Script::Put { name, file } => api.put(
                &["api", "scripts", &name],
                json!({ "source": read_text(file.as_deref())? }),
            ),
            Script::Delete { name } => api.delete(&["api", "scripts", &name]),
        },
        Command::Task(t) => match t {
            Task::List => api.get(&["api", "tasks"]),
            Task::Get { name } => api.get(&["api", "tasks", &name]),
            Task::Put { name, file } => api.put(
                &["api", "tasks", &name],
                json!({ "wcl": read_text(file.as_deref())? }),
            ),
            Task::Delete { name } => api.delete(&["api", "tasks", &name]),
            Task::Run { name } => api.call(
                Method::POST,
                &["api", "tasks", &name, "run"],
                &[],
                Body::None,
            ),
            Task::History { name, limit } => {
                let query: Vec<_> = limit
                    .map(|l| ("limit", l.to_string()))
                    .into_iter()
                    .collect();
                api.call(
                    Method::GET,
                    &["api", "tasks", &name, "runs"],
                    &query,
                    Body::None,
                )
            }
            Task::ShowRun { name, id } => api.get(&["api", "tasks", &name, "runs", &id]),
        },
        Command::Token(t) => match t {
            Token::List => api.get(&["api", "tokens"]),
            Token::Create { name, scope } => {
                api.post(&["api", "tokens"], json!({ "name": name, "scope": scope }))
            }
            Token::Revoke { name } => {
                api.delete(&["api", "tokens", &name])?;
                Ok(json!({ "name": name, "revoked": true }))
            }
        },
        Command::Secret(s) => match s {
            Secret::List => api.get(&["api", "secrets"]),
            Secret::Get { name } => api.get(&["api", "secrets", &name]),
            Secret::Set { name } => {
                let raw = read_text(None)?;
                let value = raw.strip_suffix('\n').unwrap_or(&raw);
                api.put(&["api", "secrets", &name], json!({ "value": value }))
            }
            Secret::Delete { name } => api.delete(&["api", "secrets", &name]),
        },
        Command::Canvas(c) => match c {
            Canvas::Get { id, name } => api.get(&["api", "notes", &id, "canvas", &name]),
            Canvas::Put {
                id,
                name,
                base,
                file,
            } => api.put(
                &["api", "notes", &id, "canvas", &name],
                json!({ "wcl": read_text(file.as_deref())?, "base_version": base }),
            ),
        },
        Command::Commands
        | Command::Skill { .. }
        | Command::Init { .. }
        | Command::Passwd { .. }
        | Command::Reindex
        | Command::Backup { .. }
        | Command::RestoreBackup { .. } => unreachable!("local commands are handled in run"),
    }
}

/// The agent skill document that ships with the binary (D32).
const SKILL: &str = include_str!("../SKILL.md");

/// Every leaf command, as the words that name it, with its one-line description.
fn commands() -> Value {
    fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<Value>) {
        for sub in cmd.get_subcommands().filter(|c| c.get_name() != "help") {
            let name = format!("{prefix}{}", sub.get_name());
            if sub.has_subcommands() {
                walk(sub, &format!("{name} "), out);
            } else {
                let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
                out.push(json!({ "command": name, "about": about }));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Cli::command(), "", &mut out);
    json!({ "commands": out })
}

fn asset(api: &Api, a: Asset) -> Result<Value, Fail> {
    match a {
        Asset::List => api.get(&["api", "assets"]),
        Asset::Upload { path, filename } => {
            let filename = match filename {
                Some(f) => f,
                None => path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_owned)
                    .ok_or_else(|| Fail::new("usage", "name the asset with --filename"))?,
            };
            let bytes = std::fs::read(&path)
                .map_err(|e| Fail::new("io", format!("{}: {e}", path.display())))?;
            api.call(
                Method::POST,
                &["api", "assets"],
                &[("filename", filename)],
                Body::Bytes(bytes),
            )
        }
        Asset::Get { id, out } => {
            let bytes = api.download(&["api", "assets", &id])?;
            std::fs::write(&out, &bytes)
                .map_err(|e| Fail::new("io", format!("{}: {e}", out.display())))?;
            Ok(json!({ "id": id, "out": out, "bytes": bytes.len() }))
        }
        Asset::Delete { id, unlink } => {
            let query: Vec<_> = unlink
                .then(|| ("unlink", "true".to_owned()))
                .into_iter()
                .collect();
            api.call(Method::DELETE, &["api", "assets", &id], &query, Body::None)
        }
    }
}

fn type_cmd(api: &Api, t: Type) -> Result<Value, Fail> {
    match t {
        Type::List => api.get(&["api", "types"]),
        Type::Get { name } => api.get(&["api", "types", &name]),
        Type::Put { name, file } => api.put(
            &["api", "types", &name],
            json!({ "wcl": read_text(file.as_deref())? }),
        ),
        Type::Delete { name } => api.delete(&["api", "types", &name]),
        Type::Change {
            name,
            file,
            renames,
            migration,
            function,
            apply,
        } => {
            let mut body = json!({
                "wcl": read_text(file.as_deref())?,
                "renames": pairs(&renames)?,
            });
            if let Some(script) = migration {
                body["migration"] = json!({ "script": script, "function": function });
            }
            if let Some(plan) = apply {
                body["plan"] = json!(plan);
            }
            api.post(&["api", "types", &name, "change"], body)
        }
    }
}

impl NoteInput {
    /// The body members that carry the note: `wcl`, or `note` with --json.
    fn read(&self) -> Result<Map<String, Value>, Fail> {
        let text = read_text(self.file.as_deref())?;
        let mut body = Map::new();
        if self.json {
            let note: Value = serde_json::from_str(&text)
                .map_err(|e| Fail::new("usage", format!("the note is not JSON: {e}")))?;
            body.insert("note".into(), note);
        } else {
            body.insert("wcl".into(), json!(text));
        }
        Ok(body)
    }
}

impl Paging {
    /// Fetch one page, or with --all every page, merged into the first page's shape.
    fn fetch(
        &self,
        page: impl Fn(Option<String>, Option<usize>) -> Result<Value, Fail>,
    ) -> Result<Value, Fail> {
        let mut first = page(self.cursor.clone(), self.page_size)?;
        if !self.all {
            return Ok(first);
        }
        let mut next = first["next"].as_str().map(str::to_owned);
        while let Some(cursor) = next {
            let more = page(Some(cursor), self.page_size)?;
            if let (Some(rows), Value::Array(extra)) =
                (first["rows"].as_array_mut(), more["rows"].clone())
            {
                rows.extend(extra);
            }
            next = more["next"].as_str().map(str::to_owned);
        }
        first["next"] = Value::Null;
        Ok(first)
    }
}

/// `old=new` arguments as a JSON object.
fn pairs(raw: &[String]) -> Result<Value, Fail> {
    let mut out = Map::new();
    for r in raw {
        let (old, new) = r
            .split_once('=')
            .ok_or_else(|| Fail::new("usage", format!("{r:?} is not OLD=NEW")))?;
        out.insert(old.to_owned(), json!(new));
    }
    Ok(Value::Object(out))
}

/// A file's text, or stdin's when `path` is missing or `-`.
fn read_text(path: Option<&Path>) -> Result<String, Fail> {
    match path {
        Some(p) if p != Path::new("-") => {
            std::fs::read_to_string(p).map_err(|e| Fail::new("io", format!("{}: {e}", p.display())))
        }
        _ => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| Fail::new("io", format!("stdin: {e}")))?;
            Ok(text)
        }
    }
}
