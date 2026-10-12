//! Seam S2: the real `vault` binary against a running server, asserting its JSON output (D25,
//! D28, D32).
//!
//! Each test gives the binary its own config folder through `XDG_CONFIG_HOME` and `HOME`, so key
//! files never land in the real home folder.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;
use testkit::TestServer;

const STATUS: &str = r#"label = "Status"

term "Inbox"

term "Active" {
  term "In progress"

  term "Blocked"
}

term "Done"
"#;

const TASK: &str = r#"label = "Task"

field ext_id {
  kind = "string"
}

field estimate {
  kind = "int"
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
  }
}
"#;

const BOARD: &str = r#"label = "Board"

field board {
  kind = "canvas"
}

template default {
  tab "Board" {
    slot "title"
    slot "board"
  }
}
"#;

fn seed(vault: &Path) {
    for (dir, name, body) in [
        ("taxonomies", "status", STATUS),
        ("types", "task", TASK),
        ("types", "board", BOARD),
    ] {
        std::fs::create_dir_all(vault.join(dir)).unwrap();
        std::fs::write(vault.join(dir).join(format!("{name}.wcl")), body).unwrap();
    }
}

/// What one run of `vault` did.
struct Out {
    ok: bool,
    json: Value,
    stdout: String,
}

/// Runs `vault` with its own config folder, against `url` with `token` when given.
struct Vault {
    url: Option<String>,
    token: Option<String>,
    config: TempDir,
}

impl Vault {
    fn offline() -> Vault {
        Vault {
            url: None,
            token: None,
            config: TempDir::new().unwrap(),
        }
    }

    fn against(server: &TestServer) -> Vault {
        Vault {
            url: Some(server.base_url()),
            token: Some(server.admin_token().to_owned()),
            ..Vault::offline()
        }
    }

    /// The folder `XDG_CONFIG_HOME` points at.
    fn config_dir(&self) -> &Path {
        self.config.path()
    }

    fn run(&self, args: &[&str], stdin: &str) -> Out {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vault"));
        cmd.args(args)
            .env_remove("VAULT_URL")
            .env_remove("VAULT_TOKEN")
            .env_remove("VAULT_DIR")
            .env_remove("VAULT_SECRET_KEY_FILE")
            .env("XDG_CONFIG_HOME", self.config.path())
            .env("HOME", self.config.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(url) = &self.url {
            cmd.env("VAULT_URL", url);
        }
        if let Some(token) = &self.token {
            cmd.env("VAULT_TOKEN", token);
        }
        let mut child = cmd.spawn().expect("start vault");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let json = serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "vault {args:?} printed something that is not JSON ({e}):\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
        Out {
            ok: output.status.success(),
            json,
            stdout,
        }
    }

    /// Run, expecting success.
    fn ok(&self, args: &[&str], stdin: &str) -> Value {
        let out = self.run(args, stdin);
        assert!(out.ok, "vault {args:?} failed:\n{}", out.stdout);
        out.json
    }

    /// Run, expecting failure.
    fn fails(&self, args: &[&str], stdin: &str) -> Value {
        let out = self.run(args, stdin);
        assert!(!out.ok, "vault {args:?} succeeded:\n{}", out.stdout);
        out.json
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

fn tmp_path(dir: &TempDir, name: &str) -> String {
    dir.path().join(name).to_str().unwrap().to_owned()
}

/// Every vault file outside `.vault/`, with its bytes.
fn files(vault: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if rel == Path::new(".vault") {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(vault, vault, &mut out);
    out.sort();
    out
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).unwrap();
        }
    }
}

#[test]
fn put_of_wcl_then_get_returns_the_canonical_form() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    // Untidy on purpose: a comment, odd spacing, and fields out of order.
    let messy = "// a comment\nstatus=\"Inbox\"\n   title   =    \"Ship it\"\nestimate=3\n";
    let created = vault.ok(&["put", "--type", "task"], messy);
    let id = s(&created["id"]).to_owned();

    let got = vault.ok(&["get", &id], "");
    let wcl = s(&got["wcl"]);
    let on_disk =
        std::fs::read_to_string(server.vault_path().join(format!("notes/task/{id}.wcl"))).unwrap();
    assert_eq!(wcl, on_disk, "get did not return the file as stored");
    assert_eq!(wcl, s(&created["wcl"]));
    assert!(!wcl.contains("comment"), "{wcl}");
    assert!(wcl.contains("title    = \"Ship it\"\n"), "{wcl}");

    // Writing the canonical form back changes nothing but the version stamp.
    let again = vault.ok(
        &["put", &id, "--base", s(&got["version"])],
        &wcl.replace("Ship it", "Ship it now"),
    );
    let reread = vault.ok(&["get", &id], "");
    assert_eq!(reread["wcl"], again["wcl"]);
    assert!(s(&reread["wcl"]).contains("Ship it now"));

    // A stale base is a 409, printed as JSON.
    let stale = vault.fails(&["put", &id, "--base", s(&got["version"])], wcl);
    assert_eq!(stale["status"], 409, "{stale}");
    assert_eq!(stale["error"], "conflict");
}

/// A batch report with what a dry run cannot know replaced: the versions, the ids of notes the
/// batch creates, the `created` and `updated` stamps, and the flags that name the run.
fn comparable(report: &Value) -> Value {
    let mut r = report.clone();
    let obj = r.as_object_mut().unwrap();
    obj.remove("dry_run");
    obj.remove("committed");
    for item in r["items"].as_array_mut().unwrap() {
        item["version"] = Value::Null;
        if item["result"] == "created" {
            item["id"] = json!("<new>");
        }
        // A create that fails names the file the new note would have had.
        if item["op"] == "create"
            && let Some(file) = item["error"]["file"].as_str().map(str::to_owned)
        {
            let message = s(&item["error"]["message"]).replace(&file, "<new>");
            item["error"]["file"] = json!("<new>");
            item["error"]["message"] = json!(message);
        }
        if let Some(wcl) = item["wcl"].as_str() {
            let kept: Vec<&str> = wcl
                .lines()
                .filter(|l| !l.starts_with("created") && !l.starts_with("updated"))
                .collect();
            item["wcl"] = json!(kept.join("\n"));
        }
    }
    for also in r["also"].as_array_mut().unwrap() {
        also["version"] = Value::Null;
    }
    r
}

#[test]
fn batch_dry_run_prints_what_the_real_run_prints() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let old = vault.ok(&["put", "--type", "task"], "title = \"Old\"\n");
    let doomed = vault.ok(&["put", "--type", "task"], "title = \"Doomed\"\n");
    let ops = json!([
        { "op": "create", "type": "task", "note": { "fields": { "title": "New", "status": "Active/Blocked" } } },
        { "op": "update", "id": old["id"], "base_version": old["version"],
          "wcl": "title = \"Old, edited\"\nstatus = \"Done\"\n" },
        { "op": "delete", "id": doomed["id"], "base_version": doomed["version"] },
        { "op": "upsert", "type": "task", "key": "ext_id",
          "note": { "fields": { "title": "Imported", "ext_id": "EXT-1" } } },
    ])
    .to_string();

    let before = files(server.vault_path());
    let dry = vault.ok(&["batch", "--dry-run"], &ops);
    assert_eq!(
        files(server.vault_path()),
        before,
        "a dry run changed files"
    );
    assert_eq!(dry["dry_run"], true);

    let real = vault.ok(&["batch"], &ops);
    assert_eq!(real["committed"], true);
    assert_eq!(comparable(&dry), comparable(&real));
    let results: Vec<&str> = real["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| s(&i["result"]))
        .collect();
    assert_eq!(results, ["created", "updated", "deleted", "created"]);

    // A failing batch fails the same way dry or real, and the CLI exits non-zero for both.
    let bad = json!({ "ops": [
        { "op": "create", "type": "task", "note": { "fields": { "title": "Bad", "status": "Nope" } } },
    ] })
    .to_string();
    let before = files(server.vault_path());
    let dry = vault.fails(&["batch", "--dry-run"], &bad);
    let real = vault.fails(&["batch"], &bad);
    assert_eq!(files(server.vault_path()), before);
    assert_eq!(dry["status"], 422);
    assert_eq!(comparable(&dry), comparable(&real));
}

#[test]
fn upsert_twice_changes_nothing_the_second_time() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let note = r#"{ "fields": { "title": "Imported", "ext_id": "EXT-7", "estimate": 2 } }"#;
    let args = ["upsert", "--type", "task", "--key", "ext_id", "--json"];
    let first = vault.ok(&args, note);
    assert_eq!(first["items"][0]["result"], "created", "{first}");
    let before = files(server.vault_path());
    let second = vault.ok(&args, note);
    assert_eq!(second["items"][0]["result"], "unchanged", "{second}");
    assert_eq!(files(server.vault_path()), before);
}

#[test]
fn reindex_reports_the_one_broken_file_and_exits_non_zero() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let good = vault.ok(&["put", "--type", "task"], "title = \"Fine\"\n");
    let broken = vault.ok(&["put", "--type", "task"], "title = \"Soon broken\"\n");
    // The server's vault goes when the server does, so reindex a copy of it.
    let copy = TempDir::new().unwrap();
    copy_dir(server.vault_path(), copy.path());
    drop(server);
    let root = copy.path();

    let dir = root.to_str().unwrap();
    let clean = vault.ok(&["reindex", "--vault", dir], "");
    assert_eq!(clean["ok"], true);
    assert_eq!(clean["indexed"], 2);
    assert_eq!(clean["failures"], json!([]));

    let path = format!("notes/task/{}.wcl", s(&broken["id"]));
    std::fs::write(root.join(&path), "title = \"x\"\nstatus = \"Nope\"\n").unwrap();
    let out = vault.fails(&["reindex", "--vault", dir], "");
    assert_eq!(out["ok"], false);
    assert_eq!(out["indexed"], 1);
    let failures = out["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{out}");
    assert_eq!(failures[0]["path"], json!(path));
    assert!(!s(&failures[0]["diagnostic"]).is_empty());
    assert!(!out.to_string().contains(s(&good["id"])));
}

#[test]
fn a_restored_backup_serves_the_same_notes_and_history() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let a = vault.ok(&["put", "--type", "task"], "title = \"First\"\n");
    let id = s(&a["id"]).to_owned();
    vault.ok(
        &["put", &id, "--base", s(&a["version"])],
        "title = \"First, edited\"\n",
    );
    let b = vault.ok(&["put", "--type", "task"], "title = \"Second\"\n");
    vault.ok(&["delete", s(&b["id"]), "--base", s(&b["version"])], "");
    let bytes = b"\x89PNG not really".to_vec();
    let out = TempDir::new().unwrap();
    let asset_file = tmp_path(&out, "cover.png");
    std::fs::write(&asset_file, &bytes).unwrap();
    let asset = vault.ok(&["asset", "upload", &asset_file], "");

    let list = vault.ok(&["list"], "");
    let history = vault.ok(&["history", &id], "");
    let trash = vault.ok(&["trash", "list"], "");
    let note = vault.ok(&["get", &id], "");

    let archive = tmp_path(&out, "vault.backup");
    let dir = server.vault_path().to_str().unwrap().to_owned();
    let made = vault.run(&["backup", &archive, "--vault", &dir], "");
    assert!(made.ok, "{}", made.stdout);
    assert_eq!(made.json["key"]["included"], false);
    assert!(
        s(&made.json["key"]["message"]).contains("--include-key"),
        "{}",
        made.stdout
    );
    assert_eq!(made.json["files"], json!(files(server.vault_path()).len()));
    // Never overwrites.
    assert_eq!(
        vault.fails(&["backup", &archive, "--vault", &dir], "")["error"],
        "exists"
    );

    let target = TempDir::new().unwrap();
    let into = target.path().join("restored");
    let restored = vault.ok(&["restore-backup", &archive, into.to_str().unwrap()], "");
    assert_eq!(restored["vault_id"], made.json["vault_id"]);
    assert_eq!(files(&into), files(server.vault_path()));
    let token = server.admin_token().to_owned();
    drop(server);

    // Serve the restored folder; the old token lives in its state.db.
    let moved = TempDir::new_in(target.path()).unwrap();
    std::fs::remove_dir(moved.path()).unwrap();
    std::fs::rename(&into, moved.path()).unwrap();
    let server = TestServer::start_existing(moved, &token, &[]);
    let vault = Vault::against(&server);
    assert_eq!(vault.ok(&["list"], ""), list);
    assert_eq!(vault.ok(&["history", &id], ""), history);
    assert_eq!(vault.ok(&["trash", "list"], ""), trash);
    assert_eq!(vault.ok(&["get", &id], ""), note);
    let copy = tmp_path(&out, "copy.png");
    vault.ok(&["asset", "get", s(&asset["id"]), "--out", &copy], "");
    assert_eq!(std::fs::read(&copy).unwrap(), bytes);
}

#[test]
fn backup_with_include_key_carries_the_key_to_a_new_machine() {
    let here = Vault::offline();
    let dirs = TempDir::new().unwrap();
    let vault_dir = tmp_path(&dirs, "v");
    let init = here.ok(&["init", &vault_dir, "--password-stdin"], "pw\n");
    let archive = tmp_path(&dirs, "with-key.backup");
    let made = here.ok(
        &["backup", &archive, "--include-key", "--vault", &vault_dir],
        "",
    );
    assert_eq!(made["key"]["included"], true);

    // Another machine: a fresh config folder with no key in it.
    let there = Vault::offline();
    let into = tmp_path(&dirs, "w");
    let restored = there.ok(&["restore-backup", &archive, &into], "");
    assert_eq!(restored["key"]["restored"], true);
    let key = there
        .config_dir()
        .join(format!("thevault/{}.key", s(&init["vault_id"])));
    assert_eq!(
        std::fs::read(&key).unwrap(),
        std::fs::read(s(&init["key_file"])).unwrap()
    );
}

#[test]
fn init_creates_the_layout_and_key_file_and_sets_the_password() {
    let vault = Vault::offline();
    let dirs = TempDir::new().unwrap();
    let dir = dirs.path().join("vault");
    let out = vault.ok(
        &["init", dir.to_str().unwrap(), "--password-stdin"],
        "hunter22\n",
    );
    for sub in [
        "notes",
        "types",
        "taxonomies",
        "scripts",
        "tasks",
        "assets",
        ".vault",
    ] {
        assert!(dir.join(sub).is_dir(), "no {sub}/");
    }
    let vault_id = s(&out["vault_id"]).to_owned();
    let key = vault
        .config_dir()
        .join("thevault")
        .join(format!("{vault_id}.key"));
    assert_eq!(out["key_file"], json!(key));
    assert_eq!(std::fs::read(&key).unwrap().len(), 32);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    {
        let state = state::State::open(&dir.join(".vault/state.db"), None).unwrap();
        assert!(state.verify_password("hunter22").unwrap());
        assert!(!state.verify_password("hunter22\n").unwrap());
        assert_eq!(state.vault_id().unwrap(), vault_id);
    }

    // A second init refuses, and changes nothing.
    let again = vault.fails(
        &["init", dir.to_str().unwrap(), "--password-stdin"],
        "other\n",
    );
    assert_eq!(again["error"], "already_a_vault");

    // passwd replaces the password.
    vault.ok(
        &[
            "passwd",
            "--vault",
            dir.to_str().unwrap(),
            "--password-stdin",
        ],
        "new password\n",
    );

    // A server on the new vault logs in with the password and finds the key on its own.
    let folder = TempDir::new_in(dirs.path()).unwrap();
    std::fs::remove_dir(folder.path()).unwrap();
    std::fs::rename(&dir, folder.path()).unwrap();
    let server = TestServer::start_existing(folder, "", &[("XDG_CONFIG_HOME", vault.config_dir())]);
    let http = reqwest::blocking::Client::new();
    let res = http
        .post(server.url("/api/login"))
        .json(&json!({ "password": "hunter22" }))
        .send()
        .unwrap();
    assert_eq!(res.status(), 401);
    let res = http
        .post(server.url("/api/login"))
        .json(&json!({ "password": "new password" }))
        .send()
        .unwrap();
    assert_eq!(res.status(), 200);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let res = http
        .put(server.url("/api/secrets/api_key"))
        .header("cookie", cookie)
        .json(&json!({ "value": "s3cret" }))
        .send()
        .unwrap();
    assert_eq!(res.status(), 200, "{}", res.text().unwrap());
}

/// A starter vault made by `vault init --starter`, with the `init` output.
struct Starter {
    vault: Vault,
    /// Holds `dir` until `serve` moves it out.
    _dirs: TempDir,
    dir: PathBuf,
    init: Value,
}

impl Starter {
    fn new() -> Starter {
        let vault = Vault::offline();
        let dirs = TempDir::new().unwrap();
        let dir = dirs.path().join("vault");
        let init = vault.ok(
            &[
                "init",
                dir.to_str().unwrap(),
                "--starter",
                "--password-stdin",
            ],
            "pw\n",
        );
        Starter {
            vault,
            _dirs: dirs,
            dir,
            init,
        }
    }

    /// The ids of the starter notes.
    fn note_ids(&self) -> BTreeSet<String> {
        self.init["starter"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| s(&n["id"]).to_owned())
            .collect()
    }

    /// Serve the starter vault, with an admin token added to it.
    fn serve(self) -> (TestServer, Vault, Value) {
        let token = testkit::seed_credentials(&self.dir);
        // The server owns the vault from here, so it moves out of `dirs`, which goes away.
        let folder = TempDir::new().unwrap();
        std::fs::remove_dir(folder.path()).unwrap();
        std::fs::rename(&self.dir, folder.path()).unwrap();
        let server = TestServer::start_existing(
            folder,
            &token,
            &[("XDG_CONFIG_HOME", self.vault.config_dir())],
        );
        let vault = Vault::against(&server);
        (server, vault, self.init)
    }
}

#[test]
fn init_starter_writes_files_that_all_validate() {
    let starter = Starter::new();
    let dir = &starter.dir;
    for path in [
        "types/task.wcl",
        "types/project.wcl",
        "types/person.wcl",
        "types/note.wcl",
        "taxonomies/status.wcl",
    ] {
        assert!(dir.join(path).is_file(), "no {path}");
    }
    assert_eq!(
        starter.init["starter"]["types"],
        json!(["person", "project", "task", "note"])
    );
    let ids = starter.note_ids();
    assert!(ids.contains(s(&starter.init["starter"]["dashboard"])));
    let written: BTreeSet<String> = files(dir)
        .iter()
        .filter(|(p, _)| p.starts_with("notes"))
        .map(|(p, _)| p.file_stem().unwrap().to_str().unwrap().to_owned())
        .collect();
    assert_eq!(written, ids);

    // Every file is already canonical.
    for (path, bytes) in files(dir) {
        let text = String::from_utf8(bytes).unwrap();
        let rel = path.to_str().unwrap();
        assert_eq!(
            store::canonical(&text, rel).unwrap(),
            text,
            "{rel} is not canonical"
        );
    }

    let out = starter
        .vault
        .ok(&["reindex", "--vault", dir.to_str().unwrap()], "");
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["failures"], json!([]));
    assert_eq!(out["indexed"], json!(ids.len()));
}

#[test]
fn the_dashboard_queries_return_starter_notes() {
    let starter = Starter::new();
    let ids = starter.note_ids();
    let (_server, vault, init) = starter.serve();
    let dashboard = s(&init["starter"]["dashboard"]).to_owned();

    let note = vault.ok(&["get", &dashboard], "");
    let mut views = Vec::new();
    fn walk(blocks: &Value, out: &mut Vec<Value>) {
        for b in blocks.as_array().into_iter().flatten() {
            if b["kind"] == "query_view" {
                out.push(b.clone());
            }
            walk(&b["blocks"], out);
        }
    }
    walk(&note["note"]["blocks"], &mut views);
    let kinds: BTreeSet<String> = views
        .iter()
        .flat_map(|v| v["blocks"].as_array().unwrap().clone())
        .map(|view| s(&view["fields"]["kind"]).to_owned())
        .collect();
    assert!(kinds.contains("kanban"), "{kinds:?}");
    assert!(kinds.contains("calendar"), "{kinds:?}");

    for view in &views {
        let query = s(&view["fields"]["query"]);
        let out = vault.ok(&["query", query, "--this", &dashboard, "--all"], "");
        let rows = out["rows"].as_array().unwrap();
        assert!(!rows.is_empty(), "{query} returned nothing");
        for row in rows {
            assert!(ids.contains(s(&row["id"])), "{query} returned {row}");
        }
    }
}

/// The fenced code blocks of `lang` in a Markdown document.
fn fenced<'a>(doc: &'a str, lang: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<Vec<&'a str>> = None;
    for line in doc.lines() {
        match &mut current {
            None if line.trim_start() == format!("```{lang}") => current = Some(Vec::new()),
            Some(lines) if line.trim_start() == "```" => {
                out.push(lines.join("\n") + "\n");
                current = None;
            }
            Some(lines) => lines.push(line),
            None => {}
        }
    }
    out
}

#[test]
fn every_command_in_the_skill_document_runs_against_a_starter_vault() {
    let doc = Vault::offline().ok(&["skill"], "");
    let text = s(&doc["document"]).to_owned();
    let raw = Command::new(env!("CARGO_BIN_EXE_vault"))
        .args(["skill", "--raw"])
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert_eq!(String::from_utf8(raw.stdout).unwrap(), text);
    assert!(text.starts_with("---\nname: thevault\n"), "no front matter");

    // Every command block is `sh`, so every command the document shows is run below.
    for other in ["bash", "shell", "console", "text"] {
        assert!(
            fenced(&text, other).iter().all(|b| !b.contains("vault ")),
            "a ```{other} block holds a command"
        );
    }
    let blocks = fenced(&text, "sh");
    assert!(blocks.len() >= 10, "only {} command blocks", blocks.len());
    for needed in [
        "vault schema",
        "--dry-run",
        "vault upsert",
        "vault query",
        "vault init",
    ] {
        assert!(
            blocks.iter().any(|b| b.contains(needed)),
            "no block runs {needed}"
        );
    }

    let (server, vault, _) = Starter::new().serve();
    let bin_dir = Path::new(env!("CARGO_BIN_EXE_vault")).parent().unwrap();
    let path = std::env::join_paths(std::iter::once(bin_dir.to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    for block in &blocks {
        // `$ID` and `$VERSION`: a task of the block's own, so blocks cannot disturb each other.
        let note = vault.ok(
            &["put", "--type", "task"],
            "title = \"Scratch\"\nstatus = \"Todo\"\n",
        );
        let cwd = TempDir::new().unwrap();
        let out = Command::new("sh")
            .args(["-e", "-c", block])
            .current_dir(cwd.path())
            .env("PATH", &path)
            .env("VAULT_URL", server.base_url())
            .env("VAULT_TOKEN", server.admin_token())
            .env("VAULT_PASSWORD", "a password")
            .env("ID", s(&note["id"]))
            .env("VERSION", s(&note["version"]))
            .env_remove("VAULT_DIR")
            .env_remove("VAULT_SECRET_KEY_FILE")
            .env("XDG_CONFIG_HOME", cwd.path())
            .env("HOME", cwd.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "this block of the skill document failed:\n{block}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// What the skill document says about batches, references, upsert keys and rows, against the
/// starter types it describes.
#[test]
fn the_skill_documents_how_batches_report_and_refer() {
    let (_server, vault, _) = Starter::new().serve();
    let upsert = |ty: &str, key: &str, fields: Value| json!({ "op": "upsert", "type": ty, "key": key, "note": { "fields": fields } });
    let first = json!([
        upsert(
            "person",
            "email",
            json!({ "title": "Robin", "email": "robin@example.com" })
        ),
        upsert("project", "title", json!({ "title": "Launch the shop" })),
    ])
    .to_string();

    // A dry run's ids are not the ids the real run creates.
    let dry = vault.ok(&["batch", "--dry-run"], &first);
    assert_eq!(dry["items"][0]["version"], Value::Null);
    let real = vault.ok(&["batch"], &first);
    assert_ne!(dry["items"][0]["id"], real["items"][0]["id"]);
    let person = s(&real["items"][0]["id"]).to_owned();
    let project = s(&real["items"][1]["id"]).to_owned();

    // Setting one side of a two-way reference writes the other, and `also` reports it.
    let second = json!([upsert(
        "task",
        "title",
        json!({ "title": "Order the stock", "assignee": person, "project": project }),
    )])
    .to_string();
    let out = vault.ok(&["batch"], &second);
    let task = s(&out["items"][0]["id"]).to_owned();
    assert_eq!(out["also"][0]["id"], json!(project), "{out}");
    assert_eq!(out["also"][0]["type"], "project");
    assert_eq!(out["also"][0]["event"], "update");
    let got = vault.ok(&["get", &project], "");
    assert_eq!(got["note"]["fields"]["tasks"], json!([task]), "{got}");

    // A failed batch carries `status` and `error` at the top as well as per item.
    let bad = json!([upsert(
        "task",
        "title",
        json!({ "title": "Bad", "status": "Nope" })
    )])
    .to_string();
    let failed = vault.fails(&["batch", "--dry-run"], &bad);
    assert_eq!(failed["status"], 422);
    assert_eq!(failed["error"], Value::Null);
    assert_eq!(failed["items"][0]["error"]["error"], "invalid");

    // An upsert key matching two notes is refused with their ids; a list field is no key.
    vault.ok(&["put", "--type", "task"], "title = \"Twin\"\n");
    vault.ok(&["put", "--type", "task"], "title = \"Twin\"\n");
    let twins = vault.fails(
        &["upsert", "--type", "task", "--key", "title"],
        "title = \"Twin\"\n",
    );
    let item = &twins["items"][0]["error"];
    assert_eq!(item["error"], "key_not_unique", "{twins}");
    assert_eq!(item["status"], 409);
    assert_eq!(item["ids"].as_array().unwrap().len(), 2);
    vault.fails(
        &["upsert", "--type", "project", "--key", "tasks"],
        "title = \"X\"\n",
    );

    // An upsert keeps the fields it was not sent.
    let out = vault.ok(
        &["upsert", "--type", "task", "--key", "title"],
        "title = \"Order the stock\"\npriority = \"high\"\n",
    );
    assert_eq!(out["items"][0]["result"], "updated", "{out}");
    let got = vault.ok(&["get", &task], "");
    assert_eq!(got["note"]["fields"]["assignee"], json!(person));

    // A JSON content field is a block, and rows leave it out.
    let note = json!({
        "fields": { "title": "With a body" },
        "blocks": [{ "kind": "content", "labels": ["body"],
                     "blocks": [{ "kind": "paragraph", "fields": { "text": "Hello" } }] }],
    })
    .to_string();
    let put = vault.ok(&["put", "--type", "task", "--json"], &note);
    assert!(s(&put["wcl"]).contains("content body {"), "{put}");
    let rows = vault.ok(&["query", "from task where title == \"With a body\""], "");
    let row = &rows["rows"][0];
    assert_eq!(row["title"], "With a body", "{rows}");
    assert!(row["fields"].get("body").is_none(), "{row}");
}

#[test]
fn a_builder_model_round_trips_through_query_parse_and_query_text() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let text = "from task where (title == \"A\" || estimate > 2) && status != \"Done\" sort title desc limit 5";
    let parsed = vault.ok(&["query-parse", text], "");
    let model = parsed["model"].clone();
    assert!(model.is_object(), "no builder model: {parsed}");

    // The model alone on stdin, and the whole `query-parse` answer from a file.
    let printed = vault.ok(&["query-text"], &model.to_string());
    assert_eq!(printed["text"], parsed["text"]);
    let scratch = TempDir::new().unwrap();
    let file = tmp_path(&scratch, "parsed.json");
    std::fs::write(&file, parsed.to_string()).unwrap();
    assert_eq!(
        vault.ok(&["query-text", "-f", &file], "")["text"],
        parsed["text"]
    );

    let again = vault.ok(&["query-parse", s(&printed["text"])], "");
    assert_eq!(again["model"], model);
    assert_eq!(again["text"], printed["text"]);

    let not_json = vault.fails(&["query-text"], "from task");
    assert_eq!(not_json["error"], "usage");
    let not_a_model = vault.fails(&["query-text"], r#"{"from": 3}"#);
    assert_eq!(not_a_model["status"], 400);
}

#[test]
fn init_without_a_terminal_needs_password_stdin() {
    let vault = Vault::offline();
    let dirs = TempDir::new().unwrap();
    let dir = tmp_path(&dirs, "v");
    let out = vault.fails(&["init", &dir], "");
    assert_eq!(out["error"], "usage");
    assert!(!Path::new(&dir).exists());
}

#[test]
fn every_command_prints_json() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let scratch = TempDir::new().unwrap();
    let mut seen = BTreeSet::new();
    let mut run = |words: &str, args: &[&str], stdin: &str| -> Value {
        seen.insert(words.to_owned());
        let mut all: Vec<&str> = words.split(' ').collect();
        all.extend(args);
        // `Vault::run` panics on anything that is not JSON.
        let out = vault.run(&all, stdin);
        assert!(out.ok, "vault {all:?} failed:\n{}", out.stdout);
        out.json
    };

    let listing = run("commands", &[], "");
    run("skill", &[], "");
    run("health", &[], "");
    run("schema", &[], "");

    let a = run("put", &["--type", "task"], "title = \"A\"\n");
    let id = s(&a["id"]).to_owned();
    let b = run(
        "put",
        &[&id, "--base", s(&a["version"])],
        "title = \"A2\"\n",
    );
    run("list", &["--type", "task"], "");
    run("get", &[&id, "--version", s(&a["version"])], "");
    run("history", &[&id], "");
    let c = run(
        "restore",
        &[&id, s(&a["version"]), "--base", s(&b["version"])],
        "",
    );
    run("delete", &[&id, "--base", s(&c["version"])], "");
    run("trash list", &[], "");
    run("trash restore", &[&id], "");
    let ops = json!([{ "op": "create", "type": "task", "wcl": "title = \"B\"\n" }]).to_string();
    run("batch", &["--dry-run"], &ops);
    run(
        "upsert",
        &["--type", "task", "--key", "ext_id", "--dry-run"],
        "title = \"C\"\next_id = \"X\"\n",
    );
    run("query", &["from task", "--all", "--page-size", "1"], "");
    let parsed = run("query-parse", &["from task sort title asc"], "");
    run("query-text", &[], &parsed["model"].to_string());
    run("search", &["A2"], "");
    run("flagged", &[], "");

    let file = tmp_path(&scratch, "f.txt");
    std::fs::write(&file, "hello").unwrap();
    let asset = run("asset upload", &[&file], "");
    run("asset list", &[], "");
    run(
        "asset get",
        &[s(&asset["id"]), "--out", &tmp_path(&scratch, "g.txt")],
        "",
    );
    run("asset delete", &[s(&asset["id"])], "");

    run("type list", &[], "");
    run("type get", &["task"], "");
    run(
        "type put",
        &["memo"],
        "label = \"Memo\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n",
    );
    run("type delete", &["memo"], "");
    let changed = TASK.replace(
        "field estimate {",
        "field owner {\n  kind = \"string\"\n}\n\nfield estimate {",
    );
    let plan = run("type change", &["task"], &changed);
    run(
        "type change",
        &["task", "--apply", s(&plan["plan"])],
        &changed,
    );

    run("template list", &["task"], "");
    run("template get", &["task", "default"], "");
    run(
        "template put",
        &["task", "card"],
        "template card {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n",
    );
    run("template delete", &["task", "card"], "");

    run("taxonomy list", &[], "");
    run("taxonomy get", &["status"], "");
    run(
        "taxonomy put",
        &["colour"],
        "label = \"Colour\"\n\nterm \"Red\"\n",
    );
    run("taxonomy delete", &["colour"], "");

    run(
        "script put",
        &["tick"],
        "use vault\nfn run() { vault::log(\"tick\") }\n",
    );
    run("script list", &[], "");
    run("script get", &["tick"], "");
    run(
        "task put",
        &["tick"],
        "schedule = \"0 0 1 1 *\"\nscript = \"tick\"\n",
    );
    run("task list", &[], "");
    run("task get", &["tick"], "");
    let ran = run("task run", &["tick"], "");
    run("task history", &["tick", "--limit", "5"], "");
    run("task show-run", &["tick", &ran["id"].to_string()], "");
    run("task delete", &["tick"], "");
    run("script delete", &["tick"], "");

    let token = run("token create", &["agent", "--scope", "read"], "");
    assert!(token["secret"].is_string());
    run("token list", &[], "");
    run("token revoke", &["agent"], "");

    run("secret set", &["api_key"], "s3cret\n");
    run("secret list", &[], "");
    run("secret get", &["api_key"], "");
    run("secret delete", &["api_key"], "");

    let board = run(
        "put",
        &["--type", "board"],
        "title = \"Plan\"\n\ncanvas board {\n  viewport {\n    x    = 0.0\n    y    = 0.0\n    zoom = 1.0\n  }\n}\n",
    );
    let canvas = run("canvas get", &[s(&board["id"]), "board"], "");
    run(
        "canvas put",
        &[s(&board["id"]), "board", "--base", s(&canvas["version"])],
        "canvas {\n  viewport {\n    x    = 1.0\n    y    = 2.0\n    zoom = 0.5\n  }\n}\n",
    );

    // The local commands, on a vault of their own.
    let dir = tmp_path(&scratch, "local");
    run("init", &[&dir, "--password-stdin"], "pw\n");
    run("passwd", &["--vault", &dir, "--password-stdin"], "pw2\n");
    run("reindex", &["--vault", &dir], "");
    let archive = tmp_path(&scratch, "local.backup");
    run("backup", &[&archive, "--vault", &dir], "");
    run(
        "restore-backup",
        &[&archive, &tmp_path(&scratch, "again")],
        "",
    );

    let all: BTreeSet<String> = listing["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| s(&c["command"]).to_owned())
        .collect();
    let missed: Vec<_> = all.difference(&seen).collect();
    assert!(missed.is_empty(), "commands never run: {missed:?}");

    // Failures print JSON too.
    let missing = vault.fails(&["get", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA"], "");
    assert_eq!(missing["status"], 404);
    let unauthorised = Vault {
        token: None,
        ..Vault::against(&server)
    }
    .fails(&["list"], "");
    assert_eq!(unauthorised["status"], 401);
    // A port nothing listens on: bound, read back, then released.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let offline = Vault {
        url: Some(format!("http://127.0.0.1:{port}")),
        ..Vault::offline()
    }
    .fails(&["health"], "");
    assert_eq!(offline["error"], "connect");
}

#[test]
fn asset_delete_is_refused_while_a_note_names_it_and_unlink_takes_it_out() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let dir = TempDir::new().unwrap();
    let file = tmp_path(&dir, "photo.png");
    std::fs::write(&file, b"bytes").unwrap();
    let asset = vault.ok(&["asset", "upload", &file], "");
    let id = s(&asset["id"]).to_owned();
    let wcl = format!(
        "title = \"Board\"\n\ncanvas board {{\n  viewport {{\n    x    = 0.0\n    y    = 0.0\n    zoom = 1.0\n  }}\n\n  node pic {{\n    kind   = \"asset\"\n    x      = 0.0\n    y      = 0.0\n    width  = 10.0\n    height = 10.0\n    asset  = \"{id}\"\n  }}\n}}\n"
    );
    let note = vault.ok(&["put", "--type", "board"], &wcl);
    let note_id = s(&note["id"]).to_owned();

    let err = vault.fails(&["asset", "delete", &id], "");
    assert_eq!(err["error"], "asset_in_use", "{err}");
    assert_eq!(err["notes"][0]["id"], note_id.as_str(), "{err}");

    let out = vault.ok(&["asset", "delete", &id, "--unlink"], "");
    assert_eq!(out["deleted"], true);
    assert_eq!(out["notes"][0]["id"], note_id.as_str(), "{out}");
    let now = vault.ok(&["get", &note_id], "");
    assert!(!s(&now["wcl"]).contains(&id), "{now}");
    assert!(
        vault.ok(&["asset", "list"], "")["assets"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
