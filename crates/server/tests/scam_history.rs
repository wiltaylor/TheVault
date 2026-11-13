use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{FakeDecisions, Reply, TestServer};

const ROOT: &str = "/api/classifiers/scam/history";
const IDS: [&str; 3] = [
    "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA",
    "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB",
    "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC",
];

fn setup(fake: &FakeDecisions) -> (TestServer, Client, String) {
    setup_with(fake, |_| {})
}

fn setup_with(fake: &FakeDecisions, seed: impl FnOnce(&std::path::Path)) -> (TestServer, Client, String) {
    let server = TestServer::start_with_args(
        |vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(
                vault.join("types/email.wcl"),
                "label = \"Email\"\nfield sent { kind = \"datetime\" }\nfield labels { kind = \"list\" items = \"taxonomy\" taxonomy = \"gmail_label\" }\nfield scam_probability { kind = \"float\" }\nfield communication_category { kind = \"string\" }\ntemplate default { tab Main { slot title } }\n",
            )
            .unwrap();
            std::fs::create_dir_all(vault.join("taxonomies")).unwrap();
            std::fs::write(
                vault.join("taxonomies/gmail_label.wcl"),
                include_str!("../../writer/defaults/taxonomies/gmail_label.wcl"),
            )
            .unwrap();
            std::fs::create_dir_all(vault.join("notes/email")).unwrap();
            for (i, id) in IDS.iter().enumerate() {
                std::fs::write(
                    vault.join(format!("notes/email/{id}.wcl")),
                    format!("title = \"Historic {i}\"\nsent = \"2026-11-01T12:00:00Z\"\n"),
                )
                .unwrap();
            }
            for (suffix, extra) in [
                ("D", "labels = [\"system/SENT\"]"),
                ("E", "labels = [\"system/DRAFT\"]"),
                ("F", "scam_probability = 0.9"),
            ] {
                std::fs::write(
                    vault.join(format!("notes/email/01JB8Z4K2MQ7V3X9T5R6W8Y2Z{suffix}.wcl")),
                    format!(
                        "title = \"Excluded fixture\"\nsent = \"2026-11-01T12:00:00Z\"\n{extra}\n"
                    ),
                )
                .unwrap();
            }
            seed(vault);
        },
        &["--test-clock".as_ref(), "2026-11-13T12:00:00Z".as_ref()],
    );
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let r = client.post(server.url("/api/integrations/ai/accounts")).bearer_auth(server.admin_token())
        .json(&json!({"name":"router", "kind":"openai_compatible", "base_url":fake.base_url(), "key":FakeDecisions::KEY})).send().unwrap();
    assert!(r.status().is_success(), "{}", r.text().unwrap());
    // Isolate historical scam work. Startup does not classify existing notes.
    std::fs::remove_file(server.vault_path().join("classifiers/agent_safety.wcl")).unwrap();
    std::fs::write(
        server.vault_path().join("classifiers/scam.wcl"),
        format!(
            "account = \"router\"\n{}",
            include_str!("../../writer/defaults/classifiers/scam.wcl")
        ),
    )
    .unwrap();
    let login = client
        .post(server.url("/api/login"))
        .json(&json!({"password":testkit::PASSWORD}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (server, client, cookie)
}

fn post(s: &TestServer, c: &Client, cookie: &str, path: &str, body: Value) -> Response {
    c.post(s.url(&format!("{ROOT}/{path}")))
        .header("cookie", cookie)
        .json(&body)
        .send()
        .unwrap()
}

fn preview(s: &TestServer, c: &Client, cookie: &str) -> Value {
    let r = post(
        s,
        c,
        cookie,
        "preview",
        json!({"days":30,"estimated_usd_per_decision":0.25}),
    );
    assert!(r.status().is_success(), "{}", r.text().unwrap());
    r.json().unwrap()
}

fn approval(p: &Value) -> Value {
    json!({"fingerprint":p["fingerprint"], "days":p["days"], "estimated_usd_per_decision":p["unit_price"], "approve":true})
}

fn progress(s: &TestServer, c: &Client, cookie: &str) -> Value {
    c.get(s.url(ROOT))
        .header("cookie", cookie)
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap()
}

fn wait(s: &TestServer, c: &Client, cookie: &str, f: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let p = progress(s, c, cookie);
        if f(&p) {
            return p;
        }
        assert!(Instant::now() < deadline, "{p}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn response() -> Value {
    json!({"id":"history", "model":"jev", "answers":{
        "scam":{"type":"noul","noul":0.1},
        "category":{"type":"choice","choice":"other","confidence":0.9,"probabilities":{"scam":0.0,"spam":0.0,"marketing":0.0,"personal":0.0,"transactional":0.0,"other":1.0}}
    },"usage":{"input_tokens":3,"output_tokens":2}})
}

#[test]
fn history_session_bounds_price_and_zero_call_preview() {
    let fake = FakeDecisions::start();
    let (s, c, cookie) = setup(&fake);
    for days in [0, 91] {
        assert_eq!(
            post(
                &s,
                &c,
                &cookie,
                "preview",
                json!({"days":days,"estimated_usd_per_decision":0.25})
            )
            .status(),
            400
        );
    }
    for price in [json!(-1), json!(null), json!(1e308)] {
        assert!(
            !post(
                &s,
                &c,
                &cookie,
                "preview",
                json!({"days":30,"estimated_usd_per_decision":price})
            )
            .status()
            .is_success()
        );
    }
    let p = preview(&s, &c, &cookie);
    assert_eq!(p["count"], 3);
    assert_eq!(p["estimated_usd"], 1.5);
    assert_eq!(p["decisions_per_item"], 2);
    assert_eq!(
        post(
            &s,
            &c,
            &cookie,
            "preview",
            json!({"days":1,"estimated_usd_per_decision":0})
        )
        .json::<Value>()
        .unwrap()["count"],
        0
    );
    assert!(fake.requests().is_empty());
    for path in ["preview", "approve"] {
        assert_eq!(
            c.post(s.url(&format!("{ROOT}/{path}")))
                .bearer_auth(s.admin_token())
                .json(&approval(&p))
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        c.get(s.url(ROOT))
            .bearer_auth(s.admin_token())
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(c.get(s.url(ROOT)).send().unwrap().status(), 401);
    let issued: Value = c
        .post(s.url("/api/agent-codes"))
        .bearer_auth(s.admin_token())
        .json(&json!({"name":"history-cli","scope":"write"}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    let redeemed: Value = c
        .post(s.url("/api/agent-codes/redeem"))
        .json(&json!({"code":issued["code"]}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    for path in ["preview", "approve"] {
        assert_eq!(
            c.post(s.url(&format!("{ROOT}/{path}")))
                .bearer_auth(redeemed["token"].as_str().unwrap())
                .json(&approval(&p))
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    // A different authenticated owner session cannot consume this session's preview.
    let p = preview(&s, &c, &cookie);
    let login = c
        .post(s.url("/api/login"))
        .json(&json!({"password":testkit::PASSWORD}))
        .send()
        .unwrap();
    let other = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    assert_eq!(post(&s, &c, other, "approve", approval(&p)).status(), 409);
    assert!(fake.requests().is_empty());
}

#[test]
fn history_fingerprint_refuses_changed_window_price_item_account_and_definition() {
    let fake = FakeDecisions::start();
    let (s, c, cookie) = setup(&fake);
    for key in [
        "days",
        "estimated_usd_per_decision",
        "fingerprint",
        "approve",
    ] {
        let p = preview(&s, &c, &cookie);
        let mut a = approval(&p);
        a[key] = match key {
            "days" => json!(31),
            "estimated_usd_per_decision" => json!(0.5),
            "approve" => json!(false),
            _ => json!("other"),
        };
        assert_eq!(post(&s, &c, &cookie, "approve", a).status(), 409);
    }
    let p = preview(&s, &c, &cookie);
    let r = c
        .post(s.url("/api/notes?type=email"))
        .bearer_auth(s.admin_token())
        .json(&json!({"note":{"fields":{"title":"New item", "sent":"2026-11-02T00:00:00Z"}}}))
        .send()
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().unwrap());
    assert_eq!(post(&s, &c, &cookie, "approve", approval(&p)).status(), 409);
    let p = preview(&s, &c, &cookie);
    let account = c
        .get(s.url("/api/ai/accounts"))
        .bearer_auth(s.admin_token())
        .send()
        .unwrap()
        .json::<Value>()
        .unwrap();
    let id = account["accounts"][0]["id"].as_str().unwrap();
    let r = c
        .put(s.url(&format!("/api/integrations/ai/accounts/{id}")))
        .bearer_auth(s.admin_token())
        .json(&json!({"model":"changed"}))
        .send()
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().unwrap());
    assert_eq!(post(&s, &c, &cookie, "approve", approval(&p)).status(), 409);
    let p = preview(&s, &c, &cookie);
    let path = s.vault_path().join("classifiers/scam.wcl");
    let source = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, format!("{source}\n// changed definition\n")).unwrap();
    assert_eq!(post(&s, &c, &cookie, "approve", approval(&p)).status(), 409);
    assert!(fake.requests().is_empty());
}

#[test]
fn history_known_price_matches_recorded_fake_charges_and_failure_is_once_visible() {
    let fake = FakeDecisions::start();
    let (mut s, c, cookie) = setup(&fake);
    fake.push(Reply::json(200, &response()));
    fake.push(Reply::json(503, &json!({"error":"private provider text"})));
    fake.push(Reply::json(200, &response()));
    let p = preview(&s, &c, &cookie);
    assert!(
        post(&s, &c, &cookie, "approve", approval(&p))
            .status()
            .is_success()
    );
    assert_eq!(post(&s, &c, &cookie, "approve", approval(&p)).status(), 409);
    let done = wait(&s, &c, &cookie, |p| p["status"] == "complete");
    let outcomes = done["attempts"].as_array().unwrap();
    assert_eq!(outcomes.len(), 3);
    assert_eq!(
        outcomes
            .iter()
            .filter(|a| a["outcome"] == "succeeded")
            .count(),
        2,
        "{done}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|a| a["outcome"] == "provider_failed")
            .count(),
        1
    );
    // Fixture contract: this fake charges $0.25 per question on each request, including failures.
    let recorded_charges: f64 = fake
        .requests()
        .iter()
        .map(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["questions"].as_object().unwrap().len() as f64 * 0.25
        })
        .sum();
    assert_eq!(p["estimated_usd"].as_f64().unwrap(), recorded_charges);
    assert!(!done.to_string().contains("private provider text"));
    s.restart();
    assert_eq!(progress(&s, &c, &cookie), done);
    assert_eq!(fake.requests().len(), 3);
}

#[test]
fn history_preview_expires_on_the_fake_clock_without_provider_work() {
    let fake = FakeDecisions::start();
    let (s, c, cookie) = setup(&fake);
    let p = preview(&s, &c, &cookie);
    c.post(s.url("/api/test/clock"))
        .bearer_auth(s.admin_token())
        .json(&json!({"advance":"PT10M"}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(post(&s, &c, &cookie, "approve", approval(&p)).status(), 409);
    assert!(fake.requests().is_empty());
}

#[test]
fn history_restart_retains_uncertain_attempt_and_resumes_only_pending_items() {
    let fake = FakeDecisions::start();
    let (mut s, c, cookie) = setup(&fake);
    fake.push(Reply::json(200, &response()).delay(Duration::from_secs(3)));
    fake.push(Reply::json(200, &response()));
    fake.push(Reply::json(200, &response()));
    let p = preview(&s, &c, &cookie);
    assert!(
        post(&s, &c, &cookie, "approve", approval(&p))
            .status()
            .is_success()
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake.requests().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    s.restart();
    let done = wait(&s, &c, &cookie, |p| p["status"] == "complete");
    let attempts = done["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 3);
    assert_eq!(attempts[0]["outcome"], "interrupted_no_retry");
    assert!(
        attempts[1..].iter().all(|a| a["outcome"] == "succeeded"),
        "{done}"
    );
    assert_eq!(fake.requests().len(), 3);
    let mut ids: Vec<_> = attempts.iter().map(|a| a["id"].as_str().unwrap()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
}

fn definition(s: &TestServer, condition: &str) {
    let path = s.vault_path().join("classifiers/scam.wcl");
    let source = std::fs::read_to_string(&path).unwrap();
    let source = source.lines().map(|line| {
        if line.starts_with("where ") {
            format!("where = {}", serde_json::to_string(condition).unwrap())
        } else { line.to_owned() }
    }).collect::<Vec<_>>().join("\n");
    std::fs::write(path, source).unwrap();
}

#[test]
fn history_or_filter_cannot_escape_window_and_nonstandard_date_is_refused() {
    let fake = FakeDecisions::start();
    let (s, c, cookie) = setup_with(&fake, |vault| {
        for (suffix, sent) in [("G", "2020-01-01T00:00:00Z"), ("H", "2026-11-13T00:00:00Z"), ("J", "2026-11-11T12:00:00Z"), ("K", "2027-01-01T00:00:00Z")] {
            std::fs::write(vault.join(format!("notes/email/01JB8Z4K2MQ7V3X9T5R6W8Y2Z{suffix}.wcl")), format!("title = \"Outside history\"\nsent = \"{sent}\"\n")).unwrap();
        }
    });
    definition(&s, "type == \"email\" || sent >= now() - days(2)");
    assert_eq!(preview(&s, &c, &cookie)["count"], 3);
    definition(&s, "sent >= now() - days(3)");
    let r = post(&s, &c, &cookie, "preview", json!({"days":30,"estimated_usd_per_decision":0}));
    assert_eq!(r.status(), 400);
    assert!(r.text().unwrap().contains("standard sent-date filter"));
    assert!(fake.requests().is_empty());
}

#[test]
fn history_excludes_outgoing_messages() {
    let fake = FakeDecisions::start();
    let (s, c, cookie) = setup_with(&fake, |vault| {
        std::fs::write(vault.join("types/message.wcl"), "label = \"Message\"\nfield sent { kind = \"datetime\" }\nfield direction { kind = \"string\" }\nfield scam_probability { kind = \"float\" }\nfield communication_category { kind = \"string\" }\ntemplate default { tab Main { slot title } }\n").unwrap();
        std::fs::create_dir_all(vault.join("notes/message")).unwrap();
        for (suffix, direction) in [("G", "in"), ("H", "out")] {
            std::fs::write(vault.join(format!("notes/message/01JB8Z4K2MQ7V3X9T5R6W8Y2Z{suffix}.wcl")), format!("title = \"Message fixture\"\nsent = \"2026-11-01T12:00:00Z\"\ndirection = \"{direction}\"\n")).unwrap();
        }
    });
    assert_eq!(preview(&s, &c, &cookie)["count"], 4);
    assert!(fake.requests().is_empty());
}

#[test]
fn history_refuses_more_than_five_thousand_eligible_items_even_at_zero_price() {
    let fake = FakeDecisions::start();
    let (s, c, cookie) = setup_with(&fake, |vault| {
        for i in 0..4998_u128 {
            let id = format!("{:026}", i + 1);
            std::fs::write(vault.join(format!("notes/email/{id}.wcl")), "title = \"Cap fixture\"\nsent = \"2026-11-01T12:00:00Z\"\n").unwrap();
        }
    });
    let r = post(&s, &c, &cookie, "preview", json!({"days":30,"estimated_usd_per_decision":0}));
    assert_eq!(r.status(), 400);
    assert!(r.text().unwrap().contains("5,000 items"));
    assert!(fake.requests().is_empty());
}

#[test]
fn history_skips_notes_and_configuration_changed_after_approval() {
    for change_config in [false, true] {
        let fake = FakeDecisions::start();
        let (s, c, cookie) = setup(&fake);
        fake.push(Reply::json(200, &response()).delay(Duration::from_secs(2)));
        fake.push(Reply::json(200, &response()));
        let p = preview(&s, &c, &cookie);
        post(&s, &c, &cookie, "approve", approval(&p)).error_for_status().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while fake.requests().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        if change_config {
            let path = s.vault_path().join("classifiers/scam.wcl");
            let source = std::fs::read_to_string(&path).unwrap();
            std::fs::write(path, format!("{source}\n// changed after approval\n")).unwrap();
        } else {
            let url = s.url(&format!("/api/notes/{}", IDS[1]));
            let n: Value = c.get(&url).bearer_auth(s.admin_token()).send().unwrap().json().unwrap();
            c.put(&url).bearer_auth(s.admin_token()).json(&json!({"base_version":n["version"],"note":{"fields":{"title":"Changed after approval"}}})).send().unwrap().error_for_status().unwrap();
        }
        let done = wait(&s, &c, &cookie, |p| p["status"] == "complete");
        let expected = if change_config { "skipped_configuration_changed" } else { "skipped_note_changed" };
        assert_eq!(done["attempts"][1]["outcome"], expected, "{done}");
        assert_eq!(fake.requests().len(), if change_config { 1 } else { 2 });
    }
}
