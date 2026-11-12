//! The exec service's HTTP contract (browser B2), on a free port over temporary work and home
//! directories.

use std::path::Path;
use std::time::{Duration, Instant};

use browser_agent::{Config, FILE_CAP, MAX_RUNS, OUTPUT_CAP};
use serde_json::{Value, json};
use tempfile::TempDir;

const TOKEN: &str = "test-token-0123456789abcdef";

#[tokio::test(flavor = "multi_thread")]
async fn long_component_deletion_has_bounded_peak_memory() {
    use rustix::fs::{AtFlags, Mode, OFlags, mkdirat, open, openat, unlinkat};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    // Keep the fixture on the checkout's filesystem rather than the /tmp tmpfs.
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&work).unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = reservation.local_addr().unwrap();
    drop(reservation);
    let child = Child(
        std::process::Command::new(env!("CARGO_BIN_EXE_browser-agent"))
            .env("BROWSER_TOKEN", TOKEN)
            .env("BROWSER_LISTEN", addr.to_string())
            .env("BROWSER_HOME_DIR", &home)
            .env("BROWSER_WORK_DIR", &work)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let http = reqwest::Client::new();
    let base = format!("http://{addr}");
    let start = Instant::now();
    loop {
        if http.get(format!("{base}/health")).send().await.is_ok() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        identity(http.post(format!("{base}/run")), "test")
            .json(&json!({"command":"true"}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let profile = home.join("profiles/test");
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut fd = open(profile.join("home"), flags, Mode::empty()).unwrap();
    let mut name = "d".repeat(255);
    let mut depth = 0;
    let mut long_components = 0;
    let mut stop_reason = "reached 4000 levels";
    let mut target_depth = 4000;
    while depth < target_depth {
        match mkdirat(&fd, name.as_str(), Mode::RWXU) {
            Ok(()) => {}
            Err(rustix::io::Errno::NAMETOOLONG) if name.len() == 255 => {
                eprintln!(
                    "ENAMETOOLONG at depth {depth} with 255-byte components; rebuilding with 1-byte components"
                );
                // AppArmor mediates the full path even for fd-relative calls.
                // Remove the probe from the bottom up before restarting at home.
                while depth > 0 {
                    let parent = openat(&fd, "..", flags, Mode::empty()).unwrap();
                    unlinkat(&parent, name.as_str(), AtFlags::REMOVEDIR).unwrap();
                    fd = parent;
                    depth -= 1;
                }
                long_components = 0;
                name.truncate(1);
                let base_bytes = std::fs::canonicalize(profile.join("home"))
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .len();
                target_depth = 4000.min(7600usize.saturating_sub(base_bytes) / 2);
                stop_reason = "reached 1-byte depth budget below 7600 path bytes";
                continue;
            }
            Err(rustix::io::Errno::NAMETOOLONG) => {
                stop_reason = "ENAMETOOLONG with 1-byte components";
                break;
            }
            Err(error) => panic!("mkdirat at depth {depth}: {error}"),
        }
        fd = openat(&fd, name.as_str(), flags, Mode::empty()).unwrap();
        depth += 1;
        if name.len() == 255 {
            long_components += 1;
        }
    }
    eprintln!(
        "deep deletion fixture: depth {depth}, {long_components} x 255-byte and {} x 1-byte components; stopped: {stop_reason}",
        depth - long_components
    );
    assert!(depth >= 1000, "only {depth} levels: {stop_reason}");
    openat(
        &fd,
        "Cookies",
        OFlags::CREATE | OFlags::WRONLY,
        Mode::RUSR | Mode::WUSR,
    )
    .unwrap();
    drop(fd);
    let proc = std::path::PathBuf::from(format!("/proc/{}", child.0.id()));
    let peak = || -> u64 {
        std::fs::read_to_string(proc.join("status"))
            .unwrap()
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmHWM:")
                    .map(|s| s.split_whitespace().next().unwrap().parse().unwrap())
            })
            .unwrap()
    };
    std::fs::write(proc.join("clear_refs"), "5").unwrap();
    let before = peak();
    let response = identity(http.delete(format!("{base}/profile")), "test")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    assert!(!profile.exists());
    let growth = peak().saturating_sub(before);
    assert!(
        growth < 64 * 1024,
        "{depth}-level walk ({long_components} x 255-byte components) grew peak RSS by {growth}KiB"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deletion_repairs_root_home_and_work_permissions() {
    use std::os::unix::fs::PermissionsExt;
    for (part, mode) in [
        ("home", 0),
        ("home", 0o600),
        ("", 0),
        ("", 0o600),
        ("work", 0),
    ] {
        let b = Agent::start().await;
        let root = b.home.path().join("profiles/test");
        std::fs::write(root.join("home/Cookies"), "fixture").unwrap();
        std::fs::set_permissions(root.join(part), std::fs::Permissions::from_mode(mode)).unwrap();
        let response = identity(b.http.delete(b.url("/profile")), "test")
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "{part} {mode:o}: {}",
            response.text().await.unwrap()
        );
        assert!(!root.exists());
        assert_eq!(b.run_status(json!({"command":"true"})).await.0, 410);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn deletion_walk_is_iterative_at_thousands_of_levels() {
    use rustix::fs::{Mode, OFlags, fchmod, mkdirat, open, openat};
    for depth in [1000, 3000] {
        let b = Agent::start().await;
        let root = b.home.path().join("profiles/test");
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let mut fd = open(root.join("home"), flags, Mode::empty()).unwrap();
        for _ in 0..depth {
            mkdirat(&fd, "d", Mode::RWXU).unwrap();
            fd = openat(&fd, "d", flags, Mode::empty()).unwrap();
        }
        openat(
            &fd,
            "Cookies",
            OFlags::CREATE | OFlags::WRONLY,
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        fchmod(&fd, Mode::empty()).unwrap();
        drop(fd);
        let response = identity(b.http.delete(b.url("/profile")), "test")
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "depth{depth}: {}",
            response.text().await.unwrap()
        );
        assert!(!root.exists());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn deletion_repairs_nonsearchable_and_long_directory_trees() {
    use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
    use std::os::unix::fs::PermissionsExt;
    let b = Agent::start().await;
    let root = b.home.path().join("profiles/test/home");
    for (name, mode) in [("no-search", 0o600), ("no-access", 0o000)] {
        let dir = root.join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Cookies"), "fixture").unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut fd = open(&root, flags, Mode::empty()).unwrap();
    let component = "d".repeat(200);
    for _ in 0..30 {
        mkdirat(&fd, component.as_str(), Mode::RWXU).unwrap();
        fd = openat(&fd, component.as_str(), flags, Mode::empty()).unwrap();
    }
    openat(
        &fd,
        "Cookies",
        OFlags::CREATE | OFlags::WRONLY,
        Mode::RUSR | Mode::WUSR,
    )
    .unwrap();
    drop(fd);
    let response = identity(b.http.delete(b.url("/profile")), "test")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    assert!(!root.parent().unwrap().exists());
    let (status, _) = b.run_status(json!({"command":"true"})).await;
    assert_eq!(status, 410);
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_cache_directories_do_not_block_profile_deletion() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let b = Agent::start().await;
    let cache = b.home.path().join("profiles/test/home/.cache/go-mod/x");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("Cookies"), "fixture session").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::set_permissions(outside.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    symlink(outside.path(), cache.join("outside")).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o555)).unwrap();
    let response = identity(b.http.delete(b.url("/profile")), "test")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    assert!(!cache.join("Cookies").exists());
    assert!(!b.home.path().join("profiles/test").exists());
    assert_eq!(
        std::fs::metadata(outside.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
    std::fs::set_permissions(outside.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_profile_stops_detached_children_and_prevents_regrowth() {
    let b = Agent::start().await;
    let result = b.run("setsid sh -c 'while :; do mkdir -p \"$HOME/regrow\"; sleep 0.05; done' </dev/null >/dev/null 2>&1 & printf '%s' $!").await;
    let pid: i32 = result["stdout"].as_str().unwrap().trim().parse().unwrap();
    let response = identity(b.http.delete(b.url("/profile")), "test")
        .send()
        .await
        .unwrap();
    // Always stop our test child even if the assertion fails on an old service.
    let alive = std::fs::read_link(format!("/proc/{pid}/cwd")).is_ok();
    let _ = rustix::process::kill_process(
        rustix::process::Pid::from_raw(pid).unwrap(),
        rustix::process::Signal::KILL,
    );
    assert_eq!(response.status(), 200);
    assert!(!alive, "detached child survived DELETE");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!b.home.path().join("profiles/test").exists());
}

fn identity(r: reqwest::RequestBuilder, id: &str) -> reqwest::RequestBuilder {
    r.bearer_auth(TOKEN).header("x-vault-agent", id).header(
        "x-vault-agent-signature",
        browser_agent::identity_signature(TOKEN, id),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn retired_index_survives_restart_and_prunes_legacy_marker_files() {
    let b = Agent::start().await;
    assert_eq!(
        identity(b.http.delete(b.url("/profile")), "test")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let markers = b.home.path().join(".retired-profiles");
    std::fs::create_dir(&markers).unwrap();
    std::fs::write(markers.join("legacy-retired"), "").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        token: TOKEN.into(),
        work_dir: b._work.path().to_path_buf(),
        home_dir: b.home.path().to_path_buf(),
        display: ":1".into(),
        playwright_cli: None,
    };
    // Construct synchronously to prove migration happened before requests are served.
    let router = browser_agent::router(config).unwrap();
    assert!(!markers.exists());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    for id in ["test", "legacy-retired"] {
        assert_eq!(
            identity(b.http.post(format!("{base}/run")), id)
                .json(&json!({"command":"true"}))
                .send()
                .await
                .unwrap()
                .status(),
            410
        );
    }
    let ids: Vec<String> = serde_json::from_slice(
        &std::fs::read(b.home.path().join(".retired-profile-ids.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(ids, ["legacy-retired", "test"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn resetting_an_over_quota_profile_admits_an_empty_new_identity() {
    let b = Agent::start().await;
    std::fs::File::create(b.work().join("oversized"))
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    let (status, message) = b.run_status(json!({"command":"true"})).await;
    assert_eq!(status, 507);
    assert!(
        message["error"]
            .as_str()
            .unwrap()
            .contains("Reset browser profile")
    );
    assert_eq!(
        identity(b.http.delete(b.url("/profile")), "test")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let r: Value = identity(b.http.post(b.url("/run")), "reset-new-id")
        .json(&json!({"command":"test ! -e oversized && test ! -e storage-state.json"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["exit"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn deletion_waits_for_running_commands_and_rejects_the_retired_identity() {
    let b = Agent::start().await;
    let request = b
        .http
        .post(b.url("/run"))
        .bearer_auth(TOKEN)
        .json(&json!({"command":"touch started; sleep 0.2; printf finished > result"}));
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    let began = Instant::now();
    while !b.work().join("started").exists() {
        assert!(began.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let response = identity(b.http.delete(b.url("/profile")), "test")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(task.await.unwrap().status(), 200);
    assert!(!b.work().exists());
    assert_eq!(
        b.http
            .post(b.url("/run"))
            .bearer_auth(TOKEN)
            .json(&json!({"command":"true"}))
            .send()
            .await
            .unwrap()
            .status(),
        410
    );
    assert_eq!(b.get("/file", &[("path", "result")]).await.status(), 410);
    assert!(b.home.path().join(".retired-profile-ids.json").is_file());
    assert!(!b.home.path().join(".retired-profiles").exists());
    assert_eq!(
        identity(b.http.post(b.url("/run")), "new-incarnation")
            .json(&json!({"command":"true"}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn identities_are_required_and_cannot_be_forged() {
    let b = Agent::start().await;
    for route in ["/run", "/file?path=x", "/files", "/profile"] {
        for forged in [false, true] {
            let client = reqwest::Client::new();
            let r = match route {
                "/run" => client.post(b.url(route)).json(&json!({"command":"true"})),
                "/profile" => client.delete(b.url(route)),
                _ => client.get(b.url(route)),
            }
            .bearer_auth(TOKEN);
            let r = if forged {
                r.header("x-vault-agent", "someone-else")
                    .header("x-vault-agent-signature", "fake")
            } else {
                r
            };
            assert_eq!(r.send().await.unwrap().status(), 401, "{route}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_run_and_capture_paths_are_separate() {
    let b = Agent::start().await;
    let a = identity(b.http.post(b.url("/run")), "parent")
        .json(&json!({"command":"mkdir -p captures; printf parent-cookie > captures/session.json; printf parent-storage > \"$HOME/storage.json\""}))
        .send().await.unwrap();
    assert_eq!(a.status(), 200);
    for id in ["agent-b", "throwaway-fresh-run"] {
        let r: Value = identity(b.http.post(b.url("/run")), id)
            .json(&json!({"command":"test ! -e captures/session.json && test ! -e \"$HOME/storage.json\""}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(r["exit"], 0);
        assert_eq!(
            identity(b.http.get(b.url("/file?path=captures/session.json")), id)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        let sibling = b
            .home
            .path()
            .join("profiles/parent/work/captures/session.json");
        assert_eq!(
            identity(b.http.get(b.url("/file")), id)
                .query(&[("path", sibling)])
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        identity(
            b.http.get(b.url("/file?path=captures/session.json")),
            "parent"
        )
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap(),
        "parent-cookie"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_profile_is_confined_and_does_not_follow_symlinks() {
    let b = Agent::start().await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), "keep").unwrap();
    let managed = b.home.path().join("profiles/test");
    std::os::unix::fs::symlink(outside.path(), managed.join("link")).unwrap();
    let response = identity(b.http.delete(b.url("/profile")), "test")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(!managed.exists());
    assert!(outside.path().join("keep").exists());
    std::os::unix::fs::symlink(outside.path(), &managed).unwrap();
    assert_eq!(
        identity(b.http.delete(b.url("/profile")), "test")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert!(outside.path().join("keep").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_admission_limits_have_clear_errors_and_allow_cleanup() {
    let b = Agent::start().await;
    std::fs::File::create(b.work().join("large"))
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    let r = b
        .http
        .post(b.url("/run"))
        .bearer_auth(TOKEN)
        .json(&json!({"command":"true"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 507);
    assert!(r.text().await.unwrap().contains("512 MiB"));
    assert_eq!(
        identity(b.http.delete(b.url("/profile")), "test")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let root = b.home.path().join("profiles");
    for n in 0..64 {
        std::fs::create_dir(root.join(format!("profile-{n}"))).unwrap();
    }
    let r = identity(b.http.post(b.url("/run")), "new-profile")
        .json(&json!({"command":"true"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 507);
    assert!(r.text().await.unwrap().contains("64"));
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 64);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_b_cannot_read_agent_a_browser_storage() {
    let b = Agent::start().await;
    // A storage-state export contains the same credentials as a live profile.
    // The identity header is supplied by the server, never by the model.
    let a = b.http.post(b.url("/run"))
        .bearer_auth(TOKEN)
        .header("x-vault-agent", "agent-a")
        .header("x-vault-agent-signature", browser_agent::identity_signature(TOKEN, "agent-a"))
        .json(&json!({"command": "printf '%s' '{\"cookies\":[{\"name\":\"session\",\"value\":\"fixture-only\"}]}' > storage-state.json"}))
        .send().await.unwrap();
    assert_eq!(a.status(), 200);
    let a: Value = a.json().await.unwrap();
    assert_eq!(a["exit"], 0);
    let other = b
        .http
        .get(b.url("/file"))
        .bearer_auth(TOKEN)
        .header("x-vault-agent", "agent-b")
        .header(
            "x-vault-agent-signature",
            browser_agent::identity_signature(TOKEN, "agent-b"),
        )
        .query(&[("path", "storage-state.json")])
        .send()
        .await
        .unwrap();
    assert_eq!(
        other.status(),
        404,
        "agent B can read agent A's exported cookies"
    );
}

struct Agent {
    base: String,
    _work: TempDir,
    home: TempDir,
    http: reqwest::Client,
    profile_work: std::path::PathBuf,
    profile_home: std::path::PathBuf,
}

#[tokio::test]
async fn secret_endpoint_fixed_errors_and_profile_authentication() {
    let b = Agent::start().await;
    let value = "fabricated-body-only-password";
    for endpoint in ["/secret/fill", "/secret/origin"] {
        let unauth = reqwest::Client::new().post(b.url(endpoint))
            .json(&json!({"session":"default","target":"#pw","value":value,"origin":"https://example.com"}))
            .send().await.unwrap();
        assert_eq!(unauth.status(), 401);
        assert!(!unauth.text().await.unwrap().contains(value));
        for body in [
            json!({"session":"../../other","target":"#pw","value":value}),
            json!({"unexpected":value}),
        ] {
            let response = b
                .http
                .post(b.url(endpoint))
                .bearer_auth(TOKEN)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 400);
            assert_eq!(
                response.json::<Value>().await.unwrap(),
                json!({"error":"browser_refused"})
            );
        }
    }
}

#[tokio::test]
async fn secret_debug_listener_configuration_requires_server_opt_in() {
    let b = Agent::start().await;
    b.run("true").await;
    let config = b
        .profile_home
        .join(".secret-browser/.playwright/cli.config.json");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "{}");
    let (status, _) = b
        .run_status(json!({"command":"true","secret_browser":true}))
        .await;
    assert_eq!(status, 200);
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("--remote-debugging-address=127.0.0.1")
    );
    b.run("true").await;
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "{}");
}

impl Agent {
    async fn start() -> Agent {
        let work = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let profile_work = home.path().join("profiles/test/work");
        let profile_home = home.path().join("profiles/test/home");
        std::fs::create_dir_all(&profile_work).unwrap();
        std::fs::create_dir_all(&profile_home).unwrap();
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
            _work: work,
            home,
            http: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert("x-vault-agent", "test".parse().unwrap());
                    headers.insert(
                        "x-vault-agent-signature",
                        browser_agent::identity_signature(TOKEN, "test")
                            .parse()
                            .unwrap(),
                    );
                    headers
                })
                .build()
                .unwrap(),
            profile_work,
            profile_home,
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
        &self.profile_work
    }

    fn home(&self) -> &Path {
        &self.profile_home
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
