//! [`Query`] to canonical text: one line, single spaces, and only the parentheses that change
//! the meaning. Parsing the text gives back an equal query.

use std::fmt::{self, Display, Write};

use crate::ast::{Dir, Expr, From, Lit, Path, Query, UnOp};

impl Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("from ")?;
        match &self.from {
            From::All => f.write_str("*")?,
            From::Types(t) => f.write_str(&t.join(", "))?,
        }
        if self.include_archived {
            f.write_str(" include archived")?;
        }
        if let Some(e) = &self.filter {
            write!(f, " where {e}")?;
        }
        if let Some(s) = &self.search {
            f.write_str(" search ")?;
            string(f, s)?;
        }
        if !self.sort.is_empty() {
            f.write_str(" sort ")?;
            for (i, k) in self.sort.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                f.write_str(&path(&k.path))?;
                match k.dir {
                    Some(Dir::Asc) => f.write_str(" asc")?,
                    Some(Dir::Desc) => f.write_str(" desc")?,
                    None => {}
                }
            }
        }
        if let Some(g) = &self.group_by {
            write!(f, " group by {}", path(g))?;
        }
        if !self.select.is_empty() {
            let paths: Vec<String> = self.select.iter().map(path).collect();
            write!(f, " select {}", paths.join(", "))?;
        }
        if let Some(n) = self.limit {
            write!(f, " limit {n}")?;
        }
        Ok(())
    }
}

pub(crate) fn path(p: &Path) -> String {
    p.join(".")
}

/// Binding power of an expression as an operand: atoms and postfix forms bind tightest.
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Binary(op, ..) => op.prec(),
        Expr::Closure(..) => 0,
        Expr::Unary(..) => 6,
        _ => 7,
    }
}

impl Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Lit(Lit::Bool(b)) => write!(f, "{b}"),
            Expr::Lit(Lit::Int(n)) => write!(f, "{n}"),
            Expr::Lit(Lit::Float(x)) => write!(f, "{x:?}"),
            Expr::Lit(Lit::Str(s)) => string(f, s),
            Expr::Name(n) => f.write_str(n),
            Expr::This => f.write_str("this"),
            Expr::Field(e, name) => {
                operand(f, e, 7)?;
                write!(f, ".{name}")
            }
            Expr::Call(name, args) => {
                write!(f, "{name}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{a}")?;
                }
                f.write_str(")")
            }
            Expr::Unary(op, e) => {
                f.write_str(match op {
                    UnOp::Not => "!",
                    UnOp::Neg => "-",
                })?;
                operand(f, e, 6)
            }
            Expr::Binary(op, a, b) => {
                operand(f, a, op.prec())?;
                write!(f, " {} ", op.symbol())?;
                // Left-associative: an operand of the same tier on the right needs parentheses.
                operand(f, b, op.prec() + 1)
            }
            Expr::Closure(params, body) => write!(f, "|{}| {body}", params.join(", ")),
        }
    }
}

fn operand(f: &mut fmt::Formatter<'_>, e: &Expr, min: u8) -> fmt::Result {
    if prec(e) < min {
        write!(f, "({e})")
    } else {
        write!(f, "{e}")
    }
}

/// A wscript string literal.
fn string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\t' => f.write_str("\\t")?,
            '\r' => f.write_str("\\r")?,
            '\0' => f.write_str("\\0")?,
            '{' => f.write_str("{{")?,
            '}' => f.write_str("}}")?,
            c if c.is_control() => write!(f, "\\u{{{:x}}}", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}
