//! S1 with the fake provider and the fake browser box: agents using skills (skills S6, S7). The
//! system text ends with an "Available skills" section listing only the valid skills the agent
//! is offered, by name and description; a thread keeps that list from its first turn until
//! `/update-skills` (`update_skills: true`) takes a fresh one. `use_skill` answers the body and
//! the file list; `read_skill_file` answers text, an image part and metadata, and refuses paths
//! outside the skill; `permissions.skills` limits the list and the tools; unattended runs are
//! offered the read tools; `install_skill_in_browser` copies the files into the box only with
//! the browser permission; `allowed-tools` grants nothing; an agent cannot offer a skill it is
//! not offered itself; each call is an `ai.skill` activity entry.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, FakeBrowser, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

const PDF: &str = "---\nname: pdf-forms\ndescription: Fills PDF forms. Use when a PDF form arrives.\nallowed-tools: browser_run Bash(python3:*)\n---\n\n# PDF forms\n\nRun scripts/fill.py with the form. See references/fields.md.\n";

const BRAND: &str = "---\nname: brand\ndescription: The house style for letters.\n---\n\nUse the logo in assets/logo.png.\n";

const MEMO: &str =
    "label = \"Memo\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic, browser: Option<&FakeBrowser>) -> Api {
        let url = browser.map(FakeBrowser::base_url);
        let env: Vec<(&str, &str)> = match &url {
            Some(u) => vec![
                ("VAULT_BROWSER_URL", u.as_str()),
                ("VAULT_BROWSER_TOKEN", testkit::DEFAULT_BROWSER_TOKEN),
            ],
            None => vec![("VAULT_BROWSER_URL", "")],
        };
        let server = TestServer::start_with_env(
            |vault| {
                std::fs::create_dir_all(vault.join("types")).unwrap();
                std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
            },
            &env,
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.send(
                "PUT",
                &format!("/api/secrets/{name}"),
                json!({ "value": value }),
            );
        }
        api
    }

    fn token(&self) -> &str {
        self.server.admin_token()
    }

    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let res = self
            .client
            .request(method.parse().unwrap(), self.server.url(path))
            .bearer_auth(self.token())
            .json(&body)
            .send()
            .unwrap();
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        let res = self
            .client
            .get(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        assert!(
            res.status().is_success(),
            "GET {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap()
    }

    fn put_raw(&self, path: &str, content_type: &str, body: Vec<u8>) -> Response {
        self.client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", content_type)
            .body(body)
            .send()
            .unwrap()
    }

    fn agent(&self, name: &str, wcl: &str) {
        let res = self.put_raw(&format!("/api/agents/{name}"), "text/plain", wcl.into());
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn skill(&self, name: &str, md: &str) {
        let res = self.put_raw(
            &format!("/api/skills/{name}"),
            "text/markdown; charset=utf-8",
            md.into(),
        );
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn skill_file(&self, name: &str, path: &str, bytes: Vec<u8>) {
        let res = self.put_raw(
            &format!("/api/skills/{name}/files/{path}"),
            "application/octet-stream",
            bytes,
        );
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    /// The two skills, with their files, and a skill with errors.
    fn skills(&self) {
        self.skill("pdf-forms", PDF);
        self.skill_file(
            "pdf-forms",
            "scripts/fill.py",
            b"#!/usr/bin/env python3\nprint('filled')\n".to_vec(),
        );
        self.skill_file(
            "pdf-forms",
            "references/fields.md",
            b"# Fields\n\nName, date.\n".to_vec(),
        );
        self.skill_file("pdf-forms", "assets/blank.bin", vec![0, 1, 2, 3, 255]);
        self.skill("brand", BRAND);
        self.skill_file("brand", "assets/logo.png", png(8, 8));
        // A folder whose SKILL.md names another skill is listed with an error, offered to none.
        let dir = self.server.vault_path().join("skills/broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: other\ndescription: Broken.\n---\nBody\n",
        )
        .unwrap();
    }

    fn chat(&self, body: Value) -> Vec<(String, Value)> {
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

    fn turn(&self, agent: Option<&str>) -> Vec<(String, Value)> {
        let mut body = json!({ "messages": [user("Go.")] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        self.chat(body)
    }

    /// One turn as `agent` that makes `calls`, then answers.
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

    fn thread(&self, agent: &str) -> String {
        let v = self.send("POST", &format!("/api/agents/{agent}/threads"), json!({}));
        v["id"].as_str().unwrap().to_owned()
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
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

fn find<'a>(events: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    events.iter().find(|(n, _)| n == name).map(|(_, d)| d)
}

fn system(fake: &FakeAnthropic, request: usize) -> String {
    fake.message_requests()[request].json()["system"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn tool_names(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The `i`th tool result of the last message of request `request`.
fn result(fake: &FakeAnthropic, request: usize, i: usize) -> Value {
    let r = fake.message_requests()[request].json();
    r["messages"].as_array().unwrap().last().unwrap()["content"][i].clone()
}

fn result_text(result: &Value) -> String {
    match &result["content"] {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks[0]["text"].as_str().unwrap().to_owned(),
        other => panic!("{other}"),
    }
}

fn result_json(fake: &FakeAnthropic, request: usize, i: usize) -> Value {
    serde_json::from_str(&result_text(&result(fake, request, i))).unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The skill lines of a system text's Available skills section.
fn listed(system: &str) -> Vec<String> {
    let Some((_, section)) = system.split_once("## Available skills") else {
        return Vec::new();
    };
    section
        .lines()
        .filter(|l| l.starts_with("- `"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_system_text_ends_with_the_offered_valid_skills() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.skills();
    api.agent(
        "helper",
        "label = \"Helper\"\nsystem_prompt = \"You help.\"\n",
    );
    api.agent("filer", "permissions {\n  skills = [\"pdf-forms\"]\n}\n");
    api.agent("plain", "permissions {\n  skills = []\n}\n");
    api.agent("narrow", "permissions {\n  tools = [\"get_note\"]\n}\n");
    let note = api.send(
        "POST",
        "/api/notes?type=memo",
        json!({ "note": { "fields": { "title": "Open one" } } }),
    );
    api.send(
        "POST",
        "/api/agents/helper/memories",
        json!({ "title": "Likes tea", "text": "Tea, not coffee." }),
    );

    fake.push(AnthropicMessage::text("Hi."));
    api.chat(json!({ "agent": "helper", "note_id": note["id"], "messages": [user("Hi")] }));
    let s = system(&fake, 0);
    // Only the valid skills, each by name and description, sorted by name.
    assert_eq!(
        listed(&s),
        [
            "- `brand`: The house style for letters.",
            "- `pdf-forms`: Fills PDF forms. Use when a PDF form arrives.",
        ]
    );
    assert!(!s.contains("broken"), "{s}");
    assert!(s.contains("call use_skill"), "{s}");
    // No browser: the scripts can be read but not run.
    assert!(s.contains("can be read here but not run"), "{s}");
    assert!(!s.contains("install_skill_in_browser"), "{s}");
    // Last, after the prompt, the memories and the open note.
    let skills_at = s.find("## Available skills").unwrap();
    assert!(s.find("You help.").unwrap() < skills_at);
    assert!(s.find("Likes tea").unwrap() < skills_at);
    assert!(s.find("## The open note").unwrap() < skills_at);
    assert!(
        s.trim_end()
            .ends_with("Fills PDF forms. Use when a PDF form arrives."),
        "{s}"
    );
    let names = tool_names(&fake, 0);
    assert!(names.iter().any(|n| n == "use_skill"));
    assert!(names.iter().any(|n| n == "read_skill_file"));
    // allowed-tools grants nothing: no browser tool for an agent without the permission.
    assert!(
        !names.iter().any(|n| n.starts_with("browser_")),
        "{names:?}"
    );

    // permissions.skills limits the list.
    fake.push(AnthropicMessage::text("Hi."));
    api.turn(Some("filer"));
    assert_eq!(
        listed(&system(&fake, 1)),
        ["- `pdf-forms`: Fills PDF forms. Use when a PDF form arrives."]
    );
    // An empty list: no section and no skill tools; a tools list without use_skill likewise.
    for (i, agent) in ["plain", "narrow"].into_iter().enumerate() {
        fake.push(AnthropicMessage::text("Hi."));
        api.turn(Some(agent));
        let s = system(&fake, 2 + i);
        assert!(!s.contains("## Available skills"), "{agent}: {s}");
        let names = tool_names(&fake, 2 + i);
        // The tools that use skills; managing them (skills S9) is apart.
        assert!(
            !names.iter().any(
                |n| ["use_skill", "read_skill_file", "install_skill_in_browser"]
                    .contains(&n.as_str())
            ),
            "{agent}: {names:?}"
        );
    }
    // The agent's JSON form carries the list; absent is null.
    assert_eq!(
        api.get("/api/agents/filer")["permissions"]["skills"],
        json!(["pdf-forms"])
    );
    assert_eq!(
        api.get("/api/agents/helper")["permissions"]["skills"],
        Value::Null
    );
}

#[test]
fn use_skill_answers_the_body_and_files_and_keeps_to_the_offered_skills() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.skills();
    api.agent("filer", "permissions {\n  skills = [\"pdf-forms\"]\n}\n");
    let events = api.calls(
        &fake,
        Some("filer"),
        &[
            ("use_skill", json!({ "name": "pdf-forms" })),
            ("use_skill", json!({ "name": "brand" })),
            ("use_skill", json!({ "name": "broken" })),
            ("use_skill", json!({ "name": "nothing" })),
        ],
    );
    let r = result_json(&fake, 1, 0);
    assert_eq!(r["name"], "pdf-forms");
    assert!(
        r["instructions"]
            .as_str()
            .unwrap()
            .contains("Run scripts/fill.py")
    );
    let files: Vec<&str> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        files,
        [
            "SKILL.md",
            "assets/blank.bin",
            "references/fields.md",
            "scripts/fill.py"
        ]
    );
    assert!(r["files"][3]["size"].as_u64().unwrap() > 0);
    // allowed-tools is shown, and grants nothing.
    assert_eq!(
        r["allowed_tools"],
        json!(["browser_run", "Bash(python3:*)"])
    );
    assert!(
        r["allowed_tools_note"]
            .as_str()
            .unwrap()
            .contains("grants nothing")
    );
    assert!(r["note"].as_str().unwrap().contains("not run here"));
    // Not offered, invalid and missing skills are errors.
    let r = result(&fake, 1, 1);
    assert_eq!(r["is_error"], true);
    assert!(
        result_text(&r).contains("not offered the skill \\\"brand\\\""),
        "{r}"
    );
    assert_eq!(result(&fake, 1, 2)["is_error"], true);
    assert_eq!(result(&fake, 1, 3)["is_error"], true);
    // The step's summary names the skill.
    let step = events
        .iter()
        .find(|(n, d)| n == "tool_result" && d["id"] == "toolu_0")
        .unwrap();
    assert_eq!(step.1["summary"]["skill"]["name"], "pdf-forms");
    assert_eq!(step.1["summary"]["skill"]["kind"], "use");

    // Each call is an ai.skill entry in the activity log.
    let log = api.get("/api/activity?kind=ai.skill&limit=50");
    let items = log["items"].as_array().unwrap();
    assert_eq!(items.len(), 4, "{log}");
    assert!(
        items
            .iter()
            .any(|e| e["detail"]["skill"] == "pdf-forms" && e["detail"]["is_error"] == false)
    );
}

#[test]
fn read_skill_file_answers_text_images_and_metadata_inside_the_skill_only() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.skills();
    api.agent("helper", "label = \"Helper\"\n");
    api.calls(
        &fake,
        Some("helper"),
        &[
            (
                "read_skill_file",
                json!({ "name": "pdf-forms", "path": "references/fields.md" }),
            ),
            (
                "read_skill_file",
                json!({ "name": "brand", "path": "assets/logo.png" }),
            ),
            (
                "read_skill_file",
                json!({ "name": "pdf-forms", "path": "assets/blank.bin" }),
            ),
            (
                "read_skill_file",
                json!({ "name": "pdf-forms", "path": "../brand/SKILL.md" }),
            ),
            (
                "read_skill_file",
                json!({ "name": "pdf-forms", "path": "/etc/passwd" }),
            ),
            (
                "read_skill_file",
                json!({ "name": "pdf-forms", "path": "scripts/./fill.py" }),
            ),
        ],
    );
    let text = result_json(&fake, 1, 0);
    assert_eq!(text["text"], "# Fields\n\nName, date.\n");
    assert_eq!(text["media_type"], "text/markdown");
    // The image goes as an image part.
    let image = result_json(&fake, 1, 1);
    assert_eq!(image["note"], "The image follows.");
    let images = fake.images();
    assert_eq!(images[1].len(), 1, "{:?}", images[1].len());
    assert_eq!(images[1][0].tool_call_id, "toolu_1");
    assert_eq!(images[1][0].media_type, "image/png");
    assert_eq!(images[1][0].bytes, png(8, 8));
    // Anything else is metadata.
    let meta = result_json(&fake, 1, 2);
    assert_eq!(meta["size"], 5);
    assert!(meta.get("text").is_none(), "{meta}");
    // Paths out of the skill are refused.
    for i in [3, 4] {
        let r = result(&fake, 1, i);
        assert_eq!(r["is_error"], true, "{r}");
        assert!(result_text(&r).contains("inside the skill"), "{r}");
    }
    // A path with `.` reads as its normal form.
    assert!(
        result_json(&fake, 1, 5)["text"]
            .as_str()
            .unwrap()
            .contains("filled")
    );
}

#[test]
fn a_skill_image_is_read_from_the_skill_again_in_a_later_question() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.skills();
    api.agent("helper", "label = \"Helper\"\n");
    let events = api.calls(
        &fake,
        Some("helper"),
        &[(
            "read_skill_file",
            json!({ "name": "brand", "path": "assets/logo.png" }),
        )],
    );
    let added = find(&events, "done").unwrap()["messages"]
        .as_array()
        .unwrap()
        .clone();
    let mut messages = vec![user("Go.")];
    messages.extend(added);
    messages.push(user("And again?"));
    fake.push(AnthropicMessage::text("Same logo."));
    api.chat(json!({ "agent": "helper", "messages": messages }));
    let images = fake.images();
    assert_eq!(images[2].len(), 1);
    assert_eq!(images[2][0].bytes, png(8, 8));
}

#[test]
fn a_thread_keeps_its_skills_until_update_skills() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.skill("brand", BRAND);
    api.agent("helper", "label = \"Helper\"\n");
    let thread = api.thread("helper");
    let body =
        |text: &str| json!({ "agent": "helper", "thread": thread, "messages": [user(text)] });

    fake.push(AnthropicMessage::text("One."));
    api.chat(body("First"));
    assert_eq!(listed(&system(&fake, 0)).len(), 1);
    let first = system(&fake, 0);

    // A skill added mid-thread does not appear: the system text stays the same.
    api.skill("pdf-forms", PDF);
    fake.push(AnthropicMessage::text("Two."));
    api.chat(body("Second"));
    assert_eq!(system(&fake, 1), first);
    // A thread with no thread note is fresh every turn.
    fake.push(AnthropicMessage::text("Three."));
    api.turn(Some("helper"));
    assert_eq!(listed(&system(&fake, 2)).len(), 2);

    // /update-skills: no message of the user's, a fresh snapshot, the fixed note to the model.
    fake.push(AnthropicMessage::text("Noted."));
    let events = api.chat(json!({
        "agent": "helper", "thread": thread, "messages": [], "update_skills": true,
    }));
    let updated = find(&events, "skills_updated").expect("a skills_updated event");
    assert_eq!(updated["count"], 2);
    assert_eq!(updated["skills"], json!(["brand", "pdf-forms"]));
    assert_eq!(updated["message"]["content"][0]["type"], "notice");
    assert_eq!(
        updated["message"]["content"][0]["text"],
        "Skills updated (2)"
    );
    let s = system(&fake, 3);
    assert_eq!(listed(&s).len(), 2, "{s}");
    let sent = fake.message_requests()[3].json();
    let last = sent["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["role"], "user");
    let text = last["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(
        text,
        "The available skills changed; the list at the end of your instructions is current."
    );

    // The thread keeps the new snapshot and the notice; its title is not the note to the model.
    let t = api.get(&format!("/api/agents/helper/threads/{thread}"));
    assert_eq!(t["skills"].as_array().unwrap().len(), 2, "{t}");
    let notice = t["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["content"][0]["kind"] == "skills_updated");
    assert!(notice.is_some(), "{t}");
    assert_eq!(t["title"], "First");
    fake.push(AnthropicMessage::text("Four."));
    api.chat(body("Again"));
    assert_eq!(system(&fake, 4), s);

    // An empty message list without update_skills is still refused.
    let res = api
        .client
        .post(api.server.url("/api/ai/chat"))
        .bearer_auth(api.token())
        .json(&json!({ "agent": "helper", "thread": thread, "messages": [] }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn unattended_runs_are_offered_the_read_tools() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.agent(
        "nightly",
        "trigger nightly {\n  schedule = \"0 2 * * *\"\n  prompt = \"Go.\"\n}\n",
    );
    let runs = api.get("/api/agents/nightly/runs");
    let tools: Vec<&str> = runs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(tools.contains(&"use_skill"), "{tools:?}");
    assert!(tools.contains(&"read_skill_file"), "{tools:?}");
    assert!(!tools.contains(&"propose_edit"), "{tools:?}");
}

#[test]
fn install_skill_in_browser_copies_the_files_with_the_browser_permission_only() {
    let fake = FakeAnthropic::start();
    let browser = FakeBrowser::start();
    let api = Api::start(&fake, Some(&browser));
    api.skills();
    api.agent("helper", "label = \"Helper\"\n");
    api.agent("driver", "permissions {\n  browser = true\n}\n");
    api.agent(
        "boxed",
        "permissions {\n  browser = true\n  skills = [\"brand\"]\n}\n",
    );

    // Without the permission, there is no install tool, and the prompt says scripts do not run.
    fake.push(AnthropicMessage::text("Hi."));
    api.turn(Some("helper"));
    assert!(
        !tool_names(&fake, 0)
            .iter()
            .any(|n| n == "install_skill_in_browser")
    );

    let events = api.calls(
        &fake,
        Some("driver"),
        &[("install_skill_in_browser", json!({ "name": "pdf-forms" }))],
    );
    assert!(
        tool_names(&fake, 1)
            .iter()
            .any(|n| n == "install_skill_in_browser")
    );
    let s = system(&fake, 1);
    assert!(s.contains("install_skill_in_browser"), "{s}");
    let r = result_json(&fake, 2, 0);
    assert_eq!(r["installed"], "/work/skills/pdf-forms");
    assert_eq!(r["files"].as_array().unwrap().len(), 4);
    let step = events
        .iter()
        .find(|(n, d)| n == "tool_result" && d["id"] == "toolu_0")
        .unwrap();
    assert_eq!(step.1["is_error"], false, "{}", step.1);
    assert_eq!(step.1["summary"]["skill"]["kind"], "install");

    // The commands rebuild every file under /work/skills/pdf-forms.
    let commands: Vec<String> = browser
        .commands()
        .iter()
        .map(|c| c["command"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        commands[0],
        "rm -rf '/work/skills/pdf-forms' && mkdir -p '/work/skills/pdf-forms'"
    );
    let all = commands.join(" && ");
    let decoded = |path: &str| -> Vec<u8> {
        let target = format!("'/work/skills/pdf-forms/{path}'");
        let mut out = Vec::new();
        for part in all
            .split(" && ")
            .filter(|p| p.ends_with(&target) && p.contains("base64 -d"))
        {
            let b64 = part.split('\'').nth(1).unwrap();
            out.extend(STANDARD.decode(b64).unwrap());
        }
        out
    };
    assert_eq!(decoded("assets/blank.bin"), vec![0, 1, 2, 3, 255]);
    assert_eq!(decoded("SKILL.md"), PDF.as_bytes());
    assert!(all.contains("chmod +x '/work/skills/pdf-forms/scripts/fill.py'"));

    // A skill the agent is not offered is refused, with nothing run.
    let before = browser.commands().len();
    api.calls(
        &fake,
        Some("boxed"),
        &[("install_skill_in_browser", json!({ "name": "pdf-forms" }))],
    );
    let r = result(&fake, 4, 0);
    assert_eq!(r["is_error"], true);
    assert_eq!(browser.commands().len(), before);

    let log = api.get("/api/activity?kind=ai.skill&limit=50");
    assert!(log["items"].as_array().unwrap().iter().any(|e| e["detail"]["tool"]
        == "install_skill_in_browser"
        && e["detail"]["files"] == 4));
}

#[test]
fn an_agent_cannot_offer_a_skill_it_is_not_offered() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake, None);
    api.skills();
    api.agent(
        "manager",
        "permissions {\n  skills = [\"brand\"]\n  confirm {\n    agents = false\n  }\n}\n",
    );
    api.calls(
        &fake,
        Some("manager"),
        &[
            (
                "propose_agent_change",
                json!({ "name": "clerk", "wcl": "permissions {\n  skills = [\"pdf-forms\"]\n}\n" }),
            ),
            (
                "propose_agent_change",
                json!({ "name": "clerk2", "wcl": "label = \"Clerk\"\n" }),
            ),
            (
                "propose_agent_change",
                json!({ "name": "clerk3", "wcl": "permissions {\n  skills = [\"brand\"]\n}\n" }),
            ),
        ],
    );
    let r = result(&fake, 1, 0);
    assert_eq!(r["is_error"], true);
    assert!(result_text(&r).contains("pdf-forms"), "{r}");
    let r = result(&fake, 1, 1);
    assert_eq!(r["is_error"], true);
    assert!(result_text(&r).contains("every skill"), "{r}");
    assert_eq!(
        result(&fake, 1, 2)["is_error"],
        false,
        "{}",
        result(&fake, 1, 2)
    );
}
