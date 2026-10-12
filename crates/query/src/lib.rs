//! The query language of PRD Appendix B (D27, D42): parser, evaluator over the index, builder
//! model and cursor pagination.
//!
//! ```text
//! from task where under(status, "Active") && due < today() + days(7) group by status
//! ```
//!
//! [`parse`] turns text into a [`Query`], reporting the line and column of any error.
//! [`run`] evaluates it against an [`index::Index`]: `search` goes to tantivy and every other
//! clause reads `index.db` (D16). Results come a page at a time, and the cursor of one page
//! fetches the next; `limit` caps the whole result, not a page (D33). [`Model`] is the
//! structured form the visual builder edits, and converts to and from text (D8).
//!
//! Expressions are wscript expressions: the same operators with the same precedence, wscript
//! string literals, and closures written `|x| expr`. A dot follows a single reference, so
//! `project.status` reads the status of the note in the `project` field. Missing values are
//! false in a condition and never compare as less or greater than anything.

mod ast;
mod model;
mod parser;
mod print;
mod run;
mod value;

pub use ast::{BinOp, Dir, Expr, FUNCTIONS, From, Lit, Path, Query, SortKey, UnOp};
pub use model::{Condition, Group, Item, Join, Model, ModelError, Op, Operand, Order, Sort};
pub use parser::{ParseError, parse};
pub use run::{Context, Error, GroupCount, MAX_PAGE, Page, PageRequest, Result, Row, run};

/// Text in canonical form: one line, single spaces and only the parentheses that matter.
pub fn format(text: &str) -> std::result::Result<String, ParseError> {
    Ok(parse(text)?.to_string())
}
