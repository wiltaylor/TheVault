//! The exec service's HTTP contract (browser B2), on a free port over temporary work and home
//! directories.

use std::path::Path;
use std::time::{Duration, Instant};

use browser_agent::{Config, FILE_CAP, MAX_RUNS, OUTPUT_CAP};
use serde_json::{Value, json};
use tempfile::TempDir;

const TOKEN: &str = "test-token-0123456789abcdef";

struct Agent {
    base: String,
    work: TempDir,
    home: TempDir,
    http: reqwest::Client,
}

impl Agent {
    async fn start() -> Agent {
        let work = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let config = Config {
            token: TOKEN.into(),
            work_dir: work.path().to_path_buf(),
            home_dir: home.path().to_path_buf(),
            display: ":1".into(),
            playwright_cli: Some("0.0.0-test".into()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(browser_agent::serve(
            listener,
            config,
            std::future::pending(),
        ));
        Agent {
            base,
            work,
            home,
            http: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn run_status(&self, body: Value) -> (u16, Value) {
        let resp = self
            .http
            .post(self.url("/run"))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap())
    }

    async fn run(&self, command: &str) -> Value {
        let (status, body) = self.run_status(json!({ "command": command })).await;
        assert_eq!(status, 200, "{body}");
        body
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> reqwest::Response {
        self.http
            .get(self.url(path))
            .bearer_auth(TOKEN)
            .query(query)
            .send()
            .await
            .unwrap()
    }

    fn work(&self) -> &Path {
        self.work.path()
    }

    fn home(&self) -> &Path {
        self.home.path()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn health_needs_no_token() {
    let b = Agent::start().await;
    let resp = b.http.get(b.url("/health")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["display"], ":1");
    assert_eq!(body["playwright_cli"], "0.0.0-test");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_other_route_needs_the_token() {
    let b = Agent::start().await;
    std::fs::write(b.work().join("a.txt"), "a").unwrap();
    for auth in [None, Some("wrong"), Some("")] {
        let mut run = b
            .http
            .post(b.url("/run"))
            .json(&json!({ "command": "touch ran" }));
        let mut file = b.http.get(b.url("/file?path=a.txt"));
        let mut files = b.http.get(b.url("/files"));
        if let Some(token) = auth {
            run = run.bearer_auth(token);
            file = file.bearer_auth(token);
            files = files.bearer_auth(token);
        }
        for req in [run, file, files] {
            let resp = req.send().await.unwrap();
            assert_eq!(resp.status(), 401, "token {auth:?}");
            let body: Value = resp.json().await.unwrap();
            assert!(body["error"].as_str().unwrap().contains("Bearer"));
        }
    }
    // A token without the Bearer scheme is refused too.
    let resp = b
        .http
        .get(b.url("/files"))
        .header("authorization", TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert!(!b.work().join("ran").exists(), "an unauthorised run ran");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_answers_exit_and_output() {
    let b = Agent::start().await;
    let r = b.run("echo out; echo err >&2; exit 3").await;
    assert_eq!(r["exit"], 3);
    assert_eq!(r["stdout"], "out\n");
    assert_eq!(r["stderr"], "err\n");
    assert_eq!(r["truncated"], false);
    assert_eq!(r["timed_out"], false);
    assert!(r["ms"].as_u64().is_some());

    // DISPLAY, HOME and the working directory.
    let r = b.run("echo \"$DISPLAY|$HOME|$(pwd)\"").await;
    let want = format!(
        ":1|{}|{}\n",
        b.home().canonicalize().unwrap().display(),
        b.work().canonicalize().unwrap().display()
    );
    assert_eq!(r["stdout"], want);
    assert_eq!(r["exit"], 0);

    // A command killed by a signal answers 128 plus the signal.
    let r = b.run("kill -TERM $$").await;
    assert_eq!(r["exit"], 128 + 15);
}

#[tokio::test(flavor = "multi_thread")]
async fn run_takes_a_cwd_under_work_or_home() {
    let b = Agent::start().await;
    std::fs::create_dir(b.work().join("sub")).unwrap();
    let (status, r) = b
        .run_status(json!({ "command": "pwd", "cwd": "sub" }))
        .await;
    assert_eq!(status, 200);
    let sub = b.work().join("sub").canonicalize().unwrap();
    assert_eq!(r["stdout"], format!("{}\n", sub.display()));

    let home = b.home().canonicalize().unwrap();
    let (status, r) = b
        .run_status(json!({ "command": "pwd", "cwd": home.to_str().unwrap() }))
        .await;
    assert_eq!(status, 200);
    assert_eq!(r["stdout"], format!("{}\n", home.display()));

    let (status, _) = b
        .run_status(json!({ "command": "pwd", "cwd": "/etc" }))
        .await;
    assert_eq!(status, 403);
    let (status, _) = b
        .run_status(json!({ "command": "pwd", "cwd": "../.." }))
        .await;
    assert_eq!(status, 403);
    let (status, _) = b
        .run_status(json!({ "command": "pwd", "cwd": "missing" }))
        .await;
    assert_eq!(status, 404);
    std::fs::write(b.work().join("f"), "").unwrap();
    let (status, _) = b.run_status(json!({ "command": "pwd", "cwd": "f" })).await;
    assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn run_checks_its_request() {
    let b = Agent::start().await;
    for body in [
        json!({ "command": "true", "timeout_s": 0 }),
        json!({ "command": "true", "timeout_s": 301 }),
        json!({ "command": "  " }),
        json!({ "timeout_s": 5 }),
    ] {
        let (status, r) = b.run_status(body.clone()).await;
        assert_eq!(status, 400, "{body}: {r}");
        assert!(r["error"].is_string());
    }
    let (status, _) = b
        .run_status(json!({ "command": "true", "timeout_s": 300 }))
        .await;
    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn run_keeps_the_last_64_kib_of_each_stream() {
    let b = Agent::start().await;
    let r = b
        .run("head -c 100000 /dev/zero | tr '\\0' a; printf END; head -c 70000 /dev/zero | tr '\\0' b >&2; printf ERR >&2")
        .await;
    let out = r["stdout"].as_str().unwrap();
    let err = r["stderr"].as_str().unwrap();
    assert_eq!(out.len(), OUTPUT_CAP);
    assert!(out.ends_with("aaaEND"));
    assert_eq!(err.len(), OUTPUT_CAP);
    assert!(err.ends_with("bbbERR"));
    assert_eq!(r["truncated"], true);

    // Exactly the cap is not cut.
    let r = b
        .run(&format!("head -c {OUTPUT_CAP} /dev/zero | tr '\\0' c"))
        .await;
    assert_eq!(r["stdout"].as_str().unwrap().len(), OUTPUT_CAP);
    assert_eq!(r["truncated"], false);

    // One stream over the cap is enough to say so.
    let r = b
        .run(&format!(
            "head -c {} /dev/zero | tr '\\0' c >&2",
            OUTPUT_CAP + 1
        ))
        .await;
    assert_eq!(r["stdout"], "");
    assert_eq!(r["truncated"], true);
}

fn gone_or_zombie(pid: &str) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timeout_kills_the_whole_process_group() {
    let b = Agent::start().await;
    let started = Instant::now();
    let (status, r) = b
        .run_status(json!({
            "command": "sleep 300 & echo $! > bg.pid; echo before; sleep 300",
            "timeout_s": 1,
        }))
        .await;
    let took = started.elapsed();
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["timed_out"], true);
    assert_eq!(r["exit"], 128 + 9);
    assert_eq!(r["stdout"], "before\n");
    assert!(r["ms"].as_u64().unwrap() >= 1000, "{r}");
    assert!(took < Duration::from_secs(10), "took {took:?}");

    let pid = std::fs::read_to_string(b.work().join("bg.pid")).unwrap();
    let pid = pid.trim();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gone_or_zombie(pid) {
        assert!(
            Instant::now() < deadline,
            "the background sleep {pid} survived the timeout"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_process_holding_the_pipes_does_not_hold_the_run() {
    let b = Agent::start().await;
    let started = Instant::now();
    let r = b.run("sleep 30 & echo $! > bg.pid; echo done").await;
    assert_eq!(r["exit"], 0);
    assert_eq!(r["stdout"], "done\n");
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid = std::fs::read_to_string(b.work().join("bg.pid")).unwrap();
    let _ = std::process::Command::new("kill").arg(pid.trim()).status();
}

#[tokio::test(flavor = "multi_thread")]
async fn four_runs_at_a_time_then_429() {
    let b = Agent::start().await;
    let mut running = Vec::new();
    for i in 0..MAX_RUNS {
        let http = b.http.clone();
        let url = b.url("/run");
        running.push(tokio::spawn(async move {
            http.post(url)
                .bearer_auth(TOKEN)
                .json(&json!({ "command": format!("touch started.{i}; sleep 3") }))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let started = std::fs::read_dir(b.work())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("started.")
            })
            .count();
        if started == MAX_RUNS {
            break;
        }
        assert!(Instant::now() < deadline, "only {started} runs started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (status, r) = b.run_status(json!({ "command": "touch fifth" })).await;
    assert_eq!(status, 429, "{r}");
    assert!(!b.work().join("fifth").exists());
    for run in running {
        assert_eq!(run.await.unwrap(), 200);
    }
    // With the four done, a run goes again.
    let r = b.run("echo again").await;
    assert_eq!(r["stdout"], "again\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_reads_under_work_and_home() {
    let b = Agent::start().await;
    std::fs::create_dir(b.work().join(".playwright-cli")).unwrap();
    std::fs::write(
        b.work().join(".playwright-cli/page.yml"),
        "- button \"Go\" [ref=e2]\n",
    )
    .unwrap();
    std::fs::write(b.work().join("shot.png"), [0x89, b'P', b'N', b'G']).unwrap();
    std::fs::write(b.home().join("notes.txt"), "home file").unwrap();

    let resp = b
        .get("/file", &[("path", ".playwright-cli/page.yml")])
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "application/yaml");
    assert_eq!(resp.text().await.unwrap(), "- button \"Go\" [ref=e2]\n");

    let abs = b.work().join("shot.png");
    let resp = b.get("/file", &[("path", abs.to_str().unwrap())]).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "image/png");
    assert_eq!(
        resp.bytes().await.unwrap().as_ref(),
        [0x89, b'P', b'N', b'G']
    );

    let home = b.home().join("notes.txt");
    let resp = b.get("/file", &[("path", home.to_str().unwrap())]).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/plain; charset=utf-8");
    assert_eq!(resp.text().await.unwrap(), "home file");

    let resp = b.get("/file", &[("path", "missing.png")]).await;
    assert_eq!(resp.status(), 404);
    let resp = b.get("/file", &[("path", ".playwright-cli")]).await;
    assert_eq!(resp.status(), 400);
    let resp = b.get("/file", &[]).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn file_refuses_paths_and_symlinks_that_leave_the_roots() {
    let b = Agent::start().await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), b.work().join("link")).unwrap();
    std::os::unix::fs::symlink(outside.path(), b.home().join("linkdir")).unwrap();
    std::fs::write(b.home().join("inside"), "inside").unwrap();
    std::os::unix::fs::symlink(b.home().join("inside"), b.work().join("ok-link")).unwrap();

    let escape = format!("../{}", "x".repeat(3));
    let outside_secret = outside.path().join("secret");
    let through_dir = b.home().join("linkdir/secret");
    for path in [
        "/etc/passwd",
        "link",
        escape.as_str(),
        "../../../../etc/passwd",
        outside_secret.to_str().unwrap(),
        through_dir.to_str().unwrap(),
    ] {
        let resp = b.get("/file", &[("path", path)]).await;
        assert_eq!(resp.status(), 403, "{path}");
        let body: Value = resp.json().await.unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("outside"),
            "{body}"
        );
    }
    // A symlink that stays inside the roots is followed.
    let resp = b.get("/file", &[("path", "ok-link")]).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "inside");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_caps_at_25_mib() {
    let b = Agent::start().await;
    let at = std::fs::File::create(b.work().join("at.bin")).unwrap();
    at.set_len(FILE_CAP).unwrap();
    let over = std::fs::File::create(b.work().join("over.bin")).unwrap();
    over.set_len(FILE_CAP + 1).unwrap();

    let resp = b.get("/file", &[("path", "at.bin")]).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "application/octet-stream");
    assert_eq!(resp.bytes().await.unwrap().len() as u64, FILE_CAP);

    let resp = b.get("/file", &[("path", "over.bin")]).await;
    assert_eq!(resp.status(), 413);
    let body: Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("25 MiB"), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_lists_a_directory() {
    let b = Agent::start().await;
    std::fs::write(b.work().join("b.png"), [1, 2, 3]).unwrap();
    std::fs::create_dir(b.work().join("a-dir")).unwrap();
    std::os::unix::fs::symlink("/etc", b.work().join("c-link")).unwrap();
    std::fs::write(b.work().join("a-dir/inner.txt"), "x").unwrap();

    let resp = b.get("/files", &[]).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["dir"],
        b.work().canonicalize().unwrap().to_str().unwrap()
    );
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<_> = entries
        .iter()
        .map(|e| (e["name"].as_str().unwrap(), e["kind"].as_str().unwrap()))
        .collect();
    assert_eq!(
        names,
        [("a-dir", "dir"), ("b.png", "file"), ("c-link", "symlink")]
    );
    assert_eq!(entries[1]["size"], 3);
    assert!(entries[1]["modified_ms"].as_u64().unwrap() > 0);

    let resp = b.get("/files", &[("dir", "a-dir")]).await;
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["entries"][0]["name"], "inner.txt");

    let home = b.home().to_str().unwrap().to_string();
    let resp = b.get("/files", &[("dir", home.as_str())]).await;
    assert_eq!(resp.status(), 200);

    for dir in ["/etc", "c-link", ".."] {
        let resp = b.get("/files", &[("dir", dir)]).await;
        assert_eq!(resp.status(), 403, "{dir}");
    }
    let resp = b.get("/files", &[("dir", "b.png")]).await;
    assert_eq!(resp.status(), 400);
}
