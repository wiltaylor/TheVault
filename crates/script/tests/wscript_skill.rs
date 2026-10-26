//! The built-in `wscript` skill (script-tools T5) cannot teach code that does not work: every
//! ```wscript block in its files compiles as a task, and every ```wscript run block also runs,
//! its `main` called with no arguments on a vault with a few `task` notes, and returns `Ok`.
//! Every ```wcl block parses as WCL, and `SKILL.md` has valid frontmatter.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use script::{Host, Job, Kind, Limits, Vault};
use serde_json::json;
use state::{Author, SecretKey, State};
use writer::{Input, Writer};

const TASK: &str = r#"label = "Task"

field done {
  kind = "bool"
}

field status {
  kind = "string"
}

field priority {
  kind = "int"
}

field due {
  kind = "date"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "done"
    slot "status"
    slot "priority"
    slot "due"
    slot "body"
  }
}
"#;

fn skill_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../writer/defaults/skills/wscript")
}

/// Every Markdown file of the skill, by path from its folder, sorted.
fn files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![skill_dir()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                let rel = p
                    .strip_prefix(skill_dir())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The fenced blocks of `text`: their info string and body.
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        match &mut open {
            None => {
                if let Some(info) = line.strip_prefix("```") {
                    open = Some((info.trim().to_owned(), String::new()));
                }
            }
            Some((_, body)) => {
                if line.trim_end() == "```" {
                    out.push(open.take().unwrap());
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    assert!(open.is_none(), "an unclosed fence");
    out
}

fn fixture() -> (tempfile::TempDir, Arc<Writer>, Host) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::write(dir.path().join("types/task.wcl"), TASK).unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    let state = State::open(
        &vault.server_dir().join("state.db"),
        Some(SecretKey::generate()),
    )
    .unwrap();
    let writer = Arc::new(Writer::open(vault, state).unwrap());
    writer.ensure_built_in_types().unwrap();
    for (title, done, status) in [
        ("Write report", false, "Todo"),
        ("Book flights", true, "Done"),
        ("Call the bank", false, "Waiting"),
    ] {
        writer
            .create(
                "task",
                &Input::Json(json!({ "fields": {
                    "title": title, "done": done, "status": status, "priority": 2,
                } })),
                &Author::Session("test".into()),
            )
            .unwrap();
    }
    let host = Host::new(
        writer.clone() as Arc<dyn Vault>,
        2,
        Limits::default(),
        script::Ai::of(&writer),
    );
    (dir, writer, host)
}

#[test]
fn every_example_in_the_wscript_skill_compiles_and_the_runnable_ones_run() {
    let (_dir, _writer, host) = fixture();
    let (mut compiled, mut ran, mut wcl) = (0, 0, 0);
    for (path, text) in files() {
        for (i, (info, body)) in blocks(&text).into_iter().enumerate() {
            let at = format!("{path} block {}", i + 1);
            let mut words = info.split_whitespace();
            match words.next() {
                Some("wscript") => {
                    if let Err(e) = host.check(Kind::Task, "example", &body) {
                        panic!("{at} does not compile: {e}\n{body}");
                    }
                    compiled += 1;
                    let words: Vec<&str> = words.collect();
                    if words.contains(&"run") {
                        let mut job = Job::new(Kind::Task, "example", &body, "main");
                        // `args=<JSON array>` passes `main` its arguments, as run_script does.
                        if let Some(args) = words.iter().find_map(|w| w.strip_prefix("args=")) {
                            let args: serde_json::Value = serde_json::from_str(args)
                                .unwrap_or_else(|e| panic!("{at}: bad args: {e}"));
                            job.args = args.as_array().unwrap().clone();
                        }
                        job.author = Some(Author::Agent("assistant".into()));
                        let out = host.run(job);
                        if let Err(e) = out.result {
                            panic!("{at} fails: {e}\nlog: {}\n{body}", out.log);
                        }
                        ran += 1;
                    }
                }
                Some("wcl") => {
                    if let Err(e) = store::parse_data_only(&body, "example.wcl") {
                        panic!("{at} is not valid WCL: {e}\n{body}");
                    }
                    wcl += 1;
                }
                _ => {}
            }
        }
    }
    assert!(compiled >= 35, "{compiled} wscript examples");
    assert!(ran >= 20, "{ran} runnable examples");
    assert!(wcl >= 2, "{wcl} wcl examples");
}

#[test]
fn the_wscript_skill_md_is_valid() {
    let text = std::fs::read_to_string(skill_dir().join("SKILL.md")).unwrap();
    let (md, issues) = schema::parse_skill_md(&text, "wscript");
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(md.name.as_deref(), Some("wscript"));
    // Every reference SKILL.md points to exists.
    for (path, _) in files() {
        if path.starts_with("references/") {
            assert!(text.contains(&path), "SKILL.md does not list {path}");
        }
    }
    for line in text.lines().filter(|l| l.contains("`references/")) {
        let name = line.split('`').nth(1).unwrap();
        assert!(skill_dir().join(name).is_file(), "{name} is missing");
    }
}
