//! A query run as SQL (D33). The filter and the order keys go to SQLite, which reads them from
//! the indexes on `field_values` and `notes`. Only the notes of the page are read in full.
//!
//! [`run`] returns `None` for a query it cannot translate exactly, and [`crate::run`] then
//! evaluates that query note by note. The translation covers what the builder writes: a field
//! compared with a value that does not depend on the note, `has()`, `under()`, a bool field on
//! its own, `&&`, `||` and `!`, and sorting or grouping by a field of the note itself. It never
//! covers a comparison that would fail on a value of the field's kind, so a query that is an
//! error note by note is still an error. It assumes each stored value has its field's kind,
//! which validation ensures.
//!
//! One field condition that every result meets drives the statement: SQLite reads its rows
//! from `field_values_by_str` or `field_values_by_num`, and checks every other condition on
//! each of them by the primary key. A query with no such condition reads `notes_by_type`.
//! One statement returns the order keys of every match, and the total, the groups and the
//! page are cut from those keys. A query sorted newest first by `updated` or `created`, or not
//! sorted at all, instead walks `notes_by_updated`, `notes_by_created` or the ids in order and
//! stops after the page, when enough notes match that the page comes early in that order.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use rusqlite::types::{Value as Sql, ValueRef};
use rusqlite::{Connection, params_from_iter};
use schema::{FieldKind, Scalar};

use crate::ast::{BinOp, Dir, Expr, Lit, Path, Query, UnOp};
use crate::run::{
    BUILTINS, Error, GroupCount, MAX_PAGE, Page, PageRequest, Result, Row, Runner, decode_cursor,
    encode_cursor,
};
use crate::value::{self, Key, V};

/// How the values of a field compare, which decides what they can be compared with.
#[derive(Debug, Clone, PartialEq)]
enum Class {
    /// Strings: `string`, `url`, `select` and `asset`.
    Text,
    /// Note ids, which compare for equality only.
    Ref,
    /// `int` and `float`, in `num`.
    Num,
    /// `bool`, as 0 or 1 in `num`.
    Bool,
    /// `date` and `datetime`, as Unix time in `num`.
    Time,
    /// A taxonomy term path, sorted in the taxonomy's term order.
    Tax(String),
}

/// Something a condition or a sort key reads.
enum Col {
    /// A column of `notes`.
    Builtin(&'static str, Class),
    /// A field, with whether it holds a list.
    Field(String, Class, bool),
}

fn class_of(kind: &FieldKind) -> Option<Class> {
    Some(match kind {
        FieldKind::String | FieldKind::Url | FieldKind::Select { .. } | FieldKind::Asset => {
            Class::Text
        }
        FieldKind::Int | FieldKind::Float => Class::Num,
        FieldKind::Bool => Class::Bool,
        FieldKind::Date | FieldKind::Datetime => Class::Time,
        FieldKind::Taxonomy { taxonomy } => Class::Tax(taxonomy.clone()),
        FieldKind::Reference(_) => Class::Ref,
        FieldKind::List { items } => match items {
            Scalar::String | Scalar::Url => Class::Text,
            Scalar::Int | Scalar::Float => Class::Num,
            Scalar::Bool => Class::Bool,
            Scalar::Date | Scalar::Datetime => Class::Time,
        },
        _ => return None,
    })
}

/// A comparison of a field with a value: `=` for `==` and `has()`, or an ordering.
#[derive(Clone, Copy)]
enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    fn of(op: BinOp) -> Option<Cmp> {
        Some(match op {
            BinOp::Eq => Cmp::Eq,
            BinOp::Lt => Cmp::Lt,
            BinOp::Le => Cmp::Le,
            BinOp::Gt => Cmp::Gt,
            BinOp::Ge => Cmp::Ge,
            _ => return None,
        })
    }

    fn sql(self) -> &'static str {
        match self {
            Cmp::Eq => "=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }

    /// The comparison with its sides swapped.
    fn flip(self) -> Cmp {
        match self {
            Cmp::Eq => Cmp::Eq,
            Cmp::Lt => Cmp::Gt,
            Cmp::Le => Cmp::Ge,
            Cmp::Gt => Cmp::Lt,
            Cmp::Ge => Cmp::Le,
        }
    }
}

/// Where a statement reads its notes from.
#[derive(Clone, Copy, PartialEq)]
enum Read {
    /// The rows of the driving field condition, when the filter has one.
    Field,
    /// `notes` in the order of the first sort key.
    InOrder,
}

/// One term of the order: its SQL expression and whether it is descending.
struct Order {
    expr: String,
    desc: bool,
}

/// A query as the parts of an SQL statement. Every statement [`run`] makes starts from a fresh
/// one, so its parameters are numbered from 1.
struct Source {
    params: Vec<Sql>,
    /// `FROM ... WHERE ...`, which a statement may extend with `AND`.
    from: String,
    /// The note id: `d.note` when a field condition drives, else `n.id`.
    id: &'static str,
    /// The group key, the sort keys, then the id, which makes the order total.
    order: Vec<Order>,
    /// Whether a note may meet the driving condition in more than one row, as a list can.
    distinct: bool,
    /// Whether a field condition drives.
    driven: bool,
}

impl Source {
    fn bind(&mut self, v: Sql) -> String {
        self.params.push(v);
        format!("?{}", self.params.len())
    }

    fn order_by(&self) -> String {
        self.order
            .iter()
            .map(|o| match o {
                o if o.expr == self.id => o.expr.clone(),
                // SQLite sorts a missing value first ascending and last descending. It goes
                // last either way.
                Order { expr, desc: true } => format!("{expr} DESC"),
                Order { expr, desc: false } => format!("{expr} IS NULL, {expr}"),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Every order key, named `c0`, `c1` and so on.
    fn keys(&self) -> String {
        let distinct = if self.distinct { "DISTINCT " } else { "" };
        let keys: Vec<String> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, o)| format!("{} AS c{i}", o.expr))
            .collect();
        format!("{distinct}{}", keys.join(", "))
    }

    /// The rows after `keys` in the order: those after it on the first key that differs, where
    /// a missing value is last.
    fn after(&mut self, keys: &[Key]) -> Result<String> {
        let mut any = Vec::new();
        let mut equal: Vec<String> = Vec::new();
        let order: Vec<(String, bool)> = self
            .order
            .iter()
            .map(|o| (o.expr.clone(), o.desc))
            .collect();
        for ((e, desc), key) in order.into_iter().zip(keys) {
            let v = match key {
                Key::Null => {
                    equal.push(format!("{e} IS NULL"));
                    continue;
                }
                Key::Int(n) => Sql::Integer(*n),
                Key::Float(x) => Sql::Real(*x),
                Key::Str(s) => Sql::Text(s.clone()),
                Key::Bool(_) | Key::List(_) => return Err(Error::BadCursor),
            };
            let p = self.bind(v);
            let cmp = if desc { "<" } else { ">" };
            let mut terms = equal.clone();
            terms.push(format!("({e} {cmp} {p} OR {e} IS NULL)"));
            any.push(format!("({})", terms.join(" AND ")));
            equal.push(format!("{e} = {p}"));
        }
        Ok(if any.is_empty() {
            "0".to_owned()
        } else {
            any.join(" OR ")
        })
    }
}

/// Run `q` as SQL, or return `None` when it cannot be translated exactly.
pub(crate) fn run(
    q: &Query,
    types: &[String],
    r: &Runner<'_>,
    page: &PageRequest,
) -> Result<Option<Page>> {
    if q.search.is_some() {
        return Ok(None);
    }
    let Some(main) = source(q, types, r, Read::Field) else {
        return Ok(None);
    };
    let cursor = match &page.cursor {
        Some(c) => Some(decode_cursor(c, main.order.len())?),
        None => None,
    };
    let want = Want {
        limit: q.limit.map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
        size: match page.size {
            0 => MAX_PAGE,
            n => n.min(MAX_PAGE),
        },
        cursor: cursor.as_deref(),
    };
    let first = main.order.first().map(|o| (o.expr.as_str(), o.desc));
    let in_order = match first {
        Some(("n.updated_at" | "n.created_at", true)) => q.group_by.is_none(),
        _ => main.order.len() == 1,
    };
    if in_order && let Some(p) = walk(q, types, r, &main, &want)? {
        return Ok(Some(p));
    }
    stream(q, r, &main, &want).map(Some)
}

/// What page of the result to return.
struct Want<'c> {
    limit: Option<usize>,
    size: usize,
    cursor: Option<&'c [Key]>,
}

/// Read every match's order keys in one statement, then count, group and cut the page from
/// them. Only the notes of the page are read.
fn stream(q: &Query, r: &Runner<'_>, s: &Source, want: &Want<'_>) -> Result<Page> {
    let sql = format!("SELECT {} {}", s.keys(), s.from);
    let mut all = rows_of(r.conn, &sql, &s.params, s.order.len())?;
    let dirs: Vec<bool> = s.order.iter().map(|o| o.desc).collect();
    let order = |a: &Vec<Key>, b: &Vec<Key>| {
        a.iter()
            .zip(b)
            .zip(&dirs)
            .map(|((x, y), desc)| x.ordered(y, *desc))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    };
    if let Some(l) = want.limit
        && l < all.len()
    {
        all.select_nth_unstable_by(l, order);
        all.truncate(l);
    }
    let total = all.len();

    let mut groups = Vec::new();
    if let Some(g) = &q.group_by {
        // Each group's row count and its first note, whose stored value names the group.
        let mut counts: BTreeMap<GroupKey, (usize, &str)> = BTreeMap::new();
        for keys in &all {
            let id = id_of(keys);
            let e = counts.entry(GroupKey(keys[0].clone())).or_insert((0, id));
            e.0 += 1;
            e.1 = e.1.min(id);
        }
        for (count, id) in counts.into_values() {
            let value = match r.note(id)? {
                Some(note) => r.raw_path(&note, g)?,
                None => serde_json::Value::Null,
            };
            groups.push(GroupCount { value, count });
        }
    }

    if let Some(c) = want.cursor {
        let c = c.to_vec();
        all.retain(|keys| order(keys, &c) == Ordering::Greater);
    }
    let more = all.len() > want.size;
    if more {
        all.select_nth_unstable_by(want.size, order);
        all.truncate(want.size);
    }
    all.sort_by(order);
    let next = if more {
        all.last().map(|keys| encode_cursor(keys))
    } else {
        None
    };
    Ok(Page {
        rows: page_rows(q, r, &all)?,
        next,
        total,
        groups,
    })
}

/// Walk `notes` in the order of the first sort key, or of the id, and stop after the page. That
/// reads about `size / share` notes, where `share` is the part of them that match, so it is
/// only done when that is fewer than reading the matches. `None` when it is not.
fn walk(
    q: &Query,
    types: &[String],
    r: &Runner<'_>,
    main: &Source,
    want: &Want<'_>,
) -> Result<Option<Page>> {
    let conn = r.conn;
    let counted = if main.distinct {
        format!("count(DISTINCT {})", main.id)
    } else {
        "count(*)".to_owned()
    };
    let matched = count(
        conn,
        &format!("SELECT {counted} {}", main.from),
        &main.params,
    )?;
    let of_types = count(
        conn,
        &format!(
            "SELECT count(*) FROM notes WHERE type IN ({})",
            vec!["?"; types.len()].join(", ")
        ),
        &types
            .iter()
            .map(|t| Sql::Text(t.clone()))
            .collect::<Vec<_>>(),
    )?;
    let read = if main.driven { matched } else { of_types };
    if matched > 0 && (want.size + 1).saturating_mul(of_types) / matched >= read {
        return Ok(None);
    }
    let Some(mut s) = source(q, types, r, Read::InOrder) else {
        return Ok(None);
    };
    let total = want.limit.map_or(matched, |l| matched.min(l));

    let mut after = String::new();
    let mut start = 0;
    if let Some(keys) = want.cursor {
        if want.limit.is_some() {
            let mut before = source(q, types, r, Read::InOrder).ok_or(Error::BadCursor)?;
            let cond = before.after(keys)?;
            let sql = format!(
                "SELECT count(*) FROM (SELECT {} {} AND NOT ({cond}))",
                before.keys(),
                before.from
            );
            start = count(conn, &sql, &before.params)?;
        }
        after = format!(" AND ({})", s.after(keys)?);
    }
    let remaining = want.limit.map(|l| l.saturating_sub(start));
    let len = remaining.map_or(want.size, |left| want.size.min(left));
    if len == 0 || matched == 0 {
        return Ok(Some(Page {
            rows: Vec::new(),
            next: None,
            total,
            groups: Vec::new(),
        }));
    }
    let sql = format!(
        "SELECT {} {}{after} ORDER BY {} LIMIT {}",
        s.keys(),
        s.from,
        s.order_by(),
        len + 1
    );
    let mut found = rows_of(conn, &sql, &s.params, s.order.len())?;
    let more = found.len() > len && remaining.is_none_or(|left| len < left);
    found.truncate(len);
    let next = if more {
        found.last().map(|keys| encode_cursor(keys))
    } else {
        None
    };
    Ok(Some(Page {
        rows: page_rows(q, r, &found)?,
        next,
        total,
        groups: Vec::new(),
    }))
}

/// The order keys of every row a statement returns.
fn rows_of(conn: &Connection, sql: &str, params: &[Sql], width: usize) -> Result<Vec<Vec<Key>>> {
    let rows = conn
        .prepare(sql)?
        .query_map(params_from_iter(params), |row| {
            (0..width)
                .map(|i| key_of(row.get_ref(i)?))
                .collect::<rusqlite::Result<Vec<_>>>()
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The id of the note a row of keys belongs to: the last key.
fn id_of(keys: &[Key]) -> &str {
    match keys.last() {
        Some(Key::Str(id)) => id,
        _ => "",
    }
}

/// The rows of a page, read from the notes its keys name.
fn page_rows(q: &Query, r: &Runner<'_>, keys: &[Vec<Key>]) -> Result<Vec<Row>> {
    let mut rows = Vec::with_capacity(keys.len());
    for k in keys {
        let Some(note) = r.note(id_of(k))? else {
            continue;
        };
        let group = match &q.group_by {
            Some(g) => Some(r.raw_path(&note, g)?),
            None => None,
        };
        rows.push(r.row(&note, &q.select, group)?);
    }
    Ok(rows)
}

/// A group key in the order groups are listed: ascending, missing last.
struct GroupKey(Key);

impl PartialEq for GroupKey {
    fn eq(&self, other: &GroupKey) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for GroupKey {}

impl PartialOrd for GroupKey {
    fn partial_cmp(&self, other: &GroupKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GroupKey {
    fn cmp(&self, other: &GroupKey) -> Ordering {
        self.0.ordered(&other.0, false)
    }
}

fn count(conn: &Connection, sql: &str, params: &[Sql]) -> Result<usize> {
    let n: i64 = conn
        .prepare_cached(sql)?
        .query_row(params_from_iter(params), |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or_default())
}

fn key_of(v: ValueRef<'_>) -> rusqlite::Result<Key> {
    Ok(match v {
        ValueRef::Null | ValueRef::Blob(_) => Key::Null,
        ValueRef::Integer(n) => Key::Int(n),
        ValueRef::Real(x) => Key::Float(x),
        ValueRef::Text(t) => Key::Str(String::from_utf8_lossy(t).into_owned()),
    })
}

/// `q` as the parts of a statement that reads as `read` says, or `None` when it cannot be
/// translated exactly.
fn source(q: &Query, types: &[String], r: &Runner<'_>, read: Read) -> Option<Source> {
    let driver = match (read, &q.filter) {
        (Read::Field, Some(e)) => driver(e),
        _ => None,
    };
    let mut p = Planner {
        r,
        types,
        params: Vec::new(),
        joins: String::new(),
        id: if driver.is_some() { "d.note" } else { "n.id" },
        notes: false,
        driver: driver.map(|e| e as *const Expr),
        driven: None,
    };
    let filter = match &q.filter {
        Some(e) => p.pred(e, false)?,
        None => "1".to_owned(),
    };
    let mut order = Vec::new();
    if let Some(g) = &q.group_by {
        order.push(Order {
            expr: p.key(g)?,
            desc: false,
        });
    }
    for k in &q.sort {
        order.push(Order {
            expr: p.key(&k.path)?,
            desc: k.dir == Some(Dir::Desc),
        });
    }
    order.push(Order {
        expr: p.id.to_owned(),
        desc: false,
    });
    let marks: Vec<String> = types.iter().map(|t| p.bind(Sql::Text(t.clone()))).collect();
    let marks = marks.join(", ");
    let joins = std::mem::take(&mut p.joins);
    let (from, distinct, driven) = match p.driven.take() {
        Some((field, cond, list)) => {
            let notes = if p.notes {
                " JOIN notes n ON n.id = d.note"
            } else {
                ""
            };
            let from = format!(
                "FROM field_values d{notes}{joins} \
                 WHERE d.field = {field} AND d.type IN ({marks}) AND {cond} AND {filter}"
            );
            (from, list, true)
        }
        None => {
            // `+` keeps SQLite from reading the notes through notes_by_type, so that it walks
            // the index the order needs.
            let plus = if read == Read::InOrder { "+" } else { "" };
            let from = format!("FROM notes n{joins} WHERE {plus}n.type IN ({marks}) AND {filter}");
            (from, false, false)
        }
    };
    Some(Source {
        params: p.params,
        from,
        id: p.id,
        order,
        distinct,
        driven,
    })
}

/// The condition that drives the statement: one that every result meets, found among the
/// top-level `&&` terms of the filter, and read from a field index. An equality reads fewest
/// rows, then `under()`, then a range, then a bool field.
fn driver(filter: &Expr) -> Option<&Expr> {
    fn terms<'e>(e: &'e Expr, out: &mut Vec<(u8, &'e Expr)>) {
        let field = |e: &Expr| matches!(e, Expr::Name(n) if !BUILTINS.contains(&n.as_str()));
        let rank = match e {
            Expr::Binary(BinOp::And, a, b) => {
                terms(a, out);
                terms(b, out);
                return;
            }
            Expr::Binary(BinOp::Eq, a, b) if field(a) || field(b) => 0,
            Expr::Call(f, args) if f == "has" && args.first().is_some_and(field) => 0,
            Expr::Call(f, args) if f == "under" && args.first().is_some_and(field) => 1,
            Expr::Binary(BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, a, b)
                if field(a) || field(b) =>
            {
                2
            }
            Expr::Name(_) if field(e) => 3,
            _ => return,
        };
        out.push((rank, e));
    }
    let mut found = Vec::new();
    terms(filter, &mut found);
    found
        .into_iter()
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, e)| e)
}

/// Whether an expression has the same value for every note, so it can be worked out once.
fn constant(e: &Expr) -> bool {
    match e {
        Expr::Lit(_) | Expr::This => true,
        Expr::Unary(_, x) => constant(x),
        Expr::Binary(_, a, b) => constant(a) && constant(b),
        Expr::Call(f, args) => {
            matches!(f.as_str(), "now" | "today" | "days") && args.iter().all(constant)
        }
        _ => false,
    }
}

struct Planner<'r, 'a> {
    r: &'r Runner<'a>,
    types: &'r [String],
    params: Vec<Sql>,
    joins: String,
    /// The note id: `d.note` when a field condition drives, else `n.id`.
    id: &'static str,
    /// Whether a condition or key reads a column of `notes`.
    notes: bool,
    /// The condition chosen to drive, compared by address.
    driver: Option<*const Expr>,
    /// The driving condition once translated: the field's parameter, the condition on `d`, and
    /// whether the field holds a list.
    driven: Option<(String, String, bool)>,
}

impl Planner<'_, '_> {
    fn bind(&mut self, v: Sql) -> String {
        self.params.push(v);
        format!("?{}", self.params.len())
    }

    /// What a name reads, when every queried type that has it agrees on how it compares.
    fn col(&mut self, name: &str) -> Option<Col> {
        let builtin = match name {
            "id" => Some(("n.id", Class::Ref)),
            "type" => Some(("n.type", Class::Text)),
            "title" => Some(("n.title", Class::Text)),
            "created" => Some(("n.created_at", Class::Time)),
            "updated" => Some(("n.updated_at", Class::Time)),
            _ => None,
        };
        if let Some((col, class)) = builtin {
            if col == "n.id" {
                return Some(Col::Builtin(self.id, class));
            }
            self.notes = true;
            return Some(Col::Builtin(col, class));
        }
        let schema = self.r.ctx.schema;
        let mut found: Option<(Class, bool)> = None;
        for t in self.types {
            let Some(def) = schema.type_def(t).and_then(|t| t.field(name)) else {
                continue;
            };
            let this = (
                class_of(&def.kind)?,
                def.multiple || matches!(def.kind, FieldKind::List { .. }),
            );
            match &found {
                Some(seen) if *seen != this => return None,
                _ => found = Some(this),
            }
        }
        let (class, list) = found?;
        Some(Col::Field(name.to_owned(), class, list))
    }

    /// The value of an expression that does not depend on the note.
    fn value(&self, e: &Expr) -> Option<V> {
        if !constant(e) {
            return None;
        }
        self.r
            .eval(e, &crate::run::Note::empty(), &mut Vec::new())
            .ok()
    }

    /// The column and parameter that compare a value of `class` with `v`, or `None` when that
    /// comparison could fail or is not one SQL can make the same way.
    fn operand(&mut self, class: &Class, cmp: Cmp, v: &V) -> Option<(&'static str, String)> {
        let eq = matches!(cmp, Cmp::Eq);
        let (col, sql) = match (class, v) {
            (Class::Text | Class::Tax(_), V::Str(s)) => ("str", Sql::Text(s.clone())),
            (Class::Text | Class::Tax(_) | Class::Ref, V::Str(s) | V::Ref(s)) if eq => {
                ("str", Sql::Text(s.clone()))
            }
            (Class::Num, V::Int(n)) => ("num", Sql::Integer(*n)),
            (Class::Num, V::Float(x)) if x.is_finite() => ("num", Sql::Real(*x)),
            (Class::Bool, V::Bool(b)) if eq => ("num", Sql::Integer(i64::from(*b))),
            (Class::Time, V::Date(d)) => ("num", Sql::Real(*d as f64 * 86_400.0)),
            (Class::Time, V::Datetime(t)) => ("num", Sql::Real(*t)),
            (Class::Time, V::Str(s)) => ("num", Sql::Real(value::str_time(s)?)),
            _ => return None,
        };
        Some((col, self.bind(sql)))
    }

    /// Whether `e` is the driving condition.
    fn drives(&self, e: &Expr) -> bool {
        self.driver == Some(e as *const Expr)
    }

    /// A condition on the rows of one field: true for a note when any of them meets `cond`,
    /// which `cond` writes over the columns `str` and `num` with the prefix it is given. `neg`
    /// negates it. The driving condition becomes the statement's `FROM`, and `1` here.
    fn field_rows(
        &mut self,
        e: &Expr,
        field: &str,
        list: bool,
        cond: impl Fn(&str) -> String,
        neg: bool,
    ) -> String {
        let f = self.bind(Sql::Text(field.to_owned()));
        if !neg && self.drives(e) {
            self.driven = Some((f, cond("d."), list));
            return "1".to_owned();
        }
        let not = if neg { "NOT " } else { "" };
        format!(
            "{not}EXISTS (SELECT 1 FROM field_values v WHERE v.note = {} AND v.field = {f} AND {})",
            self.id,
            cond("v.")
        )
    }

    /// `name cmp v`, or `has(name, v)` when `has`.
    fn compare(
        &mut self,
        e: &Expr,
        name: &str,
        cmp: Cmp,
        v: &Expr,
        has: bool,
        neg: bool,
    ) -> Option<String> {
        let v = self.value(v)?;
        match self.col(name)? {
            Col::Builtin(col, class) => {
                let (_, p) = self.operand(&class, cmp, &v)?;
                let not = if neg { "NOT " } else { "" };
                Some(format!("{not}coalesce({col} {} {p}, 0)", cmp.sql()))
            }
            // A list compared as a whole fails on every note that has one.
            Col::Field(_, _, true) if !has => None,
            Col::Field(field, class, list) => {
                let (col, p) = self.operand(&class, cmp, &v)?;
                let op = cmp.sql();
                Some(self.field_rows(e, &field, list, |t| format!("{t}{col} {op} {p}"), neg))
            }
        }
    }

    /// A filter expression as an SQL condition that is never NULL, negated when `neg`.
    fn pred(&mut self, e: &Expr, neg: bool) -> Option<String> {
        match e {
            Expr::Unary(UnOp::Not, inner) => self.pred(inner, !neg),
            Expr::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                let (a, b) = (self.pred(a, neg)?, self.pred(b, neg)?);
                // !(a && b) is !a || !b, and !(a || b) is !a && !b.
                let and = matches!(op, BinOp::And) != neg;
                Some(format!("({a} {} {b})", if and { "AND" } else { "OR" }))
            }
            Expr::Binary(BinOp::Ne, a, b) => {
                self.pred(&Expr::Binary(BinOp::Eq, a.clone(), b.clone()), !neg)
            }
            Expr::Binary(op, a, b) => {
                let c = Cmp::of(*op)?;
                match (a.as_ref(), b.as_ref()) {
                    (Expr::Name(n), v) if constant(v) => self.compare(e, n, c, v, false, neg),
                    (v, Expr::Name(n)) if constant(v) => {
                        self.compare(e, n, c.flip(), v, false, neg)
                    }
                    _ => None,
                }
            }
            Expr::Name(n) => match self.col(n)? {
                Col::Field(field, Class::Bool, false) => {
                    Some(self.field_rows(e, &field, false, |t| format!("{t}num = 1"), neg))
                }
                _ => None,
            },
            Expr::Call(f, args) if f == "has" => match args.as_slice() {
                [Expr::Name(n), v] => self.compare(e, n, Cmp::Eq, v, true, neg),
                _ => None,
            },
            Expr::Call(f, args) if f == "under" => {
                let [Expr::Name(n), term] = args.as_slice() else {
                    return None;
                };
                let Some(V::Str(term)) = self.value(term) else {
                    return None;
                };
                let Col::Field(field, Class::Text | Class::Tax(_), list) = self.col(n)? else {
                    return None;
                };
                // The term and every path below it: every string from `term` up to `term0`,
                // since `0` follows `/`, that is the term itself or at least `term/`. The range
                // reads the index.
                let at = self.bind(Sql::Text(term.clone()));
                let below = self.bind(Sql::Text(format!("{term}/")));
                let end = self.bind(Sql::Text(format!("{term}0")));
                let cond = |t: &str| {
                    format!(
                        "{t}str >= {at} AND {t}str < {end} AND ({t}str = {at} OR {t}str >= {below})"
                    )
                };
                Some(self.field_rows(e, &field, list, cond, neg))
            }
            Expr::Lit(Lit::Bool(b)) => Some(if *b != neg { "1" } else { "0" }.to_owned()),
            _ => None,
        }
    }

    /// The sort key of a path as an SQL expression. Joins what it reads into `joins`.
    fn key(&mut self, path: &Path) -> Option<String> {
        let [name] = path.as_slice() else {
            return None;
        };
        match self.col(name)? {
            Col::Builtin(col, _) => Some(col.to_owned()),
            Col::Field(_, _, true) => None,
            Col::Field(field, class, false) => {
                let at = format!("k{}", self.params.len());
                let f = self.bind(Sql::Text(field));
                self.joins.push_str(&format!(
                    " LEFT JOIN field_values {at} ON {at}.note = {} AND {at}.field = {f} \
                     AND {at}.pos = 0 AND {at}.part = ''",
                    self.id
                ));
                Some(match class {
                    Class::Tax(taxonomy) => {
                        let t = self.bind(Sql::Text(taxonomy));
                        self.joins.push_str(&format!(
                            " LEFT JOIN terms {at}t ON {at}t.taxonomy = {t} AND {at}t.path = {at}.str"
                        ));
                        format!("coalesce({at}t.pos, {at}.str)")
                    }
                    _ => format!("coalesce({at}.num, {at}.str)"),
                })
            }
        }
    }
}
