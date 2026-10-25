//! S1 with the fake Anthropic and the fake Google: the bell notifications of the Outbox (outbox
//! X10). An agent's run that drafts items into Review leaves one notification for the run, its
//! count raised in place, linking to `/outbox`; the next run starts another. A send that fails
//! leaves one "Could not send <title>" linking to the item; the user's own edits notify nothing.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::fake_google::{
    DEFAULT_ACCOUNT, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN,
};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, FakeGoogle, TestAccount, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MEMO: &str = r#"label = "Memo"

field status {
  kind = "string"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
  }
}
"#;

const CLERK: &str = r#"label = "Clerk"

permissions {
  confirm {
    notes = false
  }
}

trigger opened {
  on     = ["create"]
  type   = "memo"
  prompt = "Draft the replies."
}
"#;

const RECONNECT: &str = "Reconnect Google in Settings > Integrations to send email.";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(vault: impl FnOnce(&std::path::Path)) -> Api {
        let server =
            TestServer::start_with_args(vault, &[OsStr::new("--test-clock"), OsStr::new(START)]);
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap();
        Api { server, client }
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Response {
        let req = self
            .client
            .request(method.parse().unwrap(), self.server.url(path))
            .bearer_auth(self.server.admin_token());
        match body {
            Some(b) => req.json(&b),
            None => req,
        }
        .send()
        .unwrap()
    }

    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let res = self.request(method, path, Some(body));
        let status = res.status();
        let text = res.text().unwrap();
        assert!(status.is_success(), "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        let res = self.request("GET", path, None);
        let status = res.status();
        let text = res.text().unwrap();
        assert!(status.is_success(), "GET {path}: {text}");
        serde_json::from_str(&text).unwrap()
    }

    fn put_text(&self, path: &str, body: String) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.server.admin_token())
            .header("content-type", "text/plain")
            .body(body)
            .send()
            .unwrap();
        let status = res.status();
        assert!(status.is_success(), "PUT {path}: {}", res.text().unwrap());
    }

    fn put_secret(&self, name: &str, value: &str) {
        self.send(
            "PUT",
            &format!("/api/secrets/{name}"),
            json!({ "value": value }),
        );
    }

    fn create(&self, type_name: &str, note: Value) -> String {
        let v = self.send(
            "POST",
            &format!("/api/notes?type={type_name}"),
            json!({ "note": note }),
        );
        v["id"].as_str().unwrap().to_owned()
    }

    /// Change fields of note `id` as the user would.
    fn edit(&self, id: &str, changes: Value) {
        let mut note = self.get(&format!("/api/notes/{id}"));
        for (k, v) in changes.as_object().unwrap() {
            note["note"]["fields"][k] = v.clone();
        }
        self.send(
            "PUT",
            &format!("/api/notes/{id}"),
            json!({ "note": note["note"], "base_version": note["version"] }),
        );
    }

    fn advance(&self, by: &str) {
        self.send("POST", "/api/test/clock", json!({ "advance": by }));
    }

    fn notifications(&self) -> Vec<Value> {
        self.get("/api/notifications")["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn settle(&self) {
        let failed = self.server.settle_handlers();
        assert!(failed.is_empty(), "{failed:#?}");
    }
}

fn email_draft(subject: &str) -> String {
    json!({
        "fields": {
            "title": subject,
            "kind": "email",
            "to": ["ada@example.com"],
            "subject": subject,
        }
    })
    .to_string()
}

/// One agent message drafting an email item for each of `subjects`.
fn drafting(id: &str, subjects: &[&str]) -> AnthropicMessage {
    subjects
        .iter()
        .enumerate()
        .fold(AnthropicMessage::new(), |m, (i, s)| {
            m.tool_use(
                &format!("toolu_{id}{i}"),
                "propose_create",
                json!({ "type": "outbox", "note": email_draft(s) }),
            )
        })
}

#[test]
fn an_agent_run_that_drafts_items_leaves_one_notification_counting_them() {
    let fake = FakeAnthropic::start();
    let api = Api::start(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
    });
    for (name, value) in AiSeed::new()
        .account(TestAccount::anthropic(&fake).model("claude-opus-5"))
        .secrets()
    {
        api.put_secret(&name, &value);
    }
    api.put_text("/api/agents/clerk", CLERK.to_owned());

    // One run drafting two items in one message and a third in the next.
    fake.push(drafting("a", &["Re: Roof", "Re: Gutters"]));
    fake.push(drafting("b", &["Re: Fence"]));
    fake.push(AnthropicMessage::text("Drafted three replies."));
    api.create(
        "memo",
        json!({ "fields": { "title": "Roof", "status": "open" } }),
    );
    api.advance("PT31S");
    assert_eq!(fake.message_requests().len(), 3);

    let drafts = api.send(
        "POST",
        "/api/query",
        json!({ "query": "from outbox where status == \"draft\"", "page_size": 10 }),
    );
    assert_eq!(drafts["total"], 3, "{drafts:#}");
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["title"], "Clerk drafted 3 items");
    assert_eq!(items[0]["url"], "/outbox?tab=draft");
    assert_eq!(items[0]["note"], Value::Null);
    assert_eq!(items[0]["source"], "agent:clerk");

    // The next run's draft is a notification of its own.
    fake.push(drafting("c", &["Re: Shed"]));
    fake.push(AnthropicMessage::text("Drafted one."));
    api.create(
        "memo",
        json!({ "fields": { "title": "Shed", "status": "open" } }),
    );
    api.advance("PT31S");
    let items = api.notifications();
    assert_eq!(items.len(), 2, "{items:#?}");
    assert_eq!(items[0]["title"], "Clerk drafted 1 item");
    assert_eq!(items[1]["title"], "Clerk drafted 3 items");

    // What the user drafts and edits notifies nothing.
    let mine = api.create(
        "outbox",
        json!({ "fields": { "title": "Mine", "kind": "email", "to": ["bo@example.com"] } }),
    );
    api.edit(&mine, json!({ "subject": "Mine, edited" }));
    assert_eq!(api.notifications().len(), 2);
}

fn email_input(subject: &str) -> Value {
    json!({
        "to": ["ann@example.com"],
        "cc": [],
        "bcc": [],
        "subject": subject,
        "body": "Are you free on Friday?",
        "attachments": [],
        "in_reply_to": "",
    })
}

#[test]
fn a_chat_turn_that_drafts_leaves_one_notification_for_the_turn() {
    let fake = FakeAnthropic::start();
    let api = Api::start(|_| {});
    for (name, value) in AiSeed::new()
        .account(TestAccount::anthropic(&fake).model("claude-opus-5"))
        .secrets()
    {
        api.put_secret(&name, &value);
    }
    let chat = || {
        let res = api
            .client
            .post(api.server.url("/api/ai/chat"))
            .bearer_auth(api.server.admin_token())
            .json(&json!({ "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Draft them." }] }] }))
            .send()
            .unwrap();
        assert!(res.status().is_success());
        // The turn's events, read to the end so the turn is over.
        res.text().unwrap()
    };

    // The Assistant drafts two emails and an SMS in one turn.
    fake.push(
        AnthropicMessage::new()
            .tool_use("toolu_1", "draft_email", email_input("Lunch"))
            .tool_use("toolu_2", "draft_email", email_input("Dinner")),
    );
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_3",
        "draft_sms",
        json!({ "to": "+61 400 000 001", "body": "Running late", "device": "" }),
    ));
    fake.push(AnthropicMessage::text("Drafted."));
    chat();
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["title"], "Assistant drafted 3 items");
    assert_eq!(items[0]["url"], "/outbox?tab=draft");

    // Read, it stays as it is; the next turn's draft is a notification of its own.
    api.send(
        "POST",
        &format!(
            "/api/notifications/{}/read",
            items[0]["id"].as_str().unwrap()
        ),
        json!({}),
    );
    fake.push(AnthropicMessage::new().tool_use("toolu_4", "draft_email", email_input("Tea")));
    fake.push(AnthropicMessage::text("Drafted."));
    chat();
    let items = api.notifications();
    assert_eq!(items.len(), 2, "{items:#?}");
    assert_eq!(items[0]["title"], "Assistant drafted 1 item");
    assert_eq!(items[1]["title"], "Assistant drafted 3 items");
}

#[test]
fn a_failed_send_leaves_one_notification_linking_to_the_item() {
    let google = FakeGoogle::start();
    let api = Api::start(|_| {});
    // Connected without the scope to send, so the send fails with the reconnect message.
    for (name, value) in [
        ("google_client_id", DEFAULT_CLIENT_ID.to_owned()),
        ("google_client_secret", DEFAULT_CLIENT_SECRET.to_owned()),
        ("google_refresh_token", DEFAULT_REFRESH_TOKEN.to_owned()),
        ("google_account", DEFAULT_ACCOUNT.to_owned()),
        (
            "google_settings",
            json!({ "endpoints": google.endpoints(), "scopes": [] }).to_string(),
        ),
    ] {
        api.put_secret(name, &value);
    }
    let allow = format!("allow_domains = [\"{}\", ", google.host());
    let outbox = std::fs::read_to_string(api.server.vault_path().join("types/outbox.wcl")).unwrap();
    api.put_text(
        "/api/types/outbox",
        outbox.replace("allow_domains = [", &allow),
    );

    let id = api.create(
        "outbox",
        json!({ "fields": {
            "title": "Lunch on Friday",
            "kind": "email",
            "to": ["ada@example.com"],
            "subject": "Lunch on Friday",
        } }),
    );
    api.settle();
    assert!(api.notifications().is_empty(), "a draft notifies nothing");
    api.edit(&id, json!({ "status": "ready" }));
    api.settle();
    let fields = api.get(&format!("/api/notes/{id}"))["note"]["fields"].clone();
    assert_eq!(fields["status"], "failed", "{fields:#}");

    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["title"], "Could not send Lunch on Friday");
    assert_eq!(items[0]["body"], RECONNECT);
    assert_eq!(items[0]["note"], id.as_str());
    assert_eq!(items[0]["source"], "outbox");

    // Retried and failed again: one more.
    api.edit(&id, json!({ "status": "ready" }));
    api.settle();
    assert_eq!(api.notifications().len(), 2);
}
