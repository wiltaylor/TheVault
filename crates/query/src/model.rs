//! The structured model the visual builder edits (D8). [`Model::from_text`] refuses any query
//! the model cannot hold exactly, so the builder never changes a query it does not understand:
//! such a query opens as text.

use serde::{Deserialize, Serialize};

use crate::ast::{self, BinOp, Expr, Lit, Query, UnOp};
use crate::parser::{ParseError, parse};
use crate::print::path;

/// A query as the builder shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// Type names, or empty for `from *`.
    pub from: Vec<String>,
    /// `include archived`: archived notes as well as the rest.
    #[serde(default)]
    pub include_archived: bool,
    /// The `where` clause, or `None` when there is none.
    pub filter: Option<Group>,
    pub search: Option<String>,
    pub sort: Vec<Sort>,
    pub group_by: Option<String>,
    pub select: Vec<String>,
    pub limit: Option<u64>,
}

/// Conditions joined by one operator. A group inside a group is written in parentheses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub join: Join,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Join {
    /// `&&`
    All,
    /// `||`
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Item {
    Condition(Condition),
    Group(Group),
}

/// One test of one field. `field` is a name or a dotted path such as `project.status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub field: String,
    pub op: Op,
    /// The right-hand side. `None` exactly when `op` is `is_true` or `is_false`.
    pub value: Option<Operand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `under(field, "term")`
    Under,
    /// `has(field, value)`
    Has,
    /// `field`
    IsTrue,
    /// `!field`
    IsFalse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Operand {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// `this`
    This,
    /// `today()`, or `today() + days(n)` and `today() - days(n)` for a non-zero offset.
    Today(i64),
    /// `now()`
    Now,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// No direction written, which sorts ascending.
    Default,
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sort {
    pub field: String,
    pub order: Order,
}

/// Why text did not become a model.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModelError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// The query is valid but the builder cannot show it. It opens as text.
    #[error("the builder cannot show this query: {0}")]
    Unrepresentable(String),
    /// A model edited into something that is not a query, such as an empty field name.
    #[error("the model is not a valid query: {0}")]
    Invalid(String),
}

type Result<T> = std::result::Result<T, ModelError>;

fn unrepresentable<T>(why: impl Into<String>) -> Result<T> {
    Err(ModelError::Unrepresentable(why.into()))
}

impl Model {
    /// Parse text into a model. A query the model cannot hold exactly is
    /// [`ModelError::Unrepresentable`].
    pub fn from_text(text: &str) -> Result<Model> {
        Model::from_query(&parse(text)?)
    }

    /// The model of a parsed query, checked to convert back to an equal query.
    pub fn from_query(q: &Query) -> Result<Model> {
        let from = match &q.from {
            ast::From::All => Vec::new(),
            ast::From::Types(t) => t.clone(),
        };
        let filter = q.filter.as_ref().map(group).transpose()?;
        let model = Model {
            from,
            include_archived: q.include_archived,
            filter,
            search: q.search.clone(),
            sort: q
                .sort
                .iter()
                .map(|k| Sort {
                    field: path(&k.path),
                    order: match k.dir {
                        None => Order::Default,
                        Some(ast::Dir::Asc) => Order::Asc,
                        Some(ast::Dir::Desc) => Order::Desc,
                    },
                })
                .collect(),
            group_by: q.group_by.as_ref().map(path),
            select: q.select.iter().map(path).collect(),
            limit: q.limit,
        };
        // The rules above are meant to be exact. This makes sure of it: anything that would come
        // back different opens as text instead.
        if model.to_query().ok().as_ref() != Some(q) {
            return unrepresentable("converting it back would change it");
        }
        Ok(model)
    }

    /// The model as canonical query text.
    pub fn to_text(&self) -> Result<String> {
        Ok(self.to_query()?.to_string())
    }

    /// The model as a query.
    pub fn to_query(&self) -> Result<Query> {
        let from = if self.from.is_empty() {
            ast::From::All
        } else {
            for t in &self.from {
                name(t)?;
            }
            ast::From::Types(self.from.clone())
        };
        let filter = match &self.filter {
            Some(g) => Some(group_expr(g)?),
            None => None,
        };
        let sort = self
            .sort
            .iter()
            .map(|s| {
                Ok(ast::SortKey {
                    path: field_path(&s.field)?,
                    dir: match s.order {
                        Order::Default => None,
                        Order::Asc => Some(ast::Dir::Asc),
                        Order::Desc => Some(ast::Dir::Desc),
                    },
                })
            })
            .collect::<Result<_>>()?;
        Ok(Query {
            from,
            include_archived: self.include_archived,
            filter,
            search: self.search.clone(),
            sort,
            group_by: self.group_by.as_deref().map(field_path).transpose()?,
            select: self
                .select
                .iter()
                .map(|s| field_path(s))
                .collect::<Result<_>>()?,
            limit: self.limit,
        })
    }
}

fn name(s: &str) -> Result<()> {
    let mut chars = s.chars();
    let ok = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(ModelError::Invalid(format!("{s:?} is not a name")))
    }
}

fn field_path(s: &str) -> Result<ast::Path> {
    let parts: Vec<String> = s.split('.').map(str::to_owned).collect();
    for p in &parts {
        name(p)?;
    }
    Ok(parts)
}

/// The fields of a dotted path expression, or `None` when it is something else.
fn expr_path(e: &Expr) -> Option<String> {
    match e {
        Expr::Name(n) => Some(n.clone()),
        Expr::Field(inner, n) => Some(format!("{}.{n}", expr_path(inner)?)),
        _ => None,
    }
}

fn path_expr(s: &str) -> Result<Expr> {
    let mut parts = field_path(s)?.into_iter();
    let mut e = Expr::Name(parts.next().unwrap_or_default());
    for p in parts {
        e = Expr::Field(Box::new(e), p);
    }
    Ok(e)
}

fn group(e: &Expr) -> Result<Group> {
    let join = match e {
        Expr::Binary(BinOp::And, ..) => Join::All,
        Expr::Binary(BinOp::Or, ..) => Join::Any,
        _ => {
            return Ok(Group {
                join: Join::All,
                items: vec![item(e)?],
            });
        }
    };
    let mut items = Vec::new();
    flatten(e, join, &mut items)?;
    Ok(Group { join, items })
}

fn join_op(j: Join) -> BinOp {
    match j {
        Join::All => BinOp::And,
        Join::Any => BinOp::Or,
    }
}

/// The operands of a left-leaning chain of one operator, in order.
fn flatten(e: &Expr, join: Join, out: &mut Vec<Item>) -> Result<()> {
    match e {
        Expr::Binary(op, a, b) if *op == join_op(join) => {
            flatten(a, join, out)?;
            out.push(item(b)?);
            Ok(())
        }
        _ => {
            out.push(item(e)?);
            Ok(())
        }
    }
}

fn item(e: &Expr) -> Result<Item> {
    match e {
        Expr::Binary(BinOp::And | BinOp::Or, ..) => Ok(Item::Group(group(e)?)),
        _ => Ok(Item::Condition(condition(e)?)),
    }
}

fn condition(e: &Expr) -> Result<Condition> {
    let cond = |field: String, op: Op, value: Option<Operand>| Condition { field, op, value };
    if let Some(field) = expr_path(e) {
        return Ok(cond(field, Op::IsTrue, None));
    }
    match e {
        Expr::Unary(UnOp::Not, inner) => match expr_path(inner) {
            Some(field) => Ok(cond(field, Op::IsFalse, None)),
            None => unrepresentable("`!` applies to something other than a field"),
        },
        Expr::Binary(op, a, b) => {
            let op = match op {
                BinOp::Eq => Op::Eq,
                BinOp::Ne => Op::Ne,
                BinOp::Lt => Op::Lt,
                BinOp::Le => Op::Le,
                BinOp::Gt => Op::Gt,
                BinOp::Ge => Op::Ge,
                _ => {
                    return unrepresentable(format!("`{}` is not a comparison", op.symbol()));
                }
            };
            let Some(field) = expr_path(a) else {
                return unrepresentable("a comparison does not start with a field");
            };
            Ok(cond(field, op, Some(operand(b)?)))
        }
        Expr::Call(f, args) if f == "under" || f == "has" => {
            let Some(field) = expr_path(&args[0]) else {
                return unrepresentable(format!("`{f}` is not given a field"));
            };
            let op = if f == "under" { Op::Under } else { Op::Has };
            Ok(cond(field, op, Some(operand(&args[1])?)))
        }
        Expr::Call(f, _) => unrepresentable(format!("the builder has no condition for `{f}`")),
        _ => unrepresentable("a condition is not a field test"),
    }
}

fn operand(e: &Expr) -> Result<Operand> {
    let days = |e: &Expr| match e {
        Expr::Call(f, args) if f == "days" => match args.as_slice() {
            [Expr::Lit(Lit::Int(n))] if *n > 0 => Some(*n),
            _ => None,
        },
        _ => None,
    };
    let is_today = |e: &Expr| matches!(e, Expr::Call(f, _) if f == "today");
    Ok(match e {
        Expr::Lit(Lit::Str(s)) => Operand::Str(s.clone()),
        Expr::Lit(Lit::Int(n)) => Operand::Int(*n),
        Expr::Lit(Lit::Float(x)) => Operand::Float(*x),
        Expr::Lit(Lit::Bool(b)) => Operand::Bool(*b),
        Expr::This => Operand::This,
        Expr::Call(f, _) if f == "now" => Operand::Now,
        e if is_today(e) => Operand::Today(0),
        Expr::Binary(op @ (BinOp::Add | BinOp::Sub), a, b) if is_today(a) => match days(b) {
            Some(n) if *op == BinOp::Add => Operand::Today(n),
            Some(n) => Operand::Today(-n),
            None => return unrepresentable("a date offset is not `days(n)` with n above zero"),
        },
        _ => return unrepresentable("a value is not a literal, `this`, `today()` or `now()`"),
    })
}

fn group_expr(g: &Group) -> Result<Expr> {
    let mut exprs = g.items.iter().map(|i| match i {
        Item::Condition(c) => condition_expr(c),
        Item::Group(g) => group_expr(g),
    });
    let first = exprs
        .next()
        .ok_or_else(|| ModelError::Invalid("a group has no conditions".to_owned()))??;
    exprs.try_fold(first, |acc, e| {
        Ok(Expr::Binary(join_op(g.join), Box::new(acc), Box::new(e?)))
    })
}

fn condition_expr(c: &Condition) -> Result<Expr> {
    let field = path_expr(&c.field)?;
    let value = || -> Result<Expr> {
        match &c.value {
            Some(v) => Ok(operand_expr(v)),
            None => Err(ModelError::Invalid(format!(
                "the condition on {} has no value",
                c.field
            ))),
        }
    };
    let binary = |op: BinOp| -> Result<Expr> {
        Ok(Expr::Binary(
            op,
            Box::new(field.clone()),
            Box::new(value()?),
        ))
    };
    match c.op {
        Op::Eq => binary(BinOp::Eq),
        Op::Ne => binary(BinOp::Ne),
        Op::Lt => binary(BinOp::Lt),
        Op::Le => binary(BinOp::Le),
        Op::Gt => binary(BinOp::Gt),
        Op::Ge => binary(BinOp::Ge),
        Op::Under => Ok(Expr::Call(
            "under".to_owned(),
            vec![field.clone(), value()?],
        )),
        Op::Has => Ok(Expr::Call("has".to_owned(), vec![field.clone(), value()?])),
        Op::IsTrue => Ok(field),
        Op::IsFalse => Ok(Expr::Unary(UnOp::Not, Box::new(field))),
    }
}

fn operand_expr(v: &Operand) -> Expr {
    let call = |f: &str, args: Vec<Expr>| Expr::Call(f.to_owned(), args);
    match v {
        Operand::Str(s) => Expr::Lit(Lit::Str(s.clone())),
        Operand::Int(n) => Expr::Lit(Lit::Int(*n)),
        Operand::Float(x) => Expr::Lit(Lit::Float(*x)),
        Operand::Bool(b) => Expr::Lit(Lit::Bool(*b)),
        Operand::This => Expr::This,
        Operand::Now => call("now", vec![]),
        Operand::Today(0) => call("today", vec![]),
        Operand::Today(n) => Expr::Binary(
            if *n > 0 { BinOp::Add } else { BinOp::Sub },
            Box::new(call("today", vec![])),
            Box::new(call("days", vec![Expr::Lit(Lit::Int(n.abs()))])),
        ),
    }
}
