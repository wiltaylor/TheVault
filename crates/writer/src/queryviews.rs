//! A field rename in a type change renames the field in the `query_view` blocks of templates:
//! those of the changed type, and those of any other type whose query views read the changed type
//! (D6, D8, D14). [`schema::FieldRename`] says what a query and a view item become; this module
//! applies it to the text of a type file.

use schema::{FieldRename, VIEW_FIELD_ITEMS};
use wcl_lang::ast::{Block, Expr, Item};

use crate::Result;

/// `text`, the type file at `path`, with every `query_view` in its templates following `r`.
/// `None` when no query view changes.
pub(crate) fn rename_in_templates(
    text: &str,
    path: &str,
    r: &FieldRename,
) -> Result<Option<String>> {
    if r.renames.is_empty() {
        return Ok(None);
    }
    let mut ast = store::parse_data_only(text, path)?;
    let mut changed = false;
    for item in &mut ast.items {
        if let Item::Block(b) = item
            && b.kind == "template"
        {
            changed |= walk(&mut b.items, r);
        }
    }
    Ok(changed.then(|| store::to_canonical(&ast)))
}

fn walk(items: &mut [Item], r: &FieldRename) -> bool {
    let mut changed = false;
    for item in items {
        if let Item::Block(b) = item {
            changed |= if b.kind == "query_view" {
                query_view(b, r)
            } else {
                walk(&mut b.items, r)
            };
        }
    }
    changed
}

fn query_view(b: &mut Block, r: &FieldRename) -> bool {
    let Some(query) = b.items.iter().find_map(|i| match i {
        Item::Field(f) if f.name == "query" => match &f.expr {
            Expr::Utf8(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }) else {
        return false;
    };
    let mut changed = false;
    for item in &mut b.items {
        match item {
            Item::Field(f) if f.name == "query" => {
                if let Some(to) = r.query(&query) {
                    f.expr = Expr::Utf8(to);
                    changed = true;
                }
            }
            Item::Block(view) if view.kind == "view" => {
                for item in &mut view.items {
                    let Item::Field(f) = item else { continue };
                    if !VIEW_FIELD_ITEMS.contains(&f.name.as_str()) {
                        continue;
                    }
                    let values: Vec<&mut Expr> = match &mut f.expr {
                        Expr::ListLit { elements, .. } => elements.iter_mut().collect(),
                        e => vec![e],
                    };
                    for v in values {
                        if let Expr::Utf8(s) = v
                            && let Some(to) = r.view_field(&query, s)
                        {
                            *s = to;
                            changed = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    changed
}
