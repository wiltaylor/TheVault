//! A plain view of a data-only vault file: fields of literal values and labelled blocks.
//!
//! `store` parses and checks the file (D20). Everything in `schema` reads this view instead of
//! the syntax tree, so the rules below never meet trivia, spans or expression variants.

use wcl_lang::ast::{Block, Expr, Item, Source};

use crate::{Invalid, Issue};

/// A literal value. Heredocs are strings; a number without a fraction or exponent is an `Int`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<Value>),
    /// Record members in source order.
    Record(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A number read as a float, which is how a `float` field reads an integer literal.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Value::Str(_) => "a string",
            Value::Int(_) => "an integer",
            Value::Float(_) => "a float",
            Value::Bool(_) => "a boolean",
            Value::List(_) => "a list",
            Value::Record(_) => "a record",
        }
    }
}

/// One body: the fields and the blocks of a file or of a block, each in source order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub fields: Vec<(String, Value)>,
    pub blocks: Vec<DocBlock>,
}

/// A block: its kind, its labels and its body.
#[derive(Debug, Clone, PartialEq)]
pub struct DocBlock {
    pub kind: String,
    pub labels: Vec<String>,
    pub body: Doc,
}

impl DocBlock {
    /// The first label, which every labelled vault block uses as its name.
    pub fn label(&self) -> Option<&str> {
        self.labels.first().map(String::as_str)
    }

    /// `kind` or `kind "label"`, for diagnostics.
    pub fn name(&self) -> String {
        match self.label() {
            Some(l) => format!("{} {l:?}", self.kind),
            None => self.kind.clone(),
        }
    }
}

impl Doc {
    /// Parse `source`, check it is data-only through `store`, and return its view. `file` names
    /// the source in diagnostics.
    pub fn parse(source: &str, file: &str) -> Result<Doc, Invalid> {
        let ast = store::parse_data_only(source, file).map_err(|e| Invalid {
            file: file.to_owned(),
            issues: vec![Issue::new("file", e.to_string())],
        })?;
        Ok(Doc::from_source(&ast))
    }

    /// The view of a syntax tree that `store::parse_data_only` accepted. Items outside the
    /// data-only subset are skipped. Canvas nodes of the kinds older vaults used are read as the
    /// cards that replace them (see [`crate::migrate`]).
    pub fn from_source(ast: &Source) -> Doc {
        let mut doc = body(&ast.items);
        crate::migrate::canvases(&mut doc);
        doc
    }

    /// The first field called `name`.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn blocks_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a DocBlock> + 'a {
        self.blocks.iter().filter(move |b| b.kind == kind)
    }
}

fn body(items: &[Item]) -> Doc {
    let mut doc = Doc::default();
    for item in items {
        match item {
            Item::Field(f) => {
                if let Some(v) = value(&f.expr) {
                    doc.fields.push((f.name.clone(), v));
                }
            }
            Item::Block(b) => doc.blocks.push(block(b)),
            _ => {}
        }
    }
    doc
}

fn block(b: &Block) -> DocBlock {
    DocBlock {
        kind: b.kind.clone(),
        labels: b
            .labels
            .iter()
            .filter_map(|l| match l {
                Expr::Utf8(s) | Expr::Identifier(s, _) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        body: body(&b.items),
    }
}

fn value(e: &Expr) -> Option<Value> {
    Some(match e {
        Expr::Utf8(s) => Value::Str(s.clone()),
        Expr::I64(i) => Value::Int(*i),
        Expr::F64(f) => Value::Float(*f),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::ListLit { elements, .. } => Value::List(elements.iter().filter_map(value).collect()),
        Expr::Record { fields, .. } => Value::Record(
            fields
                .iter()
                .filter_map(|f| Some((f.name.clone(), value(&f.value)?)))
                .collect(),
        ),
        _ => return None,
    })
}
