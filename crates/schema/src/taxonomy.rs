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
    /// An open taxonomy: a note saved with a term path it does not hold yet adds that term.
    pub allow_new: bool,
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
            fields: None,
            template: false,
        };
        c.items(
            doc,
            &[
                req("label", Form::Str),
                opt("description", Form::Str),
                opt("allow_new", Form::Bool),
            ],
            &[],
            "taxonomy",
        );
        c.only_blocks(doc, &["term"], "taxonomy");
        let allow_new = doc.field("allow_new") == Some(&Value::Bool(true));
        // An open taxonomy may start empty: its notes add its terms.
        if !allow_new && doc.blocks_of("term").next().is_none() {
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
            allow_new,
            terms,
        })
    }

    /// The path of the term `path` names, matching each name exactly or else without case, and
    /// whether it is new. A term that does not exist yet is added under its parent when `add` is
    /// set; its parent must exist. With `exact`, names match only exactly.
    ///
    /// The error says why `path` cannot name or add a term.
    pub fn resolve_term(
        &mut self,
        path: &str,
        exact: bool,
        add: bool,
    ) -> Result<(String, bool), String> {
        let names: Vec<&str> = path.split('/').collect();
        if names.iter().any(|n| n.is_empty()) {
            return Err(format!("{path:?}: a term name cannot be empty"));
        }
        let taxonomy = self.name.clone();
        let mut level = &mut self.terms;
        let mut found: Vec<String> = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            let at = level.iter().position(|t| t.name == *name).or_else(|| {
                (!exact)
                    .then(|| {
                        level
                            .iter()
                            .position(|t| t.name.to_lowercase() == name.to_lowercase())
                    })
                    .flatten()
            });
            let at = match at {
                Some(at) => at,
                None if i + 1 == names.len() && add => {
                    level.push(Term {
                        name: (*name).to_owned(),
                        colour: None,
                        description: None,
                        children: Vec::new(),
                    });
                    found.push((*name).to_owned());
                    return Ok((found.join("/"), true));
                }
                None if i + 1 == names.len() => {
                    return Err(format!("{path:?} is not a term of taxonomy {taxonomy:?}"));
                }
                None => {
                    let parent = names[..=i].join("/");
                    return Err(format!(
                        "{path:?}: taxonomy {taxonomy:?} has no term {parent:?} to add {:?} under",
                        names[names.len() - 1]
                    ));
                }
            };
            let here = level;
            found.push(here[at].name.clone());
            level = &mut here[at].children;
        }
        Ok((found.join("/"), false))
    }

    /// The taxonomy file's text, before canonical formatting.
    pub fn to_wcl(&self) -> String {
        fn quote(out: &mut String, s: &str) {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        fn item(out: &mut String, indent: &str, name: &str, value: &str) {
            out.push_str(indent);
            out.push_str(name);
            out.push_str(" = ");
            quote(out, value);
            out.push('\n');
        }
        fn write_terms(out: &mut String, terms: &[Term], indent: &str) {
            for t in terms {
                out.push('\n');
                out.push_str(indent);
                out.push_str("term ");
                quote(out, &t.name);
                if t.colour.is_none() && t.description.is_none() && t.children.is_empty() {
                    out.push('\n');
                    continue;
                }
                out.push_str(" {\n");
                let inner = format!("{indent}  ");
                if let Some(c) = &t.colour {
                    item(out, &inner, "colour", c);
                }
                if let Some(d) = &t.description {
                    item(out, &inner, "description", d);
                }
                write_terms(out, &t.children, &inner);
                out.push_str(indent);
                out.push_str("}\n");
            }
        }
        let mut out = String::new();
        item(&mut out, "", "label", &self.label);
        if let Some(d) = &self.description {
            item(&mut out, "", "description", d);
        }
        if self.allow_new {
            out.push_str("allow_new = true\n");
        }
        write_terms(&mut out, &self.terms, "");
        out
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
    fn reads_term_colours_and_leaves_an_uncoloured_term_without_one() {
        let t = Taxonomy::parse("status", STATUS).unwrap();
        let colour = |p: &str| t.term(p).unwrap().colour.as_deref();
        assert_eq!(colour("Inbox"), Some("gray"));
        assert_eq!(colour("Active"), Some("blue"));
        // Inheritance is the client's to resolve; the file holds only what was written.
        assert_eq!(colour("Active/In progress"), None);
        assert_eq!(colour("Active/Blocked"), Some("red"));
    }

    #[test]
    fn every_palette_colour_is_a_term_colour_and_others_are_named_in_the_error() {
        for c in crate::encoding::PALETTE {
            let src = format!("label = \"X\"\nterm \"A\" {{\n  colour = \"{c}\"\n}}\n");
            let t = Taxonomy::parse("x", &src).unwrap();
            assert_eq!(t.terms[0].colour.as_deref(), Some(c));
        }
        let src = "label = \"X\"\nterm \"A\" {\n  term \"B\" {\n    colour = \"teal\"\n  }\n}\n";
        let issues = Taxonomy::parse("x", src).unwrap_err().issues;
        assert_eq!(issues.len(), 1, "{issues:?}");
        let text = issues[0].to_string();
        assert!(text.contains("term \"A\" > term \"B\""), "{text}");
        assert!(text.contains("\"teal\" is not a palette colour"), "{text}");
        assert!(text.contains("gray, brown, orange"), "{text}");
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

    #[test]
    fn resolves_and_adds_terms_by_path() {
        let mut t = Taxonomy::parse("status", STATUS).unwrap();
        let ok = |r: Result<(String, bool), String>| r.unwrap();
        // An existing term, exactly or in another case.
        assert_eq!(
            ok(t.resolve_term("Active/In progress", false, true)),
            ("Active/In progress".into(), false)
        );
        assert_eq!(
            ok(t.resolve_term("active/IN PROGRESS", false, true)),
            ("Active/In progress".into(), false)
        );
        // Without `add`, a missing term is an error; with `exact`, another case is missing.
        let err = t.resolve_term("Active/Waiting", false, false).unwrap_err();
        assert!(err.contains("is not a term of taxonomy"), "{err}");
        assert_eq!(
            ok(t.resolve_term("inbox", true, true)),
            ("inbox".into(), true)
        );
        // A new term under an existing parent, matched without case, and a new top-level term.
        assert_eq!(
            ok(t.resolve_term("active/Waiting", false, true)),
            ("Active/Waiting".into(), true)
        );
        assert_eq!(
            ok(t.resolve_term("Archived", false, true)),
            ("Archived".into(), true)
        );
        assert!(t.contains("Active/Waiting") && t.contains("Archived"));
        // A missing parent and an empty name are refused.
        let err = t.resolve_term("Later/Maybe", false, true).unwrap_err();
        assert!(
            err.contains("has no term \"Later\" to add \"Maybe\" under"),
            "{err}"
        );
        for bad in ["", "/A", "Active/", "Active//B"] {
            assert!(t.resolve_term(bad, false, true).is_err(), "{bad:?}");
        }
        assert!(!t.contains("Later"));
    }

    #[test]
    fn writes_itself_back_as_the_same_taxonomy() {
        let mut t = Taxonomy::parse("status", STATUS).unwrap();
        t.allow_new = true;
        t.description = Some("Say \"hi\"\nthen go".into());
        t.resolve_term("Active/Wait \\ see", false, true).unwrap();
        let back = Taxonomy::parse("status", &t.to_wcl()).unwrap();
        assert_eq!(back, t);
    }
}
