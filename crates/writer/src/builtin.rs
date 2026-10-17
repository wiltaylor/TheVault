//! The built-in types every vault has: `dailynote`, `fleeting`, the phone data types `message`,
//! `call` and `conversation` (D15), and the Google sync types `email`, `email_thread`, `event` and
//! `google_sync` (google-sync D4, D6, D15, D17), with the `gmail_label` taxonomy `email` uses
//! (google-sync D14).
//!
//! A built-in type is added when a vault lacks it, can be customised, and cannot be deleted.
//! `dailynote` and `fleeting` keep the fields the app relies on: see [`crate::daily`] and
//! [`crate::fleeting`].
//!
//! The phone data types and the Google sync types reference `person`, which is a starter type rather than a built-in one. A
//! vault without a `person` type gets [`DEFAULT_PERSON_TYPE`] along with them, and can delete it
//! once nothing references it.

use schema::Schema;

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

/// The name of the built-in Google sync settings type: one note holds the sync's settings and
/// cursors.
pub const GOOGLE_SYNC_TYPE: &str = "google_sync";

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

/// The `google_sync` type a vault without it gets.
pub const DEFAULT_GOOGLE_SYNC_TYPE: &str = include_str!("../defaults/google_sync.wcl");

/// The `gmail_label` taxonomy a vault without it gets. It is open, so the Gmail sync adds each
/// label it meets; the file is never replaced once written.
pub const DEFAULT_GMAIL_LABEL_TAXONOMY: &str =
    include_str!("../defaults/taxonomies/gmail_label.wcl");

/// The `person` type a vault gets when it has none, because the phone data types reference it
/// (D24). The starter pack's `person` adds to it.
pub const DEFAULT_PERSON_TYPE: &str = include_str!("../defaults/person.wcl");

/// Every built-in type, as `(name, the type file a vault without it gets)`.
pub const BUILT_IN_TYPES: [(&str, &str); 9] = [
    (daily::DAILY_TYPE, daily::DEFAULT_DAILY_TYPE),
    (fleeting::FLEETING_TYPE, fleeting::DEFAULT_FLEETING_TYPE),
    (MESSAGE_TYPE, DEFAULT_MESSAGE_TYPE),
    (CALL_TYPE, DEFAULT_CALL_TYPE),
    (CONVERSATION_TYPE, DEFAULT_CONVERSATION_TYPE),
    (EMAIL_TYPE, DEFAULT_EMAIL_TYPE),
    (EMAIL_THREAD_TYPE, DEFAULT_EMAIL_THREAD_TYPE),
    (EVENT_TYPE, DEFAULT_EVENT_TYPE),
    (GOOGLE_SYNC_TYPE, DEFAULT_GOOGLE_SYNC_TYPE),
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

/// Whether `name` is a built-in type, which cannot be deleted.
pub fn is_built_in_type(name: &str) -> bool {
    BUILT_IN_TYPES.iter().any(|(n, _)| *n == name)
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
        self.put_types(&missing)?;
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
