//! The parsed form of a query. Two queries that differ only in layout, comments or redundant
//! parentheses parse to equal values.

/// A whole query: the clauses of PRD Appendix B, in their fixed order.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub from: From,
    /// `include archived`: read archived notes as well as the rest. Without it, and with a
    /// filter that does not name `archived`, archived notes are left out.
    pub include_archived: bool,
    pub filter: Option<Expr>,
    pub search: Option<String>,
    pub sort: Vec<SortKey>,
    pub group_by: Option<Path>,
    pub select: Vec<Path>,
    pub limit: Option<u64>,
}

/// The types a query reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum From {
    /// `from *`
    All,
    /// `from task, project`, in the order written.
    Types(Vec<String>),
}

/// A field name, or a dotted path through references and records such as `project.status`.
pub type Path = Vec<String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub path: Path,
    /// `None` when the query names no direction, which sorts ascending.
    pub dir: Option<Dir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

/// An expression in wscript syntax.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Lit),
    /// A field of the note in scope, or a closure parameter.
    Name(String),
    /// The note that holds the query.
    This,
    /// `expr.name`: follow a single reference, or read a record member.
    Field(Box<Expr>, String),
    /// A host function. The parser only accepts the names in [`FUNCTIONS`].
    Call(String, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `|x| expr`, accepted only as the last argument of `any` and `all`.
    Closure(Vec<String>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinOp {
    /// Binding power, tightest last. The tiers are wscript's: equality and ordering share one,
    /// so `a < b == c` is `(a < b) == c`. Every operator is left-associative.
    pub fn prec(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 3,
            BinOp::Add | BinOp::Sub => 4,
            BinOp::Mul | BinOp::Div | BinOp::Rem => 5,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Or => "||",
            BinOp::And => "&&",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
        }
    }
}

/// The built-in flag of an archived note.
pub const ARCHIVED: &str = "archived";

/// The host functions queries add to wscript, with their argument counts.
pub const FUNCTIONS: [(&str, usize); 10] = [
    ("today", 0),
    ("now", 0),
    ("days", 1),
    ("has", 2),
    ("under", 2),
    ("any", 2),
    ("all", 2),
    ("linked_from", 2),
    ("contains", 2),
    ("empty", 1),
];

impl Expr {
    /// Whether the expression reads the `archived` flag of the note in scope: the bare name
    /// `archived`, outside any closure that binds it. Such a filter decides for itself which
    /// archived notes a query reads.
    pub fn names_archived(&self) -> bool {
        match self {
            Expr::Name(n) => n == ARCHIVED,
            Expr::Lit(_) | Expr::This => false,
            // `x.archived` reads another note's flag, so only the base can name this note's.
            Expr::Field(e, _) | Expr::Unary(_, e) => e.names_archived(),
            Expr::Closure(params, e) => !params.iter().any(|p| p == ARCHIVED) && e.names_archived(),
            Expr::Call(_, args) => args.iter().any(Expr::names_archived),
            Expr::Binary(_, a, b) => a.names_archived() || b.names_archived(),
        }
    }

    /// Whether `this` appears anywhere in the expression.
    pub fn uses_this(&self) -> bool {
        match self {
            Expr::This => true,
            Expr::Lit(_) | Expr::Name(_) => false,
            Expr::Field(e, _) | Expr::Unary(_, e) | Expr::Closure(_, e) => e.uses_this(),
            Expr::Call(_, args) => args.iter().any(Expr::uses_this),
            Expr::Binary(_, a, b) => a.uses_this() || b.uses_this(),
        }
    }
}
