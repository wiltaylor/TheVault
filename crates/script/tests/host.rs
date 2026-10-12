//! The script host: limits, sandboxing, vault host functions and HTTP against S7's stand-in.

use std::sync::Arc;
use std::time::{Duration, Instant};

use script::{Error, Host, Job, Kind, Limits, Vault};
use serde_json::json;
use state::{Author, SecretKey, State};
use tempfile::TempDir;
use testkit::{Reply, StandIn};
use writer::Writer;

const TASK_TYPE: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate"
  }
}
"#;

struct Fixture {
    _dir: TempDir,
    writer: Arc<Writer>,
    host: Host,
}

/// A wall-clock limit no test that is not about the limit can reach, however loaded the machine
/// running the tests is. The tests about the limit set a tight one on the job instead.
const ROOMY: Duration = Duration::from_secs(600);

/// The default limits with a [`ROOMY`] wall-clock limit.
fn roomy() -> Limits {
    Limits {
        timeout: ROOMY,
        ..Limits::default()
    }
}

fn fixture() -> Fixture {
    fixture_with(roomy())
}

fn fixture_with(limits: Limits) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::write(dir.path().join("types/task.wcl"), TASK_TYPE).unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    let state = State::open(
        &vault.server_dir().join("state.db"),
        Some(SecretKey::generate()),
    )
    .unwrap();
    let writer = Arc::new(Writer::open(vault, state).unwrap());
    let host = Host::new(writer.clone() as Arc<dyn Vault>, 2, limits);
    Fixture {
        _dir: dir,
        writer,
        host,
    }
}

fn field(note: &writer::Note, name: &str) -> serde_json::Value {
    writer::doc_to_json(&note.doc)["fields"][name].clone()
}

fn job(kind: Kind, source: &str) -> Job {
    Job::new(kind, "specimen", source, "run")
}

#[test]
fn an_infinite_loop_stops_at_the_instruction_limit() {
    let f = fixture_with(Limits {
        instructions: 100_000,
        ..roomy()
    });
    let out = f.host.run(job(Kind::Task, "fn run() {\n    loop { }\n}\n"));
    let err = out.result.unwrap_err();
    assert_eq!(
        err,
        Error::InstructionLimit {
            script: "specimen".into(),
            limit: 100_000
        }
    );
    assert_eq!(
        err.to_string(),
        "script \"specimen\" stopped: it ran more than its limit of 100000 instructions"
    );
}

#[test]
fn unbounded_recursion_stops_at_the_call_depth_limit() {
    let f = fixture_with(Limits {
        call_depth: 50,
        ..roomy()
    });
    let src = "fn down(n: int) -> int { down(n + 1) }\nfn run() -> int { down(0) }\n";
    let err = f.host.run(job(Kind::Task, src)).result.unwrap_err();
    assert_eq!(
        err,
        Error::CallDepth {
            script: "specimen".into(),
            limit: 50
        }
    );
}

#[test]
fn a_script_sleeping_in_a_host_call_stops_at_the_wall_clock_timeout() {
    let f = fixture();
    let api = StandIn::start();
    api.route(
        "/slow",
        Reply::text(200, "late").delay(Duration::from_secs(60)),
    );
    let src = format!(
        r#"use http
use vault
fn run() -> string {{
    vault::log("asking")
    match http::get("{}", #{{}}) {{
        Ok(r) => r.body,
        Err(e) => "caught: " + e,
    }}
}}
"#,
        api.url("/slow")
    );
    let mut j = job(Kind::Task, &src);
    j.allow_domains = vec![api.host()];
    j.timeout = Some(Duration::from_millis(500));
    let started = Instant::now();
    let out = f.host.run(j);
    // The timeout ends the run: the script cannot catch it as an `Err` and carry on.
    assert_eq!(
        out.result.unwrap_err(),
        Error::Timeout {
            script: "specimen".into(),
            limit: Duration::from_millis(500)
        }
    );
    // The run ended itself, inside the HTTP call: the host's own wait gives up with an empty log.
    assert_eq!(out.log, "asking\n");
    assert!(started.elapsed() < Duration::from_secs(60));
    assert_eq!(api.requests().len(), 1);
}

#[test]
fn a_host_call_after_the_deadline_ends_the_run() {
    let f = fixture();
    // Pure computation is metered by instructions, not time, so it may run past the deadline;
    // the next host call ends the run.
    let src = r#"use vault
fn run() -> int {
    let i = 0
    while i < 3000000 { i = i + 1 }
    vault::log("too late")
    i
}
"#;
    let mut j = job(Kind::Task, src);
    j.timeout = Some(Duration::from_millis(1));
    let out = f.host.run(j);
    assert!(
        matches!(out.result, Err(Error::Timeout { .. })),
        "{:?}",
        out.result
    );
    assert_eq!(out.log, "");
}

#[test]
fn the_fs_and_process_modules_do_not_exist() {
    let f = fixture();
    let sources = [
        "use fs\nfn run() -> string { fs::read_to_string(\"/etc/passwd\").unwrap() }\n",
        "fn run() -> string { fs::read_to_string(\"/etc/passwd\").unwrap() }\n",
        "use process\nfn run() { process::exit(1) }\n",
        "use time\nfn run() { time::sleep(10) }\n",
    ];
    for kind in [Kind::Before, Kind::After, Kind::Task, Kind::Migration] {
        for src in sources {
            let err = f.host.run(job(kind, src)).result.unwrap_err();
            assert!(
                matches!(&err, Error::Compile { script, .. } if script == "specimen"),
                "{kind:?} {src:?}: {err:?}"
            );
            assert!(f.host.check(kind, "specimen", src).is_err());
        }
    }
}

#[test]
fn a_script_creates_a_note_recorded_as_made_by_the_script() {
    let f = fixture();
    let src = r#"use vault
fn run() -> Result[string, string] {
    let body = Value::Map(#{
        "fields": Value::Map(#{ "title": Value::String("From a script"), "estimate": Value::Int(3) }),
    })
    let saved = vault::create("task", body)?
    vault::log("created")
    Ok(saved.get("id").unwrap().as_string().unwrap())
}
"#;
    let mut j = Job::new(Kind::Task, "rollup", src, "run");
    j.timeout = Some(ROOMY);
    let out = f.host.run(j);
    let id = out.result.unwrap();
    let id = id.as_str().unwrap();
    assert_eq!(out.log, "created\n");

    let note = f.writer.read(id).unwrap();
    assert_eq!(note.type_name, "task");
    assert_eq!(field(&note, "title"), json!("From a script"));
    assert_eq!(field(&note, "estimate"), json!(3));
    let history = f.writer.history(id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].author, Author::Handler("rollup".into()));
}

#[test]
fn a_script_reads_queries_updates_and_deletes_notes() {
    let f = fixture();
    let make = |title: &str, estimate: i64| {
        f.writer
            .create(
                "task",
                &writer::Input::Json(json!({ "fields": { "title": title, "estimate": estimate } })),
                &Author::Session("test".into()),
            )
            .unwrap()
    };
    let a = make("Alpha", 2);
    let b = make("Beta", 5);

    // Query, then bump the estimate of every matching note through `get` and `update`.
    let src = r#"use vault
fn run() -> Result[int, string] {
    let rows = vault::query("from task where estimate > 1 sort title")?
    let n = 0
    for row in rows {
        let id = row.get("id").unwrap().as_string().unwrap()
        let note = vault::get(id)?
        let version = note.get("version").unwrap().as_string().unwrap()
        let title = note.get("note").unwrap().get("fields").unwrap().get("title").unwrap().as_string().unwrap()
        let body = Value::Map(#{
            "fields": Value::Map(#{
                "title": Value::String(title + "!"),
                "estimate": Value::Int(10),
            }),
        })
        vault::update(id, body, version)?
        n = n + 1
    }
    Ok(n)
}
"#;
    let out = f.host.run(job(Kind::Before, src));
    assert_eq!(out.result.unwrap(), json!(2));
    let a2 = f.writer.read(&a.id).unwrap();
    assert_eq!(field(&a2, "title"), json!("Alpha!"));
    assert_eq!(field(&a2, "estimate"), json!(10));
    assert_eq!(
        f.writer.history(&a.id).unwrap()[1].author,
        Author::Handler("specimen".into())
    );

    // A stale version is the same 409 conflict a client gets, as an `Err` the script sees.
    let stale = format!(
        r#"use vault
fn run() -> Result[string, string] {{
    match vault::delete("{}", "{}") {{
        Ok(_) => Ok("deleted"),
        Err(e) => Ok("refused: " + e),
    }}
}}
"#,
        b.id, b.version
    );
    let out = f.host.run(job(Kind::After, &stale));
    let msg = out.result.unwrap();
    assert!(msg.as_str().unwrap().starts_with("refused: note"), "{msg}");

    let current = f.writer.read(&b.id).unwrap().version;
    let delete = format!(
        "use vault\nfn run() -> Result[Value, string] {{ vault::delete(\"{}\", \"{current}\") }}\n",
        b.id
    );
    let out = f.host.run(job(Kind::After, &delete));
    assert_eq!(out.result.unwrap()["id"], json!(b.id));
    assert!(f.writer.read(&b.id).is_err());
    assert_eq!(f.writer.trash().unwrap()[0].note_id, b.id);
}

#[test]
fn an_invalid_note_from_a_script_is_an_err_it_can_handle() {
    let f = fixture();
    let src = r#"use vault
fn run() -> string {
    let body = Value::Map(#{ "fields": Value::Map(#{ "estimate": Value::String("lots") }) })
    match vault::create("task", body) {
        Ok(_) => "created",
        Err(e) => e,
    }
}
"#;
    let out = f.host.run(job(Kind::Task, src));
    let msg = out.result.unwrap();
    assert!(msg.as_str().unwrap().contains("title"), "{msg}");
    assert!(f.writer.list(None).unwrap().is_empty());
}

#[test]
fn http_reaches_an_allowed_host_and_refuses_any_other() {
    let f = fixture();
    let api = StandIn::start();
    api.route(
        "/hook",
        Reply::json(201, &json!({ "ok": true })).header("x-api", "stand-in"),
    );
    let src = format!(
        r#"use http
use secrets
fn run() -> Result[string, string] {{
    let token = secrets::get("hook_token")?
    let r = http::post("{}", #{{ "authorization": "Bearer " + token }}, "hello")?
    Ok(fmt("{{}} {{}} {{}}", r.status, r.headers.get("x-api").unwrap(), r.body))
}}
"#,
        api.url("/hook")
    );
    f.writer.state().set_secret("hook_token", "s3cret").unwrap();

    for kind in [Kind::After, Kind::Task] {
        let mut allowed = job(kind, &src);
        allowed.allow_domains = vec![api.host()];
        allowed.secrets = vec!["hook_token".into()];
        let out = f.host.run(allowed);
        assert_eq!(out.result.unwrap(), json!("201 stand-in {\"ok\":true}"));
    }
    let seen = api.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/hook");
    assert_eq!(seen[0].header("authorization"), Some("Bearer s3cret"));
    assert_eq!(seen[0].body, b"hello");

    // `localhost` names the same machine but is not the host on the allowlist.
    for (allow, url) in [
        (vec!["example.com".to_owned()], api.url("/hook")),
        (vec![], api.url("/hook")),
        (
            vec![api.host()],
            format!("http://localhost:{}/hook", api.addr().port()),
        ),
    ] {
        let src = format!(
            "use http\nfn run() -> Result[int, string] {{ Ok(http::get(\"{url}\", #{{}})?.status) }}\n"
        );
        let mut denied = job(Kind::Task, &src);
        denied.allow_domains = allow;
        let err = f.host.run(denied).result.unwrap_err();
        let Error::Returned { message, .. } = err else {
            panic!("{err:?}");
        };
        assert!(
            message.contains("is not on this script's allowlist"),
            "{message}"
        );
    }
    assert_eq!(
        api.requests().len(),
        2,
        "a refused call never leaves the host"
    );
}

#[test]
fn http_does_not_follow_a_redirect_to_another_host() {
    let f = fixture();
    let api = StandIn::start();
    api.route(
        "/moved",
        Reply::text(302, "").header("location", "http://example.com/"),
    );
    let src = format!(
        "use http\nfn run() -> Result[int, string] {{ Ok(http::get(\"{}\", #{{}})?.status) }}\n",
        api.url("/moved")
    );
    let mut j = job(Kind::Task, &src);
    j.allow_domains = vec![api.host()];
    assert_eq!(f.host.run(j).result.unwrap(), json!(302));
}

#[test]
fn an_http_response_over_the_size_limit_is_refused() {
    let f = fixture_with(Limits {
        max_http_response: 1000,
        ..roomy()
    });
    let api = StandIn::start();
    api.route("/big", Reply::text(200, "x".repeat(1001)));
    api.route("/fits", Reply::text(200, "x".repeat(1000)));
    let run = |path: &str| {
        let src = format!(
            "use http\nfn run() -> Result[int, string] {{ Ok(http::get(\"{}\", #{{}})?.body.len()) }}\n",
            api.url(path)
        );
        let mut j = job(Kind::Task, &src);
        j.allow_domains = vec![api.host()];
        f.host.run(j).result
    };
    assert_eq!(run("/fits").unwrap(), json!(1000));
    let err = run("/big").unwrap_err();
    assert!(
        err.to_string().contains("larger than the 1000 bytes"),
        "{err}"
    );
}

#[test]
fn http_and_secrets_are_unavailable_in_a_before_handler() {
    let f = fixture();
    let api = StandIn::start();
    api.route("/hook", Reply::text(200, "ok"));
    let http = format!(
        "use http\nfn run() -> Result[int, string] {{ Ok(http::get(\"{}\", #{{}})?.status) }}\n",
        api.url("/hook")
    );
    let secret = "use secrets\nfn run() -> Result[string, string] { secrets::get(\"x\") }\n";
    for src in [http.as_str(), secret] {
        for kind in [Kind::Before, Kind::Migration] {
            let mut j = job(kind, src);
            j.allow_domains = vec![api.host()];
            j.secrets = vec!["x".into()];
            let err = f.host.run(j).result.unwrap_err();
            assert!(matches!(err, Error::Compile { .. }), "{kind:?}: {err:?}");
        }
        assert!(f.host.check(Kind::After, "specimen", src).is_ok());
        assert!(f.host.check(Kind::Task, "specimen", src).is_ok());
    }
    assert!(api.requests().is_empty());
}

#[test]
fn a_script_reads_only_the_secrets_it_was_granted() {
    let f = fixture();
    f.writer.state().set_secret("granted", "one").unwrap();
    f.writer.state().set_secret("other", "two").unwrap();
    let src = r#"use secrets
fn read(name: string) -> string {
    match secrets::get(name) {
        Ok(v) => v,
        Err(e) => e,
    }
}
fn run() -> List[string] { [read("granted"), read("other"), read("missing")] }
"#;
    let mut j = job(Kind::Task, src);
    j.secrets = vec!["granted".into(), "missing".into()];
    assert_eq!(
        f.host.run(j).result.unwrap(),
        json!([
            "one",
            "secret \"other\" is not granted to this script",
            "there is no secret named \"missing\""
        ])
    );
}

#[test]
fn arguments_go_in_and_errors_come_out() {
    let f = fixture();
    let src = r#"fn before_update(note: Value) -> Result[Value, string] {
    let title = note.get("fields").unwrap().get("title").unwrap().as_string().unwrap()
    if title == "" { return Err("a title is required") }
    println("checked " + title)
    Ok(note)
}
"#;
    let mut j = Job::new(Kind::Before, "specimen", src, "before_update");
    let note = json!({ "fields": { "title": "Hi", "estimate": 2 }, "blocks": [] });
    j.args = vec![note.clone()];
    let out = f.host.run(j.clone());
    assert_eq!(out.result.unwrap(), note);
    assert_eq!(out.log, "checked Hi\n");

    j.args = vec![json!({ "fields": { "title": "" } })];
    assert_eq!(
        f.host.run(j.clone()).result.unwrap_err(),
        Error::Returned {
            script: "specimen".into(),
            message: "a title is required".into()
        }
    );

    j.args = vec![];
    assert!(matches!(
        f.host.run(j.clone()).result,
        Err(Error::Entry { .. })
    ));
    j.function = "after_update".into();
    assert!(matches!(f.host.run(j).result, Err(Error::Entry { .. })));
}

#[test]
fn a_fault_is_reported_with_its_message() {
    let f = fixture();
    let src = "fn run() -> int { let xs = [1]\n xs[5] }\n";
    let err = f.host.run(job(Kind::Task, src)).result.unwrap_err();
    assert!(matches!(err, Error::Fault { .. }), "{err:?}");
}

#[test]
fn the_log_is_capped() {
    let f = fixture_with(Limits {
        max_log: 100,
        ..roomy()
    });
    let src = "use vault\nfn run() { for i in 0..50 { vault::log(\"0123456789\") } }\n";
    let out = f.host.run(job(Kind::Task, src));
    out.result.unwrap();
    assert!(out.log.len() < 130, "{}", out.log.len());
    assert!(out.log.ends_with("[log truncated]\n"), "{}", out.log);
}

#[test]
fn scripts_run_in_parallel_on_the_workers() {
    let f = fixture();
    let api = StandIn::start();
    api.route(
        "/wait",
        Reply::text(200, "ok").delay(Duration::from_secs(3)),
    );
    let src = format!(
        "use http\nfn run() -> Result[int, string] {{ Ok(http::get(\"{}\", #{{}})?.status) }}\n",
        api.url("/wait")
    );
    let host = Arc::new(f.host);
    let started = Instant::now();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let host = host.clone();
            let mut j = job(Kind::Task, &src);
            j.allow_domains = vec![api.host()];
            std::thread::spawn(move || host.run(j).result)
        })
        .collect();
    for t in threads {
        assert_eq!(t.join().unwrap().unwrap(), json!(200));
    }
    // One after the other would take 6 s.
    let took = started.elapsed();
    assert!(took < Duration::from_millis(5500), "took {took:?}");
}
