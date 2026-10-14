//! Fleeting notes: the built-in `fleeting` type, quick captures of text or images that wait in
//! the inbox until they are processed.
//!
//! The type can be customised but not deleted. It always keeps a `processed` field of kind
//! `bool`, because that field is how the inbox tells waiting notes from processed ones.

use schema::{FieldKind, Invalid, Issue, Schema, SchemaError};

use crate::{Error, Result, type_path};

/// The name of the built-in fleeting note type.
pub const FLEETING_TYPE: &str = "fleeting";

/// The field that marks a fleeting note as processed, so it leaves the inbox.
pub const FLEETING_PROCESSED_FIELD: &str = "processed";

/// The type a vault gets when it has none, and the one a missing type file is recreated from.
pub const DEFAULT_FLEETING_TYPE: &str = include_str!("../defaults/fleeting.wcl");

/// Refuse a schema whose `fleeting` type has lost its `processed` field, or changed it so it no
/// longer holds one bool. A schema without the type passes; the writer adds it.
pub(crate) fn check_fleeting_type(schema: &Schema) -> Result<()> {
    let Some(ty) = schema.type_def(FLEETING_TYPE) else {
        return Ok(());
    };
    let field = FLEETING_PROCESSED_FIELD;
    let message = match ty.fields.iter().find(|f| f.name == field) {
        Some(f) if f.kind == FieldKind::Bool && !f.multiple && !f.computed => return Ok(()),
        Some(_) => {
            format!("the built-in {FLEETING_TYPE} type's `{field}` field must stay one bool")
        }
        None => format!("the built-in {FLEETING_TYPE} type must keep its `{field}` field"),
    };
    Err(Error::InvalidType(SchemaError(vec![Invalid {
        file: type_path(FLEETING_TYPE),
        issues: vec![Issue::new("type", message)],
    }])))
}
