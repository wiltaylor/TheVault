//! Keeping a vault's shipped defaults up to date: the built-in types, the default device file and
//! the default device scripts, and the one-time upgrade steps (D15, D16, D24).
//!
//! [`Writer::upgrade_shipped_defaults`] runs on every start of the home server and of the
//! embedded writer. It does four things:
//!
//! 1. **Upgrade steps** ([`UPGRADE_STEPS`]), on the home server only, each applied once in order
//!    and recorded in `state.db`. They run first, because a step such as a field rename must see
//!    the vault before the additive seeding below adds the field under its new name.
//! 2. **Built-in types.** A type the vault lacks is added whole. A built-in type the vault has,
//!    and the `person` type the phone data types reference, gain every field the shipped default
//!    declares and the vault's type lacks, as a planned schema change that edits no note. A field
//!    the vault has is never removed, renamed or changed. The `default` template gains a slot for
//!    an added field when the shipped `default` template has one, after the slot the shipped
//!    template puts before it, else before the one it puts after it; no other template is
//!    touched.
//! 3. **Missing device files and scripts.** Each file of [`SHIPPED_FILES`] the vault lacks is
//!    written, the scripts before the device file that names them.
//! 4. **Shipped file upgrades.** A file of [`SHIPPED_FILES`] whose content is still a version the
//!    app shipped is replaced by the current one. A file the user changed is left alone, and the
//!    user hears once, under the bell, that a newer shipped version exists.
//!
//! Steps 2 to 4 are one commit, and a start that finds nothing to do commits nothing.

use std::collections::BTreeSet;

use schema::{Doc, Schema, TypeDef, Value as DocValue};
use sha2::{Digest, Sha256};
use state::{Author, NewNotification};
use store::Change;
use wcl_lang::ast::{Block, Expr, Item, Source};

use crate::builtin::{BUILT_IN_TYPES, DEFAULT_PERSON_TYPE, PERSON_TYPE};
use crate::types::{NativeMigration, TypeChange, rebuild_schema};
use crate::{CALL_TYPE, CONVERSATION_TYPE, MESSAGE_TYPE, Result, Writer, pairs, type_path};

/// The author of the changes the server makes to keep its shipped defaults up to date.
pub const SERVER_AUTHOR: &str = "server";

/// A file the app ships into every vault, and every version of it the app has ever shipped.
#[derive(Debug, Clone, Copy)]
pub struct ShippedFile {
    /// The vault path, such as `scripts/device_calls.ws`.
    pub path: &'static str,
    /// The current shipped content.
    pub source: &'static str,
    /// The SHA-256 hex of every version ever shipped, oldest first, ending with the current one.
    /// Changing a shipped file means appending its new hash here: a vault holding any listed
    /// version is upgraded, so one that skipped releases still is.
    pub hashes: &'static [&'static str],
}

/// The shipped files, scripts before the device file that names them.
pub const SHIPPED_FILES: [ShippedFile; 6] = [
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
];

/// One one-time vault upgrade: a change that is not additive, such as a field rename with a
/// migration script. It runs once per vault, on the home server, before the shipped defaults are
/// seeded, and is recorded in `state.db` by its id once it returns `Ok`. A step that fails stops
/// the server from starting and runs again on the next start.
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
];

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
    /// The types added whole.
    pub types_added: Vec<String>,
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
        let mut done = self.seed_shipped_defaults(upkeep)?;
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
            (step.run)(self)?;
            self.state.record_upgrade(step.id)?;
            ran.push(step.id.to_owned());
        }
        Ok(ran)
    }

    /// Steps 2 to 4 of the module documentation, in one commit.
    fn seed_shipped_defaults(&self, upkeep: Upkeep) -> Result<Upgraded> {
        let _w = self.write_lock();
        let mut done = Upgraded::default();
        let mut current = self.schema();

        // Built-in types the vault lacks, with `person` when a phone data type needs it.
        let mut new_types: Vec<(String, String)> = Vec::new();
        let mut missing: Vec<(&str, &str)> = BUILT_IN_TYPES
            .into_iter()
            .filter(|(name, _)| current.type_def(name).is_none())
            .collect();
        let needs_person = missing
            .iter()
            .any(|(name, _)| [MESSAGE_TYPE, CALL_TYPE, CONVERSATION_TYPE].contains(name));
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
        for file in SHIPPED_FILES {
            let text = file.text()?;
            let hash = content_hash(&text);
            let on_disk = self.read_synced(file.path)?;
            let record = self.state.shipped_file(file.path)?;
            let plan = match on_disk {
                None => Plan::Add,
                Some(disk) => {
                    let disk_hash = content_hash(&disk);
                    let written = record.as_ref().map(|r| r.hash.as_str()).unwrap_or("");
                    if disk_hash == hash {
                        Plan::Current
                    } else if file.hashes.contains(&disk_hash.as_str()) || disk_hash == written {
                        Plan::Upgrade
                    } else {
                        // The user changed a version the server wrote: a newer one exists unless
                        // what it wrote is the current one.
                        let notified = record.as_ref().and_then(|r| r.notified.as_deref());
                        Plan::Kept {
                            notify: upkeep == Upkeep::Home
                                && written != hash
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
            let schema = if new_types.is_empty() && changed_types.is_empty() {
                None
            } else {
                let mut types = self.definitions("types")?;
                for (name, text) in new_types.iter().chain(&changed_types) {
                    types.retain(|(n, _)| n != name);
                    types.push((name.clone(), text.clone()));
                }
                let taxonomies = self.definitions("taxonomies")?;
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
/// slots for them in the `default` template, with the names of the added fields. `None` when
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

    // Slots in the default template, for the added fields the shipped default template places.
    let shipped_slots: Vec<(String, Item)> = shipped
        .items
        .iter()
        .filter_map(|i| block(i, "template").filter(|b| label(b) == Some("default")))
        .flat_map(|t| t.items.iter().filter_map(|i| block(i, "tab")))
        .flat_map(|tab| tab.items.iter())
        .filter_map(|i| block(i, "slot").and_then(|s| Some((label(s)?.to_owned(), i.clone()))))
        .collect();
    if let Some(template) = ast
        .items
        .iter_mut()
        .find_map(|i| block_mut(i, "template").filter(|b| label(b) == Some("default")))
    {
        for field in &added {
            let Some(n) = shipped_slots.iter().position(|(f, _)| f == field) else {
                continue;
            };
            let slot = shipped_slots[n].1.clone();
            // After the nearest slot the shipped template puts before it, else before the nearest
            // one it puts after it, else at the end of the first tab.
            let mut placed = false;
            for (before, _) in shipped_slots[..n].iter().rev() {
                for tab in template
                    .items
                    .iter_mut()
                    .filter_map(|i| block_mut(i, "tab"))
                {
                    if let Some(k) = tab
                        .items
                        .iter()
                        .position(|i| block(i, "slot").and_then(label) == Some(before.as_str()))
                    {
                        tab.items.insert(k + 1, slot.clone());
                        placed = true;
                        break;
                    }
                }
                if placed {
                    break;
                }
            }
            // Else before the nearest slot it puts after it.
            if !placed {
                for (after, _) in &shipped_slots[n + 1..] {
                    for tab in template
                        .items
                        .iter_mut()
                        .filter_map(|i| block_mut(i, "tab"))
                    {
                        if let Some(k) = tab
                            .items
                            .iter()
                            .position(|i| block(i, "slot").and_then(label) == Some(after.as_str()))
                        {
                            tab.items.insert(k, slot.clone());
                            placed = true;
                            break;
                        }
                    }
                    if placed {
                        break;
                    }
                }
            }
            if !placed
                && let Some(tab) = template.items.iter_mut().find_map(|i| block_mut(i, "tab"))
            {
                tab.items.push(slot);
            }
        }
    }
    Ok(Some((store::to_canonical(&ast), added)))
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
                Some(content_hash(&text).as_str()),
                "{}: append the new hash to its `hashes` when changing a shipped file",
                file.path
            );
        }
    }

    #[test]
    fn the_shipped_files_are_the_default_device_file_and_scripts() {
        let mut paths: Vec<String> = crate::DEFAULT_DEVICE_SCRIPTS
            .iter()
            .map(|(name, _)| crate::defs::script_path(name))
            .collect();
        paths.push(crate::device_file_path("android"));
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
    fn the_shipped_device_file_is_canonical() {
        let file = SHIPPED_FILES.last().unwrap();
        assert_eq!(file.text().unwrap(), file.source);
    }
}
