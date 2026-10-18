//! The text of the query language of PRD Appendix B (D27, D42): its parsed form, the parser and
//! the canonical printer, with no dependency on the index or the schema.
//!
//! The `query` crate evaluates what this crate parses. The `schema` crate reads it too, to check
//! the `when` condition of a template tab (D14) when a type is saved.

pub mod ast;
pub mod parser;
pub mod print;

pub use ast::{ARCHIVED, BinOp, Dir, Expr, FUNCTIONS, From, Lit, Path, Query, SortKey, UnOp};
pub use parser::{ParseError, parse, parse_expr};

/// Text in canonical form: one line, single spaces and only the parentheses that matter.
pub fn format(text: &str) -> std::result::Result<String, ParseError> {
    Ok(parse(text)?.to_string())
}
