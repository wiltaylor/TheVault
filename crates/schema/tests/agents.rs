//! Agent files, `agents/<name>.wcl` (agents G1, G4): every item, its default, and every refusal.

use schema::{
    Agent, AgentConfirm, AgentWeb, MentionTrigger, NoteEvent, Schedule, Schema, Trigger,
    TriggerWhen,
};

const FULL: &str = r#"label         = "Inbox triage"
description   = "Files new email."
icon          = "bot"
enabled       = false
provider      = "anthropic"
model         = "claude-sonnet-5"
effort        = "high"
system_prompt = "You sort email.\nBe brief."

permissions {
  tools = ["search_notes", "get_note", "propose_edit"]
  types = ["email", "task"]

  confirm {
    notes  = false
    delete = true
  }

  web {
    search = false
  }
}

trigger nightly {
  schedule = "0 2 * * *"
  timezone = "Australia/Melbourne"
  prompt   = "Summarise the day."
}

trigger new_email {
  on     = ["create", "update"]
  type   = "email"
  where  = "processed != true"
  prompt = "File this email."
}
"#;

const EMAIL: &str = r#"label = "Email"

field processed {
  kind = "bool"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TASK: &str = r#"label = "Task"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn schema() -> Schema {
    Schema::from_sources(&[("email", EMAIL), ("task", TASK)], &[]).unwrap()
}

/// The issues of `source` as agent `name`, as `at: message`.
fn issues(name: &str, source: &str) -> Vec<String> {
    match Agent::parse(name, source) {
        Ok(a) => a
            .check(&schema())
            .iter()
            .map(|i| format!("{}: {}", i.at, i.message))
            .collect(),
        Err(e) => e
            .issues
            .iter()
            .map(|i| format!("{}: {}", i.at, i.message))
            .collect(),
    }
}

/// The places of the issues of `source`.
fn at(source: &str) -> Vec<String> {
    let all = issues("triage", source);
    all.iter()
        .map(|i| i.split(": ").next().unwrap().to_owned())
        .collect()
}

#[test]
fn every_item_parses() {
    let a = Agent::parse("inbox_triage", FULL).unwrap();
    assert_eq!(a.name, "inbox_triage");
    assert_eq!(a.label, "Inbox triage");
    assert_eq!(a.description.as_deref(), Some("Files new email."));
    assert_eq!(a.icon.as_deref(), Some("bot"));
    assert!(!a.enabled);
    assert_eq!(a.provider.as_deref(), Some("anthropic"));
    assert_eq!(a.model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(a.effort.as_deref(), Some("high"));
    assert_eq!(
        a.system_prompt.as_deref(),
        Some("You sort email.\nBe brief.")
    );
    let p = &a.permissions;
    assert_eq!(
        p.tools.as_deref(),
        Some(
            &[
                "search_notes".to_owned(),
                "get_note".into(),
                "propose_edit".into()
            ][..]
        )
    );
    assert!(p.allows_tool("get_note"));
    assert!(!p.allows_tool("propose_delete"));
    assert_eq!(
        p.types.as_deref(),
        Some(&["email".to_owned(), "task".into()][..])
    );
    assert!(p.allows_type("email"));
    assert!(!p.allows_type("person"));
    assert_eq!(
        p.confirm,
        AgentConfirm {
            notes: false,
            ..AgentConfirm::default()
        }
    );
    assert_eq!(
        p.web,
        AgentWeb {
            search: false,
            ..AgentWeb::default()
        }
    );
    assert_eq!(
        a.triggers,
        [
            Trigger {
                name: "nightly".into(),
                prompt: "Summarise the day.".into(),
                when: TriggerWhen::Schedule(Schedule {
                    cron: "0 2 * * *".into(),
                    timezone: "Australia/Melbourne".into(),
                }),
            },
            Trigger {
                name: "new_email".into(),
                prompt: "File this email.".into(),
                when: TriggerWhen::NoteEvent(NoteEvent {
                    on: vec!["create".into(), "update".into()],
                    types: vec!["email".into()],
                    condition: Some("processed != true".into()),
                }),
            },
        ]
    );
    assert!(a.check(&schema()).is_empty());
    let TriggerWhen::NoteEvent(e) = &a.trigger("new_email").unwrap().when else {
        panic!()
    };
    assert!(e.matches("create", "email"));
    assert!(!e.matches("delete", "email"));
    assert!(!e.matches("create", "task"));
    assert!(e.condition_expr().is_some());
}

#[test]
fn the_format_example_parses() {
    let src = include_str!("../../../docs/vault-format/vault/agents/inbox_triage.wcl");
    let a = Agent::parse("inbox_triage", src).unwrap();
    assert_eq!(a.label, "Inbox triage");
    assert_eq!(a.triggers.len(), 3);
    assert!(!a.permissions.confirm.notes);
    assert!(!a.permissions.web.search);
    assert!(a.check(&schema()).is_empty());
}

#[test]
fn an_empty_file_is_every_default() {
    let a = Agent::parse("helper", "").unwrap();
    assert_eq!(a, Agent::new("helper"));
    assert_eq!(a.label, "helper");
    assert!(a.enabled);
    assert_eq!(a.provider, None);
    assert_eq!(a.model, None);
    assert_eq!(a.effort, None);
    assert_eq!(a.system_prompt, None);
    assert_eq!(a.permissions.tools, None);
    assert_eq!(a.permissions.types, None);
    assert!(a.permissions.allows_tool("anything"));
    assert!(a.permissions.allows_type("anything"));
    assert_eq!(a.permissions.confirm, AgentConfirm::default());
    assert!(a.permissions.confirm.switches().iter().all(|(_, on)| *on));
    assert!(a.permissions.web.switches().iter().all(|(_, on)| *on));
    assert!(a.triggers.is_empty());
    assert!(!a.is_built_in());
    assert!(Agent::parse("assistant", "").unwrap().is_built_in());
}

#[test]
fn an_empty_tool_or_type_list_allows_none() {
    let a = Agent::parse("quiet", "permissions {\n  tools = []\n  types = []\n}\n").unwrap();
    assert!(!a.permissions.allows_tool("get_note"));
    assert!(!a.permissions.allows_type("email"));
}

#[test]
fn a_bad_name_is_refused() {
    for name in ["Triage", "1st", "has-dash", "", &"a".repeat(65), "calls"] {
        assert_eq!(
            issues(name, "")
                .iter()
                .map(|i| i.split(": ").next().unwrap())
                .collect::<Vec<_>>(),
            ["name"],
            "{name:?}"
        );
    }
}

#[test]
fn an_unknown_provider_or_effort_is_refused() {
    assert_eq!(at("provider = \"gemini\"\n"), ["provider"]);
    assert_eq!(at("effort = \"extreme\"\n"), ["effort"]);
    for p in ["anthropic", "openai", "chatgpt"] {
        assert!(at(&format!("provider = \"{p}\"\n")).is_empty(), "{p}");
    }
    for e in ["low", "medium", "high", "xhigh", "max"] {
        assert!(at(&format!("effort = \"{e}\"\n")).is_empty(), "{e}");
    }
}

#[test]
fn each_top_level_item_has_its_form() {
    let src = r#"label = 3
description = true
icon = "not-an-icon-at-all"
enabled = "yes"
model = ""
system_prompt = ""
colour = "red"
permissions = ["get_note"]
"#;
    assert_eq!(
        at(src),
        [
            "colour",
            "permissions",
            "label",
            "description",
            "icon",
            "enabled",
            "model",
            "system_prompt",
        ]
    );
    assert_eq!(at("label = \"  \"\n"), ["label"]);
    // WCL itself refuses a repeated field.
    assert_eq!(at("label = \"A\"\nlabel = \"B\"\n"), ["file"]);
    let long = "x".repeat(schema::AGENT_PROMPT_MAX + 1);
    assert_eq!(
        at(&format!("system_prompt = \"{long}\"\n")),
        ["system_prompt"]
    );
    assert_eq!(at("other {\n}\n"), ["other"]);
}

#[test]
fn permissions_are_checked() {
    let src = r#"permissions {
  tools = ["Get Note"]
  types = ["email", "email"]
  read  = true

  confirm {
    notes   = "no"
    publish = false
  }

  web {
    teleport = false
  }

  other {
  }
}
"#;
    assert_eq!(
        at(src),
        [
            "permissions.read",
            "permissions.tools",
            "permissions.types",
            "permissions.confirm.publish",
            "permissions.confirm.notes",
            "permissions.web.teleport",
            "permissions > other",
        ]
    );
    assert_eq!(
        at("permissions {\n}\n\npermissions {\n}\n"),
        ["permissions"]
    );
}

#[test]
fn an_unknown_type_in_permissions_or_a_trigger_is_refused() {
    let src = r#"permissions {
  types = ["email", "invoice"]
}

trigger mail {
  on     = ["create"]
  type   = ["task", "receipt"]
  prompt = "Look."
}
"#;
    assert_eq!(
        issues("triage", src),
        [
            "permissions.types: there is no type \"invoice\"",
            "trigger mail.type: there is no type \"receipt\"",
        ]
    );
}

#[test]
fn a_bad_cron_or_time_zone_is_refused() {
    let t = |schedule: &str, extra: &str| {
        at(&format!(
            "trigger t {{\n  schedule = \"{schedule}\"\n{extra}  prompt = \"Go.\"\n}}\n"
        ))
    };
    assert_eq!(t("every day", ""), ["trigger t.schedule"]);
    assert_eq!(t("0 2 * *", ""), ["trigger t.schedule"]);
    assert_eq!(t("99 2 * * *", ""), ["trigger t.schedule"]);
    assert_eq!(
        t("0 2 * * *", "  timezone = \"Mars/Olympus\"\n"),
        ["trigger t.timezone"]
    );
    assert!(t("0 7 * * 1-5", "").is_empty());
    let a = Agent::parse(
        "t",
        "trigger t {\n  schedule = \"0 2 * * *\"\n  prompt = \"Go.\"\n}\n",
    )
    .unwrap();
    let TriggerWhen::Schedule(s) = &a.triggers[0].when else {
        panic!()
    };
    assert_eq!(s.timezone, "UTC");
    // 2026-10-12 00:00 UTC; the next 02:00 UTC is two hours later.
    let monday = 1_791_763_200_000;
    assert_eq!(s.next_after(monday), Some(monday + 2 * 3_600_000));
}

#[test]
fn an_unknown_trigger_op_is_refused() {
    let t = |on: &str| {
        at(&format!(
            "trigger t {{\n  on = {on}\n  prompt = \"Go.\"\n}}\n"
        ))
    };
    assert_eq!(t("[\"publish\"]"), ["trigger t.on"]);
    assert_eq!(t("[\"create\", \"create\"]"), ["trigger t.on"]);
    assert_eq!(t("[]"), ["trigger t.on"]);
    assert_eq!(t("\"create\""), ["trigger t.on"]);
    assert!(t("[\"create\", \"update\", \"delete\"]").is_empty());
}

#[test]
fn a_bad_where_query_is_refused() {
    let t = |cond: &str| {
        at(&format!(
            "trigger t {{\n  on = [\"create\"]\n  where = \"{cond}\"\n  prompt = \"Go.\"\n}}\n"
        ))
    };
    assert_eq!(t("processed ==="), ["trigger t.where"]);
    assert_eq!(t("(status == 1"), ["trigger t.where"]);
    assert_eq!(t(" "), ["trigger t.where"]);
    assert!(t("processed != true && title == \\\"invoice\\\"").is_empty());
}

#[test]
fn a_trigger_is_a_schedule_or_a_note_event_with_a_prompt() {
    assert_eq!(at("trigger t {\n  prompt = \"Go.\"\n}\n"), ["trigger t"]);
    assert_eq!(
        at("trigger t {\n  schedule = \"0 2 * * *\"\n  on = [\"create\"]\n  prompt = \"Go.\"\n}\n"),
        ["trigger t"]
    );
    assert_eq!(
        at("trigger t {\n  schedule = \"0 2 * * *\"\n}\n"),
        ["trigger t.prompt"]
    );
    assert_eq!(
        at("trigger t {\n  schedule = \"0 2 * * *\"\n  prompt = \"\"\n}\n"),
        ["trigger t.prompt"]
    );
    assert_eq!(
        at("trigger t {\n  schedule = \"0 2 * * *\"\n  type = \"email\"\n  prompt = \"Go.\"\n}\n"),
        ["trigger t.type"]
    );
    assert_eq!(
        at("trigger t {\n  on = [\"create\"]\n  timezone = \"UTC\"\n  prompt = \"Go.\"\n}\n"),
        ["trigger t.timezone"]
    );
    assert_eq!(
        at("trigger t {\n  on = [\"create\"]\n  colour = \"red\"\n  prompt = \"Go.\"\n}\n"),
        ["trigger t.colour"]
    );
    assert_eq!(
        at("trigger Bad {\n  on = [\"create\"]\n  prompt = \"Go.\"\n}\n"),
        ["trigger Bad"]
    );
    let twice = "trigger t {\n  on = [\"create\"]\n  prompt = \"Go.\"\n}\n\ntrigger t {\n  on = [\"delete\"]\n  prompt = \"Go.\"\n}\n";
    assert_eq!(at(twice), ["trigger t"]);
}

#[test]
fn to_wcl_reads_back_as_the_same_agent() {
    let a = Agent::parse("inbox_triage", FULL).unwrap();
    let text = a.to_wcl();
    assert_eq!(Agent::parse("inbox_triage", &text).unwrap(), a);
    // Defaults are left out.
    assert_eq!(Agent::new("helper").to_wcl(), "");
    let mut b = Agent::new("helper");
    b.label = "Say \"hi\"\\ now".into();
    b.permissions.tools = Some(Vec::new());
    let text = b.to_wcl();
    assert_eq!(Agent::parse("helper", &text).unwrap(), b);
}

#[test]
fn a_mention_trigger_takes_type_and_an_optional_prompt() {
    let src = "trigger asked {\n  mention = true\n  type = [\"email\", \"task\"]\n  prompt = \"Be brief.\"\n}\n\ntrigger anyone {\n  mention = true\n}\n";
    let a = Agent::parse("clerk", src).unwrap();
    assert_eq!(
        a.triggers,
        [
            Trigger {
                name: "asked".into(),
                prompt: "Be brief.".into(),
                when: TriggerWhen::Mention(MentionTrigger {
                    types: vec!["email".into(), "task".into()],
                }),
            },
            Trigger {
                name: "anyone".into(),
                prompt: String::new(),
                when: TriggerWhen::Mention(MentionTrigger::default()),
            },
        ]
    );
    assert!(a.check(&schema()).is_empty());
    assert_eq!(a.mention_trigger("email").unwrap().name, "asked");
    assert_eq!(a.mention_trigger("note").unwrap().name, "anyone");
    // Written back, the missing prompt stays missing.
    let text = a.to_wcl();
    assert!(!text.contains("prompt = \"\""), "{text}");
    assert_eq!(Agent::parse("clerk", &text).unwrap(), a);
}

#[test]
fn a_mention_trigger_is_checked() {
    assert_eq!(
        at("trigger t {\n  mention = false\n}\n"),
        ["trigger t.mention"]
    );
    assert_eq!(
        at("trigger t {\n  mention = \"yes\"\n}\n"),
        ["trigger t.mention"]
    );
    assert_eq!(
        at("trigger t {\n  mention = true\n  on = [\"create\"]\n}\n"),
        ["trigger t"]
    );
    assert_eq!(
        at("trigger t {\n  mention = true\n  where = \"done\"\n}\n"),
        ["trigger t.where"]
    );
    assert_eq!(
        at("trigger t {\n  mention = true\n  prompt = \" \"\n}\n"),
        ["trigger t.prompt"]
    );
    assert_eq!(
        issues(
            "clerk",
            "trigger t {\n  mention = true\n  type = \"nope\"\n}\n"
        ),
        ["trigger t.type: there is no type \"nope\""]
    );
}
