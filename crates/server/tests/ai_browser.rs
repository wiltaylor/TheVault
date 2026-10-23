//! S1 with the fake provider and the fake browser box: the chat's browser tools (browser B5 to
//! B7). `browser_run` runs a command and answers its exit code and output, the tail kept, with a
//! trail line; `browser_file` answers an image as an image part, text as text and anything else
//! as metadata; `browser_save` follows the downloads switch, a proposal that `POST /api/ai/apply`
//! stores or a store at once. The tools are offered only with `VAULT_BROWSER_URL` set and the
//! agent's `permissions.browser` on, whatever its tools list says, and an agent cannot give
//! browser automation it lacks (agents G20).

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, BrowserRun, FakeAnthropic, FakeBrowser, TestServer};

const MODEL: &str = "claude-opus-5";

/// An agent with browser automation and every other default.
const DRIVER: &str = "label = \"Driver\"\n\npermissions {\n  browser = true\n}\n";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    /// A server with `browser` as its box, or none.
    fn start(fake: &FakeAnthropic, browser: Option<&FakeBrowser>) -> Api {
        let url = browser.map(FakeBrowser::base_url);
        let env: Vec<(&str, &str)> = match &url {
            Some(u) => vec![
                ("VAULT_BROWSER_URL", u.as_str()),
                ("VAULT_BROWSER_TOKEN", testkit::DEFAULT_BROWSER_TOKEN),
            ],
            // The compose file sets an empty URL when the box is off: that is no box.
            None => vec![("VAULT_BROWSER_URL", "")],
        };
        let server = TestServer::start_with_env(|_| {}, &env);
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let api = Api { server, client };
        api.secret("ai_anthropic_api_key", fake.api_key());
        api.secret(
            "ai_settings",
            &json!({
                "default_provider": "anthropic",
                "models": { "anthropic": MODEL },
                "endpoints": { "anthropic": fake.base_url() },
            })
            .to_string(),
        );
        api
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn secret(&self, name: &str, value: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/secrets/{name}")))
            .bearer_auth(self.token())
            .json(&json!({ "value": value }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn agent(&self, name: &str, wcl: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/agents/{name}")))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(wcl.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn get(&self, path: &str) -> Value {
        let res = self
            .client
            .get(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
        res.json().unwrap()
    }

    /// One turn as `agent` (the Assistant for `None`) that makes `calls`, then answers.
    fn calls(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        calls: &[(&str, Value)],
    ) -> Vec<(String, Value)> {
        let mut m = AnthropicMessage::new();
        for (i, (tool, input)) in calls.iter().enumerate() {
            m = m.tool_use(&format!("toolu_{i}"), tool, input.clone());
        }
        fake.push(m);
        fake.push(AnthropicMessage::text("Done."));
        self.turn(agent)
    }

    fn call(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        self.calls(fake, agent, &[(tool, input)])
    }

    fn turn(&self, agent: Option<&str>) -> Vec<(String, Value)> {
        let mut body = json!({ "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Go." }] }] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(self.token())
            .json(&body)
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    fn assets(&self) -> Vec<Value> {
        self.get("/api/assets")["assets"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn asset_bytes(&self, id: &str) -> Vec<u8> {
        self.client
            .get(self.server.url(&format!("/api/assets/{id}")))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .bytes()
            .unwrap()
            .to_vec()
    }

    fn apply(&self, change: &Value) -> (StatusCode, Value) {
        let res = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .bearer_auth(self.token())
            .json(change)
            .send()
            .unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }
}

fn read_events(res: Response) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    for line in BufReader::new(res).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                out.push((
                    std::mem::take(&mut name),
                    serde_json::from_str(&data).unwrap(),
                ));
                data.clear();
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("event:") {
            name = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push_str(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    out
}

fn last<'a>(events: &'a [(String, Value)], name: &str) -> &'a Value {
    &events
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
        .1
}

fn tool_names(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The `i`th tool result the model got in the second request.
fn result(fake: &FakeAnthropic, i: usize) -> Value {
    let r = fake.message_requests()[1].json();
    r["messages"].as_array().unwrap().last().unwrap()["content"][i].clone()
}

/// A tool result's text: its string content, or its first text block.
fn result_text(result: &Value) -> String {
    match &result["content"] {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks[0]["text"].as_str().unwrap().to_owned(),
        other => panic!("{other}"),
    }
}

fn result_json(fake: &FakeAnthropic) -> Value {
    serde_json::from_str(&result_text(&result(fake, 0))).unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

const BROWSER_TOOLS: [&str; 3] = ["browser_run", "browser_file", "browser_save"];

fn has_browser_tools(names: &[String]) -> bool {
    BROWSER_TOOLS.iter().all(|t| names.iter().any(|n| n == t))
}

fn has_no_browser_tool(names: &[String]) -> bool {
    !names.iter().any(|n| n.starts_with("browser_"))
}

#[test]
fn the_tools_need_a_browser_box_and_the_permission() {
    let fake = FakeAnthropic::start();
    // No box (an empty VAULT_BROWSER_URL): absent even for an agent with the permission.
    let api = Api::start(&fake, None);
    api.agent("driver", DRIVER);
    fake.push(AnthropicMessage::text("Hi."));
    api.turn(Some("driver"));
    assert!(has_no_browser_tool(&tool_names(&fake, 0)));

    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    // The Assistant ships with it off; an agent with no tools list (every tool) has none either.
    api.agent("everything", "label = \"Everything\"\n");
    api.agent("driver", DRIVER);
    api.agent(
        "narrow",
        "permissions {\n  tools = [\"get_note\"]\n  browser = true\n}\n",
    );
    for (i, agent) in [None, Some("everything"), Some("driver"), Some("narrow")]
        .into_iter()
        .enumerate()
    {
        fake.push(AnthropicMessage::text("Hi."));
        api.turn(agent);
        let names = tool_names(&fake, i);
        match agent {
            None | Some("everything") => {
                assert!(has_no_browser_tool(&names), "{agent:?}: {names:?}")
            }
            Some("driver") => {
                assert!(has_browser_tools(&names), "{names:?}");
                assert!(names.iter().any(|n| n == "get_note"));
            }
            _ => {
                // The browser tools are apart from the tools list.
                let mut names = names;
                names.sort();
                assert_eq!(
                    names,
                    ["browser_file", "browser_run", "browser_save", "get_note"]
                );
            }
        }
    }
    // A tools list cannot name them: they are not tools the list governs.
    let res = api
        .client
        .put(api.server.url("/api/agents/listed"))
        .bearer_auth(api.token())
        .header("content-type", "text/plain")
        .body("permissions {\n  tools = [\"browser_run\"]\n}\n")
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    // The agent's JSON form carries the switch.
    assert_eq!(
        api.get("/api/agents/driver")["permissions"]["browser"],
        true
    );
    assert_eq!(
        api.get("/api/agents/everything")["permissions"]["browser"],
        false
    );
}

#[test]
fn browser_run_answers_the_exit_code_and_output_and_writes_a_trail_line() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent("driver", DRIVER);
    browser.push_run(BrowserRun::exited(
        2,
        "- button \"Go\" [ref=e21]",
        "warning: slow",
    ));
    let events = api.call(
        &fake,
        Some("driver"),
        "browser_run",
        json!({ "command": "playwright-cli snapshot", "timeout": 30 }),
    );
    assert_eq!(
        browser.commands(),
        [json!({ "command": "playwright-cli snapshot", "timeout_s": 30 })]
    );
    let r = result_json(&fake);
    assert_eq!(r["exit"], 2);
    assert_eq!(r["stdout"], "- button \"Go\" [ref=e21]");
    assert_eq!(r["stderr"], "warning: slow");
    assert_eq!(r["timed_out"], false);
    assert!(r["note"].as_str().unwrap().contains("untrusted"));
    // The step keeps the command and its output for the panel and the thread (browser B7).
    let step = &last(&events, "tool_result")["summary"]["browser"];
    assert_eq!(step["kind"], "run");
    assert_eq!(step["command"], "playwright-cli snapshot");
    assert_eq!(step["exit"], 2);
    assert_eq!(step["stdout"], "- button \"Go\" [ref=e21]");
    let lines = api.server.stderr_lines();
    assert!(
        lines
            .iter()
            .any(|l| l == "browser: exit=2 ms=42 timed_out=false cmd=playwright-cli snapshot"),
        "{lines:?}"
    );
}

#[test]
fn browser_run_cuts_long_output_to_its_tail_and_reports_timeouts_and_a_busy_box() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent("driver", DRIVER);
    let long = format!("{}END", "x".repeat(30_000));
    browser.push_run(BrowserRun::exited(0, &long, ""));
    browser.push_run(BrowserRun::timed_out("partial"));
    browser.push_run(BrowserRun::busy());
    let long_cmd = format!("echo {}", "y".repeat(300));
    api.calls(
        &fake,
        Some("driver"),
        &[
            (
                "browser_run",
                json!({ "command": "cat big.txt", "timeout": null }),
            ),
            ("browser_run", json!({ "command": long_cmd, "timeout": 1 })),
            ("browser_run", json!({ "command": "true", "timeout": 999 })),
        ],
    );
    let ran = browser.commands();
    // No timeout is 60 seconds; a timeout past 300 is cut to it.
    assert_eq!(ran[0]["timeout_s"], 60);
    assert_eq!(ran[2]["timeout_s"], 300);
    let first: Value = serde_json::from_str(&result_text(&result(&fake, 0))).unwrap();
    let out = first["stdout"].as_str().unwrap();
    assert!(out.ends_with("END"), "the tail is kept");
    assert!(
        out.starts_with("[cut: the first 10003 characters were left out]"),
        "{}",
        &out[..80]
    );
    let second: Value = serde_json::from_str(&result_text(&result(&fake, 1))).unwrap();
    assert_eq!(second["timed_out"], true);
    assert_eq!(second["exit"], 137);
    let third = result(&fake, 2);
    assert_eq!(third["is_error"], true);
    assert!(result_text(&third).contains("already running 4 commands"));
    let lines = api.server.stderr_lines();
    let trail: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("browser: "))
        .collect();
    assert_eq!(trail.len(), 3, "{lines:?}");
    assert!(trail[1].starts_with("browser: exit=137 ms=1000 timed_out=true cmd=echo yyy"));
    assert_eq!(trail[1].split("cmd=").nth(1).unwrap().chars().count(), 120);
}

#[test]
fn browser_file_answers_an_image_as_an_image_part_and_text_as_text() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent("driver", DRIVER);
    let shot = png(30, 20);
    browser.file("/work/shot.png", "image/png", shot.clone());
    browser.file(
        "/work/page.yml",
        "application/yaml",
        b"- heading \"Slate\" [ref=e3]\n".to_vec(),
    );
    let long = format!("{}TAIL", "z".repeat(45_000));
    browser.file(
        "/work/big.txt",
        "text/plain; charset=utf-8",
        long.into_bytes(),
    );
    browser.file("/work/movie.webm", "video/webm", vec![1, 2, 3]);
    let events = api.calls(
        &fake,
        Some("driver"),
        &[
            ("browser_file", json!({ "path": "shot.png" })),
            ("browser_file", json!({ "path": "/work/page.yml" })),
            ("browser_file", json!({ "path": "big.txt" })),
            ("browser_file", json!({ "path": "movie.webm" })),
            ("browser_file", json!({ "path": "missing.png" })),
        ],
    );
    // The screenshot went to the model as an image part.
    let images = fake.images();
    assert_eq!(images[1].len(), 1, "{images:?}");
    assert_eq!(images[1][0].tool_call_id, "toolu_0");
    assert_eq!(images[1][0].media_type, "image/png");
    assert_eq!(images[1][0].bytes, shot);
    // The panel's step carries a thumbnail, never the image itself.
    let steps: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "tool_result")
        .map(|(_, d)| d)
        .collect();
    let shot_step = &steps[0]["summary"]["browser"];
    assert_eq!(shot_step["kind"], "image");
    assert!(
        shot_step["thumbnail"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    assert!(result_text(&result(&fake, 1)).contains("[ref=e3]"));
    assert_eq!(steps[1]["summary"]["browser"]["kind"], "text");
    let big = result_text(&result(&fake, 2));
    assert!(big.contains("[truncated"), "cut at 40000 characters");
    assert!(!big.contains("TAIL"));
    let movie = result_text(&result(&fake, 3));
    assert!(movie.contains("video/webm"));
    assert!(movie.contains("Size: 3 bytes"));
    assert_eq!(steps[3]["summary"]["browser"]["kind"], "metadata");
    let missing = result(&fake, 4);
    assert_eq!(missing["is_error"], true);
    assert!(result_text(&missing).contains("does not exist"));
}

#[test]
fn browser_file_images_count_toward_the_limit_per_question() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent("driver", DRIVER);
    browser.file("/work/shot.png", "image/png", png(8, 8));
    let calls: Vec<(&str, Value)> = (0..9)
        .map(|_| ("browser_file", json!({ "path": "shot.png" })))
        .collect();
    api.calls(&fake, Some("driver"), &calls);
    assert_eq!(fake.images()[1].len(), 8);
    let ninth = result(&fake, 8);
    assert_eq!(ninth["is_error"], true);
    assert!(result_text(&ninth).contains("limit of 8 images"));
}

#[test]
fn browser_save_with_the_switch_on_is_a_proposal_that_apply_stores() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent("driver", DRIVER);
    let bytes = b"%PDF-1.4 fake".to_vec();
    browser.file("/work/out/report.pdf", "application/pdf", bytes.clone());
    let events = api.call(
        &fake,
        Some("driver"),
        "browser_save",
        json!({ "path": "out/report.pdf", "filename": null }),
    );
    let p = last(&events, "proposal").clone();
    assert_eq!(p["op"], "download");
    assert_eq!(p["source"], "browser");
    assert_eq!(p["path"], "out/report.pdf");
    assert_eq!(p["filename"], "report.pdf");
    assert_eq!(p["size"], bytes.len());
    assert_eq!(p["agent"], "driver");
    assert!(result_text(&result(&fake, 0)).contains("awaits the user"));
    assert!(api.assets().is_empty());
    // Nothing was read yet: the size came from the folder's listing.
    assert!(browser.requests().iter().all(|r| r.route() != "/file"));

    let (status, done) = api.apply(&p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "download");
    assert_eq!(done["filename"], "report.pdf");
    let id = done["asset_id"].as_str().unwrap();
    assert_eq!(api.asset_bytes(id), bytes);

    // A proposal the Assistant made, or one naming an agent without browser automation, is
    // refused: the box is reached only for an agent that has it.
    let mut theirs = p.clone();
    theirs.as_object_mut().unwrap().remove("agent");
    let (status, body) = api.apply(&theirs);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "not_permitted");
    // A file the box does not have stores nothing.
    let mut gone = p.clone();
    gone["path"] = json!("nothing.pdf");
    let (status, _) = api.apply(&gone);
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(api.assets().len(), 1);
}

#[test]
fn browser_save_with_the_switch_off_is_stored_at_once() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent(
        "driver",
        "permissions {\n  browser = true\n\n  confirm {\n    downloads = false\n  }\n}\n",
    );
    let shot = png(12, 12);
    browser.file("/work/shot.png", "image/png", shot.clone());
    let events = api.call(
        &fake,
        Some("driver"),
        "browser_save",
        json!({ "path": "/work/shot.png", "filename": "Login page.png" }),
    );
    let applied = last(&events, "applied");
    assert_eq!(applied["op"], "download");
    assert_eq!(applied["source"], "browser");
    assert_eq!(applied["filename"], "Login page.png");
    let id = applied["asset_id"].as_str().unwrap();
    assert_eq!(api.asset_bytes(id), shot);
    let r = result_json(&fake);
    assert_eq!(r["applied"], true);
    assert_eq!(r["asset_id"], id);
}

#[test]
fn an_unattended_run_gets_the_browser_tools_but_no_save_that_would_ask() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.agent("driver", DRIVER);
    api.agent(
        "saver",
        "permissions {\n  browser = true\n\n  confirm {\n    downloads = false\n  }\n}\n",
    );
    api.agent("plain", "label = \"Plain\"\n");
    let tools = |agent: &str| -> Vec<String> {
        api.get(&format!("/api/agents/{agent}/runs"))["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_owned())
            .collect()
    };
    let driver = tools("driver");
    assert!(driver.iter().any(|t| t == "browser_run"));
    assert!(driver.iter().any(|t| t == "browser_file"));
    assert!(!driver.iter().any(|t| t == "browser_save"));
    assert!(has_browser_tools(&tools("saver")));
    assert!(has_no_browser_tool(&tools("plain")));
}

#[test]
fn an_agent_cannot_give_browser_automation_it_lacks() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    // The Assistant has every tool but not browser automation.
    let events = api.call(
        &fake,
        None,
        "propose_agent_change",
        json!({ "name": "helper", "wcl": DRIVER }),
    );
    let r = last(&events, "tool_result");
    assert_eq!(r["is_error"], true, "{events:?}");
    assert!(
        result_text(&result(&fake, 0)).contains("Browser automation"),
        "{}",
        result_text(&result(&fake, 0))
    );
    // An agent that has it may give it.
    api.agent("driver", DRIVER);
    let events = api.call(
        &fake,
        Some("driver"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": DRIVER }),
    );
    assert_eq!(last(&events, "agent_proposal")["name"], "helper");
}
