//! `person` notes reaching phone contacts (D16, D22, D23, D24).
//!
//! A person saved on the home server that no contact on a device mirrors yet becomes an
//! `upsert_contact` command for that device, so a person made in the web app reaches the phone.
//! A command is queued only for a device whose contacts-write switch is on and that has
//! `upsert_contact` enabled, and only when the person has a phone number or an address to carry
//! and its name, phones or addresses changed in the save (a create always counts). A save the
//! device itself made, such as the link it writes back, queues nothing for that device, so
//! nothing echoes back to where it came from.
//!
//! A person already linked to a device's contact is left to that device: once the phone syncs
//! the change, it settles the contact field by field against what the two last agreed on (the
//! default `device_contacts` script), which a command carrying the person's fields would
//! overwrite. Deleting a person queues nothing: deleting a contact is the user's opt-in (D16).
//!
//! When the device reports the command done, the server links the person to the contact it
//! reported, as the device, in case the person had not reached the device when the command ran.

use serde_json::{Map, Value as Json, json};
use state::{Author, CommandStatus, Device, DeviceCommand};

use crate::{Input, PERSON_TYPE, Result, Writer, doc_to_json};

/// The command that adds or changes a phone contact.
pub const UPSERT_CONTACT: &str = "upsert_contact";

/// The `source` of the commands queued here.
pub const CONTACTS_SOURCE: &str = "contacts";

/// The `dict` field linking a person to its contact on each device.
const LINKS: &str = "device_contacts";

/// A `person` created or updated by one write.
pub(crate) struct PersonChange {
    pub id: String,
    /// Who made this change.
    pub author: Author,
    /// The note before the write as `{ fields, blocks }`, `None` on create.
    pub before: Option<Json>,
    /// The note after the write as `{ fields, blocks }`.
    pub after: Json,
}

/// A device's key in `device_contacts`: a dict key is an identifier, and a device id, a ULID,
/// starts with a digit.
pub fn contact_link_key(device_id: &str) -> String {
    format!("device_{device_id}")
}

fn strings(note: &Json, field: &str) -> Vec<String> {
    note["fields"][field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .map(str::to_owned)
        .collect()
}

/// What a contact mirrors of a person: `(name, phones, addresses)`.
fn contact_fields(note: &Json) -> (String, Vec<String>, Vec<String>) {
    (
        note["fields"]["title"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        strings(note, "phones"),
        strings(note, "addresses"),
    )
}

fn is_links(block: &Json) -> bool {
    block["kind"] == "dict" && block["labels"] == json!([LINKS])
}

/// The person's contact on device `device_id`, if it has one.
fn linked_contact<'a>(note: &'a Json, device_id: &str) -> Option<&'a str> {
    note["blocks"]
        .as_array()?
        .iter()
        .find(|b| is_links(b))?
        .get("fields")?
        .get(contact_link_key(device_id))?
        .as_str()
        .filter(|c| !c.is_empty())
}

/// Whether the device's settings let a person be written to its contacts.
fn writes_contacts(device: &Device) -> bool {
    let settings: Json = serde_json::from_str(&device.settings).unwrap_or_default();
    settings["contacts_write"] == true
        && settings["commands"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c == UPSERT_CONTACT))
}

/// Whether `author` is device `device` itself.
fn by_device(author: &Author, device: &Device) -> bool {
    matches!(author, Author::Token(name) if *name == device.token_id)
}

impl Writer {
    /// Queue an `upsert_contact` for every device that should hear of `changes`. A failure is
    /// logged: the write it follows has committed.
    pub(crate) fn people_to_contacts(&self, changes: &[PersonChange]) {
        if changes.is_empty() {
            return;
        }
        let devices: Vec<Device> = match self.state.list_devices() {
            Ok(list) => list.into_iter().filter(writes_contacts).collect(),
            Err(e) => {
                eprintln!("person changes not queued for phone contacts: {e}");
                return;
            }
        };
        if devices.is_empty() {
            return;
        }
        for change in changes {
            let (name, phones, addresses) = contact_fields(&change.after);
            if phones.is_empty() && addresses.is_empty() {
                continue;
            }
            if change.before.as_ref().map(contact_fields)
                == Some((name.clone(), phones.clone(), addresses.clone()))
            {
                continue;
            }
            for device in &devices {
                if by_device(&change.author, device)
                    || linked_contact(&change.after, &device.id).is_some()
                {
                    continue;
                }
                let payload = json!({
                    "contact_id": null,
                    "name": name,
                    "phones": phones,
                    "addresses": addresses,
                    "person": change.id,
                });
                if let Err(e) =
                    self.device_command(&device.id, UPSERT_CONTACT, &payload, CONTACTS_SOURCE)
                {
                    eprintln!(
                        "person {} not queued for device {:?}: {e}",
                        change.id, device.name
                    );
                }
            }
        }
    }

    /// Link the person of `command`, an `upsert_contact` a device reported done, to the contact
    /// the device reported, unless it is linked to it already. The write is the device's own, so
    /// it queues nothing back to the device. Any other command is left alone.
    pub fn link_upserted_contact(&self, command: &DeviceCommand) -> Result<()> {
        if command.name != UPSERT_CONTACT || command.status != CommandStatus::Done {
            return Ok(());
        }
        let payload: Json = serde_json::from_str(&command.payload).unwrap_or_default();
        let result: Json = command
            .result
            .as_deref()
            .and_then(|r| serde_json::from_str(r).ok())
            .unwrap_or_default();
        let (Some(person), Some(contact)) = (
            payload["person"].as_str().filter(|p| !p.is_empty()),
            result["contact_id"].as_str().filter(|c| !c.is_empty()),
        ) else {
            return Ok(());
        };
        let device = self.state.device(&command.device_id)?;
        // A person deleted since stays deleted.
        let Ok(note) = self.read(person) else {
            return Ok(());
        };
        if note.type_name != PERSON_TYPE {
            return Ok(());
        }
        let mut body = doc_to_json(&note.doc);
        if linked_contact(&body, &device.id) == Some(contact) {
            return Ok(());
        }
        let key = contact_link_key(&device.id);
        let blocks = body["blocks"]
            .as_array_mut()
            .expect("a note body holds a list of blocks");
        match blocks.iter_mut().find(|b| is_links(b)) {
            Some(block) => {
                block["fields"][&key] = json!(contact);
            }
            None => {
                let mut fields = Map::new();
                fields.insert(key, json!(contact));
                blocks.push(json!({ "kind": "dict", "labels": [LINKS], "fields": fields }));
            }
        }
        self.update(
            person,
            &Input::Json(body),
            Some(&note.version),
            &Author::Token(device.token_id),
        )?;
        Ok(())
    }
}
