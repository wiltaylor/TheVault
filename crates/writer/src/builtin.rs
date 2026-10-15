//! The built-in types every vault has: `dailynote`, `fleeting`, and the phone data types
//! `message`, `call` and `conversation` (D15).
//!
//! A built-in type is added when a vault lacks it, can be customised, and cannot be deleted.
//! `dailynote` and `fleeting` keep the fields the app relies on: see [`crate::daily`] and
//! [`crate::fleeting`].
//!
//! The phone data types reference `person`, which is a starter type rather than a built-in one. A
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

/// The name of the type the phone data types reference.
pub const PERSON_TYPE: &str = "person";

/// The `message` type a vault without it gets.
pub const DEFAULT_MESSAGE_TYPE: &str = include_str!("../defaults/message.wcl");

/// The `call` type a vault without it gets.
pub const DEFAULT_CALL_TYPE: &str = include_str!("../defaults/call.wcl");

/// The `conversation` type a vault without it gets.
pub const DEFAULT_CONVERSATION_TYPE: &str = include_str!("../defaults/conversation.wcl");

/// The `person` type a vault gets when it has none, because the phone data types reference it
/// (D24). The starter pack's `person` adds to it.
pub const DEFAULT_PERSON_TYPE: &str = include_str!("../defaults/person.wcl");

/// Every built-in type, as `(name, the type file a vault without it gets)`.
pub const BUILT_IN_TYPES: [(&str, &str); 5] = [
    (daily::DAILY_TYPE, daily::DEFAULT_DAILY_TYPE),
    (fleeting::FLEETING_TYPE, fleeting::DEFAULT_FLEETING_TYPE),
    (MESSAGE_TYPE, DEFAULT_MESSAGE_TYPE),
    (CALL_TYPE, DEFAULT_CALL_TYPE),
    (CONVERSATION_TYPE, DEFAULT_CONVERSATION_TYPE),
];

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
    /// needs it, in one commit.
    pub fn ensure_built_in_types(&self) -> Result<()> {
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
            .any(|(name, _)| [MESSAGE_TYPE, CALL_TYPE, CONVERSATION_TYPE].contains(name));
        if needs_person && self.lock().schema.type_def(PERSON_TYPE).is_none() {
            missing.push((PERSON_TYPE, DEFAULT_PERSON_TYPE));
        }
        self.put_types(&missing)?;
        Ok(())
    }
}
