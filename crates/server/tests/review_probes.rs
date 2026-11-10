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
        let code = state.create_agent_code(name, state::Scope::Admin, 60_000).unwrap();
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
            .json(&json!({ "op": "create", "type": "outbox", "agent": "sender",
                           "after": { "fields": ready_fields() } }))
            .send()
            .unwrap(),
    );
    if s == 200 {
        let id = done["note_id"].as_str().unwrap();
        let (_, note) = body(admin.get(server.url(&format!("/api/notes/{id}"))).send().unwrap());
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
            .json(&json!({ "op": "update", "note_id": id, "base_version": made["version"],
                           "agent": "clerk", "after": after }))
            .send()
            .unwrap(),
    );
    assert!(s >= 400, "agent-cli edited clerk's draft as clerk: {s} {done}");
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
    let (s, made) = body(admin.post(server.url("/api/tokens")).json(&json!({ "name": "ordinary", "scope": "write" })).send().unwrap());
    assert_eq!(s, 201);
    let ops = client(made["secret"].as_str().unwrap());
    let (s, item) = create_ready(&server, &ops);
    assert_eq!(s, 201, "user token agent-ops lost the user's Outbox rights: {item}");
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
    assert!(s >= 400, "agent-cli deleted the user's ready item: {s} {done}");
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
    assert!(s >= 400, "agent-cli restored the user's ready item: {s} {done}");
}
