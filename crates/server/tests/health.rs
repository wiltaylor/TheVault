//! S1: the server starts on a temporary vault and a random port and reports healthy.

use std::thread;

use serde_json::{Value, json};
use testkit::TestServer;

fn health(server: &TestServer) -> Value {
    reqwest::blocking::get(server.url("/api/health"))
        .expect("GET /api/health")
        .error_for_status()
        .expect("/api/health status")
        .json()
        .expect("/api/health body is JSON")
}

#[test]
fn health_reports_ok() {
    let server = TestServer::start();
    assert_eq!(health(&server), json!({ "status": "ok" }));
    assert!(server.vault_path().is_dir());
}

#[test]
fn servers_started_together_get_different_ports() {
    let servers: Vec<TestServer> = (0..4)
        .map(|_| thread::spawn(TestServer::start))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().expect("server start"))
        .collect();

    for server in &servers {
        assert_eq!(health(server), json!({ "status": "ok" }));
    }
    let mut ports: Vec<u16> = servers.iter().map(|s| s.addr().port()).collect();
    ports.sort();
    ports.dedup();
    assert_eq!(ports.len(), servers.len(), "ports clashed: {ports:?}");
}

#[test]
fn dropping_the_handle_stops_the_server() {
    let server = TestServer::start();
    let url = server.url("/api/health");
    let vault = server.vault_path().to_path_buf();
    drop(server);

    assert!(
        reqwest::blocking::get(&url).is_err(),
        "server still answers"
    );
    assert!(!vault.exists(), "temporary vault was not removed");
}

#[test]
fn missing_vault_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(testkit::server_binary())
        .arg("--vault")
        .arg(dir.path().join("absent"))
        .arg("--listen")
        .arg("127.0.0.1:0")
        .output()
        .expect("run vault-server");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
}
