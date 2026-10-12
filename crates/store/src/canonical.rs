//! The canonical form every vault write emits (D21).
//!
//! `wcl_lang`'s own formatter neither aligns `=` nor indents heredocs the way
//! `docs/vault-format/` writes them, so the data-only subset is printed here:
//!
//! - fields come before blocks at every level, each group in its original order;
//! - the `=` of every field in one body lines up;
//! - a blank line goes above every block that is not first in its body, except between two
//!   blocks without a body and between the items of a template `tab`;
//! - a block with no items has no braces;
//! - a multi-line string a heredoc can carry is a raw `<<'END'` heredoc indented with its field;
//! - a float always has a decimal point;
//! - comments are dropped.

use wcl_lang::ast::{Block, Expr, Field, Item, Source};

use crate::{Result, parse_data_only};

const INDENT: &str = "  ";

/// Block kinds whose body is a list of slots and content blocks laid out one per line, with no
/// blank lines between them, as `docs/vault-format/` writes a template tab.
const COMPACT_KINDS: [&str; 1] = ["tab"];

/// Check that `source` is data-only and return its canonical text. `file` names it in diagnostics.
pub fn canonical(source: &str, file: &str) -> Result<String> {
    Ok(to_canonical(&parse_data_only(source, file)?))
}

/// Print a data-only syntax tree in canonical form. Items outside the subset are skipped, so pass a
/// tree from [`parse_data_only`].
pub fn to_canonical(ast: &Source) -> String {
    let mut out = String::new();
    print_items(&mut out, &ast.items, 0, false);
    out
}

/// Print one body. `compact` drops the blank lines between blocks.
fn print_items(out: &mut String, items: &[Item], depth: usize, compact: bool) {
    let fields: Vec<&Field> = items
        .iter()
        .filter_map(|i| match i {
            Item::Field(f) => Some(f),
            _ => None,
        })
        .collect();
    let blocks = items.iter().filter_map(|i| match i {
        Item::Block(b) => Some(b),
        _ => None,
    });
    let width = fields.iter().map(|f| f.name.len()).max().unwrap_or(0);
    let indent = INDENT.repeat(depth);
    for f in &fields {
        out.push_str(&indent);
        out.push_str(&f.name);
        out.push_str(&" ".repeat(width - f.name.len()));
        out.push_str(" = ");
        print_field_value(out, &f.expr, &indent);
        out.push('\n');
    }
    let mut previous: Option<&Block> = None;
    for b in blocks {
        let both_bare = previous.is_some_and(|p| p.items.is_empty()) && b.items.is_empty();
        let blank = match previous {
            None => !fields.is_empty(),
            Some(_) => !both_bare && !compact,
        };
        if blank {
            out.push('\n');
        }
        print_block(out, b, depth);
        previous = Some(b);
    }
}

fn print_block(out: &mut String, b: &Block, depth: usize) {
    out.push_str(&INDENT.repeat(depth));
    out.push_str(&b.kind);
    for label in &b.labels {
        out.push(' ');
        match label {
            Expr::Identifier(name, _) => out.push_str(name),
            other => print_value(out, other),
        }
    }
    if b.items.is_empty() {
        out.push('\n');
        return;
    }
    out.push_str(" {\n");
    print_items(
        out,
        &b.items,
        depth + 1,
        COMPACT_KINDS.contains(&b.kind.as_str()),
    );
    out.push_str(&INDENT.repeat(depth));
    out.push_str("}\n");
}

/// A field's value: the one position where a heredoc is allowed.
fn print_field_value(out: &mut String, e: &Expr, indent: &str) {
    if let Expr::Utf8(s) = e
        && heredoc_fits(s)
    {
        let tag = heredoc_tag(s);
        out.push_str("<<'");
        out.push_str(&tag);
        out.push_str("'\n");
        for line in s.lines() {
            if !line.is_empty() {
                out.push_str(indent);
                out.push_str(line);
            }
            out.push('\n');
        }
        out.push_str(indent);
        out.push_str(&tag);
        return;
    }
    print_value(out, e);
}

fn print_value(out: &mut String, e: &Expr) {
    match e {
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::I64(n) => out.push_str(&n.to_string()),
        Expr::F64(x) => out.push_str(&float(*x)),
        Expr::Utf8(s) => quote(out, s),
        Expr::ListLit { elements, .. } => {
            out.push('[');
            for (i, el) in elements.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_value(out, el);
            }
            out.push(']');
        }
        Expr::Record { fields, .. } => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&f.name);
                out.push_str(": ");
                print_value(out, &f.value);
            }
            out.push_str(" }");
        }
        // Not reachable for a data-only tree.
        other => out.push_str(&wcl_lang::format::to_source_expr(other)),
    }
}

/// A float with a decimal point and no exponent, in the shortest digits that read back as the
/// same value.
fn float(x: f64) -> String {
    let text = x.to_string();
    if text.contains('.') || !x.is_finite() {
        text
    } else {
        format!("{text}.0")
    }
}

fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Whether a raw heredoc reads back as exactly `s`. A heredoc value is its lines, each ending in a
/// newline, with the smallest indent of its non-blank lines removed and whitespace-only lines
/// blanked.
fn heredoc_fits(s: &str) -> bool {
    if !s.ends_with('\n') || s.lines().count() < 2 || s.contains('\r') {
        return false;
    }
    let mut any_flush = false;
    for line in s.lines() {
        if line.trim().is_empty() {
            if !line.is_empty() {
                return false;
            }
        } else if !line.starts_with([' ', '\t']) {
            any_flush = true;
        }
    }
    any_flush
}

/// `END`, or `END1`, `END2` and so on when a line of the body would close `END` early.
fn heredoc_tag(s: &str) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim).collect();
    let mut tag = "END".to_owned();
    let mut n = 0;
    while lines.contains(&tag.as_str()) {
        n += 1;
        tag = format!("END{n}");
    }
    tag
}
