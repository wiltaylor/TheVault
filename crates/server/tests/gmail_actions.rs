//! S1 with the fake Google server: the shipped `email` type's `google_mail` handlers send the
//! user's mail actions to Gmail, one way (TV-25). Done archives the message, delete trashes it,
//! and nothing else does: no undo, no restore, no write by the sync or another task, no catch-up,
//! nothing without `gmail.modify`. A vault-trashed email is never imported again.

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::fake_google::{DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, FakeGoogle, MailAction, TestAccount, TestServer};

const MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
const READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const DAY: i64 = 86_400_000;

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn mail(id: &str) -> Value {
    json!({
        "id": id,
        "labels": ["INBOX"],
        "at": now_ms() - DAY,
        "from": "Alex Example <alex@example.com>",
        "to": ["me@example.com"],
        "subject": format!("Subject {id}"),
        "text": format!("Body of {id}"),
    })
}

fn modify(id: &str) -> MailAction {
    MailAction {
        action: "modify".to_owned(),
        id: id.to_owned(),
        add: Vec::new(),
        remove: vec!["INBOX".to_owned()],
    }
}

fn trash(id: &str) -> MailAction {
    MailAction {
        action: "trash".to_owned(),
        id: id.to_owned(),
        add: Vec::new(),
        remove: Vec::new(),
    }
}

struct Api {
    server: TestServer,
    client: Client,
    google: FakeGoogle,
}

impl Api {
    /// A server whose shipped `email` handlers and `google_gmail` task reach a fake Gmail holding
    /// `messages`, connected with `scopes`, after one sync run pulled the messages.
    fn start(messages: &[&str], scopes: &[&str]) -> Api {
        let google = FakeGoogle::start();
        for id in messages {
            google.add_message(mail(id));
        }
        let server = TestServer::start();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap();
        let api = Api {
            server,
            client,
            google,
        };
        let settings = json!({
            "gmail_labels": ["INBOX"],
            "scopes": scopes,
            "endpoints": api.google.endpoints(),
        });
        for (name, value) in [
            ("google_client_id", DEFAULT_CLIENT_ID.to_owned()),
            ("google_client_secret", DEFAULT_CLIENT_SECRET.to_owned()),
            ("google_refresh_token", DEFAULT_REFRESH_TOKEN.to_owned()),
            ("google_settings", settings.to_string()),
        ] {
            expect(
                api.req("PUT", &format!("/api/secrets/{name}"))
                    .json(&json!({ "value": value }))
                    .send()
                    .unwrap(),
                200,
            );
        }
        // The shipped type and task, with the fake's host allowed as the Google tests do.
        let allow = format!("allow_domains = [\"{}\", ", api.google.host());
        let email = api.file("types/email.wcl");
        assert_eq!(email.matches("allow_domains = [").count(), 3, "{email}");
        expect(
            api.req("PUT", "/api/types/email")
                .body(email.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        let task = api.file("tasks/google_gmail.wcl");
        expect(
            api.req("PUT", "/api/tasks/google_gmail")
                .body(task.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        api.run_gmail();
        assert_eq!(api.emails().len(), messages.len());
        api
    }

    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(path)).unwrap()
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
    }

    fn run_gmail(&self) -> Value {
        let run = expect(
            self.req("POST", "/api/tasks/google_gmail/run?wait=true")
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run:#}");
        self.settle();
        run
    }

    /// Let every queued handler run finish, and fail on any that failed.
    fn settle(&self) {
        let failed = self.server.settle_handlers();
        assert!(failed.is_empty(), "{failed:#?}");
    }

    fn emails(&self) -> Vec<Value> {
        let out = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": "from email include archived", "page_size": 1000 }))
                .send()
                .unwrap(),
            200,
        );
        out["rows"].as_array().unwrap().clone()
    }

    /// The id of the email note of Gmail message `gmail_ref`.
    fn email(&self, gmail_ref: &str) -> String {
        let rows: Vec<Value> = self
            .emails()
            .into_iter()
            .filter(|r| r["fields"]["gmail_ref"] == gmail_ref)
            .collect();
        assert_eq!(rows.len(), 1, "{rows:#?}");
        rows[0]["id"].as_str().unwrap().to_owned()
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notes/{id}")).send().unwrap(),
            200,
        )
    }

    /// Set fields of note `id` as the user does, and let the handlers run.
    fn edit(&self, id: &str, changes: Value) {
        let mut note = self.note(id);
        for (k, v) in changes.as_object().unwrap() {
            note["note"]["fields"][k] = v.clone();
        }
        expect(
            self.req("PUT", &format!("/api/notes/{id}"))
                .json(&json!({ "note": note["note"], "base_version": note["version"] }))
                .send()
                .unwrap(),
            200,
        );
        self.settle();
    }

    fn delete(&self, id: &str) {
        let note = self.note(id);
        let version = note["version"].as_str().unwrap();
        expect(
            self.req("DELETE", &format!("/api/notes/{id}?base_version={version}"))
                .send()
                .unwrap(),
            200,
        );
        self.settle();
    }

    fn notifications(&self) -> Vec<Value> {
        // These assertions concern Gmail handlers, not the independently shipped classifier's
        // missing-account warning (the fixture intentionally configures Google only).
        expect(self.req("GET", "/api/notifications").send().unwrap(), 200)["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| {
                !n["source"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("classifier_config:"))
            })
            .cloned()
            .collect()
    }

    /// Mark every email processed with the inbox's Mark older processed, and let the handlers run.
    fn mark_older_processed(&self) -> Value {
        let done = expect(
            self.req("POST", "/api/inbox/mark-processed")
                .json(&json!({ "before": "2100-01-01T00:00:00Z" }))
                .send()
                .unwrap(),
            200,
        );
        self.settle();
        done
    }

    fn status(&self) -> Value {
        expect(
            self.req("GET", "/api/integrations/google").send().unwrap(),
            200,
        )
    }
}

#[test]
fn done_archives_the_message_in_gmail_and_nothing_else_sends_again() {
    let api = Api::start(&["one", "two"], &[MODIFY_SCOPE]);
    assert_eq!(api.status()["can_modify"], true);
    let one = api.email("one");

    api.edit(&one, json!({ "processed": true }));
    assert_eq!(api.google.mail_actions(), [modify("one")]);

    // The sync sees the label change and rewrites the note's labels, here to none, so the field
    // goes: a task's write, so the loop guard sends nothing.
    api.run_gmail();
    let labels = api.note(&one)["note"]["fields"]["labels"].clone();
    assert!(labels.is_null(), "{labels}");
    assert_eq!(api.google.mail_actions(), [modify("one")]);

    // Another edit of a processed email, and unticking it, send nothing: there is no undo.
    api.edit(&one, json!({ "title": "Renamed" }));
    api.edit(&one, json!({ "processed": false }));
    assert_eq!(api.google.mail_actions(), [modify("one")]);

    // Ticking it again archives again, which Gmail takes as it is.
    api.edit(&one, json!({ "processed": true }));
    assert_eq!(api.google.mail_actions(), [modify("one"), modify("one")]);

    // The other email was never touched.
    let two = api.email("two");
    assert_ne!(api.note(&two)["note"]["fields"]["processed"], true);
}

#[test]
fn delete_trashes_and_restore_untrashes_the_message_in_gmail() {
    let api = Api::start(&["one", "two"], &[MODIFY_SCOPE]);
    let one = api.email("one");

    api.delete(&one);
    assert_eq!(api.google.mail_actions(), [trash("one")]);

    // Gmail reports the TRASH label in its history, and a full listing runs too: neither brings
    // the email back.
    api.run_gmail();
    api.google.set_history_not_found(true);
    api.run_gmail();
    api.google.set_history_not_found(false);
    let refs: Vec<Value> = api
        .emails()
        .iter()
        .map(|r| r["fields"]["gmail_ref"].clone())
        .collect();
    assert_eq!(refs, [json!("two")]);

    // Restoring it must also undo Google's trash before its 30-day expiry.
    expect(
        api.req("POST", &format!("/api/trash/{one}/restore"))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
    let mut untrash = trash("one");
    untrash.action = "untrash".to_owned();
    assert_eq!(api.google.mail_actions(), [trash("one"), untrash]);
    assert_eq!(api.emails().len(), 2);
}

#[test]
fn writes_by_a_task_never_reach_gmail() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    expect(
        api.req("PUT", "/api/scripts/mark_done")
            .json(&json!({ "source": "use vault\n\nfn run() -> Result[(), string] {\n    for row in vault::query(\"from email\")? {\n        vault::patch(row.get(\"id\").unwrap().as_string().unwrap(), Value::Map(#{ \"processed\": Value::Bool(true) }), \"\")?;\n    }\n    Ok(())\n}\n" }))
            .send()
            .unwrap(),
        200,
    );
    expect(
        api.req("PUT", "/api/tasks/mark_done")
            .body("schedule = \"0 3 * * *\"\nscript = \"mark_done\"\npaused = true\n")
            .send()
            .unwrap(),
        200,
    );
    let run = expect(
        api.req("POST", "/api/tasks/mark_done/run?wait=true")
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(run["outcome"], "succeeded", "{run:#}");
    api.settle();
    assert_eq!(api.note(&one)["note"]["fields"]["processed"], true);
    assert!(api.google.mail_actions().is_empty());
}

#[test]
fn mark_older_processed_sends_nothing_and_a_later_done_still_archives() {
    let api = Api::start(&["one", "two"], &[MODIFY_SCOPE]);
    // Both emails were sent a day ago, so every one is before this date.
    let done = expect(
        api.req("POST", "/api/inbox/mark-processed")
            .json(&json!({ "before": "2100-01-01T00:00:00Z" }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(done["changed"], 2, "{done}");
    api.settle();
    let one = api.email("one");
    let fields = api.note(&one)["note"]["fields"].clone();
    assert_eq!(fields["processed"], true, "{fields}");
    assert!(
        fields["inbox_catch_up"]
            .as_str()
            .is_some_and(|run| !run.is_empty()),
        "{fields}"
    );
    assert!(api.google.mail_actions().is_empty());

    // The user unticks one and marks it done again: that is the user's Done.
    api.edit(&one, json!({ "processed": false }));
    api.edit(&one, json!({ "processed": true }));
    assert_eq!(api.google.mail_actions(), [modify("one")]);
}

/// The catch-up never archives, whatever the note's history (operator decision): an email the
/// user unticked after one run and a later run marks again stays in Gmail's inbox.
#[test]
fn a_second_mark_older_processed_after_an_untick_sends_nothing() {
    let api = Api::start(&["one", "two"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    assert_eq!(api.mark_older_processed()["changed"], 2);
    let first = api.note(&one)["note"]["fields"]["inbox_catch_up"].clone();

    api.edit(&one, json!({ "processed": false }));
    assert_eq!(api.mark_older_processed()["changed"], 1);
    let fields = api.note(&one)["note"]["fields"].clone();
    assert_eq!(fields["processed"], true, "{fields}");
    assert_ne!(
        fields["inbox_catch_up"], first,
        "each run writes its own id"
    );
    assert!(api.google.mail_actions().is_empty());

    // The user's own Done after that still archives.
    api.edit(&one, json!({ "processed": false }));
    api.edit(&one, json!({ "processed": true }));
    assert_eq!(api.google.mail_actions(), [modify("one")]);
}

#[test]
fn a_message_gmail_no_longer_has_counts_as_done() {
    let api = Api::start(&["one", "two"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    let two = api.email("two");
    api.google.delete_message("one");
    api.google.delete_message("two");

    // Both answer 404: the runs succeed (settle fails on a failed run) and nothing is notified.
    api.edit(&one, json!({ "processed": true }));
    api.delete(&two);
    assert_eq!(api.google.mail_actions(), [modify("one"), trash("two")]);
    assert!(api.notifications().is_empty(), "{:#?}", api.notifications());
}

#[test]
fn a_gmail_refusal_fails_the_run_and_notifies_on_the_email() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    api.google
        .set_action_error(Some((403, "Insufficient Permission")));
    let note = api.note(&one);
    let version = note["version"].as_str().unwrap();
    expect(
        api.req(
            "DELETE",
            &format!("/api/notes/{one}?base_version={version}"),
        )
        .send()
        .unwrap(),
        200,
    );
    let failed = api.server.settle_handlers();
    assert_eq!(failed.len(), 1, "{failed:#?}");
    assert!(
        failed[0].contains("Gmail answered 403 to trash of message one: Insufficient Permission"),
        "{failed:#?}"
    );
    // Nothing retries it, so the user is told. The email is in the vault's trash, so the
    // notification names it in its body and is not tied to the note.
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["title"], "Gmail did not trash an email");
    assert!(
        items[0]["body"]
            .as_str()
            .unwrap()
            .ends_with("Nothing retries it: trash the message in Gmail."),
        "{items:#?}"
    );
    assert!(
        items[0]["body"]
            .as_str()
            .unwrap()
            .contains(" Email: Subject one. "),
        "{items:#?}"
    );
    assert_eq!(items[0]["note"], Value::Null, "{items:#?}");
}

/// Gmail can answer with a long body, such as an HTML error page. The notice clips it, so it stays
/// under the notification's 2000 characters and is raised (TV-54).
#[test]
fn a_long_gmail_error_is_clipped_in_the_notice() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    let long = format!("{}end.", "x".repeat(5000));
    api.google.set_action_error(Some((500, &long)));
    let mut note = api.note(&one);
    note["note"]["fields"]["processed"] = json!(true);
    expect(
        api.req("PUT", &format!("/api/notes/{one}"))
            .json(&json!({ "note": note["note"], "base_version": note["version"] }))
            .send()
            .unwrap(),
        200,
    );
    let failed = api.server.settle_handlers();
    assert_eq!(failed.len(), 1, "{failed:#?}");
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{failed:#?}");
    let body = items[0]["body"].as_str().unwrap();
    assert!(
        body.chars().count() <= 2000,
        "{} characters",
        body.chars().count()
    );
    assert!(body.contains('…'), "{body}");
    assert!(!body.contains(".."), "{body}");
    assert!(
        body.ends_with("Nothing retries it: archive the message in Gmail."),
        "{body}"
    );
}

/// An `email` type whose `after_delete` handler lost the `google_gmail_trashed` secret cannot
/// remember the ref. The trash still goes to Gmail, and the user is told the email can come back
/// (TV-54).
#[test]
fn a_ref_the_handler_cannot_remember_still_trashes_and_notifies() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    let email = api.file("types/email.wcl");
    let without = email.replacen(", \"google_gmail_trashed\"]", "]", 1);
    assert_ne!(without, email);
    // The type has notes, so the change is planned, then applied with the plan's token.
    let plan = expect(
        api.req("POST", "/api/types/email/change")
            .json(&json!({ "wcl": without }))
            .send()
            .unwrap(),
        200,
    );
    expect(
        api.req("POST", "/api/types/email/change")
            .json(&json!({ "wcl": without, "plan": plan["plan"] }))
            .send()
            .unwrap(),
        200,
    );
    let note = api.note(&one);
    let version = note["version"].as_str().unwrap();
    expect(
        api.req(
            "DELETE",
            &format!("/api/notes/{one}?base_version={version}"),
        )
        .send()
        .unwrap(),
        200,
    );
    let failed = api.server.settle_handlers();
    assert_eq!(failed.len(), 1, "{failed:#?}");
    assert!(
        failed[0].contains("the vault could not remember it as trashed"),
        "{failed:#?}"
    );
    assert_eq!(api.google.mail_actions(), [trash("one")]);
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["title"], "A trashed email can come back");
    assert_eq!(items[0]["note"], Value::Null, "{items:#?}");
}

/// A `tasks/google_gmail.wcl` edited before mail actions does not grant `google_gmail_trashed`:
/// the sync says so in its log and imports as before (TV-25, TV-54).
#[test]
fn a_sync_task_without_the_trashed_secret_says_so() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let task = api.file("tasks/google_gmail.wcl");
    let without = task.replacen(", \"google_gmail_trashed\"]", "]", 1);
    assert_ne!(without, task);
    expect(
        api.req("PUT", "/api/tasks/google_gmail")
            .body(without)
            .send()
            .unwrap(),
        200,
    );
    let run = api.run_gmail();
    assert!(
        run["log"]
            .as_str()
            .unwrap()
            .contains("This task cannot read the google_gmail_trashed secret"),
        "{run:#}"
    );
}

#[test]
fn without_gmail_modify_nothing_is_sent_and_a_deleted_email_still_stays_out() {
    let api = Api::start(&["one", "two"], &[READONLY_SCOPE]);
    assert_eq!(api.status()["can_modify"], false);
    let one = api.email("one");
    let two = api.email("two");

    api.edit(&one, json!({ "processed": true }));
    api.delete(&two);
    assert!(api.google.mail_actions().is_empty());

    // Gmail still has the message in the inbox, and a full listing finds it: the sync skips it
    // because the user deleted it in the vault.
    api.google.set_history_not_found(true);
    api.run_gmail();
    let refs: Vec<Value> = api
        .emails()
        .iter()
        .map(|r| r["fields"]["gmail_ref"].clone())
        .collect();
    assert_eq!(refs, [json!("one")]);
}

#[test]
fn a_gmail_error_fails_the_handler_run_with_its_message() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    api.google.set_action_error(Some((500, "Backend Error")));
    let mut note = api.note(&one);
    note["note"]["fields"]["processed"] = json!(true);
    expect(
        api.req("PUT", &format!("/api/notes/{one}"))
            .json(&json!({ "note": note["note"], "base_version": note["version"] }))
            .send()
            .unwrap(),
        200,
    );
    let failed = api.server.settle_handlers();
    assert_eq!(failed.len(), 1, "{failed:#?}");
    assert!(
        failed[0].contains("Gmail answered 500 to modify of message one: Backend Error"),
        "{failed:#?}"
    );
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["title"], "Gmail did not archive an email");
    assert_eq!(items[0]["note"], one.as_str());
}

#[test]
fn interactive_agents_need_the_gmail_switch_for_archive_and_trash() {
    for allowed in [false, true] {
        let api = Api::start(&["one", "two"], &[MODIFY_SCOPE]);
        let fake = FakeAnthropic::start();
        for (name, value) in AiSeed::new().account(TestAccount::anthropic(&fake).model("claude-opus-5")).secrets() {
            expect(api.req("PUT", &format!("/api/secrets/{name}")).json(&json!({"value": value})).send().unwrap(), 200);
        }
        expect(api.req("PUT", "/api/agents/clerk").body(format!("label = \"Clerk\"\npermissions {{\n gmail_actions = {allowed}\n confirm {{ notes = false\n delete = false\n }}\n}}\n")).send().unwrap(), 200);
        let one = api.email("one");
        let two = api.email("two");
        let mut note = api.note(&one);
        note["note"]["fields"]["processed"] = json!(true);
        fake.push(AnthropicMessage::new().tool_use("edit", "propose_edit", json!({"id": one, "base_version": note["version"], "note": note["note"].to_string()})));
        fake.push(AnthropicMessage::new().tool_use("delete", "propose_delete", json!({"id": two})));
        fake.push(AnthropicMessage::text("Done."));
        let response = api.req("POST", "/api/ai/chat").json(&json!({"agent": "clerk", "messages": [{"role": "user", "content": "Process one and delete two."}]})).send().unwrap();
        assert_eq!(response.status(), 200);
        let stream = response.text().unwrap();
        assert!(stream.contains("event: applied"), "{stream}");
        api.settle();
        assert_eq!(api.note(&one)["note"]["fields"]["processed"], true);
        assert_eq!(api.emails().len(), 1);
        assert_eq!(api.google.mail_actions(), if allowed { vec![modify("one"), trash("two")] } else { vec![] });
    }
}

#[test]
fn failed_restore_keeps_the_vault_copy_and_reports_google_failure() {
    let api = Api::start(&["one"], &[MODIFY_SCOPE]);
    let one = api.email("one");
    api.delete(&one);
    api.google.set_action_error(Some((403, "Restore refused")));
    expect(api.req("POST", &format!("/api/trash/{one}/restore")).send().unwrap(), 200);
    let failed = api.server.settle_handlers();
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].contains("untrash"), "{failed:?}");
    assert_eq!(api.note(&one)["note"]["fields"]["gmail_ref"], "one");
    assert!(api.notifications().iter().any(|n| n["title"] == "Gmail did not untrash an email"));
}
