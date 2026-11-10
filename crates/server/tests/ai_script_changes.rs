//! S1 with the fake Anthropic provider: agents propose saved scripts (TV-70, S2 of TV-32).
//!
//! `propose_script_change` and `propose_script_delete` always wait for the user's Apply, with
//! every confirmation switch off too. A change must compile in every way the script runs; a delete
//! is refused while something runs the script, and a change is refused while a task with folders
//! runs it. The proposal carries the "Runs as" list, each
//! handler, task and tool with its grants, and a warning for a run that reaches any host, which
//! the tool proposal carries too. Apply needs admin, or the copy waiting in the thread of a call
//! from a note; a script changed since gives 409 stale; a discard writes nothing.
//!
//! A change to a script that a device file runs shows where and what it can do on the phone, and
//! one that adds a call to `device::sms_send` is refused at propose and at Apply (TV-98).

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, PASSWORD, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

#[test]
fn file_only_tool_bindings_with_grants_wait_for_apply() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok("PUT", "/api/integrations/ai/settings", json!({"confirm": all_off()}));
    api.ok("PUT", "/api/secrets/user_key", json!({"value": "USER-SECRET-42"}));
    let code = "use secrets\nfn run(params: Value) -> Result[string, string] {\n secrets::get(\"user_key\")\n}\n";
    let events = api.one(&fake, "propose_script_change", json!({"name": "leak", "source": code}));
    let p = find(&events, "script_proposal").unwrap();
    assert_eq!(p["runs_as"], json!([]));
    assert_eq!(api.apply_as(&api.token(), p).0, StatusCode::OK);
    let tool = "description = \"Leak\"\nscript = \"leak\"\nfunction = \"run\"\nsecrets = [\"user_key\"]\nallow_domains = [\"*\"]\n";
    let events = api.one(&fake, "propose_tool_change", json!({"name": "leak", "wcl": tool, "script_source": null}));
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert!(find(&events, "tool_proposal").is_some(), "{events:?}");
    let p = find(&events, "tool_proposal").unwrap();
    assert_eq!(api.apply_as(&api.token(), p).0, StatusCode::OK);
    api.one(&fake, "leak", json!({}));
    let (err, result) = last_result(&fake);
    assert!(!err && result.to_string().contains("USER-SECRET-42"), "{result}");
}

#[test]
fn existing_tool_rebindings_wait_for_apply() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok("PUT", "/api/scripts/stamp", json!({"source": STAMP}));
    api.ok("PUT", "/api/scripts/copy", json!({"source": STAMP}));
    api.ok("PUT", "/api/tools/echo", json!({"wcl": ECHO}));
    api.ok("PUT", "/api/integrations/ai/settings", json!({"confirm": all_off()}));
    for wcl in [ECHO.replace("stamp", "copy"), ECHO.replace("call", "run"), ECHO.replace("echo.example.com", "other.example.com")] {
        let events = api.one(&fake, "propose_tool_change", json!({"name": "echo", "wcl": wcl, "script_source": null}));
        assert!(find(&events, "applied").is_none(), "{events:?}");
        assert!(find(&events, "tool_proposal").is_some(), "{events:?}");
    }
}

#[test]
fn unchanged_google_handlers_allow_description_edits() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let edited = writer::DEFAULT_EMAIL_TYPE.replace("One email message, pulled from Gmail.", "Email description edited by the agent.");
    assert_ne!(edited, writer::DEFAULT_EMAIL_TYPE);
    let events = api.one(&fake, "propose_type_change", json!({"name": "email", "wcl": edited}));
    let p = find(&events, "type_proposal").unwrap_or_else(|| panic!("{events:?}"));
    assert_eq!(api.apply_as(&api.token(), p).0, StatusCode::OK);
}

#[test]
fn every_privileged_default_run_is_guarded_through_every_agent_route() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok(
        "PUT",
        "/api/integrations/ai/settings",
        json!({"confirm": all_off()}),
    );
    // A user-owned copy bound by a shipped handler must be guarded by grants, not its name.
    let mail = api.script("google_mail").unwrap();
    api.ok("PUT", "/api/scripts/mail_copy", json!({"source": mail}));
    api.put_text(
        "/api/types/email",
        &writer::DEFAULT_EMAIL_TYPE.replace("google_mail", "mail_copy"),
    );
    api.put_text(
        "/api/skills/probe",
        "---\nname: probe\ndescription: Test\n---\n",
    );
    let mut guarded = Vec::new();
    // Read the actual installed runs, not SHIPPED_FILES: a new handler/task can name a script
    // absent from that list. Runs include every type handler, task and device binding.
    for name in api.get("/api/scripts")["scripts"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        api.one(&fake, "get_script", json!({"name": name}));
        let (_, got) = last_result(&fake);
        if got["runs_as"].as_array().unwrap().iter().any(|r| {
            ["handler", "task", "device"].contains(&r["kind"].as_str().unwrap())
                && (!r["secrets"].as_array().unwrap().is_empty()
                    || !r["allow_domains"].as_array().unwrap().is_empty())
        }) {
            guarded.push(name.to_owned());
        }
    }
    assert!(guarded.iter().any(|n| n == "mail_copy"));
    assert!(guarded.len() >= 5, "{guarded:?}");
    for name in guarded {
        let source = format!("{}\n// agent edit\n", api.script(&name).unwrap());
        let tool = format!("description = \"Test\"\nscript = \"{name}\"\nfunction = \"run\"\n");
        let task = format!("schedule = \"0 0 1 1 *\"\nscript = \"{name}\"\n");
        let device = PHONE_FILE.replace("phone_test", &name);
        for (method, event, input) in [
            (
                "propose_script_change",
                "script_proposal",
                json!({"name": name, "source": source}),
            ),
            (
                "propose_tool_change",
                "tool_proposal",
                json!({"name": "attempt", "wcl": tool, "script_source": source}),
            ),
            (
                "propose_task_change",
                "task_proposal",
                json!({"name": "attempt", "wcl": task, "script_source": source}),
            ),
            (
                "propose_device_file_change",
                "device_file_proposal",
                json!({"platform": "android", "wcl": device, "scripts": [{"name": name, "source": source}]}),
            ),
            (
                "propose_skill_file_change",
                "skill_proposal",
                json!({"name": "probe", "path": format!("scripts/{name}.ws"), "text": source, "base64": null}),
            ),
            (
                "propose_type_change",
                "type_proposal",
                json!({"name": "attempt", "wcl": ITEM.replace("stamp", &name)}),
            ),
        ] {
            let events = api.one(&fake, method, input);
            assert!(
                find(&events, "applied").is_none(),
                "{name} {method}: {events:?}"
            );
            if matches!(method, "propose_skill_file_change" | "propose_type_change") {
                assert!(find(&events, event).is_some() || last_result(&fake).0, "{name} {method}: {events:?}");
            } else {
                assert!(find(&events, event).is_none() && last_result(&fake).0, "{name} {method}: {events:?}");
            }
        }
        // File-only tool binding has no source edit to trigger shared_paths_refused.
        // It must still be held because it adds grants to previously approved code.
        let binding = format!("{tool}secrets = [\"user_key\"]\nallow_domains = [\"*\"]\n");
        let events = api.one(&fake, "propose_tool_change", json!({"name": "binding", "wcl": binding, "script_source": null}));
        assert!(find(&events, "applied").is_none(), "{name}: {events:?}");
        assert!(find(&events, "tool_proposal").is_some(), "{name}: {events:?}");
    }
}

#[test]
fn handler_binding_cannot_grant_google_secrets_or_bypass_apply() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok(
        "PUT",
        "/api/integrations/ai/settings",
        json!({"confirm": {"types": false}}),
    );
    api.ok("PUT", "/api/scripts/stamp", json!({"source": STAMP}));
    let with_secret = ITEM.replace("  function      = \"after_update\"", "  function = \"after_update\"\n  secrets = [\"google_refresh_token\"]\n  allow_domains = [\"*\"]");
    let events = api.one(
        &fake,
        "propose_type_change",
        json!({"name": "item", "wcl": with_secret}),
    );
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert!(find(&events, "type_proposal").is_none(), "{events:?}");
    let events = api.one(
        &fake,
        "propose_type_change",
        json!({"name": "item", "wcl": ITEM}),
    );
    assert!(find(&events, "type_proposal").is_some(), "{events:?}");
    assert!(find(&events, "applied").is_none(), "{events:?}");
    let mut p = find(&events, "type_proposal").unwrap().clone();
    p["after"] = json!(with_secret);
    assert_eq!(api.apply_as(&api.token(), &p).0, StatusCode::FORBIDDEN);
    // An existing shipped binding cannot be switched to agent code with Google grants.
    let rebound = writer::DEFAULT_EMAIL_TYPE.replace("google_mail", "stamp");
    let events = api.one(
        &fake,
        "propose_type_change",
        json!({"name": "email", "wcl": rebound}),
    );
    assert!(find(&events, "type_proposal").is_none(), "{events:?}");
    assert!(find(&events, "applied").is_none(), "{events:?}");
    assert!(last_result(&fake).0);
}

#[test]
fn a_tools_own_hosts_allow_held_source_edits_but_not_forged_script_names() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok("PUT", "/api/scripts/stamp", json!({"source": STAMP}));
    api.ok("PUT", "/api/tools/echo", json!({"wcl": ECHO}));
    api.ok(
        "PUT",
        "/api/integrations/ai/settings",
        json!({"confirm": all_off()}),
    );
    let changed = format!("{STAMP}\n// edit\n");
    let events = api.one(
        &fake,
        "propose_tool_change",
        json!({"name": "echo", "wcl": ECHO, "script_source": changed}),
    );
    assert!(find(&events, "applied").is_none());
    let p = find(&events, "tool_proposal").unwrap().clone();
    for script in [Value::Null, json!("google_mail")] {
        let mut forged = p.clone();
        forged["script"] = script;
        assert_eq!(api.apply_as(&api.token(), &forged).0, StatusCode::FORBIDDEN);
    }
    assert_eq!(api.apply_as(&api.token(), &p).0, StatusCode::OK);
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
}

#[test]
fn privileged_script_sources_are_read_only_for_agents() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // Discover the shipped scripts, including every secret-bearing handler and task.
    for file in writer::SHIPPED_FILES {
        let Some(name) = file
            .path
            .strip_prefix("scripts/")
            .and_then(|p| p.strip_suffix(".ws"))
        else {
            continue;
        };
        let before = api.script(name).unwrap();
        let events = api.one(
            &fake,
            "propose_script_change",
            json!({
                "name": name, "source": format!("{before}\n// agent edit\n")
            }),
        );
        assert!(
            find(&events, "script_proposal").is_none(),
            "{name}: {events:?}"
        );
        assert!(last_result(&fake).0, "{name} must be refused");
        assert_eq!(api.script(name).as_deref(), Some(before.as_str()));
        let wcl = format!("description = \"Attempt\"\nscript = \"{name}\"\nfunction = \"run\"\n");
        let events = api.one(
            &fake,
            "propose_tool_change",
            json!({
                "name": "attempt", "wcl": wcl, "script_source": format!("{before}\n// agent edit\n")
            }),
        );
        assert!(find(&events, "tool_proposal").is_none(), "{name}");
        assert!(
            last_result(&fake).1.to_string().contains("read-only"),
            "{name}"
        );
        api.one(&fake, "propose_script_delete", json!({"name": name}));
        assert!(
            last_result(&fake).1.to_string().contains("read-only"),
            "{name}"
        );
    }
}

#[test]
fn tool_source_waits_even_with_tools_confirmation_off() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.put_text("/api/agents/keeper", KEEPER);
    let wcl = "description = \"Safe\"\nscript = \"safe\"\nfunction = \"run\"\n";
    fake.push(AnthropicMessage::new().tool_use("source", "propose_tool_change",
        json!({"name": "safe", "wcl": wcl, "script_source": "fn run(params: Value) -> string { \"safe\" }\n"})));
    fake.push(AnthropicMessage::text("Waiting."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
    api.advance("PT1S");
    assert_eq!(api.threads("keeper")[0]["pending"], 1);
    assert!(api.script("safe").is_none());
}

#[test]
fn secret_and_host_runs_refuse_script_and_tool_sources_including_stale_apply() {
    for grant in [
        "secrets = [\"api_key\"]",
        "allow_domains = [\"example.com\"]",
    ] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.ok("PUT", "/api/scripts/stamp", json!({"source": STAMP}));
        let wcl = "description = \"Echo\"\nscript = \"stamp\"\nfunction = \"call\"\n";
        let changed = format!("{STAMP}\n// edit\n");
        let mut pending = Vec::new();
        for method in ["propose_script_change", "propose_tool_change"] {
            let events = api.one(
                &fake,
                method,
                if method == "propose_script_change" {
                    json!({"name": "stamp", "source": changed})
                } else {
                    json!({"name": "echo", "wcl": wcl, "script_source": changed})
                },
            );
            let event = if method == "propose_script_change" {
                "script_proposal"
            } else {
                "tool_proposal"
            };
            pending.push(
                find(&events, event)
                    .unwrap_or_else(|| panic!("{method}: {events:?}"))
                    .clone(),
            );
        }
        api.put_text(
            "/api/tasks/guarded",
            &format!("schedule = \"0 0 1 1 *\"\nscript = \"stamp\"\npaused = true\n{grant}\n"),
        );
        for p in pending {
            assert_eq!(api.apply_as(&api.token(), &p).0, StatusCode::FORBIDDEN);
        }
        for method in ["propose_script_change", "propose_tool_change"] {
            api.one(
                &fake,
                method,
                if method == "propose_script_change" {
                    json!({"name": "stamp", "source": changed})
                } else {
                    json!({"name": "echo", "wcl": wcl, "script_source": changed})
                },
            );
            assert!(last_result(&fake).0);
        }
        assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
    }
}

const PHONE_FILE: &str =
    "handler \"sms_received\" {\n script = \"phone_test\"\n function = \"run\"\n}\n";

#[test]
fn device_file_bundle_reaches_a_synced_phone_and_runs_its_handler() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.put_text("/api/device-files/android", "");
    let paired: Value = api
        .client
        .post(api.server.url("/api/devices/pair"))
        .json(&json!({"password": PASSWORD, "name": "Bundle phone", "platform": "android"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let source = "use device\nfn run(event: Value) -> Result[(), string] {\n device::show_notification(\"Bundle\", \"Applied\", \"\")?\n Ok(())\n}\n";
    let events = api.one(&fake, "propose_device_file_change", json!({"platform": "android", "wcl": PHONE_FILE.replace("sms_received", "call_ended"), "scripts": [{"name": "phone_test", "source": source}]}));
    let p = find(&events, "device_file_proposal").unwrap_or_else(|| panic!("{events:?}"));
    let (status, body) = api.apply_as(&api.token(), p);
    assert_eq!(status, StatusCode::OK, "{body}");
    let dir = tempfile::TempDir::new().unwrap();
    let local = embedded::Embedded::open(embedded::Config {
        vault: dir.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    let remote = sync::Remote {
        url: api.server.url(""),
        token: paired["token"].as_str().unwrap().into(),
        device_id: paired["device_id"].as_str().unwrap().into(),
        device_name: "Bundle phone".into(),
    };
    local
        .sync_engine(remote.clone(), sync::AssetPolicy::android())
        .unwrap()
        .sync()
        .unwrap();
    assert_eq!(local.writer().read_script("phone_test").unwrap(), source);
    let phone = std::sync::Arc::new(super::device_runtime::Phone::default());
    let runtime = local
        .device(embedded::device::DeviceConfig {
            id: remote.device_id,
            platform: "android".into(),
            data: phone.clone(),
        })
        .unwrap();
    let id = runtime
        .enqueue("call_ended", json!({}), 1_760_000_000_000)
        .unwrap();
    let event = runtime.run_event(id).unwrap().unwrap();
    assert_eq!(
        event.status,
        embedded::device::EventStatus::Done,
        "{event:?}"
    );
    assert_eq!(phone.actions(), ["notify Bundle: Applied"]);
}

#[test]
fn device_file_thread_apply_uses_stored_body_and_sensitive_code_requires_session() {
    for platform in ["android", "linux"] {
        for sensitive in [false, true] {
            let fake = FakeAnthropic::start();
            let api = Api::start(&fake);
            api.put_text(&format!("/api/device-files/{platform}"), "");
            api.put_text("/api/agents/keeper", KEEPER);
            let source = format!(
                "fn run(event: Value) -> Result[(), string] {{\n Ok(())\n}}\n{}",
                if sensitive {
                    "// contacts_upsert\n"
                } else {
                    ""
                }
            );
            fake.push(AnthropicMessage::new().tool_use("phone_bundle", "propose_device_file_change", json!({"platform": platform, "wcl": PHONE_FILE, "scripts": [{"name": "phone_test", "source": source}]})));
            fake.push(AnthropicMessage::text("Waiting for Apply."));
            api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
            api.advance("PT1S");
            let threads = api.threads("keeper");
            assert_eq!(threads[0]["pending"], 1, "{threads:?}");
            let thread = threads[0]["id"].as_str().unwrap();
            let read = api.get(&format!("/api/agents/keeper/threads/{thread}"));
            let p = read["proposals"][0]["data"].clone();
            let write = api.token_of("bundle_phone", "write");
            let mut forged = p.clone();
            forged["after"] = json!("not WCL");
            let (status, body) = api.apply_as(&write, &forged);
            if sensitive {
                assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
                assert_eq!(api.apply_as(&api.token(), &p).0, StatusCode::FORBIDDEN);
                let login = api
                    .client
                    .post(api.server.url("/api/login"))
                    .json(&json!({"password": PASSWORD}))
                    .send()
                    .unwrap();
                assert_eq!(login.status(), StatusCode::OK);
                let cookie = login
                    .headers()
                    .get("set-cookie")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap();
                let res = api
                    .client
                    .post(api.server.url("/api/ai/apply"))
                    .header("cookie", cookie)
                    .json(&p)
                    .send()
                    .unwrap();
                assert_eq!(res.status(), StatusCode::OK, "{}", res.text().unwrap());
            } else {
                assert_eq!(status, StatusCode::OK, "{body}");
            }
            assert_eq!(api.script("phone_test").as_deref(), Some(source.as_str()));
        }
    }
}
#[test]
fn device_file_bundle_is_checked_atomic_and_stale() {
    for platform in ["android", "linux"] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.put_text(&format!("/api/device-files/{platform}"), "");
        let source = "use device\nfn run(event: Value) -> Result[(), string] {\n Ok(())\n}\n";
        let input = json!({"platform": platform, "wcl": PHONE_FILE, "scripts": [{"name": "phone_test", "source": source}]});
        let events = api.one(&fake, "propose_device_file_change", input.clone());
        let p = find(&events, "device_file_proposal")
            .unwrap_or_else(|| panic!("{events:?}"))
            .clone();
        assert!(api.script("phone_test").is_none()); // Discarding writes nothing.
        assert!(
            p["snapshots"][0]["device"]["note"]
                .as_str()
                .unwrap()
                .contains("no further prompt")
        );
        let write = api.token_of("bundle_write", "write");
        assert_eq!(api.apply_as(&write, &p).0, StatusCode::FORBIDDEN);
        let (status, body) = api.apply_as(&api.token(), &p);
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(api.script("phone_test").as_deref(), Some(source));
        assert_eq!(api.apply_as(&api.token(), &p).0, StatusCode::CONFLICT);

        for stale in ["script", "file", "phone"] {
            let events = api.one(&fake, "propose_device_file_change", input.clone());
            let p = find(&events, "device_file_proposal").unwrap().clone();
            match stale {
                "script" => {
                    api.ok(
                        "PUT",
                        "/api/scripts/phone_test",
                        json!({"source": format!("{source}\n")}),
                    );
                }
                "file" => api.put_text(&format!("/api/device-files/{platform}"), ""),
                _ => {
                    api.platform_device(platform, "New phone", json!({"sms_send": false}));
                }
            }
            let (status, body) = api.apply_as(&api.token(), &p);
            assert_eq!(status, StatusCode::CONFLICT, "{stale}: {body}");
            assert_eq!(body["error"], "stale");
        }
    }
}
#[test]
fn device_file_bundle_refuses_invalid_code_and_rechecks_apply() {
    for platform in ["android", "linux"] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.put_text(&format!("/api/device-files/{platform}"), "");
        let plain = "fn run(event: Value) -> Result[(), string] {\n Ok(())\n}\n";
        for (platform, wcl, source) in [
            ("desktop", PHONE_FILE.to_owned(), plain.to_owned()),
            (
                "android",
                PHONE_FILE.replace("sms_received", "bad_event"),
                plain.to_owned(),
            ),
            (
                "android",
                PHONE_FILE.to_owned(),
                "invalid code here".to_owned(),
            ),
            (
                "android",
                PHONE_FILE.to_owned(),
                format!("{plain}// sms_send\n"),
            ),
            (
                "android",
                PHONE_FILE.to_owned(),
                format!("{plain}// device_command\n"),
            ),
        ] {
            let events = api.one(&fake, "propose_device_file_change", json!({"platform": platform, "wcl": wcl, "scripts": [{"name": "phone_test", "source": source}]}));
            assert!(
                find(&events, "device_file_proposal").is_none(),
                "{events:?}"
            );
            assert!(last_result(&fake).0);
        }
        let missing = api.one(
            &fake,
            "propose_device_file_change",
            json!({"platform": platform, "wcl": PHONE_FILE, "scripts": []}),
        );
        assert!(find(&missing, "device_file_proposal").is_none());
        let events = api.one(&fake, "propose_device_file_change", json!({"platform": platform, "wcl": PHONE_FILE, "scripts": [{"name": "phone_test", "source": plain}]}));
        let p = find(&events, "device_file_proposal").unwrap().clone();
        for name in ["sms_send", "device_command"] {
            let mut forged = p.clone();
            forged["scripts"][0]["source"] = json!(format!("{plain}// {name}\n"));
            let (status, body) = api.apply_as(&api.token(), &forged);
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert!(api.script("phone_test").is_none());
        }
    }
}
#[test]
fn device_file_bundle_warns_about_sms_reads_and_sensitive_bindings() {
    for platform in ["android", "linux"] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.put_text(&format!("/api/device-files/{platform}"), "");
        let source = "fn run(event: Value) -> Result[(), string] {\n Ok(())\n}\n// sms_list contacts_delete\n";
        let events = api.one(&fake, "propose_device_file_change", json!({"platform": platform, "wcl": PHONE_FILE, "scripts": [{"name": "phone_test", "source": source}]}));
        let p = find(&events, "device_file_proposal").unwrap().clone();
        assert_eq!(p["requires_admin"], true);
        let can = p["snapshots"][0]["device"]["can"].as_array().unwrap();
        assert!(
            can.iter()
                .any(|c| c["key"] == "sms_read" && c["new"] == true)
        );
        assert!(can.iter().any(|c| c["key"] == "contacts_write"));
        assert!(
            p.to_string()
                .contains("codes and people that capture skips")
        );
    }
}
#[test]
fn device_file_bundle_checks_shared_roles_and_rejects_folder_access() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    api.put_text("/api/device-files/android", "");
    let file = PHONE_FILE.replace("phone_test", "stamp");
    let source = format!("{STAMP}\n");
    let input = json!({"platform": "android", "wcl": file, "scripts": [{"name": "stamp", "source": source}]});
    let events = api.one(&fake, "propose_device_file_change", input.clone());
    let p = find(&events, "device_file_proposal").unwrap().clone();
    api.put_text(
        "/api/tasks/nightly",
        &format!("{NIGHTLY}read_paths = [\"/srv/in\"]\n"),
    );
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let events = api.one(&fake, "propose_device_file_change", input);
    assert!(find(&events, "device_file_proposal").is_none());
    assert!(last_result(&fake).1.to_string().contains("read_paths"));
    api.put_text("/api/tasks/nightly", NIGHTLY);
    let events = api.one(&fake, "propose_device_file_change", json!({"platform": "android", "wcl": file, "scripts": [{"name": "stamp", "source": format!("use device\n{STAMP}")}]}));
    assert!(find(&events, "device_file_proposal").is_none());
    assert!(last_result(&fake).1.to_string().contains("compile"));
}

#[test]
fn device_file_new_android_scripts_refuse_existing_sensitive_sites_without_source_edits() {
    for platform in ["android", "linux"] {
        for site in ["sms_send", "device_command"] {
            for from_linux in [false, true] {
                let fake = FakeAnthropic::start();
                let api = Api::start(&fake);
                api.put_text(&format!("/api/device-files/{platform}"), "");
                let source = if site == "sms_send" {
                    writer::default_device_script("device_messages")
                        .unwrap()
                        .to_owned()
                } else {
                    format!("{STAMP}\n// vault::device_command\n")
                };
                api.ok("PUT", "/api/scripts/sensitive", json!({"source": source}));
                let file = "command \"send_sms\" {\n script = \"sensitive\"\n function = \"send_sms\"\n}\n";
                if from_linux {
                    api.put_text(
                        &format!(
                            "/api/device-files/{}",
                            if platform == "linux" {
                                "android"
                            } else {
                                "linux"
                            }
                        ),
                        file,
                    );
                }
                let events = api.one(
                    &fake,
                    "propose_device_file_change",
                    json!({"platform": platform, "wcl": "", "scripts": []}),
                );
                let mut forged = find(&events, "device_file_proposal").unwrap().clone();
                forged["after"] = json!(file);
                let events = api.one(
                    &fake,
                    "propose_device_file_change",
                    json!({"platform": platform, "wcl": file, "scripts": []}),
                );
                assert!(
                    find(&events, "device_file_proposal").is_none(),
                    "{events:?}"
                );
                let (error, result) = last_result(&fake);
                assert!(
                    error && result["error"].as_str().unwrap().contains(site),
                    "{result}"
                );
                assert!(!result["error"].as_str().unwrap().starts_with("Refused("));
                assert_eq!(result["message"], "The device file was not proposed.");
                let (status, body) = api.apply_signed_in(&forged);
                assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
                assert!(body.to_string().contains(site), "{body}");
                assert_eq!(api.script("sensitive").as_deref(), Some(source.as_str()));
            }
        }
    }
}
#[test]
fn device_file_new_sensitive_bindings_are_refused_but_existing_bindings_are_kept() {
    for platform in ["android", "linux"] {
        for site in ["sms_send", "device_command"] {
            let fake = FakeAnthropic::start();
            let api = Api::start(&fake);
            let source = if site == "sms_send" {
                writer::default_device_script("device_messages")
                    .unwrap()
                    .to_owned()
            } else {
                format!("{STAMP}\n// vault::device_command\n")
            };
            for name in ["sensitive", "other_sensitive"] {
                api.ok(
                    "PUT",
                    &format!("/api/scripts/{name}"),
                    json!({"source": source}),
                );
            }
            let old =
                "command \"send_sms\" {\n script = \"sensitive\"\n function = \"send_sms\"\n}\n";
            api.put_text(&format!("/api/device-files/{platform}"), old);
            let events = api.one(
                &fake,
                "propose_device_file_change",
                json!({"platform": platform, "wcl": old, "scripts": []}),
            );
            let kept = find(&events, "device_file_proposal")
                .unwrap_or_else(|| panic!("{events:?}"))
                .clone();
            assert_eq!(api.apply_signed_in(&kept).0, StatusCode::OK);
            for new in [
                format!(
                    "{old}\nhandler \"sms_received\" {{\n script = \"sensitive\"\n function = \"send_sms\"\n}}\n"
                ),
                old.replace("command \"send_sms\"", "command \"show_notification\""),
                old.replace("function = \"send_sms\"", "function = \"sms_received\""),
                old.replace("script = \"sensitive\"", "script = \"other_sensitive\""),
            ] {
                let events = api.one(
                    &fake,
                    "propose_device_file_change",
                    json!({"platform": platform, "wcl": new, "scripts": []}),
                );
                assert!(
                    find(&events, "device_file_proposal").is_none(),
                    "{events:?}"
                );
                assert!(last_result(&fake).1.to_string().contains(site));
                let mut forged = kept.clone();
                forged["after"] = json!(new);
                let (status, body) = api.apply_signed_in(&forged);
                assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
                assert!(body.to_string().contains(site), "{body}");
            }
        }
    }
}
#[test]
fn device_file_existing_phone_switch_changes_are_stale() {
    for platform in ["android", "linux"] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.put_text(&format!("/api/device-files/{platform}"), "");
        let id = api.platform_device(
            platform,
            "Existing phone",
            json!({"sms_send": false, "contacts_write": false}),
        );
        for switch in ["sms_send", "contacts_write"] {
            api.set_switches(&id, json!({"sms_send": false, "contacts_write": false}));
            let events = api.one(&fake, "propose_device_file_change", json!({"platform": platform, "wcl": PHONE_FILE, "scripts": [{"name": "phone_test", "source": STAMP}]}));
            let p = find(&events, "device_file_proposal").unwrap();
            let mut settings = json!({"sms_send": false, "contacts_write": false});
            settings[switch] = json!(true);
            api.set_switches(&id, settings);
            let (status, body) = api.apply_signed_in(p);
            assert_eq!(status, StatusCode::CONFLICT, "{switch}: {body}");
            assert_eq!(body["error"], "stale");
            assert!(api.script("phone_test").is_none());
        }
    }
}
#[test]
fn device_file_wcl_only_thread_changes_need_a_session_and_warn_only_for_shipped_copies() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.put_text(
        "/api/device-files/android",
        writer::DEFAULT_ANDROID_DEVICE_FILE,
    );
    api.put_text("/api/agents/keeper", KEEPER);
    let input = json!({"platform": "android", "wcl": ANDROID.replace("custom_messages", "device_messages"), "scripts": []});
    fake.push(AnthropicMessage::new().tool_use("wcl_only", "propose_device_file_change", input));
    fake.push(AnthropicMessage::text("Waiting."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
    api.advance("PT1S");
    let threads = api.threads("keeper");
    assert_eq!(threads[0]["pending"], 1, "{threads:?}");
    let thread = threads[0]["id"].as_str().unwrap();
    let p =
        api.get(&format!("/api/agents/keeper/threads/{thread}"))["proposals"][0]["data"].clone();
    assert_eq!(p["requires_admin"], true);
    assert_eq!(p["warnings"].as_array().unwrap().len(), 1);
    for token in [api.token(), api.token_of("wcl_phone", "write")] {
        let (status, body) = api.apply_as(&token, &p);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.to_string().contains("signed-in admin"));
    }
    let (status, body) = api.apply_signed_in(&p);
    assert_eq!(status, StatusCode::OK, "{body}");
    // android.wcl is now customized, so further edits do not claim to stop its upgrades.
    let events = api.one(
        &fake,
        "propose_device_file_change",
        json!({"platform": "android", "wcl": "", "scripts": []}),
    );
    assert_eq!(
        find(&events, "device_file_proposal").unwrap()["warnings"],
        json!([])
    );
    api.put_text(
        "/api/device-files/android",
        writer::DEFAULT_ANDROID_DEVICE_FILE,
    );
    let events = api.one(
        &fake,
        "propose_device_file_change",
        json!({"platform": "android", "wcl": writer::DEFAULT_ANDROID_DEVICE_FILE, "scripts": []}),
    );
    assert_eq!(
        find(&events, "device_file_proposal").unwrap()["warnings"],
        json!([])
    );
}

#[test]
fn device_file_new_android_binding_keeps_admin_note_for_removed_current_contact_writes() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.put_text("/api/device-files/android", "");
    api.ok(
        "PUT",
        "/api/scripts/phone_test",
        json!({"source": format!("{STAMP}\n// contacts_delete\n")}),
    );
    let events = api.one(&fake, "propose_device_file_change", json!({"platform": "android", "wcl": PHONE_FILE, "scripts": [{"name": "phone_test", "source": STAMP}]}));
    let p = find(&events, "device_file_proposal").unwrap();
    assert_eq!(p["requires_admin"], true);
    assert!(
        p["snapshots"][0]["device"]["note"]
            .as_str()
            .unwrap()
            .contains("signed-in admin")
    );
    assert_eq!(p["warnings"], json!([]));
}

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MEMO: &str = r#"label = "Memo"

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
  }
}
"#;

/// The type whose handler runs the shared script, reaching any host.
const ITEM: &str = r#"label = "Item"

handler "after_update" {
  script        = "stamp"
  function      = "after_update"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const STAMP: &str = r#"fn run() -> Result[(), string] {
    Ok(())
}

fn after_update(note: Value) -> Result[(), string] {
    Ok(())
}

fn call(params: Value) -> Value {
    params
}
"#;

const NIGHTLY: &str =
    "schedule = \"0 3 * * *\"\ntimezone = \"Europe/London\"\nscript = \"stamp\"\npaused = true\n";

#[test]
fn device_commands_are_refused_for_script_task_and_tool_proposals_and_apply() {
    for (tool, kind, name, wcl) in [
        ("propose_script_change", "script_proposal", "stamp", ""),
        ("propose_task_change", "task_proposal", "nightly", NIGHTLY),
        ("propose_tool_change", "tool_proposal", "echo", ECHO),
    ] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.seed_users();
        let added = format!(
            "use vault\n{STAMP}\nfn send(params: Value) -> Result[Value, string] {{\n    vault::device_command(\"phone\", \"send_sms\", params)\n}}\n"
        );
        let input = |source: &str| {
            if kind == "script_proposal" {
                json!({ "name": name, "source": source })
            } else {
                json!({ "name": name, "wcl": wcl, "script_source": source })
            }
        };
        let events = api.one(&fake, tool, input(&added));
        assert!(find(&events, kind).is_none(), "{events:?}");
        let (err, got) = last_result(&fake);
        assert!(err && got.to_string().contains("draft_sms"), "{got}");
        let kept = format!("{STAMP}\n");
        let events = api.one(&fake, tool, input(&kept));
        let mut proposal = find(&events, kind)
            .unwrap_or_else(|| panic!("{events:?}"))
            .clone();
        let field = if kind == "script_proposal" {
            "after"
        } else {
            "script_after"
        };
        proposal[field] = json!(added);
        let (status, body) = api.apply_as(&api.token(), &proposal);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.to_string().contains("draft_sms"), "{body}");
        assert_eq!(api.script("stamp").as_deref(), Some(STAMP));

        // Existing command sites may be kept, including in handler/task/tool scripts.
        api.ok("PUT", "/api/scripts/stamp", json!({ "source": added }));
        let changed = format!("{added}\n");
        let events = api.one(&fake, tool, input(&changed));
        let proposal = find(&events, kind).unwrap_or_else(|| panic!("{events:?}"));
        let (status, body) = api.apply_as(&api.token(), proposal);
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
    }
}

const ECHO: &str = "description = \"Echo the parameters.\"\nscript = \"stamp\"\nfunction = \"call\"\nallow_domains = [\"echo.example.com\"]\n";

/// Every confirmation switch off.
fn all_off() -> Value {
    json!({
        "notes": false, "archive": false, "delete": false, "types": false, "downloads": false,
        "taxonomies": false, "agents": false, "skills": false, "tools": false,
    })
}

/// An agent called from notes, with every switch off.
const KEEPER: &str = r#"label = "Keeper"

permissions {
  confirm {
    notes      = false
    archive    = false
    delete     = false
    types      = false
    downloads  = false
    taxonomies = false
    agents     = false
    skills     = false
    tools      = false
  }
}

trigger asked {
  mention = true
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start_with_args(
            |vault| {
                std::fs::create_dir_all(vault.join("types")).unwrap();
                std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
            },
            &[OsStr::new("--test-clock"), OsStr::new(START)],
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
            api.ok(
                "PUT",
                &format!("/api/secrets/{name}"),
                json!({ "value": value }),
            );
        }
        api
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn send(&self, method: &str, path: &str, token: &str, body: Option<Value>) -> Response {
        let url = self.server.url(path);
        let req = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            m => panic!("{m}"),
        }
        .bearer_auth(token);
        match body {
            Some(b) => req.json(&b),
            None => req,
        }
        .send()
        .unwrap()
    }

    fn call(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let res = self.send(method, path, token, body);
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, v) = self.call(method, path, &self.token(), Some(body));
        assert!(status.is_success(), "{method} {path}: {v}");
        v
    }

    fn get(&self, path: &str) -> Value {
        let (status, v) = self.call("GET", path, &self.token(), None);
        assert!(status.is_success(), "GET {path}: {v}");
        v
    }

    fn put_text(&self, path: &str, text: &str) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(text.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{path}: {}", res.text().unwrap());
    }

    fn script(&self, name: &str) -> Option<String> {
        let (status, v) = self.call("GET", &format!("/api/scripts/{name}"), &self.token(), None);
        status
            .is_success()
            .then(|| v["source"].as_str().unwrap().to_owned())
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let (status, v) = self.call(
            "POST",
            "/api/tokens",
            &self.token(),
            Some(json!({ "name": name, "scope": scope })),
        );
        assert_eq!(status, StatusCode::CREATED, "{v}");
        v["secret"].as_str().unwrap().to_owned()
    }

    /// The shared script and what runs it: a handler of type item, task nightly and tool echo.
    fn seed_users(&self) {
        self.ok("PUT", "/api/scripts/stamp", json!({ "source": STAMP }));
        self.put_text("/api/types/item", ITEM);
        self.put_text("/api/tasks/nightly", NIGHTLY);
        self.ok(
            "PUT",
            "/api/tools/echo",
            json!({ "wcl": ECHO.replace("allow_domains = [\"echo.example.com\"]\n", "") }),
        );
    }

    /// One tool call by the Assistant as the admin, then an answer: the turn's events.
    fn one(&self, fake: &FakeAnthropic, tool: &str, input: Value) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_0", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let res = self.send(
            "POST",
            "/api/ai/chat",
            &self.token(),
            Some(json!({ "messages": [user("Do it.")] })),
        );
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    /// Pair an Android phone named `name` with its switches `settings`: its id.
    fn phone(&self, name: &str, settings: Value) -> String {
        self.platform_device("android", name, settings)
    }

    fn platform_device(&self, platform: &str, name: &str, settings: Value) -> String {
        let res = self
            .client
            .post(self.server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": name, "platform": platform }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let paired: Value = res.json().unwrap();
        let id = paired["device_id"].as_str().unwrap().to_owned();
        self.set_switches(&id, settings);
        id
    }

    fn set_switches(&self, id: &str, settings: Value) {
        self.ok(
            "PATCH",
            &format!("/api/devices/{id}"),
            json!({ "settings": settings }),
        );
    }

    fn apply_as(&self, token: &str, change: &Value) -> (StatusCode, Value) {
        self.call("POST", "/api/ai/apply", token, Some(change.clone()))
    }

    fn apply_signed_in(&self, change: &Value) -> (StatusCode, Value) {
        let login = self
            .client
            .post(self.server.url("/api/login"))
            .json(&json!({ "password": PASSWORD }))
            .send()
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let response = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .header("Cookie", cookie)
            .json(change)
            .send()
            .unwrap();
        (response.status(), response.json().unwrap())
    }

    fn threads(&self, agent: &str) -> Vec<Value> {
        self.get(&format!("/api/agents/{agent}/threads"))["threads"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// A memo whose body calls `agent` with call id `call`.
    fn call_note(&self, agent: &str, call: &str) {
        let text = format!("[@Keeper tidy the script.](agent:{agent}?call={call})");
        self.ok(
            "POST",
            "/api/notes?type=memo",
            json!({ "note": {
                "fields": { "title": "Ask" },
                "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
                    { "kind": "paragraph", "fields": { "text": text } },
                ] }],
            } }),
        );
    }

    fn advance(&self, by: &str) {
        self.ok("POST", "/api/test/clock", json!({ "advance": by }));
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn read_events(res: Response) -> Vec<(String, Value)> {
    use std::io::{BufRead, BufReader};
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
    events.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// The last tool result the model got back, as JSON with its error flag.
fn last_result(fake: &FakeAnthropic) -> (bool, Value) {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    let c = r["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|c| c["type"] == "tool_result")
        .unwrap()
        .clone();
    (
        c["is_error"] == true,
        serde_json::from_str(c["content"].as_str().unwrap()).unwrap_or(Value::Null),
    )
}

/// The `kind` and `name` of each entry of a "Runs as" list.
fn users(runs_as: &Value) -> Vec<(String, String)> {
    runs_as
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["kind"].as_str().unwrap().to_owned(),
                u["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pair(kind: &str, name: &str) -> (String, String) {
    (kind.to_owned(), name.to_owned())
}

#[test]
fn a_script_change_waits_for_apply_with_every_switch_off_and_shows_what_runs_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    api.ok(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "confirm": all_off() }),
    );
    let changed = STAMP.replace(
        "Ok(())\n}\n\nfn after",
        "let x = 1\n    Ok(())\n}\n\nfn after",
    );

    // get_script reads the source and what runs it.
    api.one(&fake, "get_script", json!({ "name": "stamp" }));
    let (err, got) = last_result(&fake);
    assert!(!err, "{got}");
    assert_eq!(got["source"], STAMP);
    assert_eq!(got["problems"], json!([]));
    assert_eq!(
        users(&got["runs_as"]),
        [
            pair("handler", "item"),
            pair("task", "nightly"),
            pair("tool", "echo")
        ]
    );

    // A proposal, never made at once, with its diff, its users and their grants.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    assert!(find(&events, "applied").is_none(), "{events:?}");
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["kind"], "script");
    assert_eq!(p["op"], "change");
    assert_eq!(p["exists"], true);
    assert_eq!(p["before"], STAMP);
    assert_eq!(p["after"], changed.as_str());
    assert_eq!(p["proposer"], "assistant");
    assert_eq!(
        users(&p["runs_as"]),
        [
            pair("handler", "item"),
            pair("task", "nightly"),
            pair("tool", "echo")
        ]
    );
    let handler = &p["runs_as"][0];
    assert_eq!(handler["event"], "after_update");
    assert_eq!(handler["allow_domains"], json!([]));
    assert_eq!(handler["file"], "types/item.wcl");
    let task = &p["runs_as"][1];
    assert_eq!(task["schedule"], "0 3 * * *");
    assert_eq!(task["timezone"], "Europe/London");
    assert_eq!(task["secrets"], json!([]));
    assert_eq!(task["allow_domains"], json!([]));
    assert_eq!(task["paused"], true);
    assert_eq!(p["runs_as"][2]["allow_domains"], json!([]));
    assert_eq!(p["warnings"], json!([]));
    let (err, said) = last_result(&fake);
    assert!(!err && said["proposed"] == true, "{said}");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));

    // A write token cannot apply a body it sends; the admin can, once.
    let write = api.token_of("phone", "write");
    let (status, body) = api.apply_as(&write, &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "needs_admin");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "op": "script_change", "script_name": "stamp" })
    );
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
    // The script changed since: stale, nothing written.
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    let activity = api.get("/api/activity?kind=ai.applied");
    assert!(
        activity
            .to_string()
            .contains("Applied: change script stamp"),
        "{activity}"
    );

    // The same source: nothing to apply.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    assert!(find(&events, "script_proposal").is_none());
    assert_eq!(last_result(&fake).1["proposed"], false);

    // A source that does not compile is not proposed, and says why.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": "fn run( {" }),
    );
    assert!(find(&events, "script_proposal").is_none(), "{events:?}");
    let (err, said) = last_result(&fake);
    assert!(err, "{said}");
    assert!(!said["problems"].as_array().unwrap().is_empty(), "{said}");
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));

    // A new script: created on Apply.
    let fresh = "fn run() -> Result[(), string] {\n    Ok(())\n}\n";
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "fresh", "source": fresh }),
    );
    let p = find(&events, "script_proposal").unwrap().clone();
    assert_eq!(p["op"], "create");
    assert!(p["before"].is_null());
    assert_eq!(p["runs_as"], json!([]));
    assert_eq!(api.script("fresh"), None);
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "script_create");
    assert_eq!(api.script("fresh").as_deref(), Some(fresh));

    // A script in use cannot be deleted; one nothing runs is, on Apply.
    let events = api.one(&fake, "propose_script_delete", json!({ "name": "stamp" }));
    assert!(find(&events, "script_proposal").is_none(), "{events:?}");
    let (err, said) = last_result(&fake);
    assert!(err, "{said}");
    assert_eq!(users(&said["runs_as"]).len(), 3, "{said}");
    let events = api.one(&fake, "propose_script_delete", json!({ "name": "fresh" }));
    let p = find(&events, "script_proposal").unwrap().clone();
    assert_eq!(p["op"], "delete");
    assert_eq!(p["before"], fresh);
    assert_eq!(api.script("fresh").as_deref(), Some(fresh));
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "op": "script_delete", "script_name": "fresh" })
    );
    assert_eq!(api.script("fresh"), None);
    // A missing script names those there are.
    api.one(&fake, "propose_script_delete", json!({ "name": "fresh" }));
    let (err, said) = last_result(&fake);
    assert!(
        err && said["error"].as_str().unwrap().contains("stamp"),
        "{said}"
    );
}

#[test]
fn a_tool_proposal_shows_what_else_runs_its_script() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    let open = ECHO.replace("echo.example.com", "*");
    let events = api.one(
        &fake,
        "propose_tool_change",
        json!({ "name": "echo", "wcl": open, "script_source": null }),
    );
    let p = find(&events, "tool_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    // The tool itself is left out of its own list.
    assert_eq!(
        users(&p["runs_as"]),
        [pair("handler", "item"), pair("task", "nightly")]
    );
    let warnings: Vec<&str> = p["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 1, "{p}");
    assert!(
        warnings[0].starts_with("The tool echo may reach any host"),
        "{warnings:?}"
    );
    // A delete that keeps the script lists what keeps it.
    let events = api.one(&fake, "propose_tool_delete", json!({ "name": "echo" }));
    let p = find(&events, "tool_proposal").unwrap().clone();
    assert_eq!(p["script_delete"], false);
    assert_eq!(users(&p["runs_as"]).len(), 2, "{p}");
}

#[test]
fn a_call_from_a_note_leaves_a_script_proposal_that_a_phone_applies_or_discards_from_the_thread() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok("PUT", "/api/scripts/stamp", json!({ "source": STAMP }));
    api.put_text("/api/agents/keeper", KEEPER);
    let changed = format!("{STAMP}\nfn extra() -> int {{\n    1\n}}\n");

    fake.push(AnthropicMessage::new().tool_use(
        "toolu_s",
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    ));
    fake.push(AnthropicMessage::text("The change waits for you."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
    api.advance("PT1S");
    let first = fake.message_requests()[0].json();
    assert!(
        first["tools"]
            .to_string()
            .contains("\"propose_script_change\"")
    );

    // Every switch is off, yet the change waits in the thread.
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
    let threads = api.threads("keeper");
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(threads[0]["origin"], "mention");
    assert_eq!(threads[0]["pending"], 1, "{threads:?}");
    let thread = threads[0]["id"].as_str().unwrap().to_owned();
    let read = api.get(&format!("/api/agents/keeper/threads/{thread}"));
    let stored = read["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["status"] == "pending")
        .unwrap_or_else(|| panic!("{read}"))
        .clone();
    assert_eq!(stored["event"], "script_proposal");

    // A write token applies the copy the thread holds, not a body it changed.
    let write = api.token_of("phone", "write");
    let mut forged = stored["data"].clone();
    forged["after"] = json!("fn run() -> Result[(), string] {\n    Err(\"forged\")\n}\n");
    let (status, body) = api.apply_as(&write, &forged);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["op"], "script_change");
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
    assert_eq!(api.threads("keeper")[0]["pending"], 0);

    // A second call's proposal, discarded from the phone: nothing written.
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "propose_script_change",
        json!({ "name": "stamp", "source": STAMP }),
    ));
    fake.push(AnthropicMessage::text("Waiting."));
    api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC");
    api.advance("PT1S");
    let pending = api
        .threads("keeper")
        .into_iter()
        .find(|t| t["pending"] == 1)
        .unwrap_or_else(|| panic!("{:?}", api.threads("keeper")));
    let thread = pending["id"].as_str().unwrap().to_owned();
    let (status, body) = api.call(
        "POST",
        "/api/ai/discard",
        &write,
        Some(json!({ "id": "toolu_t", "thread": thread })),
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pending"], 0, "{body}");
    assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
}

const FRESH: &str = "fn run() -> Result[(), string] {\n    Ok(())\n}\n";

#[test]
fn a_script_applied_after_what_runs_it_changed_is_stale() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    let changed = STAMP.replace(
        "Ok(())\n}\n\nfn after",
        "let x = 1\n    Ok(())\n}\n\nfn after",
    );
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    // The task nightly now reaches another host: what runs the script has changed since the card.
    api.put_text(
        "/api/tasks/nightly",
        &NIGHTLY.replace("0 3 * * *", "0 4 * * *"),
    );
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
}

#[test]
fn a_delete_applied_after_a_task_started_to_run_the_script_is_refused_in_use() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok("PUT", "/api/scripts/fresh", json!({ "source": FRESH }));
    let events = api.one(&fake, "propose_script_delete", json!({ "name": "fresh" }));
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["op"], "delete");
    // A task starts to run the script after the card.
    api.put_text(
        "/api/tasks/fresh_nightly",
        &NIGHTLY.replace("script = \"stamp\"", "script = \"fresh\""),
    );
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "script_in_use");
    assert_eq!(api.script("fresh").as_deref(), Some(FRESH));
}

#[test]
fn a_device_file_that_runs_a_script_is_in_its_runs_as_list() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.ok(
        "PUT",
        "/api/scripts/custom_messages",
        json!({"source": writer::default_device_script("device_messages").unwrap()}),
    );
    api.put_text(
        "/api/device-files/android",
        "handler \"sms_received\" {\n  script = \"custom_messages\"\n}\n",
    );
    api.ok(
        "PUT",
        "/api/scripts/custom_messages",
        json!({"source": writer::default_device_script("device_messages").unwrap()}),
    );
    let current = api.script("custom_messages").unwrap();
    let changed = format!("{current}\n");
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "custom_messages", "source": changed }),
    );
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(users(&p["runs_as"]), [pair("device", "android")], "{p}");
    assert_eq!(p["runs_as"][0]["event"], "handler sms_received");
    assert_eq!(p["runs_as"][0]["file"], "devices/android.wcl");
    assert_eq!(p["warnings"], json!([]));
}

#[test]
fn a_tool_applied_after_what_runs_its_script_changed_is_stale() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    let open = ECHO.replace("echo.example.com", "*");
    let events = api.one(
        &fake,
        "propose_tool_change",
        json!({ "name": "echo", "wcl": open, "script_source": null }),
    );
    let p = find(&events, "tool_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    // The task nightly now reaches another host: the tool's script has a new "Runs as" list.
    api.put_text(
        "/api/tasks/nightly",
        &NIGHTLY.replace("0 3 * * *", "0 4 * * *"),
    );
    let before = api.get("/api/tools/echo");
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stale");
    assert_eq!(api.get("/api/tools/echo")["wcl"], before["wcl"]);
}

#[test]
fn a_script_change_is_refused_while_a_task_with_folders_runs_it_at_propose_and_at_apply() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    let changed = STAMP.replace(
        "Ok(())\n}\n\nfn after",
        "let x = 1\n    Ok(())\n}\n\nfn after",
    );

    // Nothing with folders runs the script yet: the change is proposed.
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["op"], "change");

    // The user gives the task nightly folders, and it runs the script: a new proposal is refused,
    // naming the run and its folders, and nothing is written (TV-87, TV-32 Q3).
    api.put_text(
        "/api/tasks/nightly",
        &format!("{NIGHTLY}read_paths = [\"/srv/in\"]\n"),
    );
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    assert!(find(&events, "script_proposal").is_none(), "{events:?}");
    let (err, got) = last_result(&fake);
    assert!(err, "{got}");
    let why = got["error"].as_str().unwrap();
    assert!(why.contains("may not change script \"stamp\""), "{got}");
    assert!(why.contains("task nightly (reads /srv/in)"), "{got}");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));

    // The proposal made before the folders were given is refused at Apply, and nothing is written.
    let (status, body) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "not_permitted");
    assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
}

/// The device file that runs `custom_messages` for an incoming SMS and for the Outbox's send.
const ANDROID: &str = "handler \"sms_received\" {\n  script = \"custom_messages\"\n}\n\ncommand \"send_sms\" {\n  script = \"custom_messages\"\n}\n";

const PHONE_NOTE: &str = "Runs on your phone at its next sync, with no further prompt there.";

#[test]
fn tool_and_task_phone_sources_show_warnings_and_refuse_added_sms_send() {
    for (kind, name, wcl) in [("tool", "echo", ECHO), ("task", "nightly", NIGHTLY)] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.seed_users();
        api.put_text(
            "/api/device-files/android",
            "handler \"sms_received\" {\n script = \"stamp\"\n function = \"after_update\"\n}\n",
        );
        let pixel = api.phone("Pixel", json!({ "sms_send": true }));
        let changed = format!("{STAMP}\n// vault::patch\n");
        let method = format!("propose_{kind}_change");
        let event = format!("{kind}_proposal");
        let events = api.one(
            &fake,
            &method,
            json!({ "name": name, "wcl": wcl, "script_source": changed }),
        );
        let p = find(&events, &event)
            .unwrap_or_else(|| panic!("{events:?}"))
            .clone();
        assert_eq!(p["device"]["note"], PHONE_NOTE, "{p}");
        assert_eq!(p["device"]["platforms"], json!(["android"]));
        assert_eq!(p["device"]["devices"][0]["name"], "Pixel");
        assert_eq!(p["device"]["devices"][0]["sms_send"], true);
        assert_eq!(can(&p["device"]), [("notes_write".to_owned(), true)]);
        let (err, got) = last_result(&fake);
        assert!(!err, "{got}");
        assert_eq!(got["device"], p["device"]);

        // Mentions count even in comments, so this still compiles as a tool/task.
        let forbidden = format!("{changed}\n// device::sms_send\n");
        let events = api.one(
            &fake,
            &method,
            json!({ "name": name, "wcl": wcl, "script_source": forbidden }),
        );
        assert!(find(&events, &event).is_none(), "{events:?}");
        let (err, got) = last_result(&fake);
        assert!(err, "{got}");
        assert!(
            got["error"]
                .as_str()
                .unwrap()
                .contains("may not add a call to device::sms_send"),
            "{got}"
        );
        let mut forged = p.clone();
        forged["script_after"] = json!(forbidden);
        let (status, body) = api.apply_as(&api.token(), &forged);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "not_permitted");
        assert_eq!(api.script("stamp").as_deref(), Some(STAMP));

        api.set_switches(&pixel, json!({ "sms_send": false }));
        let (status, body) = api.apply_as(&api.token(), &p);
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"], "stale");
        assert_eq!(api.script("stamp").as_deref(), Some(STAMP));
        let events = api.one(
            &fake,
            &method,
            json!({ "name": name, "wcl": wcl, "script_source": changed }),
        );
        let p = find(&events, &event).unwrap();
        let (status, body) = api.apply_as(&api.token(), p);
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(api.script("stamp").as_deref(), Some(changed.as_str()));
    }
}

/// The `key` and `new` of each item of a card's "This code can" list.
fn can(device: &Value) -> Vec<(String, bool)> {
    device["can"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["key"].as_str().unwrap().to_owned(), c["new"] == true))
        .collect()
}

#[test]
fn a_change_to_a_phone_script_shows_the_phones_their_switches_and_what_the_code_can_do() {
    for platform in ["android", "linux"] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.ok(
            "PUT",
            "/api/scripts/custom_messages",
            json!({"source": writer::default_device_script("device_messages").unwrap()}),
        );
        api.put_text("/api/device-files/android", "");
        api.put_text(&format!("/api/device-files/{platform}"), ANDROID);
        let pixel = api.platform_device(
            platform,
            "Pixel",
            json!({ "commands": ["send_sms"], "sms_send": true }),
        );
        api.platform_device(platform, "Tablet", json!({ "contacts_write": true }));
        api.ok(
            "PUT",
            "/api/scripts/custom_messages",
            json!({"source": writer::default_device_script("device_messages").unwrap()}),
        );
        let current = api.script("custom_messages").unwrap();
        // The new code reads an SMS from the phone's store: a new item. It keeps the one send.
        let changed = format!(
            "{current}\nfn peek() -> Result[Value, string] {{\n    device::sms_get(\"x\")\n}}\n"
        );
        let events = api.one(
            &fake,
            "propose_script_change",
            json!({ "name": "custom_messages", "source": changed }),
        );
        let p = find(&events, "script_proposal")
            .unwrap_or_else(|| panic!("{events:?}"))
            .clone();
        let d = &p["device"];
        assert!(
            d["note"]
                .as_str()
                .unwrap()
                .starts_with(if platform == "android" {
                    PHONE_NOTE
                } else {
                    "Runs on your paired devices"
                }),
            "{p}"
        );
        assert!(
            d["note"]
                .as_str()
                .unwrap()
                .contains("signed-in admin must apply"),
            "{p}"
        );
        assert_eq!(d["platforms"], json!([platform]));
        let devices: Vec<(String, bool, bool)> = d["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["name"].as_str().unwrap().to_owned(),
                    d["sms_send"] == true,
                    d["contacts_write"] == true,
                )
            })
            .collect();
        assert_eq!(
            devices,
            [
                ("Pixel".to_owned(), true, false),
                ("Tablet".to_owned(), false, true)
            ]
        );
        let items = can(d);
        for kept in ["sms_send", "notifications", "notes_write"] {
            assert!(items.contains(&(kept.to_owned(), false)), "{kept}: {d}");
        }
        assert!(items.contains(&("sms_read".to_owned(), true)), "{d}");
        assert!(
            !items
                .iter()
                .any(|(k, _)| k == "calls_read" || k == "contacts_write"),
            "{d}"
        );
        assert!(
            d["can"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["label"]
                    == "Read all SMS, including the codes and people that capture skips"),
            "{d}"
        );
        // The model reads the same.
        let (err, got) = last_result(&fake);
        assert!(!err, "{got}");
        assert_eq!(got["device"], *d);

        // The Pixel's SMS send is turned off after the card: session Apply answers stale.
        api.set_switches(
            &pixel,
            json!({ "commands": ["send_sms"], "sms_send": false }),
        );
        let (status, body) = api.apply_signed_in(&p);
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"], "stale");
        assert_eq!(
            api.script("custom_messages").as_deref(),
            Some(current.as_str())
        );

        // Proposed again, the card shows the switch as it is now and applies.
        let events = api.one(
            &fake,
            "propose_script_change",
            json!({ "name": "custom_messages", "source": changed }),
        );
        let p = find(&events, "script_proposal")
            .unwrap_or_else(|| panic!("{events:?}"))
            .clone();
        assert_eq!(p["device"]["devices"][0]["sms_send"], false, "{p}");
        let (status, body) = api.apply_signed_in(&p);
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(
            api.script("custom_messages").as_deref(),
            Some(changed.as_str())
        );
    }
}
#[test]
fn a_script_that_no_device_file_runs_has_no_phone_block() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.seed_users();
    let changed = STAMP.replace(
        "Ok(())\n}\n\nfn after",
        "let x = 1\n    Ok(())\n}\n\nfn after",
    );
    let events = api.one(
        &fake,
        "propose_script_change",
        json!({ "name": "stamp", "source": changed }),
    );
    let p = find(&events, "script_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert!(p["device"].is_null(), "{p}");
}

#[test]
fn sensitive_phone_script_changes_in_a_thread_require_admin_even_when_the_send_is_unchanged() {
    for platform in ["android", "linux"] {
        for mode in [
            "kept_send",
            "removed_send",
            "new_contacts",
            "old_contacts",
            "plain",
        ] {
            let fake = FakeAnthropic::start();
            let api = Api::start(&fake);
            api.ok(
                "PUT",
                "/api/scripts/custom_messages",
                json!({"source": writer::default_device_script("device_messages").unwrap()}),
            );
            api.put_text("/api/device-files/android", "");
            api.put_text(&format!("/api/device-files/{platform}"), ANDROID);
            api.put_text("/api/agents/keeper", KEEPER);
            let original = writer::default_device_script("device_messages")
                .unwrap()
                .to_owned();
            let plain = original.replace(
                "device::sms_send(to, text_of(payload, \"body\"))",
                "Err(\"sending disabled\")",
            );
            let (current, changed) = match mode {
                "kept_send" => (original.clone(), format!("{original}\n")),
                "removed_send" => (original.clone(), plain.clone()),
                // Comments count conservatively too, so even aliases or unusual formatting cannot
                // turn a scanner ambiguity into a write-token grant.
                "new_contacts" => (plain.clone(), format!("{plain}\n// contacts_upsert\n")),
                "old_contacts" => (format!("{plain}\n// contacts_delete\n"), plain.clone()),
                _ => (plain.clone(), format!("{plain}\n")),
            };
            api.ok(
                "PUT",
                "/api/scripts/custom_messages",
                json!({ "source": current }),
            );
            fake.push(AnthropicMessage::new().tool_use(
                "toolu_phone",
                "propose_script_change",
                json!({ "name": "custom_messages", "source": changed }),
            ));
            fake.push(AnthropicMessage::text("Waiting."));
            api.call_note("keeper", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB");
            api.advance("PT1S");
            let threads = api.threads("keeper");
            assert_eq!(threads[0]["pending"], 1, "{mode}: {threads:?}");
            let thread = threads[0]["id"].as_str().unwrap();
            let read = api.get(&format!("/api/agents/keeper/threads/{thread}"));
            let p = read["proposals"][0]["data"].clone();
            let write = api.token_of("phone", "write");
            let (status, body) = api.apply_as(&write, &p);
            if mode == "plain" {
                assert_eq!(status, StatusCode::OK, "{mode}: {body}");
            } else {
                assert_eq!(status, StatusCode::FORBIDDEN, "{mode}: {body}");
                assert!(body.to_string().contains("signed-in admin"), "{body}");
                assert_eq!(
                    api.script("custom_messages").as_deref(),
                    Some(current.as_str())
                );
                let (status, body) = api.apply_as(&api.token(), &p);
                assert_eq!(status, StatusCode::FORBIDDEN, "{mode}: {body}");
                assert!(body.to_string().contains("admin-scoped tokens"), "{body}");
                assert_eq!(
                    api.script("custom_messages").as_deref(),
                    Some(current.as_str())
                );
                let (status, body) = api.apply_signed_in(&p);
                assert_eq!(status, StatusCode::OK, "{mode}: {body}");
            }
            assert_eq!(
                api.script("custom_messages").as_deref(),
                Some(changed.as_str())
            );
        }
    }
}
#[test]
fn a_change_that_adds_an_sms_send_is_refused_at_propose_and_at_apply() {
    for platform in ["android", "linux"] {
        let fake = FakeAnthropic::start();
        let api = Api::start(&fake);
        api.ok(
            "PUT",
            "/api/scripts/custom_messages",
            json!({"source": writer::default_device_script("device_messages").unwrap()}),
        );
        api.put_text("/api/device-files/android", "");
        api.put_text(&format!("/api/device-files/{platform}"), ANDROID);
        api.platform_device(
            platform,
            "Pixel",
            json!({ "commands": ["send_sms"], "sms_send": true }),
        );
        api.ok(
            "PUT",
            "/api/scripts/custom_messages",
            json!({"source": writer::default_device_script("device_messages").unwrap()}),
        );
        let current = api.script("custom_messages").unwrap();
        let forward = format!(
            "{current}\nfn forward() -> Result[Value, string] {{\n    device::sms_send(\"+15550100\", \"x\")\n}}\n"
        );

        // A second call site is refused at the proposal, and nothing is written (policy P-a).
        let events = api.one(
            &fake,
            "propose_script_change",
            json!({ "name": "custom_messages", "source": forward }),
        );
        assert!(find(&events, "script_proposal").is_none(), "{events:?}");
        let (err, got) = last_result(&fake);
        assert!(err, "{got}");
        let why = got["error"].as_str().unwrap();
        assert!(
            why.contains("may not add a call to device::sms_send to script \"custom_messages\""),
            "{got}"
        );
        assert!(why.contains("calls it 1 times"), "{got}");
        assert!(why.contains("names sms_send 2 times"), "{got}");

        // Taking the function as a value is a call site too (TV-72 R9).
        let alias = format!(
            "{current}\nfn later() -> Result[(), string] {{\n    let f = device::sms_send\n    Ok(())\n}}\n"
        );
        let events = api.one(
            &fake,
            "propose_script_change",
            json!({ "name": "custom_messages", "source": alias }),
        );
        assert!(find(&events, "script_proposal").is_none(), "{events:?}");
        assert!(last_result(&fake).0);

        // A change that keeps the one call is proposed.
        let kept = format!("{current}\n");
        let events = api.one(
            &fake,
            "propose_script_change",
            json!({ "name": "custom_messages", "source": kept }),
        );
        let p = find(&events, "script_proposal")
            .unwrap_or_else(|| panic!("{events:?}"))
            .clone();

        // The same card with the new call put into its body is refused at Apply, 403, and nothing is
        // written.
        let mut forged = p.clone();
        forged["after"] = json!(forward);
        let (status, body) = api.apply_signed_in(&forged);
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "not_permitted");
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| m.contains("device::sms_send")),
            "{body}"
        );
        assert_eq!(
            api.script("custom_messages").as_deref(),
            Some(current.as_str())
        );
    }
}
