//! S1 with the fake Google server: the shipped `google_gmail` task pulls Gmail into `email` and
//! `email_thread` notes (google-sync D2, D3, D4, D5, D9, D11, D14, D18, D19).

use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};
use testkit::fake_google::{DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN};
use testkit::{FakeGoogle, TestServer};

const DAY: i64 = 86_400_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// `days` days before now, in milliseconds since the epoch.
fn ago(days: i64) -> i64 {
    now_ms() - days * DAY
}

/// The UTC date of `ms` as `YYYY-MM-DD`.
fn date_of(ms: i64) -> String {
    let days = ms.div_euclid(DAY);
    // Civil from days, Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

struct Gmail {
    server: TestServer,
    client: Client,
    google: FakeGoogle,
}

impl Gmail {
    /// A server whose `google_gmail` task may reach `google`, with the three secrets set and the
    /// `google_settings` secret holding `settings` and the fake's endpoints.
    fn start(google: FakeGoogle, mut settings: Value) -> Gmail {
        settings["endpoints"] = google.endpoints();
        let server = TestServer::start();
        let gmail = Gmail {
            server,
            client: Client::new(),
            google,
        };
        let settings = settings.to_string();
        for (name, value) in [
            ("google_client_id", DEFAULT_CLIENT_ID),
            ("google_client_secret", DEFAULT_CLIENT_SECRET),
            ("google_refresh_token", DEFAULT_REFRESH_TOKEN),
            ("google_settings", settings.as_str()),
        ] {
            expect(
                gmail
                    .req("PUT", &format!("/api/secrets/{name}"))
                    .json(&json!({ "value": value }))
                    .send()
                    .unwrap(),
                200,
            );
        }
        let task = format!(
            r#"label         = "Gmail sync"
schedule      = "*/10 * * * *"
script        = "google_gmail"
timeout       = "PT5M"
retries       = 0
allow_domains = ["oauth2.googleapis.com", "gmail.googleapis.com", "www.googleapis.com", "{host}"]
secrets       = ["google_client_id", "google_client_secret", "google_refresh_token", "google_settings", "google_gmail_state"]
paused        = true
"#,
            host = gmail.google.host()
        );
        expect(
            gmail
                .req("PUT", "/api/tasks/google_gmail")
                .body(task)
                .send()
                .unwrap(),
            200,
        );
        gmail
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
    }

    /// Run the task now, and return the run.
    fn run(&self) -> Value {
        expect(
            self.req("POST", "/api/tasks/google_gmail/run?wait=true")
                .send()
                .unwrap(),
            200,
        )
    }

    /// Run the task now and check it succeeded.
    fn run_ok(&self) -> Value {
        let run = self.run();
        assert_eq!(run["outcome"], "succeeded", "{run:#}");
        run
    }

    fn query(&self, text: &str) -> Vec<Value> {
        let out = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": text, "page_size": 1000 }))
                .send()
                .unwrap(),
            200,
        );
        out["rows"].as_array().unwrap().clone()
    }

    /// Every email note, archived ones too, oldest first.
    fn emails(&self) -> Vec<Value> {
        self.query("from email include archived sort sent asc")
    }

    /// The email note of Gmail message `gmail_id`.
    fn email(&self, gmail_id: &str) -> Value {
        let rows = self.query(&format!(
            "from email include archived where gmail_ref == \"{gmail_id}\""
        ));
        assert_eq!(rows.len(), 1, "{gmail_id}: {rows:#?}");
        self.note(rows[0]["id"].as_str().unwrap())
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notes/{id}")).send().unwrap(),
            200,
        )
    }

    /// The Gmail task's state: the JSON object in the `google_gmail_state` secret.
    fn state(&self) -> Value {
        self.server.secret_json("google_gmail_state")
    }

    fn create(&self, type_name: &str, fields: Value) -> String {
        let made = expect(
            self.req("POST", &format!("/api/notes?type={type_name}"))
                .json(&json!({ "note": { "fields": fields } }))
                .send()
                .unwrap(),
            201,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn notifications(&self) -> Vec<Value> {
        let out = expect(self.req("GET", "/api/notifications").send().unwrap(), 200);
        out["items"].as_array().unwrap().clone()
    }

    fn asset(&self, id: &str) -> Vec<u8> {
        let res = self
            .req("GET", &format!("/api/assets/{id}"))
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
        res.bytes().unwrap().to_vec()
    }

    /// Replace the `google_settings` secret with `settings` and the fake's endpoints.
    fn set_settings(&self, mut settings: Value) {
        settings["endpoints"] = self.google.endpoints();
        self.put_secret("google_settings", &settings.to_string());
    }

    fn put_secret(&self, name: &str, value: &str) {
        expect(
            self.req("PUT", &format!("/api/secrets/{name}"))
                .json(&json!({ "value": value }))
                .send()
                .unwrap(),
            200,
        );
    }

    /// The `labelIds` of every message listing since the requests were last cleared, "" for a
    /// listing of all mail.
    fn listings(&self) -> Vec<String> {
        self.google
            .requests_to("/gmail/v1/users/me/messages")
            .iter()
            .filter(|r| r.route() == "/gmail/v1/users/me/messages")
            .map(|r| r.query("labelIds").unwrap_or_default())
            .collect()
    }

    /// The gmail_ref of every email note, sorted.
    fn refs(&self) -> Vec<String> {
        let mut refs: Vec<String> = self
            .emails()
            .iter()
            .map(|r| r["fields"]["gmail_ref"].as_str().unwrap().to_owned())
            .collect();
        refs.sort();
        refs
    }
}

/// The text of each paragraph in content field `name` of `note`.
fn paragraphs(note: &Value, name: &str) -> Vec<String> {
    let blocks = note["note"]["blocks"].as_array().unwrap();
    let content = blocks
        .iter()
        .find(|b| b["kind"] == "content" && b["labels"] == json!([name]));
    let Some(content) = content else {
        return Vec::new();
    };
    content["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["kind"] == "paragraph")
        .map(|b| b["fields"]["text"].as_str().unwrap().to_owned())
        .collect()
}

fn mail(id: &str, labels: &[&str], days_ago: i64) -> Value {
    json!({
        "id": id,
        "labels": labels,
        "at": ago(days_ago),
        "from": "Alex Example <alex@example.com>",
        "to": ["me@example.com"],
        "subject": format!("Subject {id}"),
        "text": format!("Body of {id}"),
    })
}

fn strs(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn the_shipped_task_is_paused_and_a_vault_without_settings_ends_successfully() {
    let server = TestServer::start();
    let text = std::fs::read_to_string(server.vault_path().join("tasks/google_gmail.wcl")).unwrap();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for want in [
        "paused = true",
        "script = \"google_gmail\"",
        "schedule = \"*/10 * * * *\"",
        "timeout = \"PT5M\"",
        "\"oauth2.googleapis.com\"",
        "\"gmail.googleapis.com\"",
        "\"www.googleapis.com\"",
        "\"google_refresh_token\"",
    ] {
        assert!(text.contains(want), "{want} in {text}");
    }
    assert!(server.vault_path().join("scripts/google_gmail.ws").exists());
    let client = Client::new();
    let auth = format!("Bearer {}", server.admin_token());
    let task = expect(
        client
            .get(server.url("/api/tasks/google_gmail"))
            .header(AUTHORIZATION, &auth)
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(task["paused"], true, "{task:#}");
    let run = expect(
        client
            .post(server.url("/api/tasks/google_gmail/run?wait=true"))
            .header(AUTHORIZATION, &auth)
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(run["outcome"], "succeeded", "{run:#}");
    assert!(
        run["log"]
            .as_str()
            .unwrap()
            .contains("Google is not connected"),
        "{run:#}"
    );
}

#[test]
fn labels_mode_imports_only_the_configured_labels_inside_the_window() {
    let google = FakeGoogle::start();
    let work = google.add_label("Work/Clients");
    let other = google.add_label("Other");
    google.add_message(mail("in-work", &["INBOX", &work], 3));
    google.add_message(mail("in-inbox", &["INBOX"], 5));
    google.add_message(mail("not-wanted", &[&other], 2));
    google.add_message(mail("too-old", &["INBOX", &work], 120));
    let gmail = Gmail::start(google, json!({ "gmail_labels": ["Work/Clients", "inbox"] }));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["in-inbox", "in-work"]);

    // The labels are term paths: user labels by name, Gmail's own under system.
    let note = gmail.email("in-work");
    assert_eq!(
        strs(&note["note"]["fields"]["labels"]),
        ["Work/Clients", "system/INBOX"]
    );
    // The listing asked Gmail for each label within the window, with format=full fetches.
    let lists = gmail.google.requests_to("/gmail/v1/users/me/messages");
    let listed: Vec<&testkit::Request> = lists
        .iter()
        .filter(|r| r.route() == "/gmail/v1/users/me/messages")
        .collect();
    assert_eq!(listed.len(), 2, "{listed:#?}");
    for r in &listed {
        assert!(r.query("q").unwrap().starts_with("after:"), "{r:?}");
        assert!(r.query("labelIds").is_some(), "{r:?}");
    }
    let settings = gmail.state();
    assert!(
        settings["history_id"]
            .as_str()
            .is_some_and(|h| !h.is_empty()),
        "{settings:#}"
    );
    assert!(settings["last_run"].as_str().is_some(), "{settings:#}");
    assert!(settings["last_error"].is_null(), "{settings:#}");
}

#[test]
fn all_mode_imports_every_message_in_the_window() {
    let google = FakeGoogle::start();
    let other = google.add_label("Other");
    google.add_message(mail("a", &["INBOX"], 1));
    google.add_message(mail("b", &[&other], 2));
    google.add_message(mail("c", &[], 10));
    google.add_message(mail("too-old", &["INBOX"], 91));
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["a", "b", "c"]);
    let listed: Vec<testkit::Request> = gmail
        .google
        .requests_to("/gmail/v1/users/me/messages")
        .into_iter()
        .filter(|r| r.route() == "/gmail/v1/users/me/messages")
        .collect();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].query("labelIds").is_none());
}

#[test]
fn later_runs_create_nothing_new_update_labels_only_and_archive_deleted_mail() {
    let google = FakeGoogle::start();
    let work = google.add_label("Work");
    google.add_message(mail("one", &["INBOX"], 1));
    google.add_message(mail("two", &["INBOX"], 2));
    let gmail = Gmail::start(google, json!({ "gmail_labels": ["INBOX"] }));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one", "two"]);
    let before = gmail.email("one");

    // No new mail: nothing created, nothing fetched, and the incremental run used history.
    gmail.google.clear_requests();
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one", "two"]);
    assert_eq!(
        gmail.google.requests_to("/gmail/v1/users/me/history").len(),
        1
    );
    assert!(
        gmail
            .google
            .requests()
            .iter()
            .all(|r| !r.route().starts_with("/gmail/v1/users/me/messages")),
        "{:#?}",
        gmail.google.requests()
    );
    assert_eq!(gmail.email("one")["version"], before["version"]);

    // A label change updates the labels and nothing else.
    gmail
        .google
        .set_message_labels("one", &["INBOX", &work, "STARRED"]);
    gmail.run_ok();
    let after = gmail.email("one");
    assert_ne!(after["version"], before["version"]);
    assert_eq!(
        strs(&after["note"]["fields"]["labels"]),
        ["Work", "system/INBOX", "system/STARRED"]
    );
    let mut old_fields = before["note"]["fields"].clone();
    let mut new_fields = after["note"]["fields"].clone();
    for f in [&mut old_fields, &mut new_fields] {
        let obj = f.as_object_mut().unwrap();
        obj.remove("labels");
        obj.remove("updated");
    }
    assert_eq!(old_fields, new_fields);
    assert_eq!(after["note"]["blocks"], before["note"]["blocks"]);

    // A message deleted in Gmail archives its note.
    gmail.google.delete_message("two");
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one", "two"]);
    let archived: Vec<Value> = gmail
        .query("from email where archived")
        .iter()
        .map(|r| r["fields"]["gmail_ref"].clone())
        .collect();
    assert_eq!(archived, [json!("two")]);

    // A new message arrives through history.
    gmail.google.add_message(mail("three", &["INBOX"], 0));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one", "three", "two"]);
}

#[test]
fn a_processed_email_stays_processed_when_its_gmail_labels_change() {
    let google = FakeGoogle::start();
    let work = google.add_label("Work");
    google.add_message(mail("one", &["INBOX"], 1));
    let gmail = Gmail::start(google, json!({ "gmail_labels": ["INBOX"] }));
    gmail.run_ok();
    let email = gmail.email("one");
    assert_ne!(email["note"]["fields"]["processed"], true, "{email:#}");
    let id = email["id"].as_str().unwrap();
    let mut note = email["note"].clone();
    note["fields"]["processed"] = json!(true);
    expect(
        gmail
            .req("PUT", &format!("/api/notes/{id}"))
            .json(&json!({ "note": note, "base_version": email["version"] }))
            .send()
            .unwrap(),
        200,
    );

    // A label change rewrites the labels, and leaves processed as the user set it.
    gmail.google.set_message_labels("one", &["INBOX", &work]);
    gmail.run_ok();
    let after = gmail.email("one");
    assert_eq!(
        strs(&after["note"]["fields"]["labels"]),
        ["Work", "system/INBOX"]
    );
    assert_eq!(after["note"]["fields"]["processed"], true, "{after:#}");
    assert_eq!(gmail.query("from email where !processed").len(), 0);
    assert_eq!(gmail.query("from email where processed").len(), 1);
}

#[test]
fn a_history_404_falls_back_to_a_full_list_without_duplicating() {
    let google = FakeGoogle::start();
    google.add_message(mail("one", &["INBOX"], 1));
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    gmail.run_ok();
    gmail.google.add_message(mail("two", &["INBOX"], 0));
    gmail.google.set_history_not_found(true);
    gmail.google.clear_requests();
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one", "two"]);
    assert_eq!(
        gmail.google.requests_to("/gmail/v1/users/me/history").len(),
        1
    );
    assert!(
        gmail
            .google
            .requests()
            .iter()
            .any(|r| r.route() == "/gmail/v1/users/me/messages"),
    );
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one", "two"]);
    assert_eq!(gmail.emails().len(), 2);
}

#[test]
fn back_fill_pages_older_mail_over_runs_and_stops_at_the_back_fill_date() {
    let google = FakeGoogle::builder().gmail_page_size(1).start();
    google.add_message(mail("recent", &["INBOX"], 1));
    google.add_message(mail("old-100", &["INBOX"], 100));
    google.add_message(mail("old-120", &["INBOX"], 120));
    google.add_message(mail("old-150", &["INBOX"], 150));
    google.add_message(mail("old-250", &["INBOX"], 250));
    let gmail = Gmail::start(
        google,
        json!({ "gmail_mode": "all", "gmail_backfill_to": date_of(ago(200)) }),
    );
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["old-100", "recent"]);
    assert!(
        gmail.state()["backfill_page"]
            .as_str()
            .is_some_and(|p| !p.is_empty())
    );
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["old-100", "old-120", "recent"]);
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["old-100", "old-120", "old-150", "recent"]);
    gmail.google.clear_requests();
    gmail.run_ok();
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["old-100", "old-120", "old-150", "recent"]);
    // Once done, a run lists nothing older.
    assert!(
        gmail
            .google
            .requests()
            .iter()
            .all(|r| r.route() != "/gmail/v1/users/me/messages"),
        "{:#?}",
        gmail.google.requests()
    );
}

#[test]
fn bodies_html_and_attachments_are_stored_and_an_oversized_message_keeps_its_headers() {
    let google = FakeGoogle::start();
    google.add_message(json!({
        "id": "plain",
        "labels": ["INBOX"],
        "at": ago(1),
        "from": "alex@example.com",
        "subject": "Plain and HTML",
        "text": "Hello there — café ☕ 😀\n\nSecond paragraph \"quoted\" \\ {braces}",
        "html": "<p>Hello there</p>",
        "attachments": [
            { "filename": "small.bin", "mime": "application/octet-stream", "size": 600 * 1024 },
            { "filename": "large.zip", "mime": "application/zip", "size": 800 * 1024 },
        ],
    }));
    google.add_message(json!({
        "id": "html-only",
        "labels": ["INBOX"],
        "at": ago(2),
        "from": "alex@example.com",
        "subject": "HTML only",
        "html": "<html><head><style>p { color: red; }</style></head><body><h1>Big news</h1><p>First &amp; <b>bold</b> line.</p><p>Second<br>line &lt;3</p><script>var x = 1;</script></body></html>",
    }));
    google.add_message(json!({
        "id": "huge",
        "labels": ["INBOX"],
        "at": ago(3),
        "from": "Big Sender <big@example.com>",
        "subject": "Huge message",
        "snippet": "A very large message",
        "text": "x".repeat(1100 * 1024),
    }));
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["html-only", "huge", "plain"]);

    let plain = gmail.email("plain");
    assert_eq!(
        paragraphs(&plain, "body")[..2],
        [
            "Hello there — café ☕ 😀".to_owned(),
            // Markup characters are escaped, so the text shows as it was sent.
            "Second paragraph \"quoted\" \\\\ \\{braces\\}".to_owned()
        ]
    );
    let fields = &plain["note"]["fields"];
    let html = fields["html"].as_str().unwrap();
    assert_eq!(gmail.asset(html), b"<p>Hello there</p>");
    let attachments = fields["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1, "{fields:#}");
    let small = gmail.asset(attachments[0].as_str().unwrap());
    assert_eq!(small.len(), 600 * 1024);
    assert!(small.iter().enumerate().all(|(k, b)| *b == (k % 251) as u8));
    let listed = paragraphs(&plain, "body").join("\n");
    assert!(listed.contains("large.zip"), "{listed}");
    assert!(listed.contains("800 KB"), "{listed}");

    let html_only = gmail.email("html-only");
    let text = paragraphs(&html_only, "body");
    assert_eq!(
        text,
        ["Big news", "First & bold line.", "Second", "line <3"],
        "{text:?}"
    );
    assert!(html_only["note"]["fields"]["html"].as_str().is_some());

    let huge = gmail.email("huge");
    let fields = &huge["note"]["fields"];
    assert_eq!(fields["title"], "Huge message");
    assert_eq!(fields["from_raw"], "Big Sender <big@example.com>");
    assert_eq!(fields["snippet"], "A very large message");
    assert!(fields.get("html").is_none());
    let said = paragraphs(&huge, "body").join("\n");
    assert!(said.contains("too large"), "{said}");
}

#[test]
fn threads_group_messages_and_senders_link_to_people_by_email() {
    let google = FakeGoogle::start();
    google.add_message(json!({
        "id": "first",
        "thread": "t-1",
        "labels": ["INBOX"],
        "at": ago(3),
        "from": "Alex Example <Alex@Example.com>",
        "to": ["me@example.com", "\"Lee, Sam\" <sam@example.com>"],
        "cc": ["Stranger <stranger@example.com>"],
        "subject": "Plans",
        "text": "Shall we?",
    }));
    google.add_message(json!({
        "id": "reply",
        "thread": "t-1",
        "labels": ["INBOX"],
        "at": ago(2),
        "from": "\"Lee, Sam\" <sam@example.com>",
        "to": ["Alex Example <alex@example.com>"],
        "subject": "Re: Plans",
        "text": "Yes.",
    }));
    google.add_message(json!({
        "id": "unknown",
        "labels": ["INBOX"],
        "at": ago(1),
        "from": "Nobody <nobody@example.com>",
        "to": ["me@example.com"],
        "subject": "Who?",
        "text": "Hi",
    }));
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    let alex = gmail.create(
        "person",
        json!({ "title": "Alex", "emails": [{ "address": "alex@example.com" }] }),
    );
    let sam = gmail.create(
        "person",
        // Stored in mixed case, unlike any address the messages give: matched ignoring case.
        json!({ "title": "Sam", "emails": [{ "address": "work@sam.example" }, { "address": "Sam@Example.COM", "label": "home" }] }),
    );
    gmail.run_ok();

    let first = gmail.email("first");
    let f = &first["note"]["fields"];
    assert_eq!(f["title"], "Plans");
    assert_eq!(f["from"], alex.as_str());
    assert_eq!(f["from_raw"], "Alex Example <Alex@Example.com>");
    assert_eq!(f["to"], json!([sam.clone()]));
    assert_eq!(
        f["to_raw"],
        json!(["me@example.com", "\"Lee, Sam\" <sam@example.com>"])
    );
    assert!(f.get("cc").is_none_or(|c| c == &json!([])), "{f:#}");
    assert_eq!(f["cc_raw"], json!(["Stranger <stranger@example.com>"]));
    assert!(f["sent"].as_str().is_some());

    let reply = gmail.email("reply");
    assert_eq!(reply["note"]["fields"]["from"], sam.as_str());
    assert_eq!(reply["note"]["fields"]["thread"], f["thread"]);

    let threads = gmail.query("from email_thread");
    assert_eq!(threads.len(), 2, "{threads:#?}");
    let thread = gmail.note(f["thread"].as_str().unwrap());
    assert_eq!(thread["note"]["fields"]["title"], "Plans");
    assert_eq!(thread["note"]["fields"]["thread_ref"], "t-1");
    assert_eq!(strs(&thread["note"]["fields"]["participants"]), {
        let mut p = vec![alex.clone(), sam.clone()];
        p.sort();
        p
    });

    let unknown = gmail.email("unknown");
    let u = &unknown["note"]["fields"];
    assert!(u.get("from").is_none(), "{u:#}");
    assert_eq!(u["from_raw"], "Nobody <nobody@example.com>");
    // No person was created for an unknown address.
    assert_eq!(gmail.query("from person").len(), 2);
}

#[test]
fn invalid_grant_notifies_once_across_two_runs_and_does_not_fail_the_run() {
    let google = FakeGoogle::builder().invalid_grant(true).start();
    google.add_message(mail("one", &["INBOX"], 1));
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    let first = gmail.run_ok();
    let second = gmail.run_ok();
    assert!(gmail.refs().is_empty());
    let error = gmail.state()["last_error"].clone();
    assert!(
        error.as_str().is_some_and(|e| e.contains("invalid_grant")),
        "{error} {first:#} {second:#}"
    );
    let bells: Vec<Value> = gmail
        .notifications()
        .into_iter()
        .filter(|n| n["title"] == "Google sign-in expired")
        .collect();
    assert_eq!(bells.len(), 1, "{bells:#?}");
    assert_eq!(
        bells[0]["body"],
        "Connect Google again in Settings > Integrations."
    );
    assert_eq!(gmail.notifications().len(), 1);

    // Signed in again: the run imports and clears the error.
    gmail.google.set_invalid_grant(false);
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["one"]);
    assert!(gmail.state()["last_error"].is_null());
}

#[test]
fn a_run_with_more_mail_than_it_may_decode_stops_early_and_the_next_run_carries_on() {
    let google = FakeGoogle::start();
    // Each body is about 90 KB of text: more than a dozen is more than one run decodes.
    let line = "Ünïcödé text for the instruction budget, line after line.\n";
    let body = line.repeat(90 * 1024 / line.len());
    let wide = "日本語のテキストは一文字が三バイトです。\n".repeat(1000);
    google.add_message(json!({
        "id": "wide",
        "labels": ["INBOX"],
        "at": ago(2),
        "from": "alex@example.com",
        "subject": "Wide",
        "text": wide,
    }));
    for k in 0..14 {
        google.add_message(json!({
            "id": format!("big-{k:02}"),
            "labels": ["INBOX"],
            "at": ago(1) - k * 60_000,
            "from": "alex@example.com",
            "subject": format!("Long {k}"),
            "text": body,
        }));
    }
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    let first = gmail.run_ok();
    let after_first = gmail.refs().len();
    assert!(
        after_first > 0 && after_first < 15,
        "{after_first} {first:#}"
    );
    assert!(
        first["log"].as_str().unwrap().contains("Stopping early"),
        "{first:#}"
    );
    // The window listing did not finish, so there is no history id yet.
    assert!(gmail.state()["history_id"].is_null());
    gmail.run_ok();
    assert_eq!(gmail.refs().len(), 15);
    assert!(gmail.state()["history_id"].as_str().is_some());
    let wide = gmail.email("wide");
    let lines = paragraphs(&wide, "body");
    assert_eq!(lines.len(), 1000);
    assert!(
        lines
            .iter()
            .all(|l| l == "日本語のテキストは一文字が三バイトです。")
    );
    let note = gmail.email("big-03");
    assert_eq!(
        paragraphs(&note, "body")[0],
        "Ünïcödé text for the instruction budget, line after line."
    );
    // A body part longer than the sync decodes is cut, and says so.
    assert_eq!(
        paragraphs(&note, "body").last().unwrap(),
        "The rest of the message is longer than the sync keeps."
    );
}

// ---- settings changes (google-sync D11, D21) ----------------------------------------------------

#[test]
fn a_first_run_with_no_labels_saves_no_history_and_adding_labels_imports_their_window() {
    let google = FakeGoogle::start();
    let work = google.add_label("Work");
    google.add_message(mail("work-1", &[&work], 3));
    google.add_message(mail("work-2", &["INBOX", &work], 20));
    google.add_message(mail("inbox", &["INBOX"], 2));
    let gmail = Gmail::start(
        google,
        json!({ "gmail_mode": "labels", "gmail_labels": [] }),
    );
    gmail.run_ok();
    assert!(gmail.refs().is_empty());
    let state = gmail.state();
    assert!(state["history_id"].is_null(), "{state:#}");
    assert!(state["listed"].is_null(), "{state:#}");

    gmail.set_settings(json!({ "gmail_mode": "labels", "gmail_labels": ["Work"] }));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["work-1", "work-2"]);
    let state = gmail.state();
    assert!(state["history_id"].as_str().is_some(), "{state:#}");
    assert_eq!(
        state["listed"],
        json!({ "mode": "labels", "labels": [work], "window_days": 90 })
    );
}

#[test]
fn switching_labels_to_all_lists_the_window_again_and_restarts_the_back_fill() {
    let google = FakeGoogle::start();
    let other = google.add_label("Other");
    google.add_message(mail("inbox", &["INBOX"], 1));
    google.add_message(mail("other", &[&other], 2));
    google.add_message(mail("unlabelled", &[], 30));
    google.add_message(mail("old-inbox", &["INBOX"], 100));
    google.add_message(mail("old-other", &[&other], 110));
    google.add_message(mail("too-old", &[&other], 300));
    let backfill = date_of(ago(200));
    let gmail = Gmail::start(
        google,
        json!({ "gmail_labels": ["INBOX"], "gmail_backfill_to": backfill }),
    );
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["inbox", "old-inbox"]);
    // A later run with the same settings reads history only.
    gmail.google.clear_requests();
    gmail.run_ok();
    assert!(gmail.listings().is_empty(), "{:#?}", gmail.listings());

    gmail.set_settings(json!({ "gmail_mode": "all", "gmail_backfill_to": backfill }));
    gmail.google.clear_requests();
    let run = gmail.run_ok();
    assert_eq!(
        gmail.refs(),
        ["inbox", "old-inbox", "old-other", "other", "unlabelled"],
        "{run:#}"
    );
    // History was still read for incremental changes, and the window listed all mail.
    assert_eq!(
        gmail.google.requests_to("/gmail/v1/users/me/history").len(),
        1
    );
    assert!(gmail.listings().iter().all(String::is_empty));
    assert_eq!(
        gmail.state()["listed"],
        json!({ "mode": "all", "labels": [], "window_days": 90 })
    );

    // Nothing duplicates, and the next run lists nothing.
    gmail.google.clear_requests();
    gmail.run_ok();
    assert!(gmail.listings().is_empty(), "{:#?}", gmail.listings());
    assert_eq!(gmail.emails().len(), 5);
}

#[test]
fn adding_a_label_lists_only_that_label_and_removing_one_lists_nothing() {
    let google = FakeGoogle::start();
    let work = google.add_label("Work");
    let home = google.add_label("Home");
    google.add_message(mail("work", &[&work], 1));
    google.add_message(mail("both", &[&work, &home], 2));
    google.add_message(mail("home", &[&home], 3));
    google.add_message(mail("inbox", &["INBOX"], 4));
    let gmail = Gmail::start(google, json!({ "gmail_labels": ["Work"] }));
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["both", "work"]);

    gmail.set_settings(json!({ "gmail_labels": ["Work", "Home"] }));
    gmail.google.clear_requests();
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["both", "home", "work"]);
    assert_eq!(gmail.listings(), std::slice::from_ref(&home));
    let mut labels = vec![work.clone(), home.clone()];
    labels.sort();
    assert_eq!(gmail.state()["listed"]["labels"], json!(labels));

    gmail.set_settings(json!({ "gmail_labels": ["Home"] }));
    gmail.google.clear_requests();
    gmail.run_ok();
    assert!(gmail.listings().is_empty(), "{:#?}", gmail.listings());
    assert_eq!(gmail.state()["listed"]["labels"], json!([home]));
    assert_eq!(gmail.emails().len(), 3);
}

#[test]
fn a_state_with_a_history_id_but_no_listing_record_lists_the_window_once() {
    // The state an earlier version left after a first run with no labels.
    let google = FakeGoogle::start();
    google.add_message(mail("a", &["INBOX"], 1));
    google.add_message(mail("b", &[], 5));
    let history = google.gmail_history_id().to_string();
    let gmail = Gmail::start(google, json!({ "gmail_mode": "all" }));
    gmail.put_secret(
        "google_gmail_state",
        &json!({ "history_id": history }).to_string(),
    );
    gmail.run_ok();
    assert_eq!(gmail.refs(), ["a", "b"]);
    gmail.google.clear_requests();
    gmail.run_ok();
    assert!(gmail.listings().is_empty());
    assert_eq!(gmail.emails().len(), 2);
}

#[test]
fn a_back_fill_date_inside_the_window_is_reported_once() {
    let google = FakeGoogle::start();
    google.add_message(mail("a", &["INBOX"], 1));
    let gmail = Gmail::start(
        google,
        json!({ "gmail_mode": "all", "gmail_backfill_to": date_of(ago(10)) }),
    );
    let note = "back-fill date is inside the window; nothing older to fetch";
    let first = gmail.run_ok();
    assert!(first["log"].as_str().unwrap().contains(note), "{first:#}");
    let second = gmail.run_ok();
    assert!(
        !second["log"].as_str().unwrap().contains(note),
        "{second:#}"
    );
}
