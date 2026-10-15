//! The host functions a scheduled task reaches beyond the basics: patching and appending, paging
//! and search, links, the trash, assets, types and taxonomies, daily notes, the `fs` module and
//! HTTP to any host.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use script::{Error, Host, Job, Kind, Limits, Vault};
use serde_json::{Value, json};
use state::{Author, SecretKey, State};
use tempfile::TempDir;
use testkit::{Reply, StandIn};
use writer::{Input, Writer};

const ITEM: &str = r#"label = "Item"

field estimate {
  kind = "int"
}

field tags {
  kind     = "taxonomy"
  taxonomy = "tags"
  multiple = true
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

field parent {
  kind   = "reference"
  target = "item"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate"
    slot "tags"
    slot "status"
    slot "parent"
    slot "body"
  }
}
"#;

const TAGS: &str = "label = \"Tags\"\nallow_new = true\n\nterm \"Garden\"\n";
const STATUS: &str = "label = \"Status\"\n\nterm \"Open\"\n\nterm \"Done\"\n";

struct Fixture {
    dir: TempDir,
    /// A directory outside the vault the tests give tasks access to.
    files: TempDir,
    writer: Arc<Writer>,
    host: Host,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::create_dir_all(dir.path().join("taxonomies")).unwrap();
    std::fs::write(dir.path().join("types/item.wcl"), ITEM).unwrap();
    std::fs::write(dir.path().join("taxonomies/tags.wcl"), TAGS).unwrap();
    std::fs::write(dir.path().join("taxonomies/status.wcl"), STATUS).unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    let state = State::open(
        &vault.server_dir().join("state.db"),
        Some(SecretKey::generate()),
    )
    .unwrap();
    let writer = Arc::new(Writer::open(vault, state).unwrap());
    writer.ensure_built_in_types().unwrap();
    let host = Host::new(
        writer.clone() as Arc<dyn Vault>,
        2,
        Limits {
            timeout: Duration::from_secs(600),
            ..Limits::default()
        },
    );
    Fixture {
        dir,
        files: tempfile::tempdir().unwrap(),
        writer,
        host,
    }
}

impl Fixture {
    fn files(&self) -> PathBuf {
        std::fs::canonicalize(self.files.path()).unwrap()
    }

    fn make(&self, fields: Value) -> writer::Saved {
        self.writer
            .create(
                "item",
                &Input::Json(json!({ "fields": fields })),
                &Author::Session("test".into()),
            )
            .unwrap()
    }

    fn note(&self, id: &str) -> Value {
        writer::doc_to_json(&self.writer.read(id).unwrap().doc)
    }

    /// Run `run` of `source` as task `digest`, with read-write access to the files directory.
    fn task(&self, source: &str) -> Result<Value, Error> {
        let mut j = Job::new(Kind::Task, "digest", source, "run");
        j.author = Some(Author::Task("digest".into()));
        j.write_paths = vec![self.files().to_string_lossy().into_owned()];
        self.host.run(j).result
    }
}

fn ok(r: Result<Value, Error>) -> Value {
    r.unwrap_or_else(|e| panic!("the script failed: {e}"))
}

#[test]
fn patch_sets_some_fields_and_keeps_the_rest() {
    let f = fixture();
    let a = f.make(json!({ "title": "Alpha", "estimate": 2, "status": "Open" }));
    let src = r#"use vault
fn run() -> Result[Value, string] {
    vault::patch("$ID", Value::Map(#{ "estimate": Value::Int(5), "status": Value::Null }), "")
}
"#
    .replace("$ID", &a.id);
    let out = ok(f.task(&src));
    assert_eq!(out["note"]["fields"]["estimate"], json!(5));
    let fields = &f.note(&a.id)["fields"];
    assert_eq!(fields["title"], json!("Alpha"));
    assert_eq!(fields["estimate"], json!(5));
    assert!(fields.get("status").is_none(), "{fields}");
    // The change is recorded as the task's.
    let history = f.writer.history(&a.id).unwrap();
    assert_eq!(
        history.last().unwrap().author,
        Author::Task("digest".into())
    );

    // A stale version is refused.
    let stale = r#"use vault
fn run() -> Result[Value, string] {
    vault::patch("$ID", Value::Map(#{ "estimate": Value::Int(9) }), "$V")
}
"#
    .replace("$ID", &a.id)
    .replace("$V", &a.version);
    let err = f.task(&stale).unwrap_err().to_string();
    assert!(err.contains("version"), "{err}");
}

#[test]
fn append_text_adds_paragraphs_to_a_content_field() {
    let f = fixture();
    let a = f.make(json!({ "title": "Journal" }));
    let src = r#"use vault
fn run() -> Result[Value, string] {
    vault::append_text("$ID", "body", "First line\n\nSecond **bold**", "")?
    vault::append_blocks("$ID", "body", Value::Map(#{
        "kind": Value::String("heading"),
        "fields": Value::Map(#{ "level": Value::Int(2), "text": Value::String("Later") }),
    }), "")
}
"#
    .replace("$ID", &a.id);
    ok(f.task(&src));
    let blocks = &f.note(&a.id)["blocks"];
    let content = blocks
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "content")
        .unwrap();
    let texts: Vec<&Value> = content["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| &b["fields"]["text"])
        .collect();
    assert_eq!(
        texts,
        [
            &json!("First line"),
            &json!("Second **bold**"),
            &json!("Later")
        ]
    );
}

#[test]
fn query_pages_search_and_links() {
    let f = fixture();
    let parent = f.make(json!({ "title": "Parent" }));
    for n in 0..5 {
        f.make(json!({ "title": format!("Child {n}"), "parent": parent.id }));
    }
    let mention = f.writer
        .create(
            "item",
            &Input::Json(json!({
                "fields": { "title": "Mentions" },
                "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
                    { "kind": "paragraph", "fields": { "text": format!("See [[{}]] and zucchini", parent.id) } }
                ] }],
            })),
            &Author::Session("test".into()),
        )
        .unwrap();

    let src = r#"use vault
fn run() -> Result[Value, string] {
    let first = vault::query_page("from item where parent == \"$PARENT\" sort title", 2, "")?
    let next = first.get("next").unwrap().as_string().unwrap()
    let second = vault::query_page("from item where parent == \"$PARENT\" sort title", 2, next)?
    let hits = vault::search("zucchini", 10)?
    let back = vault::backlinks("$PARENT")?
    let out = vault::links("$MENTION")?
    Ok(Value::Map(#{
        "first": first,
        "second": second,
        "hits": Value::List(hits),
        "back": Value::List(back),
        "out": Value::List(out),
    }))
}
"#
    .replace("$PARENT", &parent.id)
    .replace("$MENTION", &mention.id);
    let out = ok(f.task(&src));
    assert_eq!(out["first"]["total"], json!(5));
    assert_eq!(out["first"]["rows"][0]["title"], json!("Child 0"));
    assert_eq!(out["second"]["rows"][0]["title"], json!("Child 2"));
    assert_eq!(out["hits"][0]["id"], json!(mention.id));
    let back = out["back"].as_array().unwrap();
    assert_eq!(back.len(), 6, "{back:?}");
    assert!(
        back.iter()
            .any(|b| b["via"] == "mention" && b["id"] == json!(mention.id))
    );
    assert!(
        back.iter()
            .any(|b| b["via"] == "field" && b["field"] == "parent")
    );
    assert_eq!(out["out"][0]["id"], json!(parent.id));
    assert_eq!(out["out"][0]["title"], json!("Parent"));
}

#[test]
fn delete_then_restore_brings_a_note_back() {
    let f = fixture();
    let a = f.make(json!({ "title": "Doomed" }));
    let src = r#"use vault
fn run() -> Result[Value, string] {
    let note = vault::get("$ID")?
    vault::delete("$ID", note.get("version").unwrap().as_string().unwrap())?
    vault::restore("$ID")
}
"#
    .replace("$ID", &a.id);
    let out = ok(f.task(&src));
    assert_eq!(out["note"]["fields"]["title"], json!("Doomed"));
    assert_eq!(f.note(&a.id)["fields"]["title"], json!("Doomed"));
}

#[test]
fn assets_upload_from_bytes_and_from_an_allowed_file() {
    let f = fixture();
    std::fs::write(
        f.files().join("photo.png"),
        [0x89u8, b'P', b'N', b'G', 0, 1, 2],
    )
    .unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("other.png"), b"x").unwrap();
    let src = r#"use vault
fn run() -> Result[Value, string] {
    let a = vault::upload_asset("hello.txt", "aGVsbG8=")?
    let b = vault::upload_asset_file("$FILES/photo.png")?
    let refused = match vault::upload_asset_file("$OUTSIDE/other.png") {
        Ok(_) => "uploaded",
        Err(e) => e,
    }
    let back = vault::read_asset(a.get("id").unwrap().as_string().unwrap())?
    Ok(Value::Map(#{
        "a": a,
        "b": b,
        "back": back,
        "all": Value::List(vault::assets()?),
        "refused": Value::String(refused),
    }))
}
"#
    .replace("$FILES", &f.files().to_string_lossy())
    .replace("$OUTSIDE", &outside.path().to_string_lossy());
    let out = ok(f.task(&src));
    assert_eq!(out["a"]["filename"], json!("hello.txt"));
    assert_eq!(out["b"]["filename"], json!("photo.png"));
    assert_eq!(out["b"]["size"], json!(7));
    assert_eq!(out["back"]["base64"], json!("aGVsbG8="));
    assert_eq!(out["all"].as_array().unwrap().len(), 2);
    assert!(
        out["refused"]
            .as_str()
            .unwrap()
            .contains("outside the directories"),
        "{out}"
    );
    let (_, bytes) = f.writer.asset(out["b"]["id"].as_str().unwrap()).unwrap();
    assert_eq!(bytes, [0x89u8, b'P', b'N', b'G', 0, 1, 2]);
}

#[test]
fn types_taxonomies_and_terms() {
    let f = fixture();
    let src = r#"use vault
fn run() -> Result[Value, string] {
    let added = vault::add_term("tags", "Garden/Roses")?
    let again = vault::add_term("tags", "Garden/Roses")?
    let closed = match vault::add_term("status", "Blocked") {
        Ok(_) => "added",
        Err(e) => e,
    }
    let orphan = match vault::add_term("tags", "Nowhere/Roses") {
        Ok(_) => "added",
        Err(e) => e,
    }
    Ok(Value::Map(#{
        "added": added,
        "again": again,
        "closed": Value::String(closed),
        "orphan": Value::String(orphan),
        "types": Value::List(vault::types()?),
        "taxonomies": Value::List(vault::taxonomies()?),
    }))
}
"#;
    let out = ok(f.task(src));
    assert_eq!(out["added"]["added"], json!(true));
    assert_eq!(out["again"]["added"], json!(false));
    assert!(out["closed"].as_str().unwrap().contains("closed"), "{out}");
    assert!(out["orphan"].as_str().unwrap().contains("no term"), "{out}");
    let item = out["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "item")
        .unwrap();
    let tags = item["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "tags")
        .unwrap();
    assert_eq!(tags["kind"], json!("taxonomy"));
    assert_eq!(tags["taxonomy"], json!("tags"));
    let tax = out["taxonomies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "tags")
        .unwrap();
    assert_eq!(tax["terms"], json!(["Garden", "Garden/Roses"]));
    assert!(
        f.writer
            .schema()
            .taxonomy("tags")
            .unwrap()
            .contains("Garden/Roses")
    );
}

#[test]
fn daily_gets_or_creates_the_day_note() {
    let f = fixture();
    let src = r#"use vault
fn run() -> Result[Value, string] {
    let first = vault::daily("2026-03-04")?
    let second = vault::daily("2026-03-04")?
    let today = vault::daily("")?
    Ok(Value::Map(#{
        "first": first,
        "second": second,
        "today": today,
        "date": Value::String(vault::today()),
        "now": Value::Int(vault::now()),
    }))
}
"#;
    let out = ok(f.task(src));
    assert_eq!(out["first"]["created"], json!(true));
    assert_eq!(out["second"]["created"], json!(false));
    assert_eq!(out["first"]["id"], out["second"]["id"]);
    assert_eq!(out["today"]["created"], json!(true));
    assert_eq!(out["date"].as_str().unwrap().len(), 10);
    assert!(out["now"].as_i64().unwrap() > 1_700_000_000_000);
}

#[test]
fn fs_reads_and_writes_inside_its_directories() {
    let f = fixture();
    let src = r#"use fs
fn run() -> Result[Value, string] {
    let dir = fs::join("$FILES", "out")
    fs::mkdir(dir)?
    let path = fs::join(dir, "log.txt")
    fs::write_text(path, "one\n")?
    fs::append_text(path, "two\n")?
    fs::write_bytes(fs::join(dir, "raw.bin"), "AAEC")?
    let text = fs::read_text(path)?
    let raw = fs::read_bytes(fs::join(dir, "raw.bin"))?
    let entries = fs::list(dir)?
    let names = ""
    for e in entries {
        names = names + e.name + ":" + fmt("{}", e.size) + " "
    }
    let had = fs::exists(path)?
    fs::remove(path)?
    let gone = fs::exists(path)?
    Ok(Value::Map(#{
        "text": Value::String(text),
        "raw": Value::String(raw),
        "names": Value::String(names),
        "had": Value::Bool(had),
        "gone": Value::Bool(gone),
    }))
}
"#
    .replace("$FILES", &f.files().to_string_lossy());
    let out = ok(f.task(&src));
    assert_eq!(out["text"], json!("one\ntwo\n"));
    assert_eq!(out["raw"], json!("AAEC"));
    assert_eq!(out["names"], json!("log.txt:8 raw.bin:3 "));
    assert_eq!(out["had"], json!(true));
    assert_eq!(out["gone"], json!(false));
}

fn try_fs(f: &Fixture, call: &str, read: &[&Path], write: &[&Path]) -> String {
    let src = format!(
        "use fs\nfn run() -> string {{\n    match {call} {{\n        Ok(_) => \"ok\",\n        Err(e) => e,\n    }}\n}}\n"
    );
    let mut j = Job::new(Kind::Task, "digest", &src, "run");
    let s = |p: &[&Path]| p.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    j.read_paths = s(read);
    j.write_paths = s(write);
    match f.host.run(j).result {
        Ok(Value::String(s)) => s,
        other => panic!("{other:?}"),
    }
}

#[test]
fn fs_refuses_paths_outside_its_directories() {
    let f = fixture();
    let files = f.files();
    let ro = files.join("ro");
    let rw = files.join("rw");
    std::fs::create_dir_all(&ro).unwrap();
    std::fs::create_dir_all(&rw).unwrap();
    std::fs::write(ro.join("in.txt"), "in").unwrap();
    std::fs::write(files.join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(files.join("secret.txt"), rw.join("link.txt")).unwrap();
    let vault = std::fs::canonicalize(f.dir.path()).unwrap();
    let p = |p: &Path| p.to_string_lossy().into_owned();

    let cases = [
        (
            format!("fs::read_text(\"{}\")", p(&ro.join("in.txt"))),
            "ok",
        ),
        (
            format!("fs::write_text(\"{}\", \"x\")", p(&ro.join("in.txt"))),
            "outside the directories this task may write",
        ),
        (
            format!("fs::read_text(\"{}\")", p(&files.join("secret.txt"))),
            "outside the directories this task may read",
        ),
        (
            format!("fs::read_text(\"{}/../secret.txt\")", p(&ro)),
            "\"..\"",
        ),
        (
            format!("fs::read_text(\"{}\")", p(&rw.join("link.txt"))),
            "outside the directories",
        ),
        (
            format!("fs::write_text(\"{}\", \"x\")", p(&rw.join("link.txt"))),
            "outside the directories",
        ),
        ("fs::read_text(\"relative.txt\")".to_owned(), "absolute"),
        (
            format!("fs::write_text(\"{}\", \"x\")", p(&rw.join("new.txt"))),
            "ok",
        ),
        (format!("fs::remove(\"{}\")", p(&rw)), "is a directory"),
    ];
    for (call, want) in &cases {
        let got = try_fs(&f, call, &[&ro], &[&rw]);
        assert!(got.contains(want), "{call}: {got}");
    }
    assert_eq!(
        std::fs::read_to_string(files.join("secret.txt")).unwrap(),
        "secret"
    );

    // The vault is refused even when a task names a directory holding it.
    let note = vault.join("types/item.wcl");
    for call in [
        format!("fs::read_text(\"{}\")", p(&note)),
        format!("fs::write_text(\"{}\", \"x\")", p(&note)),
        format!(
            "fs::write_text(\"{}\", \"x\")",
            p(&vault.join(".server/state.db"))
        ),
    ] {
        let parent = vault.parent().unwrap();
        let got = try_fs(&f, &call, &[], &[parent]);
        assert!(got.contains("inside the vault"), "{call}: {got}");
    }
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        ITEM,
        "the vault file is untouched"
    );
}

#[test]
fn fs_is_a_compile_error_outside_tasks() {
    let f = fixture();
    let src = "use fs\nfn run() -> bool { fs::exists(\"/tmp\").unwrap_or(false) }\n";
    for kind in [Kind::Before, Kind::After, Kind::Migration] {
        let err = f.host.check(kind, "s", src).unwrap_err();
        assert!(matches!(err, Error::Compile { .. }), "{kind:?}: {err:?}");
    }
    assert!(f.host.check(Kind::Task, "s", src).is_ok());
}

#[test]
fn a_star_allowlist_reaches_any_host_and_the_body_comes_as_bytes_too() {
    let f = fixture();
    let api = StandIn::start();
    api.route("/data", Reply::text(200, "hi"));
    let src = r#"use http
fn run() -> Result[string, string] {
    let r = http::get("$URL", #{})?
    Ok(r.body + " " + r.body_base64)
}
"#
    .replace("$URL", &api.url("/data"));
    let mut j = Job::new(Kind::Task, "digest", &src, "run");
    j.allow_domains = vec!["*".into()];
    assert_eq!(f.host.run(j).result.unwrap(), json!("hi aGk="));

    // Without the star, the same call is refused.
    let j = Job::new(Kind::Task, "digest", &src, "run");
    let err = f.host.run(j).result.unwrap_err().to_string();
    assert!(err.contains("allowlist"), "{err}");
}

#[test]
fn every_example_in_the_scripts_reference_compiles_as_a_task() {
    let f = fixture();
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/scripts.md"
    ))
    .unwrap();
    let mut examples = 0;
    for (i, block) in doc.split("```rust\n").skip(1).enumerate() {
        let source = block.split("```").next().unwrap();
        if let Err(e) = f.host.check(Kind::Task, "example", source) {
            panic!("example {} does not compile: {e}\n{source}", i + 1);
        }
        examples += 1;
    }
    assert!(examples >= 6, "{examples}");
}

#[test]
fn archive_and_unarchive_change_what_queries_and_search_read() {
    let f = fixture();
    let a = f.make(json!({ "title": "Shelved zucchini" }));
    f.make(json!({ "title": "Fresh zucchini" }));
    let src = r#"use vault
fn run() -> Result[Value, string] {
    let archived = vault::archive("$ID")?
    let default_hits = vault::search("zucchini", 10)?
    let only = vault::search_archived("zucchini", 10, "only")?
    let every = vault::search_archived("zucchini", 10, "include")?
    let listed = vault::query("from item where archived")?
    let all = vault::query("from item include archived")?
    let bad = match vault::search_archived("zucchini", 10, "maybe") {
        Ok(_) => "accepted",
        Err(e) => e,
    }
    Ok(Value::Map(#{
        "archived": archived,
        "default": Value::Int(default_hits.len()),
        "only": Value::List(only),
        "every": Value::Int(every.len()),
        "listed": Value::Int(listed.len()),
        "all": Value::Int(all.len()),
        "bad": Value::String(bad),
    }))
}
"#
    .replace("$ID", &a.id);
    let out = ok(f.task(&src));
    assert_eq!(out["archived"]["note"]["fields"]["archived"], true, "{out}");
    assert_eq!(out["default"], 1);
    assert_eq!(out["only"][0]["id"], a.id.as_str());
    assert_eq!(out["every"], 2);
    assert_eq!(out["listed"], 1);
    assert_eq!(out["all"], 2);
    assert!(
        out["bad"]
            .as_str()
            .unwrap()
            .contains("exclude, include or only"),
        "{out}"
    );
    assert_eq!(f.note(&a.id)["fields"]["archived"], true);

    let back = r#"use vault
fn run() -> Result[Value, string] {
    vault::unarchive("$ID")
}
"#
    .replace("$ID", &a.id);
    ok(f.task(&back));
    assert_eq!(f.note(&a.id)["fields"].get("archived"), None);
}
