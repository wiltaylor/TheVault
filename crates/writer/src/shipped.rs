//! Keeping a vault's shipped defaults up to date: the built-in types, the default device file and
//! the default device scripts, the Google sync's scripts and tasks, and the one-time upgrade steps
//! (D15, D16, D24; google-sync D19).
//!
//! [`Writer::upgrade_shipped_defaults`] runs on every start of the home server and of the
//! embedded writer. It does four things:
//!
//! 1. **Upgrade steps** ([`UPGRADE_STEPS`]), on the home server only, each applied once in order
//!    and recorded in `state.db`. They run first, because a step such as a field rename must see
//!    the vault before the additive seeding below adds the field under its new name.
//! 2. **Built-in taxonomies and types.** A built-in taxonomy the vault lacks is added whole, and
//!    is never changed once the vault has it. A type the vault lacks is added whole. A built-in type the vault has,
//!    and the `person` type the phone data types reference, gain every field the shipped default
//!    declares and the vault's type lacks, as a planned schema change that edits no note. A field
//!    the vault has is never removed, renamed or changed. Each of the vault's `default` and
//!    `card` templates gains a slot for an added field when the shipped template of the same name
//!    has one, at any depth (inside `columns > column` too), after the slot the shipped template
//!    puts before it, else before the one it puts after it (ticket 148). A vault type holding a
//!    built-in's name that is not the built-in ([`Writer::is_built_in`]), such as the user's own
//!    `event`, is skipped with one line in the log, and a built-in type found by its identity
//!    rather than by a record is recorded in `state.db` as the server's (ticket 104).
//! 3. **Missing device files, tasks, scripts and agents.** Each file of [`SHIPPED_FILES`] the vault lacks
//!    is written, the scripts before the device file or task that names them.
//! 4. **Shipped file upgrades.** A file of [`SHIPPED_FILES`] whose content is still a version the
//!    app shipped is replaced by the current one. A file the user changed is left alone, and the
//!    user hears once, under the bell, that a newer shipped version exists. A task file's `paused`
//!    field is the user's pause switch, not an edit: [`shipped_hash`] leaves it out, and an
//!    upgrade carries the vault's `paused` value into the new version (google-sync D19).
//!
//! Steps 2 to 4 are one commit, and a start that finds nothing to do commits nothing.

use std::collections::BTreeSet;

use schema::{Doc, Schema, TypeDef, Value as DocValue};
use sha2::{Digest, Sha256};
use state::{Author, NewNotification};
use store::Change;
use wcl_lang::ast::{Block, Expr, Item, Source};

use crate::builtin::{
    BUILT_IN_TAXONOMIES, BUILT_IN_TYPES, DEFAULT_EMAIL_TYPE, DEFAULT_EVENT_TYPE,
    DEFAULT_MESSAGE_TYPE, DEFAULT_PERSON_TYPE, EMAIL_TYPE, EVENT_TYPE, MESSAGE_TYPE, PERSON_TYPE,
    TYPES_NEEDING_PERSON,
};
use crate::defs::taxonomy_path;
use crate::types::{NativeMigration, TypeChange, rebuild_schema};
use crate::{Result, Writer, pairs, type_path};

/// The author of the changes the server makes to keep its shipped defaults up to date.
pub const SERVER_AUTHOR: &str = "server";

/// A file the app ships into every vault, and every version of it the app has ever shipped.
#[derive(Debug, Clone, Copy)]
pub struct ShippedFile {
    /// The vault path, such as `scripts/device_calls.ws`.
    pub path: &'static str,
    /// The current shipped content.
    pub source: &'static str,
    /// The [`shipped_hash`] of every version ever shipped, oldest first, ending with the current
    /// one.
    /// Changing a shipped file means appending its new hash here: a vault holding any listed
    /// version is upgraded, so one that skipped releases still is.
    pub hashes: &'static [&'static str],
}

/// The shipped files: the default device scripts, then the device file that names them, then the
/// Google sync's scripts, each before the task that names it (google-sync D19), the Outbox's send
/// handler (outbox X3), then the built-in agent (agents G6).
pub const SHIPPED_FILES: [ShippedFile; 13] = [
    ShippedFile {
        path: "scripts/device_messages.ws",
        source: include_str!("../defaults/devices/device_messages.ws"),
        hashes: &[
            "0120b5326e2e611d79c56abd62b705851c7762f376d813bcadfa5b2d67e8cb64",
            "cacc4fa862ca2b36215187e1ff71632930e357f8ee926f6faca3896280819b80",
        ],
    },
    ShippedFile {
        path: "scripts/device_calls.ws",
        source: include_str!("../defaults/devices/device_calls.ws"),
        hashes: &[
            "35aac11b54ac323113df67c13c778e03f607dd910d5045cb6bb62518a9c82ef2",
            "99f1e44e976ad6961ab54f74784a8d20eb4a56ac6dff32c9f83cf3e6257af848",
        ],
    },
    ShippedFile {
        path: "scripts/device_contacts.ws",
        source: include_str!("../defaults/devices/device_contacts.ws"),
        hashes: &[
            "4791759aabba0aaf89f52fb547b0640421ac0dbe158664c35a7152a1b359e7ee",
            "3310afb4541dd79a72a4a734adc07b5ec8ce59199a23a7ede53b866cba6a10be",
            "057f224d49972d84bb8bb7f41bf3f92b3ed344c9a6e74fc1b9500956781adab2",
            "6e72f49fc4eb2ca77f12d6771f8ce052d1cc7f2a568944113ffd207ba6953fa5",
        ],
    },
    ShippedFile {
        path: "scripts/device_share.ws",
        source: include_str!("../defaults/devices/device_share.ws"),
        hashes: &[
            "f5d4068ef71f446ab63cce6f59cf13947de27b533a9a0b700f5c673022252fb9",
            "eabb33ed28d9ee43b28246e8a4e4cbde562294552d4e684438681bc6fc39fc5b",
        ],
    },
    ShippedFile {
        path: "scripts/device_import.ws",
        source: include_str!("../defaults/devices/device_import.ws"),
        hashes: &["580f26f714a8b67262e7d0d3e10a131f94be5a2bde7eb643cb8199bee55210eb"],
    },
    ShippedFile {
        path: "devices/android.wcl",
        source: include_str!("../defaults/devices/android.wcl"),
        hashes: &[
            "82f1e219cdc77ac5a8fb698a9ab15fa2d4795049ca1705877744135e220e21f0",
            "02a8b77d470a9740cfeaa0cbf1ccafff99ffc2447cd8edff79a95e853880c411",
        ],
    },
    ShippedFile {
        path: "scripts/google_calendar.ws",
        source: include_str!("../defaults/google/google_calendar.ws"),
        hashes: &[
            "c3f42196166b550d457a9c1f3aa9db7b89c0502a36924efc072448bd61e70e64",
            "c8645c85e00a3fe1ede2ac7e41e98788d6936550744afc791261e7ce80a7266b",
            "a8caf6855a648b44dfe49dd682e246bb87a0ef921ba6b85cd3a36a9585f30da9",
            "05974f071ad03c95a1ad2627ece883f9b9acdf76ed1b33a42c7cc5c114579b24",
        ],
    },
    ShippedFile {
        path: "tasks/google_calendar.wcl",
        source: include_str!("../defaults/google/tasks/google_calendar.wcl"),
        hashes: &[
            "2fdbbc5d2d7417391ca5d633fd756efade9a79b858c372b0df5645f545967709",
            "8ab13d82a86b53887922f967406c5b7661e1e8f49e2018cd163f28902dca6a40",
        ],
    },
    ShippedFile {
        path: "scripts/google_gmail.ws",
        source: include_str!("../defaults/google/google_gmail.ws"),
        hashes: &[
            "40629abecc581c745657b98ad630af864b35e87eb287d0217103ea8dbd4288c7",
            "0d42ff00a0888f1a7b51fc693e75e2b150373d3a22a17f085aa76dffdc7d802a",
            "a4f144978e8d41264463826d12ca54bb75e765b0401b1e9bdc8a74204f7c54b8",
            "25bb9e337bdb15c08a696f5b96a2416ac078aafd43afd51673799528a188cd84",
            "b4782f4a6b52f51dedc82f1269f290d8a8fdad7404667526089afdee0b63b1aa",
        ],
    },
    ShippedFile {
        path: "tasks/google_gmail.wcl",
        source: include_str!("../defaults/google/google_gmail.wcl"),
        hashes: &[
            "94f9b68f9e0f6a9bcb001ce06304c1a791fb5dad6bb5eb5ed1cbead2d8997c51",
            "07fca002b04da1462058d2959114609243cf4068dec3dcd383f1bc93d641b2d4",
        ],
    },
    // The Google Calendar write-back, named by the shipped `event` type's handlers.
    ShippedFile {
        path: "scripts/google_event.ws",
        source: include_str!("../defaults/google/google_event.ws"),
        hashes: &[
            "5f4a59e376bae6ab443affc0c0fdf241b1f26347a870f35ab03cedf110584cd3",
            "87423adde9c434f260c4d07f2cb1196cb7d89a91411c783838391cf2434c9e1d",
            "dc7c8f91ab0b2c8bdda7e31ccd2422b4edc628f0beb7a11b7dbb45c52fe15b7b",
            "0d9a17a114dfdc774cef98bca4ec1b885218afeece1b4e52d8fc0bcaf3d4b573",
        ],
    },
    // The Outbox's send handler, named by the shipped `outbox` type's handlers (outbox X3).
    ShippedFile {
        path: "scripts/outbox_send.ws",
        source: include_str!("../defaults/outbox_send.ws"),
        hashes: &[
            "48d6e8cc94b6bcac9a858a539391d2e9e3e8e8bbe96d4bda2b039632399ab461",
            "bc27a3a0c03cc59ac5a20539e639d3d6845b69ad377f57d26e333f6aefe40ab8",
        ],
    },
    // The built-in agent, the chat panel's default (agents G6).
    ShippedFile {
        path: "agents/assistant.wcl",
        source: include_str!("../defaults/agents/assistant.wcl"),
        hashes: &["8662bc8dbbf4601e72392d38c20a4cd01fdf554b0bf90d3b1a083557d64dc7ba"],
    },
];

/// One one-time vault upgrade: a change that is not additive, such as a field rename with a
/// migration script. It runs once per vault, on the home server, before the shipped defaults are
/// seeded, and is recorded in `state.db` by its id once it returns `Ok`. A step that fails stops
/// the server from starting and runs again on the next start.
///
/// A step that cannot run yet, such as one that needs the secrets key on a server started without
/// it, returns [`crate::Error::StepDeferred`]: it is logged and not recorded, so it runs again on
/// the next start, and the steps after it still run. A step only defers when no later step
/// depends on what it does.
///
/// A step makes its changes through the writer's own calls, such as
/// [`Writer::plan_type_change`] and [`Writer::apply_type_change`], authored by
/// [`SERVER_AUTHOR`]. It must leave a vault already in the shape it produces unchanged, because a
/// new vault gets the current shipped defaults and still runs every step once.
#[derive(Debug, Clone, Copy)]
pub struct UpgradeStep {
    /// A unique, never reused id. Steps run in the order of [`UPGRADE_STEPS`], so number them:
    /// `0001_person_emails`.
    pub id: &'static str,
    pub run: fn(&Writer) -> Result<()>,
}

/// Every one-time vault upgrade, in the order they run. Adding one is one line here, such as
/// `UpgradeStep { id: "0001_person_emails", run: person_emails },` with `person_emails` a
/// `fn(&Writer) -> Result<()>` beside it. Never remove or reorder a step that has shipped.
pub const UPGRADE_STEPS: &[UpgradeStep] = &[
    UpgradeStep {
        id: "0001_person_emails",
        run: person_emails,
    },
    UpgradeStep {
        id: "0002_person_labels",
        run: person_labels,
    },
    UpgradeStep {
        id: "0003_event_google_handler",
        run: event_google_handler,
    },
    UpgradeStep {
        id: "0004_remove_google_sync",
        run: remove_google_sync,
    },
    UpgradeStep {
        id: "0005_list_page_defaults",
        run: list_page_defaults,
    },
    UpgradeStep {
        id: "0006_assistant_settings",
        run: assistant_settings,
    },
];

/// The secret holding Settings > AI (crates/ai's `ai_settings`, ai D7).
const AI_SETTINGS: &str = "ai_settings";

/// Step `0006_assistant_settings` (agents G6): the chat panel's system prompt and its
/// confirmation and internet switches (ai D17, D22, D28) moved from `ai_settings` to the built-in
/// agent, `agents/assistant.wcl`. Each of the three that `ai_settings` holds and that differs from
/// the default is written into the agent file when the file has none of its own: a prompt when
/// the file names no `system_prompt`, a switch turned off when the file's `permissions.confirm`
/// (or `web`) are all on. Then the three keys are taken out of `ai_settings`, keeping the rest.
///
/// A vault from before agent files has no `agents/assistant.wcl` yet: the step writes the shipped
/// one with the moved settings and records the shipped version as written, so the seeding after
/// it keeps the file without telling the user of a newer version. Without the secrets key and with
/// `ai_settings` stored, it defers. A fresh vault has no `ai_settings` and changes nothing.
fn assistant_settings(w: &Writer) -> Result<()> {
    use serde_json::Value;

    let text = match w.state.get_secret(AI_SETTINGS) {
        Ok(Some(text)) => text,
        Ok(None) => return Ok(()),
        Err(state::Error::KeyMissing { .. }) => {
            return Err(crate::Error::StepDeferred(
                "step 0006_assistant_settings needs the secrets key to read ai_settings: start \
                 the server with its key file to finish it"
                    .to_owned(),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let Ok(Value::Object(mut settings)) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    let prompt = settings.remove("system_prompt");
    let confirm = settings.remove("confirm");
    let web = settings.remove("web");
    if prompt.is_none() && confirm.is_none() && web.is_none() {
        return Ok(());
    }
    let name = schema::ASSISTANT_AGENT;
    let path = crate::agent_path(name);
    let shipped = SHIPPED_FILES
        .iter()
        .find(|f| f.path == path)
        .expect("the built-in agent is a shipped file");
    let (source, missing) = match w.read_agent(name) {
        Ok(text) => (text, false),
        Err(crate::Error::Missing(_)) => (shipped.text()?, true),
        Err(e) => return Err(e),
    };
    let mut agent = match schema::Agent::parse(name, &source) {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "step 0006_assistant_settings left ai_settings as it is: {path} does not parse: {e}"
            );
            return Ok(());
        }
    };
    let before = agent.clone();
    if let Some(Value::String(p)) = &prompt
        && !p.trim().is_empty()
        && agent.system_prompt.is_none()
    {
        agent.system_prompt = Some(p.clone());
    }
    let switches_off = |v: &Option<Value>| -> Vec<String> {
        v.as_ref()
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter(|(_, on)| on.as_bool() == Some(false))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    if agent.permissions.confirm == schema::AgentConfirm::default() {
        for k in switches_off(&confirm) {
            agent.permissions.confirm.set(&k, false);
        }
    }
    if agent.permissions.web == schema::AgentWeb::default() {
        for k in switches_off(&web) {
            agent.permissions.web.set(&k, false);
        }
    }
    if agent != before {
        if missing {
            let hash = shipped_hash(shipped.path, &shipped.text()?);
            w.state.record_shipped(shipped.path, &hash)?;
        }
        w.put_agent(name, &agent.to_wcl(), |_| Vec::new())?;
    }
    let text = Value::Object(settings).to_string();
    w.state.set_secret(AI_SETTINGS, &text)?;
    Ok(())
}

/// Step `0005_list_page_defaults` (lists L5): the `email` and `message` types from before type
/// list pages had settings have no `list_page` block, and the seeding only ever adds fields to a
/// type the vault has. Each of the two that is the built-in and has no `list_page` block gains
/// the shipped type's, after its last item, as one planned type change that edits no note.
/// Everything else in the file stays as it is, so a type the user edited, such as one with a slot
/// added to its card, keeps the edit. A type that has a `list_page` block, whatever it holds, is
/// left alone, and so is a type of the user's holding the name. When the vault's type lacks the
/// field the shipped `sort` names, the block goes in without its `sort`.
fn list_page_defaults(w: &Writer) -> Result<()> {
    for (name, source) in [
        (EMAIL_TYPE, DEFAULT_EMAIL_TYPE),
        (MESSAGE_TYPE, DEFAULT_MESSAGE_TYPE),
    ] {
        add_list_page(w, name, source)?;
    }
    Ok(())
}

/// Give vault type `name` the `list_page` block of its shipped default `source`, when it is the
/// built-in and has none.
fn add_list_page(w: &Writer, name: &str, source: &str) -> Result<()> {
    let schema = w.schema();
    let Some(ty) = schema.type_def(name) else {
        return Ok(());
    };
    if ty.list_page.is_some() {
        return Ok(());
    }
    if !w.is_built_in_def(ty) {
        eprintln!(
            "step 0005_list_page_defaults skipped type {name:?}: a type of the user's holds the name"
        );
        return Ok(());
    }
    let path = type_path(name);
    let shipped: Source = store::parse_data_only(source, &path)?;
    let mut list_page = shipped
        .items
        .into_iter()
        .find(|i| block(i, "list_page").is_some())
        .expect("the shipped type has a list_page block");
    if let Some(b) = block_mut(&mut list_page, "list_page") {
        let lacks_sort_field = |i: &Item| {
            matches!(i, Item::Field(f) if f.name == "sort"
                && matches!(&f.expr, Expr::Utf8(s)
                    if s.split_whitespace().next().is_some_and(|n| ty.field(n).is_none())))
        };
        b.items.retain(|i| !lacks_sort_field(i));
    }
    let mut ast = store::parse_data_only(&w.vault.read_text(&path)?, &path)?;
    ast.items.push(list_page);
    let change = TypeChange {
        wcl: store::to_canonical(&ast),
        ..TypeChange::default()
    };
    let plan = w.plan_type_change(name, &change)?;
    if !plan.changes.is_empty() {
        eprintln!(
            "step 0005_list_page_defaults was not applied to type {name:?}: it would edit notes"
        );
        return Ok(());
    }
    w.apply_type_change(
        name,
        &change,
        &plan.token,
        &Author::Task(SERVER_AUTHOR.to_owned()),
    )?;
    Ok(())
}

/// The settings type of 0.1.7, which step `0004_remove_google_sync` removes.
const GOOGLE_SYNC_TYPE: &str = "google_sync";

/// A field the 0.1.7 `google_sync` type declares, by which step `0004_remove_google_sync` tells
/// it from a type of the user's holding the name, which it leaves alone (ticket 104).
const GOOGLE_SYNC_KEY: &str = "gmail_history_id";

/// Step `0004_remove_google_sync` (google-sync D21): the Google sync's settings and state moved
/// from the `google_sync` note type into secrets. The step:
///
/// 1. gives every handler of the `event` type naming `google_event` the `google_settings` secret
///    it now reads, as one planned type change that edits no note;
/// 2. when the vault has a `google_sync` note, moves the first one's settings, cursors and account
///    into the four secrets of [`crate::google`], setting only keys a secret does not hold
///    already. Without the secrets key it changes nothing more and defers, to run again on the
///    next start;
/// 3. deletes every `google_sync` note for good, not to the trash, then the type.
///
/// A `google_sync` type without the 0.1.7 type's `gmail_history_id` is the user's own: steps 2
/// and 3 leave it and its notes alone.
fn remove_google_sync(w: &Writer) -> Result<()> {
    event_handlers_read_settings(w)?;
    let Some(ty) = w.schema().type_def(GOOGLE_SYNC_TYPE).cloned() else {
        return Ok(());
    };
    if ty.field(GOOGLE_SYNC_KEY).is_none() {
        eprintln!(
            "step 0004_remove_google_sync left type \"google_sync\" alone: a type of the user's \
             holds the name"
        );
        return Ok(());
    }
    let notes = w.note_ids(GOOGLE_SYNC_TYPE)?;
    if let Some(first) = notes.first() {
        if !w.state.has_secret_key() {
            let why = "step 0004_remove_google_sync needs the secrets key to move the google_sync \
                       note into secrets: start the server with its key file to finish it";
            return Err(crate::Error::StepDeferred(why.to_owned()));
        }
        let note = w.read(first)?;
        move_google_sync_note(w, &crate::doc_to_json(&note.doc))?;
    }
    let author = Author::Task(SERVER_AUTHOR.to_owned());
    for id in &notes {
        w.delete(id, Some(&w.read(id)?.version), &author)?;
        w.state.purge_from_trash(id)?;
    }
    w.delete_type(GOOGLE_SYNC_TYPE)?;
    Ok(())
}

/// Move the settings, cursors and account of a 0.1.7 `google_sync` note, in its JSON form, into
/// the Google secrets, keeping every key a secret already holds.
fn move_google_sync_note(w: &Writer, note: &serde_json::Value) -> Result<()> {
    use serde_json::{Map, Value};

    use crate::google;

    let fields = &note["fields"];
    let dict = |name: &str| -> Option<Map<String, Value>> {
        note["blocks"].as_array()?.iter().find_map(|b| {
            (b["kind"] == "dict" && b["labels"][0] == name)
                .then(|| b["fields"].as_object().cloned().unwrap_or_default())
        })
    };
    let field = |name: &str| fields.get(name).cloned().unwrap_or(Value::Null);
    let invalid = |name: &str| {
        Value::Bool(
            fields[name]
                .as_str()
                .is_some_and(|e| e.starts_with("invalid_grant")),
        )
    };

    let mut settings = google::default_settings();
    for key in [
        "gmail_mode",
        "gmail_labels",
        "gmail_window_days",
        "gmail_backfill_to",
        "calendars",
        "default_calendar",
    ] {
        if let Some(v) = fields.get(key) {
            settings.insert(key.to_owned(), v.clone());
        }
    }
    if let Some(e) = dict("endpoints") {
        settings.insert("endpoints".to_owned(), Value::Object(e));
    }

    let mut gmail = Map::new();
    gmail.insert("history_id".into(), field("gmail_history_id"));
    gmail.insert("backfill_page".into(), field("gmail_backfill_page"));
    gmail.insert("last_run".into(), field("last_gmail_run"));
    gmail.insert("last_error".into(), field("last_gmail_error"));
    gmail.insert("notified_invalid_grant".into(), invalid("last_gmail_error"));

    let tokens: Map<String, Value> = dict("calendar_sync_tokens")
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (calendar_of_token_key(&k), v))
        .collect();
    let mut calendar = Map::new();
    calendar.insert("sync_tokens".into(), Value::Object(tokens));
    calendar.insert("window_end".into(), field("calendar_window_end"));
    calendar.insert("last_run".into(), field("last_calendar_run"));
    calendar.insert("last_error".into(), field("last_calendar_error"));
    calendar.insert(
        "notified_invalid_grant".into(),
        invalid("last_calendar_error"),
    );

    for (name, moved) in [
        (google::SETTINGS, settings),
        (google::GMAIL_STATE, gmail),
        (google::CALENDAR_STATE, calendar),
    ] {
        let mut kept = google::read_object(&w.state, name)?;
        let before = kept.clone();
        for (k, v) in moved {
            kept.entry(k).or_insert(v);
        }
        if kept != before {
            google::write_object(&w.state, name, &kept)?;
        }
    }
    if let Some(account) = fields["account"].as_str().filter(|a| !a.is_empty())
        && w.state.get_secret(google::ACCOUNT)?.is_none()
    {
        w.state.set_secret(google::ACCOUNT, account)?;
    }
    Ok(())
}

/// The calendar id of a 0.1.7 `calendar_sync_tokens` key: `c_` and the id with each character
/// other than a letter or digit written as `_` and its two hex digits. A key not in that form is
/// kept as it is.
fn calendar_of_token_key(key: &str) -> String {
    let Some(rest) = key.strip_prefix("c_") else {
        return key.to_owned();
    };
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c != '_' {
            out.push(c);
            continue;
        }
        let hex: String = chars.by_ref().take(2).collect();
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some(c) if hex.len() == 2 => out.push(c),
            _ => return key.to_owned(),
        }
    }
    out
}

/// The secret the write-back now reads besides the three credentials (google-sync D21).
const SETTINGS_SECRET: &str = "google_settings";

/// Give every handler of the vault's `event` type that names `google_event` the
/// `google_settings` secret, as one planned type change that edits no note.
fn event_handlers_read_settings(w: &Writer) -> Result<()> {
    let schema = w.schema();
    let Some(ty) = schema.type_def(EVENT_TYPE) else {
        return Ok(());
    };
    let lacking = ty.handlers.iter().any(|h| {
        h.script == EVENT_HANDLER_SCRIPT && !h.secrets.iter().any(|s| s == SETTINGS_SECRET)
    });
    if !lacking {
        return Ok(());
    }
    let path = type_path(EVENT_TYPE);
    let mut ast = store::parse_data_only(&w.vault.read_text(&path)?, &path)?;
    for item in &mut ast.items {
        let Some(handler) = block_mut(item, "handler") else {
            continue;
        };
        let names_script = handler.items.iter().any(|i| {
            matches!(i, Item::Field(f) if f.name == "script"
                && matches!(&f.expr, Expr::Utf8(s) if s == EVENT_HANDLER_SCRIPT))
        });
        if !names_script {
            continue;
        }
        let mut secrets: Vec<String> = handler
            .items
            .iter()
            .find_map(|i| match i {
                Item::Field(f) if f.name == "secrets" => match &f.expr {
                    Expr::ListLit { elements, .. } => Some(
                        elements
                            .iter()
                            .filter_map(|e| match e {
                                Expr::Utf8(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect(),
                    ),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or_default();
        if secrets.iter().any(|s| s == SETTINGS_SECRET) {
            continue;
        }
        secrets.push(SETTINGS_SECRET.to_owned());
        let src = store::parse_data_only(&format!("secrets = {secrets:?}\n"), &path)?;
        let field = src.items.into_iter().next().expect("one attribute");
        match handler
            .items
            .iter()
            .position(|i| matches!(i, Item::Field(f) if f.name == "secrets"))
        {
            Some(k) => handler.items[k] = field,
            None => handler.items.push(field),
        }
    }
    let change = TypeChange {
        wcl: store::to_canonical(&ast),
        ..TypeChange::default()
    };
    let plan = w.plan_type_change(EVENT_TYPE, &change)?;
    if !plan.changes.is_empty() {
        eprintln!(
            "step 0004_remove_google_sync left the event handlers alone: it would edit notes"
        );
        return Ok(());
    }
    w.apply_type_change(
        EVENT_TYPE,
        &change,
        &plan.token,
        &Author::Task(SERVER_AUTHOR.to_owned()),
    )?;
    Ok(())
}

/// The script the shipped `event` type's write-back handlers name (google-sync D9, D10, D20).
const EVENT_HANDLER_SCRIPT: &str = "google_event";

/// Step `0003_event_google_handler` (google-sync D9, D10, D20): an `event` type from before the
/// write-back to Google Calendar has no handler naming `google_event`, and the seeding only ever
/// adds fields to a type the vault has. One planned type change that edits no note gives it the
/// shipped type's `handler` blocks, before its first template. A vault without the type gets it
/// whole, handlers included, from the seeding.
fn event_google_handler(w: &Writer) -> Result<()> {
    let schema = w.schema();
    let Some(ty) = schema.type_def(EVENT_TYPE) else {
        return Ok(());
    };
    if ty.handlers.iter().any(|h| h.script == EVENT_HANDLER_SCRIPT) {
        return Ok(());
    }
    if !w.is_built_in_def(ty) {
        eprintln!(
            "step 0003_event_google_handler skipped type \"event\": a type of the user's holds the name"
        );
        return Ok(());
    }
    let path = type_path(EVENT_TYPE);
    let shipped: Source = store::parse_data_only(DEFAULT_EVENT_TYPE, &path)?;
    let handlers: Vec<Item> = shipped
        .items
        .into_iter()
        .filter(|i| block(i, "handler").is_some())
        .collect();
    let mut ast = store::parse_data_only(&w.vault.read_text(&path)?, &path)?;
    let at = ast
        .items
        .iter()
        .position(|i| block(i, "template").is_some())
        .unwrap_or(ast.items.len());
    ast.items.splice(at..at, handlers);
    let change = TypeChange {
        wcl: store::to_canonical(&ast),
        ..TypeChange::default()
    };
    let plan = w.plan_type_change(EVENT_TYPE, &change)?;
    if !plan.changes.is_empty() {
        eprintln!("step 0003_event_google_handler was not applied: it would edit notes");
        return Ok(());
    }
    w.apply_type_change(
        EVENT_TYPE,
        &change,
        &plan.token,
        &Author::Task(SERVER_AUTHOR.to_owned()),
    )?;
    Ok(())
}

/// The person fields whose values carry labels, each with the field holding its labels.
const LABELLED: [(&str, &str); 3] = [
    ("emails", "email_labels"),
    ("phones", "phone_labels"),
    ("addresses", "address_labels"),
];

/// Step `0002_person_labels` (D16): `emails`, `phones` and `addresses` on a `person` type from
/// before a value and its label were edited as one chip are lists of strings with no `labels`.
/// One planned type change that edits no note gives each of the three that the type has its
/// `labels`, adding the shipped label field after it when the type lacks it; gives `photo`
/// `accept = "image"` when every photo in the vault is an image; and takes the slots of the three
/// label fields out of the `default` template. Nothing else the user changed in the type is
/// touched. A field the type lacks here, such as postal `addresses` before step
/// `0001_person_emails` freed the name, comes with its `labels` from the seeding that runs next.
fn person_labels(w: &Writer) -> Result<()> {
    let schema = w.schema();
    let Some(ty) = schema.type_def(PERSON_TYPE) else {
        return Ok(());
    };
    let is_strings = |name: &str| {
        ty.field(name).is_some_and(|f| {
            matches!(
                f.kind,
                schema::FieldKind::List {
                    items: schema::Scalar::String
                }
            )
        })
    };
    if ty.fields.iter().any(|f| f.labels.is_some()) {
        return Ok(());
    }
    // Each value field to label, and whether its label field must be added.
    let labelled: Vec<(&str, &str, bool)> = LABELLED
        .iter()
        .filter(|(v, l)| is_strings(v) && (is_strings(l) || ty.field(l).is_none()))
        .map(|(v, l)| (*v, *l, ty.field(l).is_none()))
        .collect();
    if labelled.is_empty() {
        return Ok(());
    }
    let photo_images = ty
        .field("photo")
        .is_some_and(|f| f.kind == schema::FieldKind::Asset && f.accept == schema::Accept::Any)
        && every_photo_is_an_image(w)?;
    let text = w.vault.read_text(&type_path(PERSON_TYPE))?;
    let change = TypeChange {
        wcl: person_with_labels(&text, &labelled, photo_images)?,
        ..TypeChange::default()
    };
    let plan = w.plan_type_change(PERSON_TYPE, &change)?;
    if !plan.changes.is_empty() {
        eprintln!("step 0002_person_labels was not applied: it would edit notes");
        return Ok(());
    }
    w.apply_type_change(
        PERSON_TYPE,
        &change,
        &plan.token,
        &Author::Task(SERVER_AUTHOR.to_owned()),
    )?;
    Ok(())
}

/// Whether every `photo` of every person note names an asset in the vault whose file is an
/// image. An asset the vault folder lacks is not known to be one.
fn every_photo_is_an_image(w: &Writer) -> Result<bool> {
    for id in w.note_ids(PERSON_TYPE)? {
        let path = crate::note_path(PERSON_TYPE, &id);
        let doc = crate::Input::Wcl(w.vault.read_text(&path)?).parse(&path)?;
        let ids: Vec<&str> = match doc.field("photo") {
            None => Vec::new(),
            Some(DocValue::Str(s)) => vec![s.as_str()],
            Some(DocValue::List(l)) => l.iter().filter_map(DocValue::as_str).collect(),
            Some(_) => return Ok(false),
        };
        for asset in ids {
            let image = w
                .find_asset(asset)
                .is_some_and(|a| schema::is_image_filename(&a.filename));
            if !image {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// The person type file `vault_text` with `labels` on each value field of `labelled`, the
/// shipped label field after it where `labelled` says the type lacks it, `accept = "image"` on
/// `photo` when `photo_images`, and no slot of a label field in the `default` template.
fn person_with_labels(
    vault_text: &str,
    labelled: &[(&str, &str, bool)],
    photo_images: bool,
) -> Result<String> {
    let path = type_path(PERSON_TYPE);
    let shipped: Source = store::parse_data_only(DEFAULT_PERSON_TYPE, &path)?;
    let shipped_field = |name: &str| {
        shipped
            .items
            .iter()
            .find(|i| block(i, "field").and_then(label) == Some(name))
            .cloned()
            .expect("the shipped person type has the label fields")
    };
    let mut ast = store::parse_data_only(vault_text, &path)?;
    let attribute = |name: &str, value: &str| -> Result<Item> {
        let src = store::parse_data_only(&format!("{name} = {value:?}\n"), &path)?;
        Ok(src.items.into_iter().next().expect("one attribute"))
    };
    let mut k = 0;
    while k < ast.items.len() {
        let mut insert = None;
        if let Some(field) = block_mut(&mut ast.items[k], "field") {
            let name = label(field).unwrap_or_default().to_owned();
            if let Some((_, labels, add)) = labelled.iter().find(|(v, _, _)| *v == name) {
                field.items.push(attribute("labels", labels)?);
                if *add {
                    insert = Some(shipped_field(labels));
                }
            } else if name == "photo" && photo_images {
                field
                    .items
                    .retain(|i| !matches!(i, Item::Field(f) if f.name == "accept"));
                field.items.push(attribute("accept", "image")?);
            }
        }
        if let Some(item) = insert {
            ast.items.insert(k + 1, item);
            k += 1;
        }
        k += 1;
    }
    let label_fields: Vec<&str> = LABELLED.iter().map(|(_, l)| *l).collect();
    for item in &mut ast.items {
        if let Some(template) = block_mut(item, "template").filter(|b| label(b) == Some("default"))
        {
            drop_slots(&mut template.items, &label_fields);
        }
    }
    Ok(store::to_canonical(&ast))
}

/// Remove every slot of one of `fields` from `items` and the blocks they hold.
fn drop_slots(items: &mut Vec<Item>, fields: &[&str]) {
    items.retain(|i| {
        !block(i, "slot")
            .and_then(label)
            .is_some_and(|f| fields.contains(&f))
    });
    for item in items {
        if let Item::Block(b) = item {
            drop_slots(&mut b.items, fields);
        }
    }
}

/// Step `0001_person_emails` (D16): a `person` type with `addresses` or `email` and no `emails`
/// is from before people carried every contact detail, when `addresses` held email addresses and
/// `email` one more. One planned type change renames `addresses` to `emails`, moves a non-empty
/// `email` value to the front of `emails` (once: a matching address already there, ignoring
/// case, is dropped), and removes `email`, with the template slots of both becoming one slot of
/// `emails`. Fields the user added are kept, and every note keeps every value. The other new
/// fields, postal `addresses` among them, are added by the seeding that runs next.
fn person_emails(w: &Writer) -> Result<()> {
    let schema = w.schema();
    let Some(ty) = schema.type_def(PERSON_TYPE) else {
        return Ok(());
    };
    if !awaits_person_emails(ty) {
        return Ok(());
    }
    let has_addresses = ty.field("addresses").is_some();
    let text = w.vault.read_text(&type_path(PERSON_TYPE))?;
    let mut change = TypeChange {
        wcl: person_with_emails(&text)?,
        ..TypeChange::default()
    };
    if has_addresses {
        change
            .renames
            .insert("addresses".to_owned(), "emails".to_owned());
    }
    let native = NativeMigration {
        script: SERVER_AUTHOR,
        function: "0001_person_emails",
        run: move_email_into_emails,
    };
    w.apply_native_type_change(
        PERSON_TYPE,
        &change,
        &native,
        &Author::Task(SERVER_AUTHOR.to_owned()),
    )?;
    Ok(())
}

/// Whether `person` type `ty` is one step `0001_person_emails` changes. A device leaves such a
/// type alone: seeding it there would add `emails` beside the old email `addresses`, and the home
/// server's step, which sees the vault as it syncs, would then find nothing to move.
fn awaits_person_emails(ty: &TypeDef) -> bool {
    ty.field("emails").is_none() && (ty.field("addresses").is_some() || ty.field("email").is_some())
}

/// The person type file `vault_text` with the shipped `emails` field where `email` was, else
/// where `addresses` was, `email` and `addresses` gone, and every template slot of either naming
/// `emails` instead, once per template.
fn person_with_emails(vault_text: &str) -> Result<String> {
    let path = type_path(PERSON_TYPE);
    let shipped: Source = store::parse_data_only(DEFAULT_PERSON_TYPE, &path)?;
    let emails = shipped
        .items
        .iter()
        .find(|i| block(i, "field").and_then(label) == Some("emails"))
        .expect("the shipped person type has emails")
        .clone();
    // As 0.1.6 shipped it: `labels` and its field come with step `0002_person_labels`.
    let mut emails = emails;
    if let Some(b) = block_mut(&mut emails, "field") {
        b.items
            .retain(|i| !matches!(i, Item::Field(f) if f.name == "labels"));
    }
    let mut ast = store::parse_data_only(vault_text, &path)?;
    let at = |ast: &Source, name: &str| {
        ast.items
            .iter()
            .position(|i| block(i, "field").and_then(label) == Some(name))
    };
    let target = at(&ast, "email").or_else(|| at(&ast, "addresses"));
    if let Some(k) = target {
        ast.items[k] = emails;
    }
    ast.items.retain(|i| {
        !matches!(
            block(i, "field").and_then(label),
            Some("email" | "addresses")
        )
    });
    for item in &mut ast.items {
        if let Some(template) = block_mut(item, "template") {
            let mut seen = false;
            emails_slots(&mut template.items, &mut seen);
        }
    }
    Ok(store::to_canonical(&ast))
}

/// Rename every `email` and `addresses` slot in `items`, and in the blocks they hold, to
/// `emails`, keeping the first and dropping the rest.
fn emails_slots(items: &mut Vec<Item>, seen: &mut bool) {
    let mut k = 0;
    while k < items.len() {
        if let Some(slot) = block_mut(&mut items[k], "slot")
            && matches!(label(slot), Some("email" | "addresses" | "emails"))
        {
            if *seen {
                items.remove(k);
                continue;
            }
            *seen = true;
            slot.labels[0] = Expr::Utf8("emails".to_owned());
        } else if let Item::Block(b) = &mut items[k] {
            emails_slots(&mut b.items, seen);
        }
        k += 1;
    }
}

/// The migration of step `0001_person_emails` for one note: a non-empty `email` moves to the
/// front of `emails`, which drops an equal address (ignoring case) further on, and `email` goes.
fn move_email_into_emails(doc: &mut Doc) {
    let email: Vec<String> = doc
        .fields
        .iter()
        .filter(|(n, _)| n == "email")
        .flat_map(|(_, v)| match v {
            DocValue::Str(s) => vec![s.trim().to_owned()],
            DocValue::List(l) => l
                .iter()
                .filter_map(DocValue::as_str)
                .map(|s| s.trim().to_owned())
                .collect(),
            _ => Vec::new(),
        })
        .filter(|s| !s.is_empty())
        .collect();
    doc.fields.retain(|(n, _)| n != "email");
    if email.is_empty() {
        return;
    }
    let mut emails: Vec<DocValue> = email.iter().cloned().map(DocValue::Str).collect();
    let rest = match doc.field("emails") {
        Some(DocValue::List(l)) => l.clone(),
        Some(v) => vec![v.clone()],
        None => Vec::new(),
    };
    for v in rest {
        let dup = v.as_str().is_some_and(|s| {
            emails
                .iter()
                .any(|e| e.as_str().is_some_and(|e| e.eq_ignore_ascii_case(s.trim())))
        });
        if !dup {
            emails.push(v);
        }
    }
    doc.fields.retain(|(n, _)| n != "emails");
    doc.fields
        .push(("emails".to_owned(), DocValue::List(emails)));
}

/// Where the writer runs, which decides how much of the upkeep it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upkeep {
    /// The home server: everything, with notifications for files the user changed.
    Home,
    /// A device's local vault (`crates/embedded`): the built-in types, device files and scripts,
    /// without notifications or upgrade steps. The home server runs the steps and tells the user
    /// about changed files, and a device takes both results through sync; a notification made on
    /// a device moves to the server, so making one here would double the server's.
    Device,
}

/// What [`Writer::upgrade_shipped_defaults`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Upgraded {
    /// The upgrade steps that ran, in order.
    pub steps: Vec<String>,
    /// The built-in taxonomies added because the vault lacked them.
    pub taxonomies_added: Vec<String>,
    /// The types added whole.
    pub types_added: Vec<String>,
    /// The built-in types skipped because a type of the user's holds the name.
    pub types_skipped: Vec<String>,
    /// `(type, field)` of every field added to a type the vault had.
    pub fields_added: Vec<(String, String)>,
    /// The shipped files written because the vault lacked them.
    pub files_added: Vec<String>,
    /// The shipped files replaced by a newer shipped version.
    pub files_upgraded: Vec<String>,
    /// The shipped files the user changed and that a newer shipped version exists for.
    pub files_kept: Vec<String>,
}

/// The SHA-256 hex of `text`.
pub fn content_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `path` is a task file, whose `paused` field [`shipped_hash`] ignores.
fn is_task(path: &str) -> bool {
    path.starts_with("tasks/") && path.ends_with(".wcl")
}

/// The hash the shipped-file comparison uses for `text` at vault path `path`: the
/// [`content_hash`] of the text, except that a task file is hashed in canonical form without its
/// top-level `paused` field, so pausing or unpausing a shipped task is not an edit of it
/// (google-sync D19). A task file that does not parse is hashed as it is.
pub fn shipped_hash(path: &str, text: &str) -> String {
    if !is_task(path) {
        return content_hash(text);
    }
    match store::parse_data_only(text, path) {
        Ok(mut ast) => {
            ast.items.retain(|i| !is_paused(i));
            content_hash(&store::to_canonical(&ast))
        }
        Err(_) => content_hash(text),
    }
}

fn is_paused(item: &Item) -> bool {
    matches!(item, Item::Field(f) if f.name == "paused")
}

/// Task file `text` with the top-level `paused` field of the vault's `disk` version in place of
/// its own, in canonical form: in the place `text` has its `paused`, else at the end. Without a
/// `paused` in `disk` the task is not paused, and the result has none either.
fn with_paused_of(text: &str, disk: &str, path: &str) -> Result<String> {
    let disk = store::parse_data_only(disk, path)?;
    let mut ast = store::parse_data_only(text, path)?;
    let at = ast.items.iter().position(is_paused);
    ast.items.retain(|i| !is_paused(i));
    if let Some(paused) = disk.items.iter().find(|i| is_paused(i)) {
        let at = at.unwrap_or(ast.items.len()).min(ast.items.len());
        ast.items.insert(at, paused.clone());
    }
    Ok(store::to_canonical(&ast))
}

impl ShippedFile {
    /// The text the vault holds for the current version: `.wcl` files in canonical form.
    fn text(&self) -> Result<String> {
        if self.path.ends_with(".wcl") {
            Ok(store::canonical(self.source, self.path)?)
        } else {
            Ok(self.source.to_owned())
        }
    }
}

/// What to do with one shipped file.
enum Plan {
    /// The vault already holds the current version.
    Current,
    Add,
    Upgrade,
    /// The user changed it. `notify` when a newer shipped version exists that the user has not
    /// been told about.
    Kept {
        notify: bool,
    },
}

impl Writer {
    /// Bring the vault's shipped defaults up to date, as the module documentation describes.
    pub fn upgrade_shipped_defaults(&self, upkeep: Upkeep) -> Result<Upgraded> {
        let steps = match upkeep {
            Upkeep::Home => self.run_upgrade_steps(UPGRADE_STEPS)?,
            Upkeep::Device => Vec::new(),
        };
        let mut done = self.seed_shipped_defaults_with(upkeep, &SHIPPED_FILES)?;
        done.steps = steps;
        Ok(done)
    }

    /// Run every step of `steps` not yet recorded as applied, in order, recording each once it
    /// succeeds. Returns the ids that ran. [`Writer::upgrade_shipped_defaults`] runs
    /// [`UPGRADE_STEPS`]; tests pass their own.
    pub fn run_upgrade_steps(&self, steps: &[UpgradeStep]) -> Result<Vec<String>> {
        let applied: BTreeSet<String> = self.state.applied_upgrades()?.into_iter().collect();
        let mut ran = Vec::new();
        for step in steps.iter().filter(|s| !applied.contains(s.id)) {
            match (step.run)(self) {
                Ok(()) => {}
                Err(crate::Error::StepDeferred(why)) => {
                    eprintln!(
                        "upgrade step {} will run again on the next start: {why}",
                        step.id
                    );
                    continue;
                }
                Err(e) => return Err(e),
            }
            self.state.record_upgrade(step.id)?;
            ran.push(step.id.to_owned());
        }
        Ok(ran)
    }

    /// Steps 2 to 4 of the module documentation, in one commit, with `files` as the shipped
    /// files. [`Writer::upgrade_shipped_defaults`] passes [`SHIPPED_FILES`]; tests pass their own,
    /// to stand for a later release.
    pub fn seed_shipped_defaults_with(
        &self,
        upkeep: Upkeep,
        files: &[ShippedFile],
    ) -> Result<Upgraded> {
        let _w = self.write_lock();
        let mut done = Upgraded::default();
        let mut current = self.schema();

        // Built-in taxonomies the vault lacks, before the types that name them.
        let mut new_taxonomies: Vec<(String, String)> = Vec::new();
        for (name, source) in BUILT_IN_TAXONOMIES {
            if current.taxonomy(name).is_some() {
                continue;
            }
            let text = store::canonical(source, &taxonomy_path(name))?;
            let taxonomy = schema::Taxonomy::parse(name, &text)
                .map_err(|e| crate::Error::InvalidType(schema::SchemaError(vec![e])))?;
            current.grow_taxonomy(taxonomy);
            new_taxonomies.push((name.to_owned(), text));
            done.taxonomies_added.push(name.to_owned());
        }

        // Built-in types the vault lacks, with `person` when a type being added needs it.
        let mut new_types: Vec<(String, String)> = Vec::new();
        let mut missing: Vec<(&str, &str)> = BUILT_IN_TYPES
            .into_iter()
            .filter(|(name, _)| current.type_def(name).is_none())
            .collect();
        let needs_person = missing
            .iter()
            .any(|(name, _)| TYPES_NEEDING_PERSON.contains(name));
        if needs_person && current.type_def(PERSON_TYPE).is_none() {
            missing.push((PERSON_TYPE, DEFAULT_PERSON_TYPE));
        }
        if !missing.is_empty() {
            let mut types: Vec<TypeDef> = current.types().cloned().collect();
            for (name, source) in &missing {
                let text = store::canonical(source, &type_path(name))?;
                types.push(
                    TypeDef::parse(name, &text)
                        .map_err(|e| crate::Error::InvalidType(schema::SchemaError(vec![e])))?,
                );
                new_types.push(((*name).to_owned(), text));
                done.types_added.push((*name).to_owned());
            }
            current = rebuild_schema(&current, types)?;
        }

        // Fields the shipped default declares that the vault's type lacks.
        let mut changed_types: Vec<(String, String)> = Vec::new();
        // Built-in types found by their identity, with no record of the server writing them.
        let mut adopted: Vec<&str> = Vec::new();
        let shipped_types = BUILT_IN_TYPES
            .into_iter()
            .chain([(PERSON_TYPE, DEFAULT_PERSON_TYPE)]);
        for (name, shipped) in shipped_types {
            if done.types_added.iter().any(|t| t == name) {
                continue;
            }
            let Some(ty) = current.type_def(name) else {
                continue;
            };
            if name != PERSON_TYPE {
                if !self.is_built_in_def(ty) {
                    eprintln!(
                        "the built-in type {name:?} was skipped: a type of the user's holds the \
                         name, so it is left as it is and gains no shipped fields"
                    );
                    done.types_skipped.push(name.to_owned());
                    continue;
                }
                if !self.was_seeded(name) {
                    adopted.push(name);
                }
            }
            if name == PERSON_TYPE && upkeep == Upkeep::Device && awaits_person_emails(ty) {
                continue;
            }
            let vault_text = self.vault.read_text(&type_path(name))?;
            let Some((wcl, added)) = with_shipped_fields(name, &vault_text, ty, shipped)? else {
                continue;
            };
            let change = TypeChange {
                wcl,
                ..TypeChange::default()
            };
            match self.work(&current, name, &change) {
                Ok(worked) if worked.plan.changes.is_empty() && worked.taxonomies.is_empty() => {
                    changed_types.push((name.to_owned(), worked.plan.wcl.clone()));
                    done.fields_added
                        .extend(added.into_iter().map(|f| (name.to_owned(), f)));
                    current = worked.schema;
                }
                Ok(_) => eprintln!(
                    "the shipped fields of type {name:?} were not added: adding them would edit notes"
                ),
                Err(e) => eprintln!("the shipped fields of type {name:?} were not added: {e}"),
            }
        }

        // The shipped device scripts and device file.
        let mut plans = Vec::new();
        for &file in files {
            let mut text = file.text()?;
            let hash = shipped_hash(file.path, &text);
            let on_disk = self.read_synced(file.path)?;
            let record = self.state.shipped_file(file.path)?;
            let plan = match on_disk {
                None => Plan::Add,
                Some(disk) => {
                    let disk_hash = shipped_hash(file.path, &disk);
                    let written = record.as_ref().map(|r| r.hash.as_str()).unwrap_or("");
                    if disk_hash == hash {
                        Plan::Current
                    } else if file.hashes.contains(&disk_hash.as_str()) || disk_hash == written {
                        // Pausing is not an edit, so the new version keeps the vault's switch.
                        if is_task(file.path) {
                            text = with_paused_of(&text, &disk, file.path)?;
                        }
                        Plan::Upgrade
                    } else {
                        // The user changed a version the server wrote: a newer one exists unless
                        // what it wrote is the current one. A record from before task files were
                        // hashed without `paused` holds the plain hash of the current text.
                        let notified = record.as_ref().and_then(|r| r.notified.as_deref());
                        Plan::Kept {
                            notify: upkeep == Upkeep::Home
                                && written != hash
                                && written != content_hash(&text)
                                && notified != Some(hash.as_str()),
                        }
                    }
                }
            };
            plans.push((file, text, hash, plan, record));
        }

        let mut changes: Vec<Change> = Vec::new();
        for (file, text, _, plan, _) in &plans {
            if matches!(plan, Plan::Add | Plan::Upgrade) && file.path.starts_with("scripts/") {
                changes.push(Change::write(file.path, text.as_bytes()));
            }
        }
        for (name, text) in &new_taxonomies {
            changes.push(Change::write(taxonomy_path(name), text.as_bytes()));
        }
        for (name, text) in new_types.iter().chain(&changed_types) {
            changes.push(Change::write(type_path(name), text.as_bytes()));
        }
        for (file, text, _, plan, _) in &plans {
            if matches!(plan, Plan::Add | Plan::Upgrade) && !file.path.starts_with("scripts/") {
                changes.push(Change::write(file.path, text.as_bytes()));
            }
        }

        if !changes.is_empty() {
            let mut inner = self.lock();
            let schema =
                if new_types.is_empty() && changed_types.is_empty() && new_taxonomies.is_empty() {
                    None
                } else {
                    let mut types = self.definitions("types")?;
                    for (name, text) in new_types.iter().chain(&changed_types) {
                        types.retain(|(n, _)| n != name);
                        types.push((name.clone(), text.clone()));
                    }
                    let mut taxonomies = self.definitions("taxonomies")?;
                    taxonomies.extend(new_taxonomies.iter().cloned());
                    let schema = Schema::from_sources(&pairs(&types), &pairs(&taxonomies))
                        .map_err(crate::Error::InvalidType)?;
                    crate::builtin::check_built_in_types(&schema)?;
                    Some(schema)
                };
            let generation = self.commit(changes)?;
            if let Some(schema) = schema {
                inner.index.set_schema(&schema)?;
                inner.schema = schema;
            }
            inner.index.advance(generation)?;
        }

        // The built-in types the server wrote, or found by their identity, stay the built-in.
        for (name, text) in &new_types {
            if crate::builtin::is_built_in_type(name) {
                self.record_seeded_type(name, text)?;
            }
        }
        for name in adopted {
            let text = self.vault.read_text(&type_path(name))?;
            self.record_seeded_type(name, &text)?;
        }

        for (file, _, hash, plan, record) in plans {
            match plan {
                Plan::Current => {
                    if record.as_ref().map(|r| r.hash.as_str()) != Some(hash.as_str()) {
                        self.state.record_shipped(file.path, &hash)?;
                    }
                }
                Plan::Add => {
                    self.state.record_shipped(file.path, &hash)?;
                    done.files_added.push(file.path.to_owned());
                }
                Plan::Upgrade => {
                    self.state.record_shipped(file.path, &hash)?;
                    done.files_upgraded.push(file.path.to_owned());
                }
                Plan::Kept { notify } => {
                    if notify {
                        self.notify(&kept_notice(file.path))?;
                        self.state.record_shipped_notice(file.path, &hash)?;
                        done.files_kept.push(file.path.to_owned());
                    }
                }
            }
        }
        Ok(done)
    }
}

/// The notification that `path`, which the user changed, has a newer shipped version.
fn kept_notice(path: &str) -> NewNotification {
    NewNotification {
        title: format!("A newer version of {path} is available"),
        body: Some(format!(
            "The app ships a newer version of {path}. You changed it, so the server left your \
             version in place. To take the new version, delete your changes to the file or copy \
             what you need from the built-in default."
        )),
        note_id: None,
        url: Some("/settings/devices".to_owned()),
        source: "system".to_owned(),
    }
}

/// The first label of `block` as a string.
fn label(block: &Block) -> Option<&str> {
    match block.labels.first()? {
        Expr::Utf8(s) | Expr::Identifier(s, _) => Some(s),
        _ => None,
    }
}

fn block<'a>(item: &'a Item, kind: &str) -> Option<&'a Block> {
    match item {
        Item::Block(b) if b.kind == kind => Some(b),
        _ => None,
    }
}

fn block_mut<'a>(item: &'a mut Item, kind: &str) -> Option<&'a mut Block> {
    match item {
        Item::Block(b) if b.kind == kind => Some(b),
        _ => None,
    }
}

/// The vault's type file `vault_text` with every field of `shipped` that type `ty` lacks, and
/// slots for them in its `default` and `card` templates, with the names of the added fields. `None` when
/// the type lacks none.
fn with_shipped_fields(
    name: &str,
    vault_text: &str,
    ty: &TypeDef,
    shipped: &str,
) -> Result<Option<(String, Vec<String>)>> {
    let path = type_path(name);
    let shipped: Source = store::parse_data_only(shipped, &path)?;
    let missing: Vec<&Block> = shipped
        .items
        .iter()
        .filter_map(|i| block(i, "field"))
        .filter(|b| label(b).is_some_and(|f| ty.field(f).is_none()))
        .collect();
    if missing.is_empty() {
        return Ok(None);
    }
    let mut ast = store::parse_data_only(vault_text, &path)?;
    let added: Vec<String> = missing
        .iter()
        .filter_map(|b| label(b).map(str::to_owned))
        .collect();

    // Each new field goes after the nearest field the shipped default declares before it, else
    // after the type's last field, else before its first template.
    let shipped_fields: Vec<&str> = shipped
        .items
        .iter()
        .filter_map(|i| block(i, "field").and_then(label))
        .collect();
    for b in &missing {
        let field = label(b).unwrap_or_default();
        let n = shipped_fields.iter().position(|f| *f == field).unwrap_or(0);
        let after = shipped_fields[..n].iter().rev().find_map(|before| {
            ast.items
                .iter()
                .position(|i| block(i, "field").and_then(label) == Some(before))
        });
        let at = after
            .or_else(|| ast.items.iter().rposition(|i| block(i, "field").is_some()))
            .map(|k| k + 1)
            .or_else(|| {
                ast.items
                    .iter()
                    .position(|i| block(i, "template").is_some())
            })
            .unwrap_or(ast.items.len());
        ast.items.insert(at, Item::Block((*b).clone()));
    }

    // Slots in each vault template the shipped default also has, `default` and `card`, for the
    // added fields the shipped template of that name places, at any depth: a slot inside
    // `columns > column` counts as one of its tab's.
    for shipped_template in shipped.items.iter().filter_map(|i| block(i, "template")) {
        let Some(template_name) = label(shipped_template) else {
            continue;
        };
        let mut shipped_slots: Vec<(String, Item)> = Vec::new();
        collect_slots(&shipped_template.items, &mut shipped_slots);
        let Some(template) = ast
            .items
            .iter_mut()
            .find_map(|i| block_mut(i, "template").filter(|b| label(b) == Some(template_name)))
        else {
            continue;
        };
        for field in &added {
            let Some(n) = shipped_slots.iter().position(|(f, _)| f == field) else {
                continue;
            };
            let slot = shipped_slots[n].1.clone();
            // After the nearest slot the shipped template puts before it, else before the nearest
            // one it puts after it, else at the end of the first tab.
            let placed = shipped_slots[..n]
                .iter()
                .rev()
                .any(|(before, _)| insert_slot(&mut template.items, before, 1, &slot))
                || shipped_slots[n + 1..]
                    .iter()
                    .any(|(after, _)| insert_slot(&mut template.items, after, 0, &slot));
            if !placed
                && let Some(tab) = template.items.iter_mut().find_map(|i| block_mut(i, "tab"))
            {
                tab.items.push(slot);
            }
        }
    }
    Ok(Some((store::to_canonical(&ast), added)))
}

/// Every slot in `items` and the blocks they hold, in document order, with its field name.
fn collect_slots(items: &[Item], out: &mut Vec<(String, Item)>) {
    for item in items {
        let Item::Block(b) = item else { continue };
        if b.kind == "slot" {
            if let Some(field) = label(b) {
                out.push((field.to_owned(), item.clone()));
            }
        } else {
            collect_slots(&b.items, out);
        }
    }
}

/// Insert `slot` beside the first slot of `field` in `items` or the blocks they hold: right after
/// it when `offset` is 1, right before it when 0. Whether it found the slot.
fn insert_slot(items: &mut Vec<Item>, field: &str, offset: usize, slot: &Item) -> bool {
    if let Some(k) = items
        .iter()
        .position(|i| block(i, "slot").and_then(label) == Some(field))
    {
        items.insert(k + offset, slot.clone());
        return true;
    }
    items.iter_mut().any(|i| match i {
        Item::Block(b) if b.kind != "slot" => insert_slot(&mut b.items, field, offset, slot),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_file_lists_its_current_hash_last() {
        for file in SHIPPED_FILES {
            let text = file.text().unwrap();
            assert_eq!(
                file.hashes.last().copied(),
                Some(shipped_hash(file.path, &text).as_str()),
                "{}: append the new hash to its `hashes` when changing a shipped file",
                file.path
            );
        }
    }

    #[test]
    fn the_shipped_files_are_the_default_device_file_and_scripts_then_the_google_sync() {
        let mut paths: Vec<String> = crate::DEFAULT_DEVICE_SCRIPTS
            .iter()
            .map(|(name, _)| crate::defs::script_path(name))
            .collect();
        paths.push(crate::device_file_path("android"));
        paths.push("scripts/google_calendar.ws".to_owned());
        paths.push("tasks/google_calendar.wcl".to_owned());
        paths.push(crate::defs::script_path("google_gmail"));
        paths.push(crate::defs::task_path("google_gmail"));
        paths.push(crate::defs::script_path("google_event"));
        paths.push(crate::defs::script_path("outbox_send"));
        paths.push(crate::agent_path(schema::ASSISTANT_AGENT));
        let shipped: Vec<String> = SHIPPED_FILES.iter().map(|f| f.path.to_owned()).collect();
        assert_eq!(shipped, paths);
        for (name, source) in crate::DEFAULT_DEVICE_SCRIPTS {
            let file = SHIPPED_FILES
                .iter()
                .find(|f| f.path == crate::defs::script_path(name))
                .unwrap();
            assert_eq!(file.source, source);
        }
    }

    #[test]
    fn the_shipped_wcl_files_are_canonical() {
        for file in SHIPPED_FILES.iter().filter(|f| f.path.ends_with(".wcl")) {
            assert_eq!(file.text().unwrap(), file.source, "{}", file.path);
        }
    }
}
