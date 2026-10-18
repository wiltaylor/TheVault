//! The shared cases of a template tab's `when` (D14, D42), run through the query evaluator note
//! by note. `web/src/when.test.ts` runs the same file through the web client's evaluator,
//! so the two agree on every operator, function and missing value.

use std::collections::BTreeMap;
use std::path::Path;

use index::Index;
use query::{Context, PageRequest};
use schema::Schema;
use serde_json::{Map, Value};
use store::Vault;

const CASES: &str = include_str!("when_cases.json");

fn write(root: &Path, path: &str, text: &str) {
    let full = root.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

/// A JSON value as a WCL value: strings quoted, objects as inline maps.
fn wcl(v: &Value) -> String {
    match v {
        Value::Array(items) => {
            format!("[{}]", items.iter().map(wcl).collect::<Vec<_>>().join(", "))
        }
        Value::Object(m) => format!(
            "{{ {} }}",
            m.iter()
                .map(|(k, v)| format!("{k}: {}", wcl(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

fn type_file(name: &str, fields: &[Value]) -> String {
    let mut out = format!("label = \"{name}\"\n");
    for f in fields {
        let f = f.as_object().unwrap();
        out += &format!("\nfield {} {{\n", f["name"].as_str().unwrap());
        for (k, v) in f.iter().filter(|(k, _)| *k != "name") {
            out += &format!("  {k} = {}\n", wcl(v));
        }
        out += "}\n";
    }
    out + "\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n"
}

/// A taxonomy from its term paths, parents before children.
fn taxonomy_file(name: &str, paths: &[Value]) -> String {
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in paths {
        let p = p.as_str().unwrap();
        let parent = p.rsplit_once('/').map_or("", |(a, _)| a);
        children
            .entry(parent.to_owned())
            .or_default()
            .push(p.to_owned());
    }
    fn terms(children: &BTreeMap<String, Vec<String>>, parent: &str, depth: usize) -> String {
        let pad = "  ".repeat(depth);
        let mut out = String::new();
        for path in children.get(parent).into_iter().flatten() {
            let name = path.rsplit('/').next().unwrap();
            let inner = terms(children, path, depth + 1);
            if inner.is_empty() {
                out += &format!("{pad}term \"{name}\"\n");
            } else {
                out += &format!("{pad}term \"{name}\" {{\n{inner}{pad}}}\n");
            }
        }
        out
    }
    format!("label = \"{name}\"\n\n{}", terms(&children, "", 0))
}

fn note_file(fields: &Map<String, Value>) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{k} = {}\n", wcl(v)))
        .collect()
}

/// Seconds since the Unix epoch of a `YYYY-MM-DDTHH:MM:SSZ` time.
fn epoch(s: &str) -> f64 {
    let n = |r: std::ops::Range<usize>| s[r].parse::<i64>().unwrap();
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (days * 86_400 + n(11..13) * 3600 + n(14..16) * 60 + n(17..19)) as f64
}

#[test]
fn the_shared_when_cases_hold_in_the_query_evaluator() {
    let file: Value = serde_json::from_str(CASES).unwrap();
    let type_name = file["type"].as_str().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    let root = vault.root();
    write(
        root,
        &format!("types/{type_name}.wcl"),
        &type_file(type_name, file["fields"].as_array().unwrap()),
    );
    for (name, paths) in file["taxonomies"].as_object().unwrap() {
        write(
            root,
            &format!("taxonomies/{name}.wcl"),
            &taxonomy_file(name, paths.as_array().unwrap()),
        );
    }
    let notes = file["notes"].as_object().unwrap();
    for note in notes.values() {
        let id = note["id"].as_str().unwrap();
        write(
            root,
            &format!("notes/{type_name}/{id}.wcl"),
            &note_file(note["fields"].as_object().unwrap()),
        );
    }
    let mut index = Index::open(&vault).unwrap();
    let report = index.rebuild(&vault).unwrap();
    assert_eq!(report.failures, vec![], "the case notes must be valid");
    assert_eq!(report.indexed, notes.len());
    let schema = Schema::load(&vault).unwrap();
    let now = epoch(file["now"].as_str().unwrap());

    let cases = file["cases"].as_array().unwrap();
    assert!(
        cases.len() >= 25,
        "the file covers every operator and function"
    );
    let mut wrong = Vec::new();
    for case in cases {
        let when = case["when"].as_str().unwrap();
        let id = notes[case["note"].as_str().unwrap()]["id"]
            .as_str()
            .unwrap();
        // The condition reads the note shown, which is `this` too; only that note is tested.
        let text = format!("from {type_name} include archived where id == this && ({when})");
        let q = query::parse(&text).unwrap_or_else(|e| panic!("{when}: {e}"));
        let ctx = Context::new(&index, &schema).at(now).with_this(id);
        // A condition that fails to evaluate counts as false, as in the web client.
        let got = query::run_scan(&q, &ctx, &PageRequest::default())
            .map(|page| page.rows.iter().any(|r| r.id == id))
            .unwrap_or(false);
        if got != case["expected"].as_bool().unwrap() {
            wrong.push(format!("{} on {}: got {got}", when, case["note"]));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_when_parses_as_an_expression_on_its_own() {
    assert!(query::parse_expr("done && due < today() + days(7)").is_ok());
    let e = query::parse_expr("done sort title").unwrap_err();
    assert_eq!((e.line, e.column), (1, 6), "{e}");
    let e = query::parse_expr("status == ").unwrap_err();
    assert_eq!((e.line, e.column), (1, 11), "{e}");
}
