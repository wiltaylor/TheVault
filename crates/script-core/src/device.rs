//! The `device` host module: what a script running on a paired phone reads from it and asks of
//! it (D6, D23). It exists only for [`Kind::Device`](crate::Kind::Device) runs.
//!
//! Every function goes through a [`DeviceData`], which the Android app implements over the phone's
//! own stores and tests fake. Reads are refused by the implementation when Android has not granted
//! the permission. Contact writes and sending SMS are refused here unless the device's switch for
//! them is on in Settings > Devices, whatever the implementation could do.

use std::sync::{Arc, RwLock};

use serde_json::{Value as Json, json};
use wscript::std_modules::DynValue;
use wscript::{Fault, HostError, Module};

use crate::{Extensions, Kind, Reply, Vault, convert, run};

/// The phone's own data and actions, as the `device` module reaches them. Every error is a message
/// the script receives as an `Err`.
///
/// Items travel in the shapes of the device event payloads in `docs/scripts.md`: an SMS as
/// `{ device_ref, thread_ref, number, number_raw, contact, body, sent, direction }`, a call as
/// `{ device_ref, direction, number, number_raw, contact, started, duration }`, a contact as
/// `{ contact_id, name, phones, phone_labels, emails, email_labels, addresses, address_labels,
/// organisation, job_title, nickname, birthday, websites, contact_note, photo, updated, person,
/// synced }`, with `photo` `{ asset, hash }` or null.
pub trait DeviceData: Send + Sync {
    /// SMS matching `filter`, newest first. `filter` may hold `thread_ref`, `number`, `since` (a
    /// time) and `limit`.
    fn sms_list(&self, filter: &Json) -> Result<Vec<Json>, String>;

    /// The SMS with `device_ref`.
    fn sms_get(&self, device_ref: &str) -> Result<Json, String>;

    /// Calls matching `filter`, newest first. `filter` may hold `number`, `since` and `limit`.
    fn calls_list(&self, filter: &Json) -> Result<Vec<Json>, String>;

    /// Contacts matching `filter`. `filter` may hold `name` (exact display name), `number` (E.164)
    /// and `limit`.
    fn contacts_list(&self, filter: &Json) -> Result<Vec<Json>, String>;

    /// The contact with `contact_id`.
    fn contacts_get(&self, contact_id: &str) -> Result<Json, String>;

    /// Add the contact, or change it when `contact_id` is not null: `{ contact_id, person }` and
    /// any of the contact's detail keys, with `photo` an asset id of the local vault or null. A
    /// detail key left out leaves that detail alone. Returns `{ contact_id }`.
    fn contacts_upsert(&self, contact: &Json) -> Result<Json, String>;

    /// Delete the contact with `contact_id`.
    fn contacts_delete(&self, contact_id: &str) -> Result<(), String>;

    /// Send `body` to `to`, a number. Returns `{ device_ref }` of the sent message.
    fn sms_send(&self, to: &str, body: &str) -> Result<Json, String>;

    /// Show a notification that opens note `note` when tapped, or the app when `note` is empty.
    fn show_notification(&self, title: &str, body: &str, note: &str) -> Result<(), String>;

    /// One page of what the phone holds of `kind` (`sms`, `mms`, `calls` or `contacts`), as the
    /// device events capture would have made of it, for an import of history. Not a `device`
    /// module function: the device runtime's import calls it.
    ///
    /// `request` is `{ kind, since, until, after, limit }`: `since` and `until` bound the item's
    /// time in milliseconds, either may be null; `after` is the key the last page ended at, null
    /// for the first page.
    ///
    /// The answer is `{ events, next, missing_permission }`. `events` is `[{ event, data, at }]`,
    /// oldest first, where `at` is the item's time in milliseconds and `data` the event payload.
    /// `next` is the key to ask for the next page with, or null on the last page.
    /// `missing_permission` names the Android permission the kind needs when it is not granted,
    /// and `events` is then empty. An MMS that already has a note may come back as its
    /// `device_ref` alone, so its parts are never stored a second time.
    ///
    /// A device that cannot read its history refuses.
    fn history(&self, request: &Json) -> Result<Json, String> {
        let _ = request;
        Err("this device cannot import its history".into())
    }

    /// Remove the links to `person` notes the phone's contacts hold for any vault but
    /// `vault_id`, and those of an older app that name no vault and a person the local vault
    /// does not have; adopt for `vault_id` the older ones whose person it has. Answers how many
    /// links it removed. Not a `device` module function: the app calls it once after joining a
    /// vault, when the device may write contacts (ticket 143). A device that keeps no links
    /// removes none.
    fn contacts_forget_links(&self, vault_id: &str) -> Result<u64, String> {
        let _ = vault_id;
        Ok(0)
    }
}

/// The device's switches for what changes the phone or reaches other people (D23).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Switches {
    /// Device scripts may add, change and delete phone contacts.
    pub contacts_write: bool,
    /// Device scripts may send SMS.
    pub sms_send: bool,
}

/// What the `device` module reaches: the device's id, its data, and its switches, which change
/// while the host runs when the user changes them in Settings > Devices.
pub struct DeviceAccess {
    id: String,
    data: Arc<dyn DeviceData>,
    switches: RwLock<Switches>,
}

impl DeviceAccess {
    pub fn new(id: &str, data: Arc<dyn DeviceData>, switches: Switches) -> DeviceAccess {
        DeviceAccess {
            id: id.to_owned(),
            data,
            switches: RwLock::new(switches),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn switches(&self) -> Switches {
        *self.switches.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_switches(&self, switches: Switches) {
        *self.switches.write().unwrap_or_else(|e| e.into_inner()) = switches;
    }

    fn require(&self, on: bool, what: &str, switch: &str) -> Result<(), String> {
        if on {
            Ok(())
        } else {
            Err(format!(
                "{what} is switched off for this device; turn on {switch} in Settings > Devices"
            ))
        }
    }
}

/// A device that is not here: what the home server compiles device scripts against, so a script
/// that uses `device` checks there and fails only if it were run.
pub struct NoDevice;

const NOT_HERE: &str = "the device module works only on a paired device";

impl DeviceData for NoDevice {
    fn sms_list(&self, _: &Json) -> Result<Vec<Json>, String> {
        Err(NOT_HERE.into())
    }
    fn sms_get(&self, _: &str) -> Result<Json, String> {
        Err(NOT_HERE.into())
    }
    fn calls_list(&self, _: &Json) -> Result<Vec<Json>, String> {
        Err(NOT_HERE.into())
    }
    fn contacts_list(&self, _: &Json) -> Result<Vec<Json>, String> {
        Err(NOT_HERE.into())
    }
    fn contacts_get(&self, _: &str) -> Result<Json, String> {
        Err(NOT_HERE.into())
    }
    fn contacts_upsert(&self, _: &Json) -> Result<Json, String> {
        Err(NOT_HERE.into())
    }
    fn contacts_delete(&self, _: &str) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
    fn sms_send(&self, _: &str, _: &str) -> Result<Json, String> {
        Err(NOT_HERE.into())
    }
    fn show_notification(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err(NOT_HERE.into())
    }
}

/// A device call within the run's deadline.
fn device_call<T>(f: impl FnOnce() -> Result<T, String>) -> Result<Result<T, String>, HostError> {
    run::with(|r| r.remaining().map(|_| ()))?;
    let out = f();
    run::with(|r| r.remaining().map(|_| ()))?;
    Ok(out)
}

fn to_dyn_list(rows: Vec<Json>) -> Vec<DynValue> {
    rows.into_iter().map(convert::to_dyn).collect()
}

pub fn device_module(d: Arc<DeviceAccess>) -> Module {
    let mut m = Module::new("device");
    m.doc("Read the phone's SMS, calls and contacts, and write contacts or send SMS when the device's switches allow it (D23)");

    let dd = d.clone();
    m.fn_("id", move || -> String { dd.id.clone() });
    let dd = d.clone();
    m.fn_named(
        "sms_list",
        ["filter"],
        move |filter: DynValue| -> Reply<Vec<DynValue>> {
            Fault(device_call(|| {
                dd.data
                    .sms_list(&convert::from_dyn(filter))
                    .map(to_dyn_list)
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "sms_get",
        ["device_ref"],
        move |device_ref: String| -> Reply<DynValue> {
            Fault(device_call(|| {
                dd.data.sms_get(&device_ref).map(convert::to_dyn)
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "calls_list",
        ["filter"],
        move |filter: DynValue| -> Reply<Vec<DynValue>> {
            Fault(device_call(|| {
                dd.data
                    .calls_list(&convert::from_dyn(filter))
                    .map(to_dyn_list)
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "contacts_list",
        ["filter"],
        move |filter: DynValue| -> Reply<Vec<DynValue>> {
            Fault(device_call(|| {
                dd.data
                    .contacts_list(&convert::from_dyn(filter))
                    .map(to_dyn_list)
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "contacts_get",
        ["contact_id"],
        move |contact_id: String| -> Reply<DynValue> {
            Fault(device_call(|| {
                dd.data.contacts_get(&contact_id).map(convert::to_dyn)
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "contacts_upsert",
        ["contact"],
        move |contact: DynValue| -> Reply<DynValue> {
            Fault(device_call(|| {
                dd.require(
                    dd.switches().contacts_write,
                    "writing contacts",
                    "contacts write",
                )?;
                dd.data
                    .contacts_upsert(&convert::from_dyn(contact))
                    .map(convert::to_dyn)
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "contacts_delete",
        ["contact_id"],
        move |contact_id: String| -> Reply<DynValue> {
            Fault(device_call(|| {
                dd.require(
                    dd.switches().contacts_write,
                    "deleting contacts",
                    "contacts write",
                )?;
                dd.data
                    .contacts_delete(&contact_id)
                    .map(|()| convert::to_dyn(json!({})))
            }))
        },
    );
    let dd = d.clone();
    m.fn_named(
        "sms_send",
        ["to", "body"],
        move |to: String, body: String| -> Reply<DynValue> {
            Fault(device_call(|| {
                dd.require(dd.switches().sms_send, "sending SMS", "SMS send")?;
                dd.data.sms_send(&to, &body).map(convert::to_dyn)
            }))
        },
    );
    m.fn_named(
        "show_notification",
        ["title", "body", "note"],
        move |title: String, body: String, note: String| -> Reply<DynValue> {
            Fault(device_call(|| {
                if title.trim().is_empty() {
                    return Err("the notification title is empty".into());
                }
                d.data
                    .show_notification(&title, &body, &note)
                    .map(|()| convert::to_dyn(json!({})))
            }))
        },
    );
    m
}

/// What a device adds to the engine: the `device` module, for device runs only.
pub struct DeviceExtensions(pub Arc<DeviceAccess>);

impl Extensions for DeviceExtensions {
    fn modules(&self, kind: Kind, _vault: &Arc<dyn Vault>) -> Vec<Module> {
        match kind {
            Kind::Device => vec![device_module(self.0.clone())],
            Kind::Before | Kind::After | Kind::Task | Kind::Migration => Vec::new(),
        }
    }
}
