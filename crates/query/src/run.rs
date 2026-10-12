//! Running a query: `search` against tantivy, everything else against `index.db` (D16), and the
//! ordered result cut into pages by an opaque cursor (D33).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params_from_iter};
use schema::{FieldDef, FieldKind, Schema};
use serde::Serialize;

use crate::ast::{BinOp, Dir, Expr, From, Lit, Path, Query, UnOp};
use crate::print::path as path_text;
use crate::value::{self, Eval, Key, V};

/// Fields every note has, besides those its type declares.
const BUILTINS: [&str; 5] = ["id", "type", "title", "created", "updated"];

/// The most rows one page may hold.
pub const MAX_PAGE: usize = 1000;

/// Why a query could not run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "the query uses `this`, which is the note that holds the query, but it was not run from a note"
    )]
    NoThis,
    #[error("there is no type {0:?}")]
    UnknownType(String),
    #[error("unknown field `{field}`: {types} {has} no such field")]
    UnknownField {
        field: String,
        types: String,
        has: &'static str,
    },
    #[error("`{field}` is not a reference field of {type_name}, so linked_from cannot follow it")]
    NotAReference { type_name: String, field: String },
    #[error("taxonomy {taxonomy} has no term {term:?}")]
    UnknownTerm { taxonomy: String, term: String },
    #[error("note {note}: {message}")]
    Eval { note: String, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("the cursor is not one this query returned")]
    BadCursor,
    #[error(transparent)]
    Index(#[from] index::Error),
    #[error("index.db: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a query runs against.
pub struct Context<'a> {
    pub index: &'a index::Index,
    pub schema: &'a Schema,
    /// The note that holds the query, bound to `this`.
    pub this: Option<String>,
    /// The current time in seconds since the Unix epoch, for `now()` and `today()`.
    pub now: f64,
    /// The offset of local time from UTC in seconds. `today()` is the local date.
    pub utc_offset: i64,
}

impl<'a> Context<'a> {
    /// A context at the current time, in UTC, with no `this`.
    pub fn new(index: &'a index::Index, schema: &'a Schema) -> Context<'a> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or_default();
        Context {
            index,
            schema,
            this: None,
            now,
            utc_offset: 0,
        }
    }

    /// Bind `this` to a note.
    pub fn with_this(mut self, id: impl Into<String>) -> Context<'a> {
        self.this = Some(id.into());
        self
    }

    /// Fix the clock.
    pub fn at(mut self, now: f64) -> Context<'a> {
        self.now = now;
        self
    }
}

/// Which page to return.
#[derive(Debug, Clone, Default)]
pub struct PageRequest {
    /// `next` of the previous page, or `None` for the first page.
    pub cursor: Option<String>,
    /// Rows per page, from 1 to [`MAX_PAGE`]. Zero means [`MAX_PAGE`].
    pub size: usize,
}

/// One page of a result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    pub rows: Vec<Row>,
    /// The cursor of the next page, or `None` on the last page.
    pub next: Option<String>,
    /// Rows in the whole result, after `limit`.
    pub total: usize,
    /// With `group by`: every group of the whole result, in order, with its row count.
    pub groups: Vec<GroupCount>,
}

/// One result note.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub id: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub title: String,
    /// The selected fields keyed by their path as written, or every stored field without
    /// `select`. Values are as stored in the note.
    pub fields: serde_json::Map<String, serde_json::Value>,
    /// With `group by`: the value this row is grouped under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupCount {
    pub value: serde_json::Value,
    pub count: usize,
}

/// A note row from `index.db`.
struct Note {
    id: String,
    type_name: String,
    title: String,
    created: Option<String>,
    updated: Option<String>,
    data: serde_json::Map<String, serde_json::Value>,
}

const NOTE_COLUMNS: &str = "id, type, title, created, updated, data";

fn read_note(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let data: String = r.get(5)?;
    Ok(Note {
        id: r.get(0)?,
        type_name: r.get(1)?,
        title: r.get(2)?,
        created: r.get(3)?,
        updated: r.get(4)?,
        data: serde_json::from_str(&data).unwrap_or_default(),
    })
}

struct Runner<'a> {
    ctx: &'a Context<'a>,
    conn: &'a Connection,
    notes: RefCell<HashMap<String, Option<Rc<Note>>>>,
    /// Display position of every term, by taxonomy and path, so taxonomy fields sort and group
    /// in the order the taxonomy lists its terms.
    term_pos: HashMap<(String, String), i64>,
}

/// Run a query and return one page of it.
pub fn run(q: &Query, ctx: &Context<'_>, page: &PageRequest) -> Result<Page> {
    let types = check(q, ctx)?;
    let conn = ctx.index.connection();
    let r = Runner {
        ctx,
        conn,
        notes: RefCell::new(HashMap::new()),
        term_pos: term_positions(conn)?,
    };

    let scores: Option<HashMap<String, f32>> = match &q.search {
        Some(text) => Some(
            ctx.index
                .search(text, None)?
                .into_iter()
                .map(|h| (h.id, h.score))
                .collect(),
        ),
        None => None,
    };
    let candidates = r.candidates(&types, scores.as_ref())?;

    // Each row's full sort key: group, sort keys, score, then id, which makes the order total.
    struct Hit {
        note: Rc<Note>,
        keys: Vec<Key>,
        group: Option<serde_json::Value>,
    }
    let mut dirs: Vec<bool> = Vec::new();
    if q.group_by.is_some() {
        dirs.push(false);
    }
    dirs.extend(q.sort.iter().map(|k| k.dir == Some(Dir::Desc)));
    let by_score = scores.is_some() && q.sort.is_empty();
    if by_score {
        dirs.push(true);
    }
    dirs.push(false);

    let mut hits = Vec::new();
    for note in candidates {
        let keep = match &q.filter {
            Some(e) => r.truth(e, &note, &mut Vec::new())?,
            None => true,
        };
        if !keep {
            continue;
        }
        let mut keys = Vec::new();
        let mut group = None;
        if let Some(g) = &q.group_by {
            keys.push(r.sort_key(&note, g)?);
            group = Some(r.raw_path(&note, g)?);
        }
        for k in &q.sort {
            keys.push(r.sort_key(&note, &k.path)?);
        }
        if by_score {
            let s = scores.as_ref().and_then(|s| s.get(&note.id)).copied();
            keys.push(Key::Float(f64::from(s.unwrap_or_default())));
        }
        keys.push(Key::Str(note.id.clone()));
        hits.push(Hit { note, keys, group });
    }
    let order = |a: &[Key], b: &[Key]| {
        a.iter()
            .zip(b)
            .zip(&dirs)
            .map(|((x, y), desc)| x.ordered(y, *desc))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    };
    hits.sort_by(|a, b| order(&a.keys, &b.keys));
    if let Some(n) = q.limit {
        hits.truncate(usize::try_from(n).unwrap_or(usize::MAX));
    }

    let mut groups: Vec<GroupCount> = Vec::new();
    for h in &hits {
        if let Some(g) = &h.group {
            match groups.last_mut() {
                Some(last) if last.value == *g => last.count += 1,
                _ => groups.push(GroupCount {
                    value: g.clone(),
                    count: 1,
                }),
            }
        }
    }

    let start = match &page.cursor {
        Some(c) => {
            let after = decode_cursor(c, dirs.len())?;
            hits.partition_point(|h| order(&h.keys, &after) != Ordering::Greater)
        }
        None => 0,
    };
    let size = match page.size {
        0 => MAX_PAGE,
        n => n.min(MAX_PAGE),
    };
    let end = (start + size).min(hits.len());
    let next = (end < hits.len()).then(|| encode_cursor(&hits[end - 1].keys));
    let mut rows = Vec::new();
    for h in &hits[start..end] {
        rows.push(r.row(&h.note, &q.select, h.group.clone())?);
    }
    Ok(Page {
        rows,
        next,
        total: hits.len(),
        groups,
    })
}

fn encode_cursor(keys: &[Key]) -> String {
    let json = serde_json::to_string(keys).unwrap_or_default();
    json.bytes().map(|b| format!("{b:02x}")).collect()
}

fn decode_cursor(c: &str, len: usize) -> Result<Vec<Key>> {
    let bytes: Option<Vec<u8>> = (0..c.len())
        .step_by(2)
        .map(|i| c.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
        .collect();
    let keys: Vec<Key> = bytes
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(Error::BadCursor)?;
    if keys.len() != len {
        return Err(Error::BadCursor);
    }
    Ok(keys)
}

fn term_positions(conn: &Connection) -> Result<HashMap<(String, String), i64>> {
    let mut stmt = conn.prepare("SELECT taxonomy, path, pos FROM terms")?;
    let rows = stmt.query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Check what can be checked before reading a note: the types, every field name, `this`,
/// `linked_from` arguments and `under` terms. Returns the types the query reads.
fn check(q: &Query, ctx: &Context<'_>) -> Result<Vec<String>> {
    let types: Vec<String> = match &q.from {
        From::All => ctx.schema.types().map(|t| t.name.clone()).collect(),
        From::Types(names) => {
            for t in names {
                if ctx.schema.type_def(t).is_none() {
                    return Err(Error::UnknownType(t.clone()));
                }
            }
            names.clone()
        }
    };
    if q.filter.as_ref().is_some_and(Expr::uses_this) && ctx.this.is_none() {
        return Err(Error::NoThis);
    }
    let c = Checker {
        schema: ctx.schema,
        types: &types,
    };
    if let Some(e) = &q.filter {
        c.expr(e, &mut Vec::new())?;
    }
    let paths = q
        .sort
        .iter()
        .map(|k| &k.path)
        .chain(&q.group_by)
        .chain(&q.select);
    for p in paths {
        c.name(&p[0])?;
    }
    Ok(types)
}

struct Checker<'a> {
    schema: &'a Schema,
    types: &'a [String],
}

impl Checker<'_> {
    fn fields(&self, name: &str) -> Vec<&FieldDef> {
        self.types
            .iter()
            .filter_map(|t| self.schema.type_def(t)?.field(name))
            .collect()
    }

    fn name(&self, name: &str) -> Result<()> {
        if BUILTINS.contains(&name) || !self.fields(name).is_empty() {
            return Ok(());
        }
        Err(Error::UnknownField {
            field: name.to_owned(),
            types: self.types.join(", "),
            has: if self.types.len() == 1 { "has" } else { "have" },
        })
    }

    fn expr(&self, e: &Expr, bound: &mut Vec<String>) -> Result<()> {
        match e {
            Expr::Lit(_) | Expr::This => Ok(()),
            Expr::Name(n) if bound.contains(n) => Ok(()),
            Expr::Name(n) => self.name(n),
            Expr::Field(inner, _) | Expr::Unary(_, inner) => self.expr(inner, bound),
            Expr::Binary(_, a, b) => {
                self.expr(a, bound)?;
                self.expr(b, bound)
            }
            Expr::Closure(params, body) => {
                let before = bound.len();
                bound.extend(params.iter().cloned());
                let r = self.expr(body, bound);
                bound.truncate(before);
                r
            }
            Expr::Call(f, args) if f == "linked_from" => {
                let type_name = name_arg(&args[0])?;
                let field = name_arg(&args[1])?;
                let ty = self
                    .schema
                    .type_def(&type_name)
                    .ok_or_else(|| Error::UnknownType(type_name.clone()))?;
                match ty.field(&field) {
                    Some(f) if f.reference().is_some() => Ok(()),
                    _ => Err(Error::NotAReference { type_name, field }),
                }
            }
            Expr::Call(f, args) => {
                for a in args {
                    self.expr(a, bound)?;
                }
                if f == "under"
                    && let [Expr::Name(field), Expr::Lit(Lit::Str(term))] = args.as_slice()
                    && !bound.contains(field)
                {
                    for def in self.fields(field) {
                        if let FieldKind::Taxonomy { taxonomy } = &def.kind
                            && let Some(tax) = self.schema.taxonomy(taxonomy)
                            && !tax.contains(term)
                        {
                            return Err(Error::UnknownTerm {
                                taxonomy: taxonomy.clone(),
                                term: term.clone(),
                            });
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

/// A type or field name given to `linked_from`, written bare or as a string.
fn name_arg(e: &Expr) -> Result<String> {
    match e {
        Expr::Name(n) | Expr::Lit(Lit::Str(n)) => Ok(n.clone()),
        _ => Err(Error::Invalid(format!(
            "linked_from takes a type name and a field name, found `{e}`"
        ))),
    }
}

impl Runner<'_> {
    fn candidates(
        &self,
        types: &[String],
        hits: Option<&HashMap<String, f32>>,
    ) -> Result<Vec<Rc<Note>>> {
        if types.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; types.len()].join(", ");
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes WHERE type IN ({marks}) ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(types), read_note)?;
        let mut out = Vec::new();
        for n in rows {
            let n = n?;
            if hits.is_none_or(|h| h.contains_key(&n.id)) {
                out.push(Rc::new(n));
            }
        }
        Ok(out)
    }

    fn note(&self, id: &str) -> Result<Option<Rc<Note>>> {
        if let Some(n) = self.notes.borrow().get(id) {
            return Ok(n.clone());
        }
        let mut stmt = self
            .conn
            .prepare_cached(&format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"))?;
        let found = stmt
            .query_map([id], read_note)?
            .next()
            .transpose()?
            .map(Rc::new);
        self.notes.borrow_mut().insert(id.to_owned(), found.clone());
        Ok(found)
    }

    fn field_def(&self, note: &Note, name: &str) -> Option<&FieldDef> {
        self.ctx.schema.type_def(&note.type_name)?.field(name)
    }

    /// A field of a note as a value. A field the note's type does not have is missing.
    fn field(&self, note: &Note, name: &str) -> V {
        let opt_time = |s: &Option<String>| {
            s.as_deref()
                .and_then(value::time::datetime)
                .map_or(V::Null, V::Datetime)
        };
        match name {
            "id" => V::Ref(note.id.clone()),
            "type" => V::Str(note.type_name.clone()),
            "title" => V::Str(note.title.clone()),
            "created" => opt_time(&note.created),
            "updated" => opt_time(&note.updated),
            _ => match (self.field_def(note, name), note.data.get(name)) {
                (Some(def), Some(raw)) => value::from_json(raw, &def.kind),
                _ => V::Null,
            },
        }
    }

    /// `base.name`: a member of a record, or a field of the note a single reference names.
    fn member(&self, base: V, name: &str) -> Eval<V> {
        match base {
            V::Null => Ok(V::Null),
            V::Ref(id) => match self.note(&id).map_err(|e| e.to_string())? {
                Some(n) => Ok(self.field(&n, name)),
                None => Ok(V::Null),
            },
            V::Record(members) => Ok(members
                .into_iter()
                .find(|(k, _)| k == name)
                .map_or(V::Null, |(_, v)| v)),
            V::List(_) => Err(format!(
                "`.{name}` follows a single reference, but this is a list: use any() or all()"
            )),
            v => Err(format!("`.{name}` cannot read a member of {}", v.kind())),
        }
    }

    fn fail(note: &Note, message: String) -> Error {
        Error::Eval {
            note: note.id.clone(),
            message,
        }
    }

    fn truth(&self, e: &Expr, note: &Note, env: &mut Vec<(String, V)>) -> Result<bool> {
        let v = self.eval(e, note, env)?;
        v.truth().map_err(|m| Self::fail(note, m))
    }

    fn eval(&self, e: &Expr, note: &Note, env: &mut Vec<(String, V)>) -> Result<V> {
        let fail = |m: String| Self::fail(note, m);
        Ok(match e {
            Expr::Lit(Lit::Bool(b)) => V::Bool(*b),
            Expr::Lit(Lit::Int(n)) => V::Int(*n),
            Expr::Lit(Lit::Float(x)) => V::Float(*x),
            Expr::Lit(Lit::Str(s)) => V::Str(s.clone()),
            Expr::This => V::Ref(self.ctx.this.clone().ok_or(Error::NoThis)?),
            Expr::Name(n) => match env.iter().rev().find(|(k, _)| k == n) {
                Some((_, v)) => v.clone(),
                None => self.field(note, n),
            },
            Expr::Field(base, name) => {
                let base = self.eval(base, note, env)?;
                self.member(base, name).map_err(fail)?
            }
            Expr::Unary(UnOp::Not, inner) => V::Bool(!self.truth(inner, note, env)?),
            Expr::Unary(UnOp::Neg, inner) => match self.eval(inner, note, env)? {
                V::Int(n) => V::Int(
                    n.checked_neg()
                        .ok_or_else(|| fail("integer overflow".into()))?,
                ),
                V::Float(x) => V::Float(-x),
                V::Duration(x) => V::Duration(-x),
                V::Null => V::Null,
                v => return Err(fail(format!("cannot negate {}", v.kind()))),
            },
            Expr::Binary(BinOp::And, a, b) => {
                V::Bool(self.truth(a, note, env)? && self.truth(b, note, env)?)
            }
            Expr::Binary(BinOp::Or, a, b) => {
                V::Bool(self.truth(a, note, env)? || self.truth(b, note, env)?)
            }
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.eval(a, note, env)?, self.eval(b, note, env)?);
                let ord = |want: fn(Ordering) -> bool| -> Result<V> {
                    let o = value::compare(&a, &b).map_err(fail)?;
                    Ok(V::Bool(o.is_some_and(want)))
                };
                match op {
                    BinOp::Eq => V::Bool(value::equals(&a, &b).map_err(fail)?),
                    BinOp::Ne => V::Bool(!value::equals(&a, &b).map_err(fail)?),
                    BinOp::Lt => ord(Ordering::is_lt)?,
                    BinOp::Le => ord(Ordering::is_le)?,
                    BinOp::Gt => ord(Ordering::is_gt)?,
                    BinOp::Ge => ord(Ordering::is_ge)?,
                    BinOp::Add => value::add_sub(a, b, true).map_err(fail)?,
                    BinOp::Sub => value::add_sub(a, b, false).map_err(fail)?,
                    BinOp::Mul => value::mul_div(a, b, '*').map_err(fail)?,
                    BinOp::Div => value::mul_div(a, b, '/').map_err(fail)?,
                    BinOp::Rem => value::mul_div(a, b, '%').map_err(fail)?,
                    BinOp::And | BinOp::Or => unreachable!("handled above"),
                }
            }
            Expr::Closure(..) => {
                return Err(fail(
                    "a closure is only an argument of any() or all()".into(),
                ));
            }
            Expr::Call(f, args) => self.call(f, args, note, env)?,
        })
    }

    fn call(&self, f: &str, args: &[Expr], note: &Note, env: &mut Vec<(String, V)>) -> Result<V> {
        let fail = |m: String| Self::fail(note, m);
        let ctx = self.ctx;
        Ok(match f {
            "now" => V::Datetime(ctx.now),
            "today" => V::Date(((ctx.now as i64) + ctx.utc_offset).div_euclid(86_400)),
            "days" => match self.eval(&args[0], note, env)? {
                V::Int(n) => V::Duration(n as f64 * 86_400.0),
                V::Float(x) => V::Duration(x * 86_400.0),
                v => return Err(fail(format!("days() takes a number, found {}", v.kind()))),
            },
            "has" => {
                let list = self.eval(&args[0], note, env)?;
                let item = self.eval(&args[1], note, env)?;
                let found = match list {
                    V::Null => false,
                    V::List(items) => {
                        let mut found = false;
                        for i in &items {
                            if value::equals(i, &item).map_err(fail)? {
                                found = true;
                                break;
                            }
                        }
                        found
                    }
                    one => value::equals(&one, &item).map_err(fail)?,
                };
                V::Bool(found)
            }
            "under" => {
                let held = self.eval(&args[0], note, env)?;
                let term = match self.eval(&args[1], note, env)? {
                    V::Str(s) => s,
                    v => {
                        return Err(fail(format!(
                            "under() takes a term name, found {}",
                            v.kind()
                        )));
                    }
                };
                let below = |v: &V| match v {
                    V::Str(s) => {
                        s == &term
                            || s.strip_prefix(term.as_str())
                                .is_some_and(|rest| rest.starts_with('/'))
                    }
                    _ => false,
                };
                V::Bool(match &held {
                    V::List(items) => items.iter().any(below),
                    v => below(v),
                })
            }
            "any" | "all" => {
                let items = match self.eval(&args[0], note, env)? {
                    V::Null => Vec::new(),
                    V::List(items) => items,
                    one => vec![one],
                };
                let Expr::Closure(params, body) = &args[1] else {
                    return Err(fail(format!("{f}() takes a closure as its last argument")));
                };
                let want_any = f == "any";
                let mut result = !want_any;
                for item in items {
                    env.push((params[0].clone(), item));
                    let t = self.truth(body, note, env);
                    env.pop();
                    if t? == want_any {
                        result = want_any;
                        break;
                    }
                }
                V::Bool(result)
            }
            "linked_from" => {
                let (type_name, field) = (name_arg(&args[0])?, name_arg(&args[1])?);
                let mut stmt = self.conn.prepare_cached(
                    "SELECT l.src FROM links l JOIN notes n ON n.id = l.src
                     WHERE l.dst = ?1 AND l.via = 'field' AND l.field = ?2 AND n.type = ?3
                     GROUP BY l.src ORDER BY l.src",
                )?;
                let ids = stmt
                    .query_map([&note.id, &field, &type_name], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                V::List(ids.into_iter().map(V::Ref).collect())
            }
            other => return Err(fail(format!("unknown function `{other}`"))),
        })
    }

    /// The value at a path, and the definition of the field it ends at.
    fn resolve(&self, note: &Rc<Note>, path: &Path) -> Result<(V, Option<FieldDef>)> {
        let mut current = Rc::clone(note);
        let mut value = V::Null;
        for (i, name) in path.iter().enumerate() {
            if i > 0 {
                match value {
                    V::Ref(id) => match self.note(&id)? {
                        Some(n) => current = n,
                        None => return Ok((V::Null, None)),
                    },
                    V::Null => return Ok((V::Null, None)),
                    base => {
                        let v = self.member(base, name).map_err(|m| Self::fail(note, m))?;
                        return Ok((v, None));
                    }
                }
            }
            value = self.field(&current, name);
            if i + 1 == path.len() {
                return Ok((value, self.field_def(&current, name).cloned()));
            }
        }
        Ok((value, None))
    }

    fn sort_key(&self, note: &Rc<Note>, path: &Path) -> Result<Key> {
        let (v, def) = self.resolve(note, path)?;
        if let Some(FieldDef {
            kind: FieldKind::Taxonomy { taxonomy },
            ..
        }) = &def
        {
            let pos = |v: &V| match v {
                V::Str(s) => self
                    .term_pos
                    .get(&(taxonomy.clone(), s.clone()))
                    .map_or(Key::Str(s.clone()), |p| Key::Int(*p)),
                v => Key::of(v),
            };
            return Ok(match &v {
                V::List(items) => Key::List(items.iter().map(pos).collect()),
                v => pos(v),
            });
        }
        Ok(Key::of(&v))
    }

    /// The stored JSON at a path, following references.
    fn raw_path(&self, note: &Rc<Note>, path: &Path) -> Result<serde_json::Value> {
        let (head, last) = path.split_at(path.len() - 1);
        let target = if head.is_empty() {
            Some(Rc::clone(note))
        } else {
            match self.resolve(note, &head.to_vec())?.0 {
                V::Ref(id) => self.note(&id)?,
                V::Record(_) => {
                    // A record member: read it from the record's stored JSON.
                    let mut raw = self.raw_path(note, &head.to_vec())?;
                    return Ok(raw
                        .get_mut(&last[0])
                        .map(serde_json::Value::take)
                        .unwrap_or_default());
                }
                _ => None,
            }
        };
        let Some(n) = target else {
            return Ok(serde_json::Value::Null);
        };
        let name = last[0].as_str();
        let text = |s: &str| serde_json::Value::String(s.to_owned());
        Ok(match name {
            "id" => text(&n.id),
            "type" => text(&n.type_name),
            "title" => text(&n.title),
            "created" => n.created.as_deref().map(text).unwrap_or_default(),
            "updated" => n.updated.as_deref().map(text).unwrap_or_default(),
            _ => n.data.get(name).cloned().unwrap_or_default(),
        })
    }

    fn row(
        &self,
        note: &Rc<Note>,
        select: &[Path],
        group: Option<serde_json::Value>,
    ) -> Result<Row> {
        let fields = if select.is_empty() {
            note.data.clone()
        } else {
            let mut m = serde_json::Map::new();
            for p in select {
                m.insert(path_text(p), self.raw_path(note, p)?);
            }
            m
        };
        Ok(Row {
            id: note.id.clone(),
            type_name: note.type_name.clone(),
            title: note.title.clone(),
            fields,
            group,
        })
    }
}
