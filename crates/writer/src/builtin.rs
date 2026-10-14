//! The built-in types every vault has: `dailynote` and `fleeting`.
//!
//! A built-in type is added when a vault lacks it, can be customised, and cannot be deleted. Each
//! keeps the fields the app relies on: see [`crate::daily`] and [`crate::fleeting`].

use schema::Schema;

use crate::{Result, Writer, daily, fleeting};

/// Every built-in type, as `(name, the type file a vault without it gets)`.
pub const BUILT_IN_TYPES: [(&str, &str); 2] = [
    (daily::DAILY_TYPE, daily::DEFAULT_DAILY_TYPE),
    (fleeting::FLEETING_TYPE, fleeting::DEFAULT_FLEETING_TYPE),
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
    /// Add every built-in type the vault does not have.
    pub fn ensure_built_in_types(&self) -> Result<()> {
        for (name, source) in BUILT_IN_TYPES {
            if self.lock().schema.type_def(name).is_some() {
                continue;
            }
            self.put_type(name, source)?;
        }
        Ok(())
    }
}
