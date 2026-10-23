//! The built-in types every vault has: `dailynote`, `fleeting`, the phone data types `message`,
//! `call` and `conversation` (D15), and the Google sync types `email`, `email_thread` and `event`
//! (google-sync D4, D15, D17), with the `gmail_label` taxonomy `email` uses
//! (google-sync D14), and `outbox`, the email, SMS and calendar invites waiting to be sent
//! (outbox X1).
//!
//! A built-in type is added when a vault lacks it, can be customised, and cannot be deleted. A
//! vault type holding a built-in's name is the built-in only when the server recorded writing it
//! or it matches the built-in's identity: see [`Writer::is_built_in`].
//! `dailynote` and `fleeting` keep the fields the app relies on: see [`crate::daily`] and
//! [`crate::fleeting`].
//!
//! The phone data types and the Google sync types reference `person`, which is a starter type rather than a built-in one. A
//! vault without a `person` type gets [`DEFAULT_PERSON_TYPE`] along with them, and can delete it
//! once nothing references it.

use schema::{Schema, TypeDef};

use crate::{Result, Writer, daily, fleeting};

/// The name of the built-in message type: one note per SMS, MMS or RCS message.
pub const MESSAGE_TYPE: &str = "message";

/// The name of the built-in call type: one note per phone call.
pub const CALL_TYPE: &str = "call";

/// The name of the built-in conversation type: one note per message thread.
pub const CONVERSATION_TYPE: &str = "conversation";

/// The name of the built-in email type: one note per Gmail message.
pub const EMAIL_TYPE: &str = "email";

/// The name of the built-in email thread type: one note per Gmail thread.
pub const EMAIL_THREAD_TYPE: &str = "email_thread";

/// The name of the built-in event type: one note per Google Calendar event or occurrence.
pub const EVENT_TYPE: &str = "event";

/// The name of the built-in agent thread type: one note per chat with an agent (agents G7).
pub const AGENT_THREAD_TYPE: &str = "agent_thread";

/// The name of the built-in outbox type: one note per email, SMS or calendar invite to send
/// (outbox X1).
pub const OUTBOX_TYPE: &str = "outbox";

/// The name of the built-in open taxonomy of Gmail labels.
pub const GMAIL_LABEL_TAXONOMY: &str = "gmail_label";

/// The name of the type the phone data types reference.
pub const PERSON_TYPE: &str = "person";

/// The `message` type a vault without it gets.
pub const DEFAULT_MESSAGE_TYPE: &str = include_str!("../defaults/message.wcl");

/// The `call` type a vault without it gets.
pub const DEFAULT_CALL_TYPE: &str = include_str!("../defaults/call.wcl");

/// The `conversation` type a vault without it gets.
pub const DEFAULT_CONVERSATION_TYPE: &str = include_str!("../defaults/conversation.wcl");

/// The `email` type a vault without it gets.
pub const DEFAULT_EMAIL_TYPE: &str = include_str!("../defaults/email.wcl");

/// The `email_thread` type a vault without it gets.
pub const DEFAULT_EMAIL_THREAD_TYPE: &str = include_str!("../defaults/email_thread.wcl");

/// The `event` type a vault without it gets.
pub const DEFAULT_EVENT_TYPE: &str = include_str!("../defaults/event.wcl");

/// The `agent_thread` type a vault without it gets (agents G7).
pub const DEFAULT_AGENT_THREAD_TYPE: &str = include_str!("../defaults/agent_thread.wcl");

/// The `outbox` type a vault without it gets (outbox X1).
pub const DEFAULT_OUTBOX_TYPE: &str = include_str!("../defaults/outbox.wcl");

/// The `gmail_label` taxonomy a vault without it gets. It is open, so the Gmail sync adds each
/// label it meets; the file is never replaced once written.
pub const DEFAULT_GMAIL_LABEL_TAXONOMY: &str =
    include_str!("../defaults/taxonomies/gmail_label.wcl");

/// The `person` type a vault gets when it has none, because the phone data types reference it
/// (D24). The starter pack's `person` adds to it.
pub const DEFAULT_PERSON_TYPE: &str = include_str!("../defaults/person.wcl");

/// Every built-in type, as `(name, the type file a vault without it gets)`.
pub const BUILT_IN_TYPES: [(&str, &str); 10] = [
    (daily::DAILY_TYPE, daily::DEFAULT_DAILY_TYPE),
    (fleeting::FLEETING_TYPE, fleeting::DEFAULT_FLEETING_TYPE),
    (MESSAGE_TYPE, DEFAULT_MESSAGE_TYPE),
    (CALL_TYPE, DEFAULT_CALL_TYPE),
    (CONVERSATION_TYPE, DEFAULT_CONVERSATION_TYPE),
    (EMAIL_TYPE, DEFAULT_EMAIL_TYPE),
    (EMAIL_THREAD_TYPE, DEFAULT_EMAIL_THREAD_TYPE),
    (EVENT_TYPE, DEFAULT_EVENT_TYPE),
    (AGENT_THREAD_TYPE, DEFAULT_AGENT_THREAD_TYPE),
    (OUTBOX_TYPE, DEFAULT_OUTBOX_TYPE),
];

/// The built-in types that reference `person`, so a vault given one of them without a `person`
/// type gets [`DEFAULT_PERSON_TYPE`] too.
pub const TYPES_NEEDING_PERSON: [&str; 6] = [
    MESSAGE_TYPE,
    CALL_TYPE,
    CONVERSATION_TYPE,
    EMAIL_TYPE,
    EMAIL_THREAD_TYPE,
    EVENT_TYPE,
];

/// Every built-in taxonomy, as `(name, the taxonomy file a vault without it gets)`. A built-in
/// type names it, so it is added before, and with, the types. Unlike a type it gains nothing
/// once the vault has it.
pub const BUILT_IN_TAXONOMIES: [(&str, &str); 1] =
    [(GMAIL_LABEL_TAXONOMY, DEFAULT_GMAIL_LABEL_TAXONOMY)];

/// Whether `name` is the name of a built-in type. A vault type of that name is the built-in only
/// when [`Writer::is_built_in`] says so: a user's own type may hold the name of a built-in that is
/// newer than it.
pub fn is_built_in_type(name: &str) -> bool {
    BUILT_IN_TYPES.iter().any(|(n, _)| *n == name)
}

/// The key field of each built-in type a sync or the phone writes, as `(type, field)`: the
/// string field by which it captures each note once. Every version of the type the app has
/// shipped declares it, and a type the user made for their own use has no reason to, so a vault
/// type of the name that declares it as a string field is the built-in even with no record of
/// the server having written it (ticket 104). `dailynote` and `fleeting` have no key: the app
/// owns those names outright and [`check_built_in_types`] holds them to the fields it needs.
pub const BUILT_IN_KEYS: [(&str, &str); 8] = [
    (MESSAGE_TYPE, "device_ref"),
    (CALL_TYPE, "device_ref"),
    (CONVERSATION_TYPE, "thread_ref"),
    (EMAIL_TYPE, "gmail_ref"),
    (EMAIL_THREAD_TYPE, "thread_ref"),
    (EVENT_TYPE, "google_ref"),
    (AGENT_THREAD_TYPE, "transcript"),
    (OUTBOX_TYPE, "sent_ref"),
];

/// The key field of built-in type `name`, if it has one: see [`BUILT_IN_KEYS`].
fn key_field(name: &str) -> Option<&'static str> {
    BUILT_IN_KEYS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, k)| *k)
}

/// Whether vault type `ty` matches the identity of the built-in of its name: it holds a
/// built-in's name, and declares that built-in's key field as a string field when it has one.
pub fn matches_built_in(ty: &TypeDef) -> bool {
    if !is_built_in_type(&ty.name) {
        return false;
    }
    key_field(&ty.name).is_none_or(|key| {
        ty.field(key)
            .is_some_and(|f| f.kind == schema::FieldKind::String && !f.multiple)
    })
}

/// The `state.db` shipped-file path under which the server records that it wrote type `name`:
/// its type file's path, as for the other shipped files.
pub(crate) fn seeded_record(name: &str) -> String {
    crate::type_path(name)
}

/// Refuse a schema in which a built-in type has lost a field the app relies on.
pub(crate) fn check_built_in_types(schema: &Schema) -> Result<()> {
    daily::check_daily_type(schema)?;
    fleeting::check_fleeting_type(schema)
}

impl Writer {
    /// Add every built-in type the vault does not have, with `person` when a type being added
    /// needs it, in one commit, after a commit adding the built-in taxonomies the vault lacks.
    pub fn ensure_built_in_types(&self) -> Result<()> {
        self.ensure_built_in_taxonomies()?;
        let mut missing: Vec<(&str, &str)> = {
            let inner = self.lock();
            BUILT_IN_TYPES
                .into_iter()
                .filter(|(name, _)| inner.schema.type_def(name).is_none())
                .collect()
        };
        if missing.is_empty() {
            return Ok(());
        }
        let needs_person = missing
            .iter()
            .any(|(name, _)| TYPES_NEEDING_PERSON.contains(name));
        if needs_person && self.lock().schema.type_def(PERSON_TYPE).is_none() {
            missing.push((PERSON_TYPE, DEFAULT_PERSON_TYPE));
        }
        let texts = self.put_types(&missing)?;
        for ((name, _), text) in missing.iter().zip(&texts) {
            if is_built_in_type(name) {
                self.record_seeded_type(name, text)?;
            }
        }
        Ok(())
    }

    /// Whether the vault's type `name` is the built-in of that name, which cannot be deleted and
    /// gains the fields its shipped default gains. It is when the name is a built-in's and the
    /// server recorded writing the type into this vault (in `state.db`), or the type matches the
    /// built-in's identity ([`matches_built_in`]). A vault without the type is given the
    /// built-in on its next start, so the name alone answers then. A user's own type that merely
    /// shares the name, such as an `event` type with unrelated fields made before `event` was a
    /// built-in, is not: it is left alone and can be deleted.
    pub fn is_built_in(&self, name: &str) -> bool {
        if !is_built_in_type(name) {
            return false;
        }
        let schema = self.schema();
        match schema.type_def(name) {
            None => true,
            Some(ty) => self.is_built_in_def(ty),
        }
    }

    /// [`Writer::is_built_in`] for vault type `ty`.
    pub(crate) fn is_built_in_def(&self, ty: &TypeDef) -> bool {
        is_built_in_type(&ty.name)
            && (key_field(&ty.name).is_none() || matches_built_in(ty) || self.was_seeded(&ty.name))
    }

    /// Whether `state.db` records the server writing built-in type `name` into the vault.
    pub(crate) fn was_seeded(&self, name: &str) -> bool {
        matches!(self.state.shipped_file(&seeded_record(name)), Ok(Some(_)))
    }

    /// Record in `state.db` that the server wrote built-in type `name`, as `text`, into the vault,
    /// so it stays the built-in whatever the user later changes in it.
    pub(crate) fn record_seeded_type(&self, name: &str, text: &str) -> Result<()> {
        self.state
            .record_shipped(&seeded_record(name), &crate::shipped::content_hash(text))?;
        Ok(())
    }

    /// Write every built-in taxonomy the vault does not have, in one commit.
    fn ensure_built_in_taxonomies(&self) -> Result<()> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let mut changes = Vec::new();
        let mut schema = inner.schema.clone();
        for (name, source) in BUILT_IN_TAXONOMIES {
            if schema.taxonomy(name).is_some() {
                continue;
            }
            let path = crate::defs::taxonomy_path(name);
            let text = store::canonical(source, &path)?;
            let taxonomy = schema::Taxonomy::parse(name, &text)
                .map_err(|e| crate::Error::InvalidType(schema::SchemaError(vec![e])))?;
            schema.grow_taxonomy(taxonomy);
            changes.push(store::Change::write(path, text));
        }
        if changes.is_empty() {
            return Ok(());
        }
        let generation = self.commit(changes)?;
        inner.index.set_schema(&schema)?;
        inner.index.advance(generation)?;
        inner.schema = schema;
        Ok(())
    }
}
