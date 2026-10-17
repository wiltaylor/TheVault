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
//!    template puts before it; no other template is touched.
//! 3. **Missing device files and scripts.** Each file of [`SHIPPED_FILES`] the vault lacks is
//!    written, the scripts before the device file that names them.
//! 4. **Shipped file upgrades.** A file of [`SHIPPED_FILES`] whose content is still a version the
//!    app shipped is replaced by the current one. A file the user changed is left alone, and the
//!    user hears once, under the bell, that a newer shipped version exists.
//!
//! Steps 2 to 4 are one commit, and a start that finds nothing to do commits nothing.

use std::collections::BTreeSet;

use schema::{Schema, TypeDef};
use sha2::{Digest, Sha256};
use state::NewNotification;
use store::Change;
use wcl_lang::ast::{Block, Expr, Item, Source};

use crate::builtin::{BUILT_IN_TYPES, DEFAULT_PERSON_TYPE, PERSON_TYPE};
use crate::types::{TypeChange, rebuild_schema};
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
pub const UPGRADE_STEPS: &[UpgradeStep] = &[];

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
            // After the nearest slot the shipped template puts before it, else at the end of the
            // first tab.
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
