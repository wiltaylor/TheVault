use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{FakeDecisions, Reply, TestServer};

const ROOT: &str = "/api/classifiers/scam/history";
const IDS: [&str; 3] = ["01JB8Z4K2MQ7V3X9T5R6W8Y2ZA", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC"];

fn setup(fake: &FakeDecisions) -> (TestServer, Client, String) {
    let server = TestServer::start_with_args(|vault| {
        std::fs::create_dir_all(vault.join("notes/email")).unwrap();
        for (i,id) in IDS.iter().enumerate() {
            std::fs::write(vault.join(format!("notes/email/{id}.wcl")), format!("title = \"Historic {i}\"\nsent = \"2026-11-01T12:00:00Z\"\n")).unwrap();
        }
    }, &["--test-clock".as_ref(), "2026-11-13T12:00:00Z".as_ref()]);
    let client = Client::builder().timeout(Duration::from_secs(15)).build().unwrap();
    let r = client.post(server.url("/api/integrations/ai/accounts")).bearer_auth(server.admin_token())
        .json(&json!({"name":"router", "kind":"openai_compatible", "base_url":fake.base_url(), "key":FakeDecisions::KEY})).send().unwrap();
    assert!(r.status().is_success(), "{}", r.text().unwrap());
    // Isolate historical scam work. Startup does not classify existing notes.
    std::fs::remove_file(server.vault_path().join("classifiers/agent_safety.wcl")).unwrap();
    std::fs::write(server.vault_path().join("classifiers/scam.wcl"), format!("account = \"router\"\n{}", include_str!("../../writer/defaults/classifiers/scam.wcl"))).unwrap();
    let login = client.post(server.url("/api/login")).json(&json!({"password":testkit::PASSWORD})).send().unwrap().error_for_status().unwrap();
    let cookie = login.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();
    (server,client,cookie)
}

fn post(s: &TestServer,c: &Client,cookie: &str,path: &str,body: Value) -> Response {
    c.post(s.url(&format!("{ROOT}/{path}"))).header("cookie",cookie).json(&body).send().unwrap()
}

fn preview(s: &TestServer,c: &Client,cookie: &str) -> Value {
    let r = post(s,c,cookie,"preview",json!({"days":30,"estimated_usd_per_decision":0.25}));
    assert!(r.status().is_success(), "{}",r.text().unwrap());
    r.json().unwrap()
}

fn approval(p: &Value) -> Value {
    json!({"fingerprint":p["fingerprint"], "days":p["days"], "estimated_usd_per_decision":p["unit_price"], "approve":true})
}

fn progress(s: &TestServer,c: &Client,cookie: &str) -> Value {
    c.get(s.url(ROOT)).header("cookie",cookie).send().unwrap().error_for_status().unwrap().json().unwrap()
}

fn wait(s: &TestServer,c: &Client,cookie: &str, f: impl Fn(&Value)->bool) -> Value {
    let deadline = Instant::now()+Duration::from_secs(15);
    loop { let p = progress(s,c,cookie); if f(&p) { return p; }
        assert!(Instant::now()<deadline,"{p}"); std::thread::sleep(Duration::from_millis(20)); }
}

fn response() -> Value {
    json!({"id":"history", "model":"jev", "answers":{
        "scam":{"type":"noul","noul":0.1},
        "category":{"type":"choice","choice":"other","confidence":0.9,"probabilities":{"other":1.0}}
    },"usage":{"input_tokens":3,"output_tokens":2}})
}

#[test]
fn history_session_bounds_price_and_zero_call_preview() {
    let fake = FakeDecisions::start(); let (s,c,cookie) = setup(&fake);
    for days in [0,91] {
        assert_eq!(post(&s,&c,&cookie,"preview",json!({"days":days,"estimated_usd_per_decision":0.25})).status(),400);
    }
    for price in [json!(-1),json!(null)] {
        assert!(!post(&s,&c,&cookie,"preview",json!({"days":30,"estimated_usd_per_decision":price})).status().is_success());
    }
    let p = preview(&s,&c,&cookie);
    assert_eq!(p["count"],3); assert_eq!(p["estimated_usd"],1.5);
    assert_eq!(p["decisions_per_item"],2);
    assert_eq!(post(&s,&c,&cookie,"preview",json!({"days":1,"estimated_usd_per_decision":0})).json::<Value>().unwrap()["count"],0);
    assert!(fake.requests().is_empty());
    for path in ["preview","approve"] {
        assert_eq!(c.post(s.url(&format!("{ROOT}/{path}"))).bearer_auth(s.admin_token()).json(&approval(&p)).send().unwrap().status(),403);
    }
    assert_eq!(c.get(s.url(ROOT)).bearer_auth(s.admin_token()).send().unwrap().status(),403);
    assert_eq!(c.get(s.url(ROOT)).send().unwrap().status(),401);
    let issued: Value = c.post(s.url("/api/agent-codes")).bearer_auth(s.admin_token()).json(&json!({"name":"history-cli","scope":"write"})).send().unwrap().error_for_status().unwrap().json().unwrap();
    let redeemed: Value = c.post(s.url("/api/agent-codes/redeem")).json(&json!({"code":issued["code"]})).send().unwrap().error_for_status().unwrap().json().unwrap();
    for path in ["preview","approve"] {
        assert_eq!(c.post(s.url(&format!("{ROOT}/{path}"))).bearer_auth(redeemed["token"].as_str().unwrap()).json(&approval(&p)).send().unwrap().status(),403);
    }
    // A different authenticated owner session cannot consume this session's preview.
    let p = preview(&s,&c,&cookie);
    let login = c.post(s.url("/api/login")).json(&json!({"password":testkit::PASSWORD})).send().unwrap();
    let other = login.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap();
    assert_eq!(post(&s,&c,other,"approve",approval(&p)).status(),409);
    assert!(fake.requests().is_empty());
}

#[test]
fn history_fingerprint_refuses_changed_window_price_item_account_and_definition() {
    let fake = FakeDecisions::start(); let (s,c,cookie) = setup(&fake);
    for key in ["days","estimated_usd_per_decision","fingerprint","approve"] {
        let p = preview(&s,&c,&cookie); let mut a = approval(&p);
        a[key] = match key { "days"=>json!(31), "estimated_usd_per_decision"=>json!(0.5), "approve"=>json!(false), _=>json!("other") };
        assert_eq!(post(&s,&c,&cookie,"approve",a).status(),409);
    }
    let p = preview(&s,&c,&cookie);
    let r = c.post(s.url("/api/notes?type=email")).bearer_auth(s.admin_token()).json(&json!({"note":{"fields":{"title":"New item", "sent":"2026-11-02T00:00:00Z"}}})).send().unwrap();
    assert!(r.status().is_success(),"{}",r.text().unwrap());
    assert_eq!(post(&s,&c,&cookie,"approve",approval(&p)).status(),409);
    let p = preview(&s,&c,&cookie);
    let account = c.get(s.url("/api/ai/accounts")).bearer_auth(s.admin_token()).send().unwrap().json::<Value>().unwrap();
    let id = account["accounts"][0]["id"].as_str().unwrap();
    let r = c.put(s.url(&format!("/api/integrations/ai/accounts/{id}"))).bearer_auth(s.admin_token()).json(&json!({"model":"changed"})).send().unwrap();
    assert!(r.status().is_success(),"{}",r.text().unwrap());
    assert_eq!(post(&s,&c,&cookie,"approve",approval(&p)).status(),409);
    let p = preview(&s,&c,&cookie);
    let path = s.vault_path().join("classifiers/scam.wcl");
    let source = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path,format!("{source}\n// changed definition\n")).unwrap();
    assert_eq!(post(&s,&c,&cookie,"approve",approval(&p)).status(),409);
    assert!(fake.requests().is_empty());
}

#[test]
fn history_known_price_matches_recorded_fake_charges_and_failure_is_once_visible() {
    let fake = FakeDecisions::start(); let (mut s,c,cookie) = setup(&fake);
    fake.push(Reply::json(200,&response())); fake.push(Reply::json(503,&json!({"error":"private provider text"}))); fake.push(Reply::json(200,&response()));
    let p = preview(&s,&c,&cookie);
    assert!(post(&s,&c,&cookie,"approve",approval(&p)).status().is_success());
    assert_eq!(post(&s,&c,&cookie,"approve",approval(&p)).status(),409);
    let done = wait(&s,&c,&cookie,|p| p["status"]=="complete");
    let outcomes = done["attempts"].as_array().unwrap(); assert_eq!(outcomes.len(),3);
    assert_eq!(outcomes.iter().filter(|a| a["outcome"]=="succeeded").count(),2);
    assert_eq!(outcomes.iter().filter(|a| a["outcome"]=="provider_failed").count(),1);
    // Fixture contract: this fake charges $0.25 per question on each request, including failures.
    let recorded_charges: f64 = fake.requests().iter().map(|r| {
        let b: Value = serde_json::from_slice(&r.body).unwrap(); b["questions"].as_object().unwrap().len() as f64 * 0.25
    }).sum();
    assert_eq!(p["estimated_usd"].as_f64().unwrap(),recorded_charges);
    assert!(!done.to_string().contains("private provider text"));
    s.restart(); assert_eq!(progress(&s,&c,&cookie),done); assert_eq!(fake.requests().len(),3);
}

#[test]
fn history_restart_retains_uncertain_attempt_and_resumes_only_pending_items() {
    let fake = FakeDecisions::start(); let (mut s,c,cookie) = setup(&fake);
    fake.push(Reply::json(200,&response()).delay(Duration::from_secs(3)));
    fake.push(Reply::json(200,&response())); fake.push(Reply::json(200,&response()));
    let p = preview(&s,&c,&cookie); assert!(post(&s,&c,&cookie,"approve",approval(&p)).status().is_success());
    let deadline = Instant::now()+Duration::from_secs(10);
    while fake.requests().is_empty() { assert!(Instant::now()<deadline); std::thread::sleep(Duration::from_millis(10)); }
    s.restart();
    let done = wait(&s,&c,&cookie,|p| p["status"]=="complete");
    let attempts = done["attempts"].as_array().unwrap(); assert_eq!(attempts.len(),3);
    assert_eq!(attempts[0]["outcome"],"interrupted_no_retry");
    assert!(attempts[1..].iter().all(|a| a["outcome"]=="succeeded"));
    assert_eq!(fake.requests().len(),3);
    let mut ids: Vec<_> = attempts.iter().map(|a| a["id"].as_str().unwrap()).collect(); ids.sort(); ids.dedup(); assert_eq!(ids.len(),3);
}
