//! The data-only check (D20): a vault file holds fields and blocks of literal values, nothing else.
//!
//! The grammar is in `docs/vault-format.md`, "The data-only subset". The syntax tree from
//! `wcl_lang::parse_for_edit` decides the shape of every item and value. It does not record how a
//! literal was spelled, so a second pass over the tokens rejects typed or unit numbers, radix
//! prefixes, `_` separators, string prefixes and escapes outside the allowed set.

use wcl_lang::ast::{Block, Expr, Field, Item, Source, Span};
use wcl_lang::{Lexer, StringLit, TokenKind};

use crate::{Error, Result};

/// A file left the data-only subset. Names the first offending item and its line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}:{line}: {item}: {message}")]
pub struct DataOnlyError {
    /// The file name the source was checked under.
    pub file: String,
    /// The 1-based line of the offending text.
    pub line: usize,
    /// The offending item: a dotted field path, a block, or the declaration as written.
    pub item: String,
    /// What is wrong with it.
    pub message: String,
}

/// Parse `source` and check it is data-only. `file` names the source in diagnostics.
pub fn parse_data_only(source: &str, file: &str) -> Result<Source> {
    let ast = wcl_lang::parse_for_edit(source, file).map_err(|e| Error::Parse {
        file: file.to_owned(),
        message: parse_message(source, &e),
    })?;
    let mut check = Check {
        source,
        file,
        items: Vec::new(),
    };
    check.items(&ast.items, "")?;
    check.tokens()?;
    Ok(ast)
}

fn parse_message(source: &str, e: &wcl_lang::ParseError) -> String {
    match e {
        wcl_lang::ParseError::Syntax(err) => {
            format!(
                "line {}: {}",
                line_of(source, err.span.offset()),
                err.message
            )
        }
        other => other.to_string(),
    }
}

struct Check<'a> {
    source: &'a str,
    file: &'a str,
    /// Every item seen, with its span and name, so a token-level error can name its item.
    items: Vec<(Span, String)>,
}

impl Check<'_> {
    fn fail(&self, at: usize, item: impl Into<String>, message: impl Into<String>) -> Error {
        Error::DataOnly(DataOnlyError {
            file: self.file.to_owned(),
            line: line_of(self.source, at),
            item: item.into(),
            message: message.into(),
        })
    }

    fn items(&mut self, items: &[Item], prefix: &str) -> Result<()> {
        for item in items {
            match item {
                Item::Field(f) => self.field(f, prefix)?,
                Item::Block(b) => self.block(b, prefix)?,
                other => {
                    let span = other.span();
                    let written = first_line(&self.source[span.start..span.end]);
                    let what = match other {
                        Item::Import(_) => "imports are not allowed in vault files",
                        Item::Let(_) => "`let` and `fn` items are expressions",
                        Item::Table(_) => "tables are not allowed in vault files",
                        Item::Connection(_) => {
                            "connection statements are not allowed in vault files"
                        }
                        _ => "declarations are not allowed in vault files",
                    };
                    return Err(self.fail(span.start, format!("`{written}`"), what));
                }
            }
        }
        Ok(())
    }

    fn field(&mut self, f: &Field, prefix: &str) -> Result<()> {
        let path = format!("{prefix}{}", f.name);
        self.items.push((f.span, path.clone()));
        if !is_ident(&f.name) {
            return Err(self.fail(f.span.start, &path, "field names must be identifiers"));
        }
        if let Some(d) = f.decorators.first() {
            return Err(self.fail(
                d.name_span.start,
                &path,
                "decorators are not allowed in vault files",
            ));
        }
        self.value(&f.expr, f.span, &path)
    }

    fn block(&mut self, b: &Block, prefix: &str) -> Result<()> {
        let mut name = b.kind.clone();
        if let Some(label) = b.labels.first() {
            match label {
                Expr::Utf8(s) => name.push_str(&format!("[{s:?}]")),
                Expr::Identifier(s, _) => name.push_str(&format!("[{s}]")),
                _ => {}
            }
        }
        let path = format!("{prefix}{name}");
        self.items.push((b.span, path.clone()));
        if !b.kind_ns.is_empty() || !is_ident(&b.kind) {
            return Err(self.fail(b.span.start, &path, "block kinds must be plain identifiers"));
        }
        if let Some(d) = b.decorators.first() {
            return Err(self.fail(
                d.name_span.start,
                &path,
                "decorators are not allowed in vault files",
            ));
        }
        if b.conditional || b.slot_decl.is_some() {
            return Err(self.fail(
                b.span.start,
                &path,
                "conditional and slot blocks are not allowed in vault files",
            ));
        }
        for label in &b.labels {
            match label {
                Expr::Utf8(_) => {}
                Expr::Identifier(s, _) if is_bare_label(s) => {}
                other => {
                    return Err(self.fail(
                        b.span.start,
                        &path,
                        format!("label {} is not a string or a bare label", describe(other)),
                    ));
                }
            }
        }
        self.items(&b.items, &format!("{path}."))
    }

    fn value(&self, e: &Expr, at: Span, path: &str) -> Result<()> {
        match e {
            Expr::Bool(_) | Expr::I64(_) | Expr::F64(_) | Expr::Utf8(_) => Ok(()),
            Expr::ListLit { elements, .. } => {
                for el in elements {
                    self.value(el, at, path)?;
                }
                Ok(())
            }
            Expr::Record { fields, .. } => {
                for f in fields {
                    if !is_ident(&f.name) {
                        return Err(self.fail(
                            f.span.start,
                            path,
                            format!("record key {:?} is not an identifier", f.name),
                        ));
                    }
                    self.value(&f.value, f.span, &format!("{path}.{}", f.name))?;
                }
                Ok(())
            }
            Expr::None => Err(self.fail(
                at.start,
                path,
                "`none` is not allowed; leave the field out instead",
            )),
            other => Err(self.fail(
                expr_start(other).unwrap_or(at.start),
                path,
                format!(
                    "{} is an expression; vault files hold literal data only",
                    describe(other)
                ),
            )),
        }
    }

    /// Reject literal spellings the syntax tree does not record.
    fn tokens(&self) -> Result<()> {
        let mut lexer = Lexer::new(self.source);
        loop {
            let tok = match lexer.next_token() {
                Ok(t) => t,
                // The parser already accepted the file, so the lexer cannot fail here.
                Err(_) => return Ok(()),
            };
            let text = &self.source[tok.span.start..tok.span.end];
            let problem = match &tok.kind {
                TokenKind::Eof => return Ok(()),
                TokenKind::Number(_) if !is_plain_number(text) => Some(format!(
                    "number `{text}` has a suffix, radix prefix or `_`; write a plain decimal"
                )),
                TokenKind::NumberWithUnit(_) => {
                    Some(format!("number `{text}` has a unit; write a plain decimal"))
                }
                TokenKind::Str(StringLit::Utf8(_)) => string_problem(text),
                TokenKind::Str(StringLit::Interpolated { .. }) => {
                    Some("string interpolation is an expression".to_owned())
                }
                TokenKind::Str(_) => Some(format!(
                    "string `{}` has an encoding prefix; strings are plain UTF-8",
                    first_line(text)
                )),
                _ => None,
            };
            if let Some(message) = problem {
                return Err(self.fail(tok.span.start, self.item_at(tok.span.start), message));
            }
        }
    }

    /// The innermost item containing byte `at`.
    fn item_at(&self, at: usize) -> String {
        self.items
            .iter()
            .filter(|(s, _)| s.start <= at && at < s.end)
            .min_by_key(|(s, _)| s.end - s.start)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "file".to_owned())
    }
}

/// A quoted string or a heredoc is fine; anything else in front of the quote is a prefix. WCL
/// itself accepts only the escapes the data-only subset allows.
fn string_problem(text: &str) -> Option<String> {
    if text.starts_with('"') || text.starts_with("<<") {
        return None;
    }
    Some(format!(
        "string `{}` has a prefix; strings are plain UTF-8",
        first_line(text)
    ))
}

fn is_plain_number(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    if !digits(&mut i) {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len()
}

pub(crate) fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_bare_label(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'))
}

fn describe(e: &Expr) -> String {
    match e {
        Expr::Identifier(name, _) => format!("reference `{name}`"),
        Expr::Symbol(name, _) => format!("symbol `:{name}`"),
        Expr::Binary { op, .. } => format!("operator `{}`", op.as_str()),
        Expr::Unary { .. } => "a prefix operator".to_owned(),
        Expr::Call { .. } => "a function call".to_owned(),
        Expr::InterpolatedString { .. } => "string interpolation".to_owned(),
        Expr::Member { .. } => "a member access".to_owned(),
        Expr::Function(_) => "a function".to_owned(),
        Expr::If { .. } | Expr::IfLet { .. } | Expr::Match { .. } => "a conditional".to_owned(),
        Expr::UnitLiteral { .. } => "a number with a unit".to_owned(),
        Expr::Ascii(_) | Expr::Utf16(_) | Expr::Utf32(_) => "a prefixed string".to_owned(),
        Expr::Paren { .. } | Expr::Block { .. } => "a grouped expression".to_owned(),
        _ => "a typed literal".to_owned(),
    }
}

fn expr_start(e: &Expr) -> Option<usize> {
    match e {
        Expr::Identifier(_, s) | Expr::Symbol(_, s) => Some(s.start),
        Expr::Binary { span, .. }
        | Expr::Unary { span, .. }
        | Expr::Call { span, .. }
        | Expr::InterpolatedString { span, .. }
        | Expr::Member { span, .. }
        | Expr::If { span, .. }
        | Expr::IfLet { span, .. }
        | Expr::Match { span, .. }
        | Expr::UnitLiteral { span, .. }
        | Expr::Paren { span, .. }
        | Expr::Block { span, .. } => Some(span.start),
        _ => None,
    }
}

fn line_of(source: &str, at: usize) -> usize {
    source[..at.min(source.len())].matches('\n').count() + 1
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim_end()
}
