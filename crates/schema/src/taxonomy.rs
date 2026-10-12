//! Global, hierarchical taxonomies (D7): `taxonomies/<taxonomy>.wcl`.

use std::collections::HashSet;

use crate::blocks::{Check, Form, opt, req};
use crate::doc::{Doc, Value};
use crate::encoding::is_name;
use crate::{Invalid, Issue};

/// One taxonomy. Its terms are in display order, which is also kanban column order.
#[derive(Debug, Clone, PartialEq)]
pub struct Taxonomy {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub terms: Vec<Term>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub name: String,
    pub colour: Option<String>,
    pub description: Option<String>,
    pub children: Vec<Term>,
}

impl Taxonomy {
    /// Parse `taxonomies/<name>.wcl`.
    pub fn parse(name: &str, source: &str) -> Result<Taxonomy, Invalid> {
        let file = format!("taxonomies/{name}.wcl");
        let doc = Doc::parse(source, &file)?;
        Taxonomy::from_doc(name, &doc).map_err(|issues| Invalid { file, issues })
    }

    pub fn from_doc(name: &str, doc: &Doc) -> Result<Taxonomy, Vec<Issue>> {
        let mut issues = Vec::new();
        if !is_name(name) {
            issues.push(Issue::new(
                "file",
                format!("{name:?} is not a valid taxonomy name"),
            ));
        }
        let mut c = Check {
            issues: &mut issues,
        };
        c.items(
            doc,
            &[req("label", Form::Str), opt("description", Form::Str)],
            &[],
            "taxonomy",
        );
        c.only_blocks(doc, &["term"], "taxonomy");
        if doc.blocks_of("term").next().is_none() {
            c.fail("taxonomy", "needs at least one term");
        }
        let terms = terms(&mut c, doc, "taxonomy");
        if !issues.is_empty() {
            return Err(issues);
        }
        Ok(Taxonomy {
            name: name.to_owned(),
            label: string(doc, "label").unwrap_or_default(),
            description: string(doc, "description"),
            terms,
        })
    }

    /// The term at `path`, root first, names joined by `/`. Any term matches, not only a leaf.
    pub fn term(&self, path: &str) -> Option<&Term> {
        let mut level = &self.terms;
        let mut found = None;
        for name in path.split('/') {
            let t = level.iter().find(|t| t.name == name)?;
            found = Some(t);
            level = &t.children;
        }
        found
    }

    pub fn contains(&self, path: &str) -> bool {
        self.term(path).is_some()
    }

    /// Every term's path, depth first in display order.
    pub fn paths(&self) -> Vec<String> {
        fn walk(terms: &[Term], prefix: &str, out: &mut Vec<String>) {
            for t in terms {
                let p = if prefix.is_empty() {
                    t.name.clone()
                } else {
                    format!("{prefix}/{}", t.name)
                };
                out.push(p.clone());
                walk(&t.children, &p, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.terms, "", &mut out);
        out
    }
}

fn terms(c: &mut Check, doc: &Doc, at: &str) -> Vec<Term> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for b in doc.blocks_of("term") {
        let path = format!("{at} > {}", b.name());
        let name = match b.labels.as_slice() {
            [l] if l.is_empty() => {
                c.fail(&path, "a term name cannot be empty");
                continue;
            }
            [l] if l.contains('/') => {
                c.fail(&path, "a term name cannot contain `/`");
                continue;
            }
            [l] => l.clone(),
            _ => {
                c.fail(&path, "needs exactly one label, the term name");
                continue;
            }
        };
        if !seen.insert(name.clone()) {
            c.fail(&path, "repeats a sibling term's name");
        }
        c.items(
            &b.body,
            &[opt("colour", Form::Palette), opt("description", Form::Str)],
            &[],
            &path,
        );
        c.only_blocks(&b.body, &["term"], &path);
        let children = terms(c, &b.body, &path);
        out.push(Term {
            name,
            colour: string(&b.body, "colour"),
            description: string(&b.body, "description"),
            children,
        });
    }
    out
}

fn string(doc: &Doc, name: &str) -> Option<String> {
    doc.field(name).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = include_str!("../../../docs/vault-format/vault/taxonomies/status.wcl");

    #[test]
    fn parses_the_example_status_taxonomy() {
        let t = Taxonomy::parse("status", STATUS).unwrap();
        assert_eq!(t.label, "Status");
        assert!(t.contains("Inbox"));
        assert!(t.contains("Active"));
        assert!(t.contains("Active/In progress"));
        assert!(!t.contains("In progress"));
        assert!(!t.contains("Active/Missing"));
        assert!(!t.contains(""));
    }

    #[test]
    fn rejects_bad_terms() {
        let src = "label = \"X\"\nterm \"A/B\"\nterm \"C\"\nterm \"C\"\nterm \"D\" {\n  colour = \"cyan\"\n}\n";
        let issues = Taxonomy::parse("x", src).unwrap_err().issues;
        let text: Vec<String> = issues.iter().map(ToString::to_string).collect();
        assert!(
            text.iter().any(|i| i.contains("cannot contain `/`")),
            "{text:?}"
        );
        assert!(text.iter().any(|i| i.contains("sibling")), "{text:?}");
        assert!(text.iter().any(|i| i.contains("palette")), "{text:?}");
    }

    #[test]
    fn needs_a_label_and_a_term() {
        let issues = Taxonomy::parse("x", "description = \"d\"\n")
            .unwrap_err()
            .issues;
        assert_eq!(issues.len(), 2, "{issues:?}");
    }
}
