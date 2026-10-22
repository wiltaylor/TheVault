//! The field names a template's `query_view` blocks hold: in the query text and in each view's
//! `columns`, `group`, `date` and `location`. A field rename in a type change renames them (D6,
//! D8), and saving a template checks them against the types the query reads (D14).
//!
//! Only what names a field of the queried type directly is followed: a bare name in the `where`
//! clause, the first name of a `sort`, `group by` or `select` path and of a view's column, and
//! `linked_from(<type>, <field>)`. A member read through a reference, as in `project.status`,
//! names a field of another type and is left alone. `this.<field>` names a field of the note that
//! holds the block, so it follows a rename of the template's own type.

use std::collections::BTreeMap;

use query_syntax::{Expr, From, Lit, Query};

use crate::doc::{DocBlock, Value};
use crate::types::{FieldDef, TabItem, Template};
use crate::{Issue, Schema};

/// Fields every note has, which a query may name besides a type's own.
const BUILTINS: [&str; 6] = ["id", "type", "title", "created", "updated", "archived"];

/// The items of a `view` that name one field, or a list of them (`columns`).
pub const VIEW_FIELD_ITEMS: [&str; 4] = ["columns", "group", "date", "location"];

/// A rename of fields of one type, as a query view sees it.
pub struct FieldRename<'a> {
    pub type_name: &'a str,
    /// Old field name to new field name.
    pub renames: &'a BTreeMap<String, String>,
    /// The block sits in a template of `type_name`, so `this` is a note of that type.
    pub this_is_type: bool,
}

/// Whether `query` reads notes of `type_name`: its `from` lists it. `from *` is left out, since
/// it reads every type and a name in it may be any of theirs.
pub fn reads_type(query: &Query, type_name: &str) -> bool {
    matches!(&query.from, From::Types(t) if t.iter().any(|t| t == type_name))
}

impl FieldRename<'_> {
    fn to(&self, name: &str) -> Option<&str> {
        self.renames.get(name).map(String::as_str)
    }

    /// `text` with the renamed fields renamed, in canonical form, or `None` when it names none
    /// of them or does not parse.
    pub fn query(&self, text: &str) -> Option<String> {
        let mut q = query_syntax::parse(text).ok()?;
        let reads = reads_type(&q, self.type_name);
        let mut changed = false;
        if let Some(e) = &mut q.filter {
            changed |= self.expr(e, reads, &mut Vec::new());
        }
        if reads {
            let paths = q
                .sort
                .iter_mut()
                .map(|k| &mut k.path)
                .chain(q.group_by.iter_mut())
                .chain(q.select.iter_mut());
            for p in paths {
                if let Some(first) = p.first_mut()
                    && let Some(to) = self.to(first)
                {
                    *first = to.to_owned();
                    changed = true;
                }
            }
        }
        changed.then(|| q.to_string())
    }

    /// A view item's field name (or dotted path) renamed, when the view's query reads the type.
    pub fn view_field(&self, query: &str, name: &str) -> Option<String> {
        let q = query_syntax::parse(query).ok()?;
        if !reads_type(&q, self.type_name) {
            return None;
        }
        let (first, rest) = name
            .split_once('.')
            .map_or((name, None), |(f, r)| (f, Some(r)));
        let to = self.to(first)?;
        Some(match rest {
            Some(r) => format!("{to}.{r}"),
            None => to.to_owned(),
        })
    }

    fn expr(&self, e: &mut Expr, reads: bool, bound: &mut Vec<String>) -> bool {
        match e {
            Expr::Lit(_) | Expr::This => false,
            Expr::Name(n) => {
                if reads
                    && !bound.contains(n)
                    && let Some(to) = self.to(n)
                {
                    *n = to.to_owned();
                    true
                } else {
                    false
                }
            }
            Expr::Field(base, member) => {
                if matches!(base.as_ref(), Expr::This) {
                    if self.this_is_type
                        && let Some(to) = self.to(member)
                    {
                        *member = to.to_owned();
                        return true;
                    }
                    return false;
                }
                self.expr(base, reads, bound)
            }
            Expr::Call(f, args) if f == "linked_from" => {
                if let [t, field] = args.as_mut_slice()
                    && bare(t) == Some(self.type_name)
                    && let Some(to) = bare(field).and_then(|n| self.to(n)).map(str::to_owned)
                {
                    match field {
                        Expr::Lit(Lit::Str(s)) => *s = to,
                        _ => *field = Expr::Name(to),
                    }
                    return true;
                }
                false
            }
            Expr::Call(_, args) => {
                let mut changed = false;
                for a in args {
                    changed |= self.expr(a, reads, bound);
                }
                changed
            }
            Expr::Unary(_, inner) => self.expr(inner, reads, bound),
            Expr::Binary(_, a, b) => {
                let a = self.expr(a, reads, bound);
                self.expr(b, reads, bound) || a
            }
            Expr::Closure(params, body) => {
                let before = bound.len();
                bound.extend(params.iter().cloned());
                let changed = self.expr(body, reads, bound);
                bound.truncate(before);
                changed
            }
        }
    }
}

/// A name given to `linked_from`, bare or as a string.
fn bare(e: &Expr) -> Option<&str> {
    match e {
        Expr::Name(n) => Some(n),
        Expr::Lit(Lit::Str(s)) => Some(s),
        _ => None,
    }
}

/// What is wrong with the field names of every `query_view` in `template` of type `host`: a
/// view's `columns`, `group`, `date` or `location`, or a name in the query's `where`, `sort`,
/// `group by` or `select`, that is not a field of any type the query reads. A query over `*`, an
/// unknown type or text that does not parse is not checked here; the query view reports those
/// when it runs.
pub fn query_view_issues(schema: &Schema, template: &Template, at: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    for tab in &template.tabs {
        let tab_at = format!("{at} > tab {:?}", tab.title);
        for (n, item) in tab.items.iter().enumerate() {
            if let TabItem::Block(b) = item {
                walk(
                    schema,
                    b,
                    &format!("{tab_at} > {}#{}", b.kind, n + 1),
                    &mut issues,
                );
            }
        }
    }
    issues
}

fn walk(schema: &Schema, b: &DocBlock, at: &str, issues: &mut Vec<Issue>) {
    if b.kind == "query_view" {
        check_query_view(schema, b, at, issues);
        return;
    }
    for (n, child) in b.body.blocks.iter().enumerate() {
        walk(
            schema,
            child,
            &format!("{at} > {}#{}", child.kind, n + 1),
            issues,
        );
    }
}

fn check_query_view(schema: &Schema, b: &DocBlock, at: &str, issues: &mut Vec<Issue>) {
    let Some(Value::Str(text)) = b.body.field("query") else {
        return;
    };
    let Ok(q) = query_syntax::parse(text) else {
        return;
    };
    let From::Types(names) = &q.from else {
        return;
    };
    let mut types = Vec::new();
    for n in names {
        match schema.type_def(n) {
            Some(t) => types.push(t),
            None => return,
        }
    }
    let known = |name: &str| {
        BUILTINS.contains(&name)
            || types
                .iter()
                .any(|t| t.fields.iter().any(|f: &FieldDef| f.name == name))
    };
    let of = names.join(", ");
    let missing = |what: String, name: &str, issues: &mut Vec<Issue>| {
        issues.push(Issue::new(
            at,
            format!("{what} names `{name}`, which is not a field of {of}"),
        ));
    };
    if let Some(e) = &q.filter {
        let mut names = Vec::new();
        expr_names(e, &mut Vec::new(), &mut names);
        for n in names {
            if !known(&n) {
                missing("the query's where clause".to_owned(), &n, issues);
            }
        }
    }
    for k in &q.sort {
        if let Some(first) = k.path.first()
            && !known(first)
        {
            missing("the query's sort".to_owned(), first, issues);
        }
    }
    if let Some(first) = q.group_by.as_ref().and_then(|p| p.first())
        && !known(first)
    {
        missing("the query's group by".to_owned(), first, issues);
    }
    for p in &q.select {
        if let Some(first) = p.first()
            && !known(first)
        {
            missing("the query's select".to_owned(), first, issues);
        }
    }
    for v in b.body.blocks_of("view") {
        let view = v.label().unwrap_or("?");
        for item in VIEW_FIELD_ITEMS {
            let values: Vec<&str> = match v.body.field(item) {
                Some(Value::Str(s)) => vec![s.as_str()],
                Some(Value::List(l)) => l.iter().filter_map(Value::as_str).collect(),
                _ => continue,
            };
            for name in values {
                let first = name.split('.').next().unwrap_or(name);
                if !known(first) {
                    missing(format!("view {view:?} {item}"), name, issues);
                }
            }
        }
    }
}

/// The names a `where` expression reads as fields of the queried notes, in order, once each.
fn expr_names(e: &Expr, bound: &mut Vec<String>, out: &mut Vec<String>) {
    match e {
        Expr::Lit(_) | Expr::This => {}
        Expr::Name(n) => {
            if !bound.contains(n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
        Expr::Field(base, _) => expr_names(base, bound, out),
        // Its arguments name a type and that type's field, not fields of the queried notes.
        Expr::Call(f, _) if f == "linked_from" => {}
        Expr::Call(_, args) => args.iter().for_each(|a| expr_names(a, bound, out)),
        Expr::Unary(_, inner) => expr_names(inner, bound, out),
        Expr::Binary(_, a, b) => {
            expr_names(a, bound, out);
            expr_names(b, bound, out);
        }
        Expr::Closure(params, body) => {
            let before = bound.len();
            bound.extend(params.iter().cloned());
            expr_names(body, bound, out);
            bound.truncate(before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renames(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn a_query_of_the_type_follows_the_rename_everywhere_it_names_the_field() {
        let r = renames(&[("amount", "total")]);
        let own = FieldRename {
            type_name: "invoice",
            renames: &r,
            this_is_type: true,
        };
        assert_eq!(
            own.query("from invoice where amount > 10 && !paid sort amount desc")
                .as_deref(),
            Some("from invoice where total > 10 && !paid sort total desc")
        );
        assert_eq!(
            own.query("from invoice group by amount select amount, client.amount")
                .as_deref(),
            Some("from invoice group by total select total, client.amount")
        );
        // A closure parameter is not a field.
        assert_eq!(
            own.query("from invoice where any(lines, |amount| amount > 1)"),
            None
        );
        assert_eq!(own.query("from invoice where !paid"), None);
        assert_eq!(own.query("from invoice where (("), None);
        // `this` is a note of the template's own type.
        assert_eq!(
            own.query("from payment where due > this.amount").as_deref(),
            Some("from payment where due > this.total")
        );
        // Another type's field of the same name is left alone.
        assert_eq!(own.query("from payment where amount > 1"), None);
        assert_eq!(own.query("from * where amount > 1"), None);
        // linked_from names the type and its field.
        assert_eq!(
            own.query("from client where has(linked_from(invoice, amount), this)")
                .as_deref(),
            Some("from client where has(linked_from(invoice, total), this)")
        );
        assert_eq!(
            own.view_field("from invoice", "amount").as_deref(),
            Some("total")
        );
        assert_eq!(
            own.view_field("from invoice", "amount.start").as_deref(),
            Some("total.start")
        );
        assert_eq!(own.view_field("from invoice", "title"), None);
        assert_eq!(own.view_field("from payment", "amount"), None);

        let other = FieldRename {
            type_name: "invoice",
            renames: &r,
            this_is_type: false,
        };
        assert_eq!(other.query("from payment where due > this.amount"), None);
        assert_eq!(
            other
                .query("from invoice, payment where amount > 1")
                .as_deref(),
            Some("from invoice, payment where total > 1")
        );
    }
}
