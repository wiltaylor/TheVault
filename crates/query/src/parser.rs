//! Query text to [`Query`]. Every error carries the line and column it was found at.

use std::fmt;

use crate::ast::{BinOp, Dir, Expr, FUNCTIONS, From, Lit, Path, Query, SortKey, UnOp};

/// A query that does not parse. `line` and `column` count from 1, and `column` counts
/// characters, not bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

type Result<T> = std::result::Result<T, ParseError>;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    Float(f64),
    Sym(&'static str),
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Str(_) => "a string".to_owned(),
            Tok::Int(_) | Tok::Float(_) => "a number".to_owned(),
            Tok::Sym(s) => format!("`{s}`"),
            Tok::Eof => "the end of the query".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Pos {
    line: usize,
    column: usize,
}

fn error(at: Pos, message: impl Into<String>) -> ParseError {
    ParseError {
        line: at.line,
        column: at.column,
        message: message.into(),
    }
}

/// Longest first, so `<=` is not read as `<` then `=`.
const SYMBOLS: [&str; 19] = [
    "==", "!=", "<=", ">=", "&&", "||", "<", ">", "!", "+", "-", "*", "/", "%", "(", ")", ",", ".",
    "|",
];

struct Lexer<'a> {
    src: &'a str,
    i: usize,
    line: usize,
    column: usize,
}

impl Lexer<'_> {
    fn pos(&self) -> Pos {
        Pos {
            line: self.line,
            column: self.column,
        }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn rest(&self) -> &str {
        &self.src[self.i..]
    }

    fn tokens(mut self) -> Result<Vec<(Tok, Pos)>> {
        let mut out = Vec::new();
        loop {
            while let Some(c) = self.peek() {
                if c.is_whitespace() {
                    self.bump();
                } else if self.rest().starts_with("//") {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                } else {
                    break;
                }
            }
            let at = self.pos();
            let Some(c) = self.peek() else {
                out.push((Tok::Eof, at));
                return Ok(out);
            };
            let tok = if c == '"' {
                self.string(at)?
            } else if c.is_ascii_digit() {
                self.number(at)?
            } else if c.is_alphabetic() || c == '_' {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    self.bump();
                }
                Tok::Ident(self.src[start..self.i].to_owned())
            } else if let Some(s) = SYMBOLS.iter().find(|s| self.rest().starts_with(**s)) {
                for _ in 0..s.len() {
                    self.bump();
                }
                Tok::Sym(s)
            } else {
                return Err(error(at, format!("unexpected character `{c}`")));
            };
            out.push((tok, at));
        }
    }

    fn number(&mut self, at: Pos) -> Result<Tok> {
        let mut text = String::new();
        let mut float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(c);
            } else if c == '_' {
            } else if c == '.'
                && !float
                && self.rest()[1..].starts_with(|c: char| c.is_ascii_digit())
            {
                float = true;
                text.push(c);
            } else {
                break;
            }
            self.bump();
        }
        if float {
            text.parse()
                .map(Tok::Float)
                .map_err(|_| error(at, format!("`{text}` is not a number")))
        } else {
            text.parse()
                .map(Tok::Int)
                .map_err(|_| error(at, format!("`{text}` is too large for an int")))
        }
    }

    /// A wscript string: the escapes `\n \t \r \0 \\ \" \' \u{...}`, and `{{` and `}}` for
    /// literal braces. A `{expr}` interpolation hole has no meaning in a query and is refused.
    fn string(&mut self, at: Pos) -> Result<Tok> {
        self.bump();
        let mut value = String::new();
        loop {
            let here = self.pos();
            match self.peek() {
                None | Some('\n') => return Err(error(at, "unterminated string")),
                Some('"') => {
                    self.bump();
                    return Ok(Tok::Str(value));
                }
                Some('\\') => {
                    self.bump();
                    let c = match self.bump() {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('0') => '\0',
                        Some('\\') => '\\',
                        Some('"') => '"',
                        Some('\'') => '\'',
                        Some('u') => self.unicode(here)?,
                        _ => {
                            return Err(error(
                                here,
                                "unknown escape: the escapes are \\n \\t \\r \\0 \\\\ \\\" \\' \\u{...}",
                            ));
                        }
                    };
                    value.push(c);
                }
                Some(c @ ('{' | '}')) if self.rest()[1..].starts_with(c) => {
                    self.bump();
                    self.bump();
                    value.push(c);
                }
                Some('{') if !self.rest()[1..].starts_with(['}', ':']) => {
                    return Err(error(
                        here,
                        "a query string cannot interpolate: write `{{` for a literal brace",
                    ));
                }
                Some(_) => value.extend(self.bump()),
            }
        }
    }

    fn unicode(&mut self, at: Pos) -> Result<char> {
        let bad = || error(at, "invalid unicode escape: expected `\\u{...}`");
        if self.bump() != Some('{') {
            return Err(bad());
        }
        let mut hex = String::new();
        loop {
            match self.bump() {
                Some('}') => break,
                Some(c) if c.is_ascii_hexdigit() && hex.len() < 6 => hex.push(c),
                _ => return Err(bad()),
            }
        }
        u32::from_str_radix(&hex, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(bad)
    }
}

/// Words that start a clause. They end the clause before them, so a field with one of these
/// names cannot be written bare in a `sort`, `group by` or `select` list.
const CLAUSES: [&str; 6] = ["where", "search", "sort", "group", "select", "limit"];

struct Parser {
    toks: Vec<(Tok, Pos)>,
    i: usize,
}

/// Parse query text.
pub fn parse(text: &str) -> Result<Query> {
    let toks = Lexer {
        src: text,
        i: 0,
        line: 1,
        column: 1,
    }
    .tokens()?;
    let mut p = Parser { toks, i: 0 };
    let q = p.query()?;
    Ok(q)
}

impl Parser {
    fn tok(&self) -> &Tok {
        &self.toks[self.i].0
    }

    fn pos(&self) -> Pos {
        self.toks[self.i].1
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i].0.clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.tok(), Tok::Ident(s) if s == w)
    }

    fn is_sym(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Sym(t) if *t == s)
    }

    fn eat_word(&mut self, w: &str) -> bool {
        let found = self.is_word(w);
        if found {
            self.bump();
        }
        found
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        let found = self.is_sym(s);
        if found {
            self.bump();
        }
        found
    }

    fn unexpected(&self, wanted: &str) -> ParseError {
        error(
            self.pos(),
            format!("expected {wanted}, found {}", self.tok().describe()),
        )
    }

    fn expect_sym(&mut self, s: &str) -> Result<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            Err(self.unexpected(&format!("`{s}`")))
        }
    }

    fn name(&mut self, what: &str) -> Result<String> {
        match self.tok() {
            Tok::Ident(s) if !CLAUSES.contains(&s.as_str()) => {
                let s = s.clone();
                self.bump();
                Ok(s)
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn query(&mut self) -> Result<Query> {
        if !self.eat_word("from") {
            return Err(self.unexpected("`from`"));
        }
        let from = if self.eat_sym("*") {
            From::All
        } else {
            let mut types = vec![self.name("a type name or `*`")?];
            while self.eat_sym(",") {
                types.push(self.name("a type name")?);
            }
            From::Types(types)
        };
        // `include` is not a clause word: it can only follow the type list, where a field name
        // cannot, so a field may still be called `include`.
        let include_archived = if self.eat_word("include") {
            if !self.eat_word("archived") {
                return Err(self.unexpected("`archived` after `include`"));
            }
            true
        } else {
            false
        };
        let filter = if self.eat_word("where") {
            Some(self.expr()?)
        } else {
            None
        };
        let search = if self.eat_word("search") {
            match self.tok().clone() {
                Tok::Str(s) => {
                    self.bump();
                    Some(s)
                }
                _ => return Err(self.unexpected("the search text as a string")),
            }
        } else {
            None
        };
        let mut sort = Vec::new();
        if self.eat_word("sort") {
            loop {
                let path = self.path()?;
                let dir = if self.eat_word("asc") {
                    Some(Dir::Asc)
                } else if self.eat_word("desc") {
                    Some(Dir::Desc)
                } else {
                    None
                };
                sort.push(SortKey { path, dir });
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        let group_by = if self.eat_word("group") {
            if !self.eat_word("by") {
                return Err(self.unexpected("`by`"));
            }
            Some(self.path()?)
        } else {
            None
        };
        let mut select = Vec::new();
        if self.eat_word("select") {
            select.push(self.path()?);
            while self.eat_sym(",") {
                select.push(self.path()?);
            }
        }
        let limit = if self.eat_word("limit") {
            match *self.tok() {
                Tok::Int(n) => {
                    self.bump();
                    Some(n as u64)
                }
                _ => return Err(self.unexpected("a whole number")),
            }
        } else {
            None
        };
        if *self.tok() != Tok::Eof {
            let wanted = match self.tok() {
                Tok::Ident(w) if CLAUSES.contains(&w.as_str()) => format!(
                    "the end of the query: clauses come in the order from, include archived, \
                     where, search, sort, group by, select, limit, and `{w}` is out of order or \
                     repeated"
                ),
                _ => "an operator, the next clause or the end of the query".to_owned(),
            };
            return Err(self.unexpected(&wanted));
        }
        Ok(Query {
            from,
            include_archived,
            filter,
            search,
            sort,
            group_by,
            select,
            limit,
        })
    }

    fn path(&mut self) -> Result<Path> {
        let mut path = vec![self.name("a field name")?];
        while self.eat_sym(".") {
            path.push(self.name("a field name after `.`")?);
        }
        Ok(path)
    }

    fn expr(&mut self) -> Result<Expr> {
        self.binary(1)
    }

    fn bin_op(&self) -> Option<BinOp> {
        let Tok::Sym(s) = self.tok() else {
            return None;
        };
        Some(match *s {
            "||" => BinOp::Or,
            "&&" => BinOp::And,
            "==" => BinOp::Eq,
            "!=" => BinOp::Ne,
            "<" => BinOp::Lt,
            "<=" => BinOp::Le,
            ">" => BinOp::Gt,
            ">=" => BinOp::Ge,
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            "%" => BinOp::Rem,
            _ => return None,
        })
    }

    fn binary(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        while let Some(op) = self.bin_op() {
            if op.prec() < min {
                break;
            }
            self.bump();
            let rhs = self.binary(op.prec() + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat_sym("!") {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)));
        }
        if self.eat_sym("-") {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat_sym(".") {
                let name = self.name("a field name after `.`")?;
                if self.is_sym("(") {
                    return Err(error(
                        self.pos(),
                        "methods cannot be called in a query: only the query functions can",
                    ));
                }
                e = Expr::Field(Box::new(e), name);
            } else if self.is_sym("(") {
                return Err(error(self.pos(), "only a query function can be called"));
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr> {
        let at = self.pos();
        let tok = self.tok().clone();
        if matches!(&tok, Tok::Ident(w) if CLAUSES.contains(&w.as_str())) || tok == Tok::Eof {
            return Err(self.unexpected("an expression"));
        }
        if matches!(tok, Tok::Sym(s) if s != "(" && s != "|") {
            return Err(self.unexpected("an expression"));
        }
        self.bump();
        match tok {
            Tok::Int(n) => Ok(Expr::Lit(Lit::Int(n))),
            Tok::Float(f) => Ok(Expr::Lit(Lit::Float(f))),
            Tok::Str(s) => Ok(Expr::Lit(Lit::Str(s))),
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Sym("|") => Err(error(
                at,
                "a closure can only be the last argument of `any` or `all`",
            )),
            Tok::Ident(w) if w == "true" => Ok(Expr::Lit(Lit::Bool(true))),
            Tok::Ident(w) if w == "false" => Ok(Expr::Lit(Lit::Bool(false))),
            Tok::Ident(w) if w == "this" => Ok(Expr::This),
            Tok::Ident(w) if self.is_sym("(") => self.call(w, at),
            Tok::Ident(w) => Ok(Expr::Name(w)),
            _ => Err(error(at, "expected an expression")),
        }
    }

    fn call(&mut self, name: String, at: Pos) -> Result<Expr> {
        let Some(&(_, arity)) = FUNCTIONS.iter().find(|(f, _)| *f == name) else {
            let names: Vec<&str> = FUNCTIONS.iter().map(|(f, _)| *f).collect();
            return Err(error(
                at,
                format!(
                    "unknown function `{name}`: the query functions are {}",
                    names.join(", ")
                ),
            ));
        };
        self.expect_sym("(")?;
        let closure_last = name == "any" || name == "all";
        let mut args = Vec::new();
        if !self.is_sym(")") {
            loop {
                let last = args.len() + 1 == arity;
                if closure_last && last {
                    args.push(self.closure(&name)?);
                } else {
                    args.push(self.expr()?);
                }
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        if args.len() != arity {
            return Err(error(
                at,
                format!(
                    "`{name}` takes {arity} argument{}, found {}",
                    if arity == 1 { "" } else { "s" },
                    args.len()
                ),
            ));
        }
        self.expect_sym(")")?;
        Ok(Expr::Call(name, args))
    }

    fn closure(&mut self, function: &str) -> Result<Expr> {
        if !self.eat_sym("|") {
            return Err(self.unexpected(&format!(
                "a closure such as `|x| x.done` as the last argument of `{function}`"
            )));
        }
        let mut params = vec![self.name("a closure parameter")?];
        while self.eat_sym(",") {
            params.push(self.name("a closure parameter")?);
        }
        self.expect_sym("|")?;
        if params.len() != 1 {
            return Err(self.unexpected("one closure parameter"));
        }
        Ok(Expr::Closure(params, Box::new(self.expr()?)))
    }
}
