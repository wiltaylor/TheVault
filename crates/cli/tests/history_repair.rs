use std::path::Path;
use std::process::{Command, Stdio};

use rusqlite::{Connection, params};
use serde_json::Value;
use state::{Author, State};
use tempfile::TempDir;
use writer::{Input, Writer};

fn open(root: &Path) -> Writer {
    let vault = store::Vault::init(root).unwrap();
    let state = State::open(&root.join(".vault/state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn fixture() -> (TempDir, String, Vec<String>) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::write(
        dir.path().join("types/task.wcl"),
        "label = \"Task\"\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n",
    )
    .unwrap();
    let w = open(dir.path());
    let by = Author::Token("fixture".into());
    let first = w
        .create("task", &Input::Wcl("title = \"first\"\n".into()), &by)
        .unwrap();
    let mut versions = vec![first.version.clone()];
    for title in ["second", "third"] {
        let saved = w
            .update(
                &first.id,
                &Input::Wcl(format!("title = \"{title}\"\n")),
                versions.last().map(String::as_str),
                &by,
            )
            .unwrap();
        versions.push(saved.version);
    }
    (dir, first.id, versions)
}

fn cli(root: &Path, args: &[&str]) -> (bool, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_vault"))
        .arg("--vault")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let body =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not JSON: {:?}", out));
    (out.status.success(), body)
}

fn payload(conn: &Connection, token: &str) -> Vec<u8> {
    conn.query_row(
        "SELECT CAST(content AS BLOB) FROM versions WHERE token=?1",
        [token],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn check_is_read_only_and_confirmation_is_required() {
    let (dir, id, versions) = fixture();
    let path = dir.path().join(".vault/state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE versions SET content=x'01' WHERE token=?1",
        [&versions[2]],
    )
    .unwrap();
    drop(conn);
    let before = std::fs::read(&path).unwrap();
    let (ok, check) = cli(dir.path(), &["history-check"]);
    assert!(!ok);
    assert_eq!(check["corrupt"][0]["note_id"], id);
    assert_eq!(check["corrupt"][0]["version"], versions[2]);
    assert_eq!(check["corrupt"][0]["head"], true);
    assert_eq!(before, std::fs::read(&path).unwrap());
    for args in [
        vec!["history-repair", id.as_str()],
        vec!["history-repair", "--all"],
        vec!["history-repair", &id, "--yes", "wrong-id"],
    ] {
        let (ok, body) = cli(dir.path(), &args);
        assert!(!ok, "{body}");
        assert_eq!(before, std::fs::read(&path).unwrap());
    }
}

#[test]
fn repair_head_opens_trashes_restores_and_audits() {
    let (dir, id, versions) = fixture();
    let conn = Connection::open(dir.path().join(".vault/state.db")).unwrap();
    // Keep a valid compressed block but break the checksum.
    let mut bytes = payload(&conn, &versions[2]);
    bytes[12] ^= 1;
    conn.execute(
        "UPDATE versions SET content=?1 WHERE token=?2",
        params![bytes, versions[2]],
    )
    .unwrap();
    // Startup invokes recovery only when the projection is behind the file commit.
    let index = Connection::open(dir.path().join(".vault/index.db")).unwrap();
    index
        .execute("DELETE FROM meta WHERE key='generation'", [])
        .unwrap();
    drop(index);
    assert!(
        Writer::open(
            store::Vault::open(dir.path()).unwrap(),
            State::open(&dir.path().join(".vault/state.db"), None).unwrap()
        )
        .is_err()
    );
    let (ok, body) = cli(dir.path(), &["history-repair", &id, "--yes", &id]);
    assert!(ok, "{body}");
    assert_eq!(body["actions"][0]["action"], "replace_corrupt_head");
    assert_ne!(body["actions"][0]["replacement"], versions[2]);
    let state = State::open(&dir.path().join(".vault/state.db"), None).unwrap();
    let head = state
        .history(&id)
        .unwrap()
        .into_iter()
        .find(|v| v.token == body["actions"][0]["replacement"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        head.content.as_bytes(),
        std::fs::read(dir.path().join(format!("notes/task/{id}.wcl"))).unwrap()
    );
    let audits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM activity WHERE kind='history.repair' AND subject=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audits, 1);
    assert!(cli(dir.path(), &["history-check"]).0);
    let w = open(dir.path());
    let by = Author::Token("fixture".into());
    w.delete(&id, body["actions"][0]["replacement"].as_str(), &by)
        .unwrap();
    let restored = w.restore_from_trash(&id, &by).unwrap();
    assert!(restored.wcl.contains("third"));
}

#[test]
fn older_corrupt_row_drops_without_touching_other_versions() {
    let (dir, id, versions) = fixture();
    let conn = Connection::open(dir.path().join(".vault/state.db")).unwrap();
    let first = payload(&conn, &versions[0]);
    let head = payload(&conn, &versions[2]);
    conn.execute(
        "UPDATE versions SET content=x'01' WHERE token=?1",
        [&versions[1]],
    )
    .unwrap();
    let (ok, body) = cli(dir.path(), &["history-check"]);
    assert!(!ok);
    assert_eq!(body["corrupt"][0]["head"], false);
    let (ok, body) = cli(dir.path(), &["history-repair", "--all", "--yes", &id]);
    assert!(ok, "{body}");
    assert_eq!(body["actions"][0]["action"], "drop_corrupt_version");
    assert_eq!(first, payload(&conn, &versions[0]));
    assert_eq!(head, payload(&conn, &versions[2]));
    let state = State::open(&dir.path().join(".vault/state.db"), None).unwrap();
    assert_eq!(state.history(&id).unwrap().len(), 2);
}

#[test]
fn lock_refuses_repair_but_not_check_and_releases_on_close() {
    let (dir, id, versions) = fixture();
    let conn = Connection::open(dir.path().join(".vault/state.db")).unwrap();
    conn.execute(
        "UPDATE versions SET content=x'01' WHERE token=?1",
        [&versions[2]],
    )
    .unwrap();
    let owner = store::lock_owner(dir.path()).unwrap();
    assert!(store::lock_owner(dir.path()).is_err());
    let (ok, body) = cli(dir.path(), &["history-repair", &id, "--yes", &id]);
    assert!(!ok);
    assert!(body.to_string().contains("in use"));
    let (_, check) = cli(dir.path(), &["history-check"]);
    assert_eq!(check["corrupt"][0]["version"], versions[2]);
    drop(owner);
    let (ok, body) = cli(dir.path(), &["history-repair", &id, "--yes", &id]);
    assert!(ok, "{body}");
}

#[test]
fn running_server_holds_ownership() {
    let server = testkit::TestServer::start();
    assert!(store::lock_owner(server.vault_path()).is_err());
    let (ok, body) = cli(server.vault_path(), &["history-repair", "--all"]);
    assert!(!ok);
    assert!(body.to_string().contains("in use"));
    let (ok, body) = cli(server.vault_path(), &["history-check"]);
    assert!(ok, "{body}");
}

#[test]
fn second_note_failure_rolls_back_first_repair_and_audit() {
    let (dir, first, _) = fixture();
    let w = open(dir.path());
    let second = w
        .create(
            "task",
            &Input::Wcl("title = \"other\"\n".into()),
            &Author::Token("fixture".into()),
        )
        .unwrap();
    drop(w);
    let mut ids = [first, second.id];
    ids.sort();
    let path = dir.path().join(".vault/state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE versions SET content=x'01' WHERE token IN (SELECT token FROM heads)",
        [],
    )
    .unwrap();
    // Abort the second action inside the transaction, after the first note and audit.
    conn.execute_batch(&format!("CREATE TRIGGER fail_second_repair BEFORE INSERT ON activity WHEN NEW.kind='history.repair' AND NEW.subject='{}' BEGIN SELECT RAISE(ABORT, 'injected second note failure'); END;", ids[1])).unwrap();
    drop(conn);
    let before = std::fs::read(&path).unwrap();
    let (ok, body) = cli(
        dir.path(),
        &["history-repair", "--all", "--yes", &ids.join(",")],
    );
    assert!(!ok, "{body}");
    assert!(
        body.to_string().contains("injected second note failure"),
        "{body}"
    );
    assert_eq!(before, std::fs::read(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let audits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM activity WHERE kind='history.repair'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audits, 0);
    assert_eq!(state::history_repair::check(&path).unwrap().len(), 2);
}
