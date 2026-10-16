//! Device files (D5, D22): `devices/<platform>.wcl` names the script function that handles each
//! device event on that platform, and the commands a device of that platform accepts from server
//! scripts.

use std::collections::HashSet;

use crate::blocks::{Check, Form, opt, req};
use crate::doc::{Doc, DocBlock};
use crate::encoding::is_name;
use crate::types::string;
use crate::{Invalid, Issue};

/// The platforms a device file can be for, the stem of `devices/<platform>.wcl`.
pub const DEVICE_PLATFORMS: [&str; 2] = ["linux", "android"];

/// The device events, a closed list (D5).
pub const DEVICE_EVENTS: [&str; 8] = [
    "sms_received",
    "sms_sent",
    "mms_received",
    "rcs_received",
    "call_ended",
    "contact_changed",
    "contact_deleted",
    "share_received",
];

/// The commands a device implements, a closed list (D22). `import_history` reads the phone's
/// messages, calls and contacts again over a range of dates, as device events.
pub const DEVICE_COMMANDS: [&str; 5] = [
    "send_sms",
    "upsert_contact",
    "delete_contact",
    "show_notification",
    "import_history",
];

/// One `devices/<platform>.wcl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFile {
    pub platform: String,
    /// In file order, which is the order a device runs an event's handlers in.
    pub handlers: Vec<DeviceHook>,
    /// In file order. A command appears at most once.
    pub commands: Vec<DeviceHook>,
}

/// A `handler "<event>"` or `command "<name>"` block: the event or command, and the script
/// function that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceHook {
    /// The event or the command name: the block's label.
    pub name: String,
    pub script: String,
    /// The function to call: the event or command name when the block names none.
    pub function: String,
}

impl DeviceFile {
    /// Parse `devices/<platform>.wcl`. Every problem is reported at once.
    pub fn parse(platform: &str, source: &str) -> Result<DeviceFile, Invalid> {
        let file = format!("devices/{platform}.wcl");
        let doc = Doc::parse(source, &file)?;
        DeviceFile::from_doc(platform, &doc).map_err(|issues| Invalid { file, issues })
    }

    pub fn from_doc(platform: &str, doc: &Doc) -> Result<DeviceFile, Vec<Issue>> {
        let mut issues = Vec::new();
        if !DEVICE_PLATFORMS.contains(&platform) {
            issues.push(Issue::new(
                "file",
                format!(
                    "{platform:?} is not a device platform: {}",
                    DEVICE_PLATFORMS.join(", ")
                ),
            ));
        }
        let mut c = Check {
            issues: &mut issues,
            fields: None,
            template: false,
        };
        c.items(doc, &[], &[], "device file");
        c.only_blocks(doc, &["handler", "command"], "device file");
        let handlers = doc
            .blocks_of("handler")
            .map(|b| hook(&mut c, b, "handler", "event", &DEVICE_EVENTS))
            .collect();
        let mut seen = HashSet::new();
        let commands = doc
            .blocks_of("command")
            .map(|b| {
                let h = hook(&mut c, b, "command", "command", &DEVICE_COMMANDS);
                if !seen.insert(h.name.clone()) {
                    c.fail(&format!("command {:?}", h.name), "appears more than once");
                }
                h
            })
            .collect();
        if issues.is_empty() {
            Ok(DeviceFile {
                platform: platform.to_owned(),
                handlers,
                commands,
            })
        } else {
            Err(issues)
        }
    }

    /// The `command` block for `name`, when the file declares it.
    pub fn command(&self, name: &str) -> Option<&DeviceHook> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// The scripts the file names, each once, in file order.
    pub fn scripts(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for h in self.handlers.iter().chain(&self.commands) {
            if !out.contains(&h.script.as_str()) {
                out.push(&h.script);
            }
        }
        out
    }
}

fn hook(c: &mut Check, b: &DocBlock, kind: &str, what: &str, names: &[&str]) -> DeviceHook {
    let name = b.label().unwrap_or("").to_owned();
    let at = format!("{kind} {name:?}");
    if b.labels.len() != 1 || !names.contains(&name.as_str()) {
        c.fail(
            &at,
            format!("the label must be one {what}: {}", names.join(", ")),
        );
    }
    let d = &b.body;
    c.items(
        d,
        &[req("script", Form::Str), opt("function", Form::Ident)],
        &[],
        &at,
    );
    c.only_blocks(d, &[], &at);
    let script = string(d, "script").unwrap_or_default();
    if d.field("script").is_some() && !is_name(&script) {
        c.fail(
            &format!("{at}.script"),
            format!("{script:?} is not a valid script name"),
        );
    }
    DeviceHook {
        function: string(d, "function").unwrap_or_else(|| name.clone()),
        script,
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANDROID: &str = r#"handler "sms_received" {
  script = "device_messages"
}

handler "sms_received" {
  script   = "forward"
  function = "on_sms"
}

handler "call_ended" {
  script = "device_calls"
}

command "send_sms" {
  script = "device_messages"
}
"#;

    fn issues(platform: &str, source: &str) -> Vec<String> {
        match DeviceFile::parse(platform, source) {
            Ok(f) => panic!("accepted: {f:?}"),
            Err(inv) => inv.issues.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn a_device_file_lists_handlers_and_commands_in_file_order() {
        let f = DeviceFile::parse("android", ANDROID).unwrap();
        let handlers: Vec<(&str, &str, &str)> = f
            .handlers
            .iter()
            .map(|h| (h.name.as_str(), h.script.as_str(), h.function.as_str()))
            .collect();
        assert_eq!(
            handlers,
            [
                ("sms_received", "device_messages", "sms_received"),
                ("sms_received", "forward", "on_sms"),
                ("call_ended", "device_calls", "call_ended"),
            ]
        );
        let send = f.command("send_sms").unwrap();
        assert_eq!(send.function, "send_sms");
        assert!(f.command("delete_contact").is_none());
        assert_eq!(f.scripts(), ["device_messages", "forward", "device_calls"]);
        assert!(DeviceFile::parse("linux", "").unwrap().handlers.is_empty());
    }

    #[test]
    fn unknown_events_commands_and_platforms_are_errors() {
        let got = issues(
            "android",
            "handler \"sms_deleted\" {\n  script = \"x\"\n}\n",
        );
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(
            got[0]
                .starts_with("handler \"sms_deleted\": the label must be one event: sms_received,"),
            "{got:?}"
        );

        let got = issues("android", "command \"ring\" {\n  script = \"x\"\n}\n");
        assert!(
            got[0].starts_with("command \"ring\": the label must be one command: send_sms,"),
            "{got:?}"
        );

        let got = issues("windows", "");
        assert_eq!(
            got,
            ["file: \"windows\" is not a device platform: linux, android"]
        );
    }

    #[test]
    fn malformed_blocks_are_errors() {
        for (source, expected) in [
            ("handler \"sms_sent\" {\n}\n", "handler \"sms_sent\".script"),
            (
                "handler \"sms_sent\" {\n  script = \"Bad Name\"\n}\n",
                "handler \"sms_sent\".script: \"Bad Name\" is not a valid script name",
            ),
            (
                "handler \"sms_sent\" {\n  script = \"x\"\n  secrets = [\"a\"]\n}\n",
                "handler \"sms_sent\".secrets: is not an item of this block",
            ),
            (
                "command \"send_sms\" {\n  script = \"x\"\n}\n\ncommand \"send_sms\" {\n  script = \"y\"\n}\n",
                "command \"send_sms\": appears more than once",
            ),
            (
                "label = \"Phone\"\n",
                "device file.label: is not an item of this block",
            ),
            (
                "task \"x\" {\n}\n",
                "device file > task \"x\": is not allowed here",
            ),
            (
                "handler {\n  script = \"x\"\n}\n",
                "handler \"\": the label must be one event",
            ),
        ] {
            let got = issues("android", source).join("\n");
            assert!(got.contains(expected), "{source}\n{got}");
        }
    }
}
