//! Second security review probes for TV-142 (tv-142-review). Each test asserts the secure
//! behaviour; a failing test is a finding.

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const SENDER: &str = r#"label = "Sender"

permissions {
  tools = ["draft_email"]

  outbox {
    send_without_review = true
  }
}
"#;

const CLERK: &str = r#"label = "Clerk"

permissions {
  tools = ["draft_email"]
}
"#;

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

fn body(res: Response) -> (u16, Value) {
    let status = res.status().as_u16();
    let text = res.text().unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn install(server: &TestServer, admin: &Client, name: &str, scope: &str) -> Client {
    if scope == "admin" {
        // Legacy codes could grant admin. The new API refuses them.
        let state = state::State::open(&server.vault_path().join(".vault/state.db"), None).unwrap();
        let code = state
            .create_agent_code(name, state::Scope::Admin, 60_000)
            .unwrap();
        let (_, secret) = state.redeem_agent_code(&code.code).unwrap().unwrap();
        return client(&secret);
    }
    let (s, issued) = body(
        admin
            .post(server.url("/api/agent-codes"))
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    let (s, token) = body(
        Client::new()
            .post(server.url("/api/agent-codes/redeem"))
            .json(&json!({ "code": issued["code"] }))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    client(token["token"].as_str().unwrap())
}

fn put_agent(server: &TestServer, admin: &Client, name: &str, wcl: &str) {
    let res = admin
        .put(server.url(&format!("/api/agents/{name}")))
        .header("content-type", "text/plain")
        .body(wcl.to_owned())
        .send()
        .unwrap();
    assert!(res.status().is_success(), "{}", res.text().unwrap());
}

fn ready_fields() -> Value {
    json!({ "title": "Unreviewed", "kind": "email", "status": "ready",
            "to": ["outside@example.com"], "subject": "no review" })
}

fn create_ready(server: &TestServer, c: &Client) -> (u16, Value) {
    body(
        c.post(server.url("/api/notes?type=outbox"))
            .json(&json!({ "note": { "fields": ready_fields() } }))
            .send()
            .unwrap(),
    )
}

/// P1: an admin-scope install-code token mints an ordinary token and sends with it.
#[test]
fn p1_admin_agent_token_cannot_mint_plain_token_that_sends() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let agent = install(&server, &admin, "cli", "admin");
    let (s, made) = body(
        agent
            .post(server.url("/api/tokens"))
            .json(&json!({ "name": "plain", "scope": "write" }))
            .send()
            .unwrap(),
    );
    if s != 201 {
        return;
    }
    let plain = client(made["secret"].as_str().unwrap());
    let (s, item) = create_ready(&server, &plain);
    assert!(
        s >= 400,
        "agent-cli minted token plain, which created a ready item: {s} {item}"
    );
}

/// P2: an admin-scope install-code token pairs a device and sends with the device token.
#[test]
fn p2_admin_agent_token_cannot_pair_device_that_sends() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let agent = install(&server, &admin, "cli", "admin");
    let (s, code) = body(
        agent
            .post(server.url("/api/devices/pair-codes"))
            .send()
            .unwrap(),
    );
    if s != 201 && s != 200 {
        return;
    }
    let (s, paired) = body(
        Client::new()
            .post(server.url("/api/devices/pair"))
            .json(&json!({ "code": code["code"], "name": "Phone", "platform": "android" }))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    let device = client(paired["token"].as_str().unwrap());
    let (s, item) = create_ready(&server, &device);
    assert!(
        s >= 400,
        "agent-cli paired a device whose token created a ready item: {s} {item}"
    );
}

/// P3: a write-scope install-code token applies a forged proposal as an in-app agent that has
/// send_without_review.
#[test]
fn p3_agent_token_cannot_apply_as_sending_agent() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    put_agent(&server, &admin, "sender", SENDER);
    let agent = install(&server, &admin, "cli", "write");
    let (s, done) = body(
        agent
            .post(server.url("/api/ai/apply"))
            .json(
                &json!({ "op": "create", "type": "outbox", "agent": "sender",
                           "after": { "fields": ready_fields() } }),
            )
            .send()
            .unwrap(),
    );
    if s == 200 {
        let id = done["note_id"].as_str().unwrap();
        let (_, note) = body(
            admin
                .get(server.url(&format!("/api/notes/{id}")))
                .send()
                .unwrap(),
        );
        assert_ne!(
            note["note"]["fields"]["status"], "ready",
            "agent-cli created a ready item as agent sender: {note}"
        );
    }
}

/// P4: a write-scope install-code token edits another agent's draft by naming it in a proposal.
#[test]
fn p4_agent_token_cannot_edit_other_agents_draft_via_apply() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    put_agent(&server, &admin, "clerk", CLERK);
    let agent = install(&server, &admin, "cli", "write");
    let (s, made) = body(
        admin
            .post(server.url("/api/notes?type=outbox"))
            .json(&json!({ "note": { "fields": {
                "title": "Clerk draft", "kind": "email", "status": "draft", "drafted_by": "clerk",
                "to": ["ann@example.com"], "subject": "orig" }}}))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    let id = made["id"].as_str().unwrap();
    // Direct edit is refused (control).
    let mut note = made["note"].clone();
    note["fields"]["subject"] = json!("direct");
    let (s, _) = body(
        agent
            .put(server.url(&format!("/api/notes/{id}")))
            .json(&json!({ "note": note, "base_version": made["version"] }))
            .send()
            .unwrap(),
    );
    assert!(s >= 400, "control: direct edit of clerk draft allowed");
    let mut after = made["note"].clone();
    after["fields"]["subject"] = json!("changed by cli");
    let (s, done) = body(
        agent
            .post(server.url("/api/ai/apply"))
            .json(
                &json!({ "op": "update", "note_id": id, "base_version": made["version"],
                           "agent": "clerk", "after": after }),
            )
            .send()
            .unwrap(),
    );
    assert!(
        s >= 400,
        "agent-cli edited clerk's draft as clerk: {s} {done}"
    );
}

/// P5: an install code named `user` makes `agent-user`, whose drafts are `drafted_by = user`,
/// the marker of the user's own items.
#[test]
fn p5_agent_named_user_cannot_edit_users_draft() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let (s, made) = body(
        admin
            .post(server.url("/api/notes?type=outbox"))
            .json(&json!({ "note": { "fields": {
                "title": "User draft", "kind": "email", "status": "draft",
                "to": ["ann@example.com"], "subject": "mine" }}}))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    assert_eq!(made["note"]["fields"]["drafted_by"], "user");
    let agent = install(&server, &admin, "user", "write");
    let mut note = made["note"].clone();
    note["fields"]["to"] = json!(["attacker@example.com"]);
    let (s, done) = body(
        agent
            .put(server.url(&format!("/api/notes/{}", made["id"].as_str().unwrap())))
            .json(&json!({ "note": note, "base_version": made["version"] }))
            .send()
            .unwrap(),
    );
    assert!(s >= 400, "agent-user edited the user's draft: {s} {done}");
}

/// P6: an ordinary token the user names `agent-ops` keeps the user's rights.
#[test]
fn p6_user_token_reserved_prefix_is_refused() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let (s, made) = body(
        admin
            .post(server.url("/api/tokens"))
            .json(&json!({ "name": "agent-ops", "scope": "write" }))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 400);
    let _ = made;
    let (s, made) = body(
        admin
            .post(server.url("/api/tokens"))
            .json(&json!({ "name": "ordinary", "scope": "write" }))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    let ops = client(made["secret"].as_str().unwrap());
    let (s, item) = create_ready(&server, &ops);
    assert_eq!(
        s, 201,
        "user token agent-ops lost the user's Outbox rights: {item}"
    );
}

/// P7: an install-code token deletes (cancels) the user's ready item.
#[test]
fn p7_agent_token_cannot_delete_users_ready_item() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let (s, made) = create_ready(&server, &admin);
    assert_eq!(s, 201);
    let agent = install(&server, &admin, "cli", "write");
    let (s, done) = body(
        agent
            .delete(server.url(&format!("/api/notes/{}", made["id"].as_str().unwrap())))
            .query(&[("base_version", made["version"].as_str().unwrap())])
            .send()
            .unwrap(),
    );
    assert!(
        s >= 400,
        "agent-cli deleted the user's ready item: {s} {done}"
    );
}

/// P8: batch create of a ready item and batch delete of a Google event by an install-code token.
#[test]
fn p8_agent_token_batch_is_guarded() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let agent = install(&server, &admin, "cli", "write");
    let (s, done) = body(
        agent
            .post(server.url("/api/batch"))
            .json(&json!({ "ops": [ { "op": "create", "type": "outbox",
                                      "note": { "fields": ready_fields() } } ] }))
            .send()
            .unwrap(),
    );
    assert!(s >= 400, "batch ready create: {s} {done}");
    let (s, ev) = body(
        admin
            .post(server.url("/api/notes?type=event"))
            .json(&json!({ "note": { "fields": {
                "title": "Meeting", "google_ref": "m1", "calendar": "primary",
                "when": { "start": "2026-04-03", "end": "2026-04-03" } }}}))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    let (s, done) = body(
        agent
            .post(server.url("/api/batch"))
            .json(&json!({ "ops": [ { "op": "delete", "id": ev["id"],
                                      "base_version": ev["version"] } ] }))
            .send()
            .unwrap(),
    );
    let (gs, _) = body(
        admin
            .get(server.url(&format!("/api/notes/{}", ev["id"].as_str().unwrap())))
            .send()
            .unwrap(),
    );
    assert_eq!(gs, 200, "batch delete removed the Google event: {s} {done}");
}

/// P9: an install-code token restores a version of the user's item and restores it from trash.
#[test]
fn p9_agent_token_cannot_restore_users_ready_item() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let (s, made) = create_ready(&server, &admin);
    assert_eq!(s, 201);
    let id = made["id"].as_str().unwrap().to_owned();
    let (s, _) = body(
        admin
            .delete(server.url(&format!("/api/notes/{id}")))
            .query(&[("base_version", made["version"].as_str().unwrap())])
            .send()
            .unwrap(),
    );
    assert!(s < 300);
    let agent = install(&server, &admin, "cli", "write");
    let (s, done) = body(
        agent
            .post(server.url(&format!("/api/trash/{id}/restore")))
            .send()
            .unwrap(),
    );
    assert!(
        s >= 400,
        "agent-cli restored the user's ready item: {s} {done}"
    );
}

#[test]
fn agent_token_owns_only_its_drafts_and_legacy_admin_cannot_grant() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let agent = install(&server, &admin, "cli", "admin");
    for path in ["/api/tokens", "/api/agent-codes", "/api/devices/pair-codes"] {
        assert_eq!(
            agent
                .post(server.url(path))
                .json(&json!({"name":"escape", "scope":"write"}))
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        agent
            .put(server.url("/api/agents/sender"))
            .body(SENDER)
            .send()
            .unwrap()
            .status(),
        403
    );
    for name in ["user", "agent-clerk", "device-clerk"] {
        assert!(
            admin
                .put(server.url(&format!("/api/agents/{name}")))
                .body(CLERK)
                .send()
                .unwrap()
                .status()
                .is_client_error()
        );
    }
    let (s, made) = body(
        agent
            .post(server.url("/api/notes?type=outbox"))
            .json(&json!({"note":{"fields":{"title":"Draft", "kind":"email", "subject":"Review"}}}))
            .send()
            .unwrap(),
    );
    assert_eq!(s, 201);
    assert_eq!(made["note"]["fields"]["drafted_by"], "agent-cli");
    let id = made["id"].as_str().unwrap();
    let url = server.url(&format!("/api/notes/{id}"));
    let other = install(&server, &admin, "other", "write");
    assert!(
        other
            .delete(&url)
            .query(&[("base_version", made["version"].as_str().unwrap())])
            .send()
            .unwrap()
            .status()
            .is_client_error()
    );
    assert!(
        agent
            .delete(&url)
            .query(&[("base_version", made["version"].as_str().unwrap())])
            .send()
            .unwrap()
            .status()
            .is_success()
    );
    assert!(
        other
            .post(server.url(&format!("/api/trash/{id}/restore")))
            .send()
            .unwrap()
            .status()
            .is_client_error()
    );
    // Safety policy refuses all agent trash restores: no live note has clearance.
    assert_eq!(
        agent
            .post(server.url(&format!("/api/trash/{id}/restore")))
            .send()
            .unwrap()
            .status(),
        403
    );
}

#[test]
fn p10_legacy_admin_agent_token_cannot_change_ai_accounts() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let agent = install(&server, &admin, "cli", "admin");
    let account = json!({ "name": "test", "kind": "openai_compatible", "base_url": "http://127.0.0.1:9/v1", "model": "m" });
    let (_, made) = body(
        admin
            .post(server.url("/api/integrations/ai/accounts"))
            .json(&account)
            .send()
            .unwrap(),
    );
    let id = made["account"]["id"]
        .as_str()
        .or(made["id"].as_str())
        .unwrap();
    let statuses = [
        agent
            .post(server.url("/api/integrations/ai/accounts"))
            .json(&account)
            .send()
            .unwrap()
            .status(),
        agent
            .put(server.url("/api/integrations/ai/default"))
            .json(&json!({ "id": id }))
            .send()
            .unwrap()
            .status(),
        agent
            .put(server.url(&format!("/api/integrations/ai/accounts/{id}")))
            .json(&account)
            .send()
            .unwrap()
            .status(),
        agent
            .put(server.url(&format!("/api/integrations/ai/accounts/{id}/key")))
            .json(&json!({ "key": "fixture" }))
            .send()
            .unwrap()
            .status(),
    ];
    assert!(
        statuses.iter().all(|status| status.as_u16() == 403),
        "{statuses:?}"
    );
}

#[test]
fn legacy_reserved_agent_files_warn_at_startup_and_in_settings_data() {
    let server = TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("agents")).unwrap();
        for name in ["user", "agent-old", "device-old", "ordinary"] {
            std::fs::write(
                vault.join(format!("agents/{name}.wcl")),
                "label = \"Legacy\"\n",
            )
            .unwrap();
        }
    });
    let admin = client(server.admin_token());
    let (s, list) = body(admin.get(server.url("/api/agents")).send().unwrap());
    assert_eq!(s, 200);
    let warnings = list["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 3);
    let lines = server.stderr_lines_when(|lines| {
        ["user", "agent-old", "device-old"].iter().all(|name| {
            lines
                .iter()
                .any(|line| line.contains(&format!("warning: agent {name:?} ")))
        })
    });
    for name in ["user", "agent-old", "device-old"] {
        assert_eq!(
            warnings
                .iter()
                .filter(|w| w.as_str().unwrap().contains(&format!("Agent {name:?} ")))
                .count(),
            1
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains(&format!("warning: agent {name:?} ")))
                .count(),
            1
        );
    }
    for name in ["Agent-ops", "DEVICE-x"] {
        assert_eq!(
            admin
                .post(server.url("/api/tokens"))
                .json(&json!({ "name": name, "scope": "write" }))
                .send()
                .unwrap()
                .status(),
            400
        );
    }
}

#[test]
fn agent_integration_guard_covers_methods_paths_and_future_routes() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let agent = install(&server, &admin, "cli", "admin");
    for path in [
        "/api/integrations",
        "/api/integrations/future",
        "/api/integrations/ai/accounts?x=1",
        "/api/integrations/ai/accounts/",
        "/api/integrations/%61i/accounts",
    ] {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let res = agent
                .request(method.parse().unwrap(), server.url(path))
                .json(&json!({}))
                .send()
                .unwrap();
            assert_eq!(res.status(), 403, "{method} {path}");
        }
    }
}

#[test]
fn p18_legacy_admin_agent_cannot_rewrite_integration_secrets() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    let legacy = install(&server, &admin, "legacy", "admin");
    let mut statuses = Vec::new();
    for name in [
        "ai_accounts",
        "ai_settings",
        "ai_account_key_probe",
        "brave_settings",
        "gitsync_settings",
        "google_client_id",
    ] {
        let url = server.url(&format!("/api/secrets/{name}"));
        server.seed_secret(name, "fixture");
        statuses.push(
            legacy
                .put(&url)
                .json(&json!({ "value": "replacement" }))
                .send()
                .unwrap()
                .status(),
        );
        statuses.push(legacy.delete(&url).send().unwrap().status());
    }
    assert!(statuses.iter().all(|s| s.as_u16() == 403), "{statuses:?}");
}

#[test]
fn p19_legacy_admin_agent_cannot_read_google_token_through_a_task() {
    let server = TestServer::start();
    let admin = client(server.admin_token());
    server.seed_secret("google_refresh_token", "synthetic-test-marker");
    let legacy = install(&server, &admin, "legacy", "admin");
    let script = "use vault\nuse secrets\nfn run() -> Result[string, string] {\n    let t = secrets::get(\"google_refresh_token\")?\n    vault::log(t)\n    Ok(\"done\")\n}\n";
    let script_status = legacy
        .put(server.url("/api/scripts/leak"))
        .json(&json!({ "source": script }))
        .send()
        .unwrap()
        .status();
    let task_status = legacy
        .put(server.url("/api/tasks/leak"))
        .body("schedule = \"0 0 1 1 *\"\nscript = \"leak\"\nsecrets = [\"google_refresh_token\"]\n")
        .send()
        .unwrap()
        .status();
    let run = legacy
        .post(server.url("/api/tasks/leak/run?wait=true"))
        .send()
        .unwrap();
    let run_status = run.status();
    let leaked = run.text().unwrap().contains("synthetic-test-marker");
    assert!(
        !leaked
            && [script_status, task_status, run_status]
                .iter()
                .all(|s| s.as_u16() == 403),
        "script={script_status} task={task_status} run={run_status} leaked={leaked}"
    );
}
