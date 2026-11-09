//! Atomic Android device-file and script proposals (TV-72). No direct execution path.
use std::collections::{BTreeMap, BTreeSet};

use ai::ToolSpec;
use schema::{Agent, DeviceFile};
use serde_json::{Value, json};
use writer::{ScriptUse, Writer};

use crate::ai_chat::{ApplyError, ToolOut, str_arg, tool};
use crate::ai_script_changes as scripts;

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool(
            "get_device_file",
            "Read the Android phone device file before proposing a change.",
            json!({"platform": {"type": "string", "enum": ["android"]}}),
        ),
        tool(
            "propose_device_file_change",
            "Propose an Android device file and its new or changed scripts as one checked change. Always waits for Apply. Phone code runs after sync without another prompt. No new sms_send or vault::device_command sites. Sensitive phone code requires a signed-in admin.",
            json!({
                "platform": {"type": "string", "enum": ["android"]},
                "wcl": {"type": "string"},
                "scripts": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "source": {"type": "string"}}, "required": ["name", "source"], "additionalProperties": false}}
            }),
        ),
    ]
}

fn android(input: &Value) -> Result<(), ToolOut> {
    if str_arg(input, "platform")? != "android" {
        return Err(ToolOut::error("Only Android device files may be proposed"));
    }
    Ok(())
}

pub(crate) fn get(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    android(input)?;
    let file = w.read_device_file("android").ok();
    Ok(output(json!({"platform": "android", "wcl": file}), None))
}

fn output(content: Value, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({"notes": []}),
        proposal: proposal.map(|p| ("device_file_proposal", p)),
    }
}

fn invalid(s: impl ToString) -> ApplyError {
    ApplyError::Invalid(s.to_string())
}

/// Reconstruct all checks and warning snapshots from current vault state. Used under the commit
/// lock at Apply as well as at propose. Supplied warning fields are never authority.
fn checked(w: &Writer, source: &str, supplied: &Value) -> Result<Value, ApplyError> {
    let after = store::canonical(source, "devices/android.wcl").map_err(invalid)?;
    let file = DeviceFile::parse("android", &after).map_err(invalid)?;
    let before = w.read_device_file("android").ok();
    let old = before
        .as_deref()
        .map(|text| DeviceFile::parse("android", text))
        .transpose()
        .map_err(invalid)?;
    let mut edits = BTreeMap::new();
    for s in supplied
        .as_array()
        .ok_or_else(|| invalid("scripts must be an array"))?
    {
        let name = s["name"]
            .as_str()
            .filter(|n| schema::is_name(n))
            .ok_or_else(|| invalid("invalid script name"))?;
        let source = s["source"]
            .as_str()
            .ok_or_else(|| invalid("script source is required"))?;
        if !file.scripts().contains(&name) || edits.insert(name, source).is_some() {
            return Err(invalid(
                "each supplied script must be named once and used by the proposed device file",
            ));
        }
    }
    let mut names: BTreeSet<String> = file.scripts().into_iter().map(str::to_owned).collect();
    if let Some(old) = &old {
        names.extend(old.scripts().into_iter().map(str::to_owned));
    }
    let mut snapshots = Vec::new();
    let mut admin = false;
    for name in names {
        let current = w.read_script(&name).ok();
        let source = edits
            .get(name.as_str())
            .copied()
            .or(current.as_deref())
            .ok_or_else(|| invalid(format!("there is no script {name:?}")))?;
        let changing = current.as_deref() != Some(source);
        let was_on_android = old
            .as_ref()
            .is_some_and(|f| f.scripts().contains(&name.as_str()));
        if changing && let Some(why) = scripts::shared_paths_refused(w, &name, None) {
            return Err(ApplyError::Refused(why));
        }
        // Bringing existing code onto Android is a new capability, not a kept call site.
        let baseline = if was_on_android {
            current.as_deref()
        } else {
            None
        };
        if (changing || !was_on_android)
            && let Some(why) = scripts::sms_send_refused(&name, baseline, source)
                .or_else(|| scripts::device_command_refused(&name, baseline, source))
        {
            return Err(ApplyError::Refused(why));
        }
        // A new event/command target can create a send path without changing any source.
        // Keep the kind, name, script and function in the comparison. Existing bindings stay.
        for (kind, hooks, previous) in [
            ("handler", &file.handlers, old.as_ref().map(|f| &f.handlers)),
            ("command", &file.commands, old.as_ref().map(|f| &f.commands)),
        ] {
            for hook in hooks.iter().filter(|h| h.script == name) {
                if !previous.is_some_and(|list| list.contains(hook))
                    && let Some(why) = scripts::sms_send_refused(&name, None, source)
                        .or_else(|| scripts::device_command_refused(&name, None, source))
                {
                    return Err(ApplyError::Refused(format!(
                        "A new {kind} binding {:?} to {}::{} is not permitted: {why}",
                        hook.name, hook.script, hook.function
                    )));
                }
            }
        }
        if file.scripts().contains(&name.as_str()) {
            let problems = w.script_problems_with_device(&name, source, Some(&file))?;
            if !problems.is_empty() {
                return Err(invalid(format!(
                    "script {name:?} does not compile: {problems:?}"
                )));
            }
        }
        admin |= scripts::sensitive_phone_source(current.as_deref(), Some(source));
        let uses = w.script_runs_as(&name, None)?;
        let (runs_before, _) = scripts::described(&uses);
        let mut proposed: Vec<_> = uses
            .into_iter()
            .filter(|u| !(u.kind == "device" && u.name == "android"))
            .collect();
        for (kind, hooks) in [("handler", &file.handlers), ("command", &file.commands)] {
            for h in hooks.iter().filter(|h| h.script == name) {
                proposed.push(ScriptUse {
                    kind: "device",
                    name: "android".into(),
                    file: "devices/android.wcl".into(),
                    event: Some(format!("{kind} {}", h.name)),
                    ..ScriptUse::default()
                });
            }
        }
        let (runs_after, mut warnings) = scripts::described(&proposed);
        if writer::default_device_script(&name).is_some() && changing {
            warnings.push("Editing this shipped script stops automatic replacement by future app upgrades. A bell notice reports newer defaults.".into());
        }
        if file
            .commands
            .iter()
            .any(|h| h.name == "send_sms" && h.script == name)
        {
            warnings.push(
                "This code handles Outbox sends and can change what an approved SMS sends.".into(),
            );
        }
        // Include Android even for removed references so removals retain phone warnings.
        let mut phones = proposed.clone();
        phones.push(ScriptUse {
            kind: "device",
            name: "android".into(),
            ..ScriptUse::default()
        });
        let mut phone = scripts::on_phone(
            w,
            &phones,
            if was_on_android {
                current.as_deref()
            } else {
                None
            },
            source,
        )?;
        // Capability highlighting uses the previous Android binding, but the admin note must
        // also consider sensitive current source in a newly bound script (including removals).
        if scripts::sensitive_phone_source(current.as_deref(), Some(source))
            && let Some(phone) = &mut phone
        {
            phone["note"] = json!(format!(
                "{} {}",
                scripts::PHONE_NOTE,
                scripts::PHONE_ADMIN_MESSAGE
            ));
        }
        snapshots.push(json!({"name": name, "before": current, "after": source,
            "runs_before": runs_before, "runs_as": runs_after, "warnings": warnings, "device": phone}));
    }
    // Settings and permission reports also invalidate the card, not only the two switches.
    let mut devices = w.state().list_devices().map_err(writer::Error::from)?;
    devices.retain(|d| d.platform == "android");
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    let devices: Vec<_> = devices
        .iter()
        .map(|d| json!({"id": d.id, "name": d.name, "settings": d.settings, "health": d.health}))
        .collect();
    let warnings: Vec<&str> = if before.as_deref() != Some(after.as_str())
        && before.as_ref().is_some_and(|text| {
            let hash = writer::shipped_hash("devices/android.wcl", text);
            writer::SHIPPED_FILES
                .iter()
                .any(|f| f.path == "devices/android.wcl" && f.hashes.contains(&hash.as_str()))
        }) {
        vec![
            "Editing this shipped device file stops automatic replacement by future app upgrades. A bell notice reports newer defaults.",
        ]
    } else {
        Vec::new()
    };
    Ok(
        json!({"before": before, "after": after, "snapshots": snapshots, "devices": devices, "requires_admin": admin, "warnings": warnings}),
    )
}

pub(crate) fn propose(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    android(input)?;
    let source = str_arg(input, "wcl")?;
    let supplied = input.get("scripts").cloned().unwrap_or_else(|| json!([]));
    let mut p = checked(w, source, &supplied).map_err(|e| {
        let mut out = ToolOut::error(e.to_string());
        out.content =
            json!({"error": e.to_string(), "message": "The device file was not proposed."})
                .to_string();
        out
    })?;
    p["id"] = json!(call_id);
    p["kind"] = json!("device_file");
    p["platform"] = json!("android");
    p["scripts"] = supplied;
    p["proposer"] = json!(by.name);
    Ok(output(
        json!({"proposed": true, "message": "The checked change awaits Apply. Nothing was written.", "details": p}),
        Some(p),
    ))
}

pub(crate) fn apply(w: &Writer, p: &Value, admin: bool) -> Result<Value, ApplyError> {
    if p["platform"] != "android" {
        return Err(invalid("Only Android is supported"));
    }
    let after = p["after"]
        .as_str()
        .ok_or_else(|| invalid("missing device file"))?;
    w.commit_checked(|w| -> Result<Vec<store::Change>, ApplyError> {
        if json!(w.read_device_file("android").ok()) != p["before"] {
            return Err(ApplyError::Stale(
                "The device file changed. Propose it again.".into(),
            ));
        }
        for s in p["snapshots"]
            .as_array()
            .ok_or_else(|| invalid("missing snapshots"))?
        {
            let name = s["name"]
                .as_str()
                .ok_or_else(|| invalid("missing script name"))?;
            if json!(w.read_script(name).ok()) != s["before"]
                || scripts::described(&w.script_runs_as(name, None)?).0 != s["runs_before"]
            {
                return Err(ApplyError::Stale(
                    "A script or its uses changed. Propose it again.".into(),
                ));
            }
        }
        let now = checked(w, after, &p["scripts"])?;
        if now["requires_admin"] == true && !admin {
            return Err(ApplyError::Refused(scripts::PHONE_ADMIN_MESSAGE.into()));
        }
        for field in [
            "before",
            "after",
            "snapshots",
            "devices",
            "requires_admin",
            "warnings",
        ] {
            if now[field] != p[field] {
                return Err(ApplyError::Stale(format!(
                    "{field} changed since the proposal. Propose it again."
                )));
            }
        }
        let mut changes = vec![store::Change::write(
            "devices/android.wcl",
            after.as_bytes(),
        )];
        for s in now["snapshots"].as_array().expect("checked snapshots") {
            if s["before"] != s["after"] {
                changes.extend(w.script_changes(s["name"].as_str().unwrap(), s["after"].as_str())?);
            }
        }
        Ok(changes)
    })?;
    Ok(json!({"op": "device_file_change", "platform": "android"}))
}
