//! Rounds over HTTPS with a token (git-sync D24), against `git http-backend` behind basic auth:
//! the right token works, a wrong one is refused with the token redacted, and no token is ever
//! in the status, the activity log or the server's stderr.

use serde_json::{Value, json};
use testkit::GitHttpServer;

use crate::instance::{Instance, memo};

const TOKEN: &str = "github_pat_rightToken1234567890";
const WRONG: &str = "github_pat_wrongToken0987654321";

fn set_token(i: &Instance, token: &str) {
    i.call(
        "PUT",
        "/api/integrations/gitsync/token",
        Some(json!({ "token": token })),
        204,
    );
}

/// Every place a token must never show: the status, the activity log and the server's stderr.
fn assert_no_token(i: &Instance, token: &str) {
    let status = i.status().to_string();
    assert!(!status.contains(token), "the status shows the token: {status}");
    let activity = i.get("/api/activity").to_string();
    assert!(!activity.contains(token), "the activity log shows the token");
    let notifications = i.get("/api/notifications").to_string();
    assert!(!notifications.contains(token), "a notification shows the token");
    for line in i.server.stderr_lines() {
        assert!(!line.contains(token), "the server's stderr shows the token: {line}");
    }
}

#[test]
fn a_round_with_the_right_token_pushes_and_imports() {
    let remote = GitHttpServer::start(TOKEN);
    let a = Instance::start("A");
    let b = Instance::start("B");
    let key = a.generate_key();
    b.set_key(&key);
    a.configure(&remote.url(), "from memo", "", json!({ "auth": "https" }));
    b.configure(&remote.url(), "", "from memo", json!({ "auth": "https" }));
    set_token(&a, TOKEN);
    set_token(&b, TOKEN);
    assert_eq!(a.status()["token_set"], true);

    let id = a.create(&memo("Over HTTPS", "1", "1"));
    assert_eq!(a.sync()["pushed"], true);
    b.sync();
    assert_eq!(b.field(&id, "title"), "Over HTTPS");
    assert_eq!(remote.branches().len(), 2);
    assert!(remote.served() > 0);
    assert_no_token(&a, TOKEN);
    assert_no_token(&b, TOKEN);
}

#[test]
fn a_wrong_token_is_refused_and_never_shown() {
    let remote = GitHttpServer::start(TOKEN);
    let a = Instance::start("A");
    a.generate_key();
    a.configure(&remote.url(), "from memo", "", json!({ "auth": "https" }));
    set_token(&a, WRONG);
    a.create(&memo("Refused", "1", "1"));

    let status = a.round();
    let error = status["last_error"].as_str().unwrap_or_default().to_owned();
    assert!(!error.is_empty(), "{status}");
    assert!(remote.refused() > 0);
    assert!(remote.branches().is_empty());
    assert_no_token(&a, WRONG);

    // With no token at all the round fails at once rather than waiting on a prompt.
    a.call("DELETE", "/api/integrations/gitsync/token", None, 204);
    let status = a.round();
    assert_ne!(status["last_error"], Value::Null, "{status}");
    assert!(remote.branches().is_empty());
}
