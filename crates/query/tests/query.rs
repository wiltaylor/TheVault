//! Seam S4: every clause and host function against a fixture vault, `this` binding, paging, parse
//! errors, and text to model to text round trips.

use std::collections::HashSet;
use std::path::Path;

use index::Index;
use query::{Context, Error, Model, ModelError, PageRequest};
use schema::Schema;
use store::Vault;

const P1: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2P1";
const P2: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2P2";
const T1: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2T1";
const T2: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2T2";
const T3: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2T3";
const T4: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2T4";
const T5: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2T5";
const B1: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2B1";
const A1: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2A1";
const A2: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2A2";

/// 2026-10-12T12:00:00Z, so `today()` is 2026-10-12.
const NOW: f64 = 1_791_806_400.0;

const STATUS: &str = r#"label = "Status"

term "Inbox"

term "Active" {
  term "In progress"

  term "Blocked"
}

term "Done"
"#;

const TOPIC: &str = r#"label = "Topic"

term "Tech" {
  term "Rust"
}

term "Life"
"#;

const PROJECT: &str = r#"label = "Project"

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

field tasks {
  kind     = "reference"
  target   = "task"
  relation = "two_way"
  inverse  = "project"
  multiple = true
}
"#;

const TASK: &str = r#"label = "Task"

field done {
  kind = "bool"
}

field due {
  kind = "date"
}

field starts_at {
  kind = "datetime"
}

field estimate {
  kind = "int"
}

field score {
  kind = "float"
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

field project {
  kind     = "reference"
  target   = "project"
  relation = "two_way"
  inverse  = "tasks"
}

field tags {
  kind  = "list"
  items = "string"
}

field topics {
  kind     = "taxonomy"
  taxonomy = "topic"
  multiple = true
}

field span {
  kind = "date_range"
}

field body {
  kind = "content"
}
"#;

const BOOK: &str = r#"label = "Book"

field author {
  kind = "string"
}

field topics {
  kind     = "taxonomy"
  taxonomy = "topic"
  multiple = true
}

field body {
  kind = "content"
}
"#;

const ARTICLE: &str = r#"label = "Article"

field body {
  kind = "content"
}
"#;

/// Every type needs a `default` template.
const TEMPLATE: &str = "\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";

fn with_template(ty: &str) -> String {
    format!("{ty}{TEMPLATE}")
}

fn body(text: &str) -> String {
    format!("\ncontent body {{\n  paragraph {{\n    text = \"{text}\"\n  }}\n}}\n")
}

fn note(title: &str, updated: &str, fields: &str) -> String {
    format!(
        "title   = \"{title}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"{updated}\"\n{fields}"
    )
}

fn notes() -> Vec<(&'static str, &'static str, String)> {
    let u = "2026-10-02T09:00:00Z";
    vec![
        (
            "project",
            P1,
            note(
                "Importer",
                u,
                &format!("status = \"Active\"\ntasks = [\"{T1}\", \"{T2}\", \"{T3}\"]\n"),
            ),
        ),
        (
            "project",
            P2,
            note(
                "Garden",
                u,
                &format!("status = \"Inbox\"\ntasks = [\"{T4}\"]\n"),
            ),
        ),
        (
            "task",
            T1,
            note(
                "Ship the importer",
                u,
                &format!(
                    "done = false\ndue = \"2026-10-15\"\nstarts_at = \"2026-10-13T09:00:00+11:00\"\n\
                     estimate = 3\nscore = 4.5\nstatus = \"Active/In progress\"\n\
                     project = \"{P1}\"\ntags = [\"urgent\", \"code\"]\ntopics = [\"Tech/Rust\"]\n\
                     span = {{ start: \"2026-10-13\", end: \"2026-10-20\" }}\n"
                ),
            ),
        ),
        (
            "task",
            T2,
            note(
                "Write the docs",
                u,
                &format!(
                    "done = true\ndue = \"2026-10-01\"\nestimate = 1\nstatus = \"Done\"\n\
                     project = \"{P1}\"\ntags = [\"docs\"]\n"
                ),
            ),
        ),
        (
            "task",
            T3,
            note(
                "Fix the blocker",
                u,
                &format!(
                    "done = false\ndue = \"2026-10-25\"\nestimate = 5\nscore = 2.0\n\
                     status = \"Active/Blocked\"\nproject = \"{P1}\"\ntags = [\"code\"]\n"
                ),
            ),
        ),
        (
            "task",
            T4,
            note(
                "Plant tomatoes",
                u,
                &format!(
                    "done = false\ndue = \"2026-10-12\"\nstatus = \"Inbox\"\nproject = \"{P2}\"\n\
                     topics = [\"Life\"]\n{}",
                    body("Tomatoes like compost.")
                ),
            ),
        ),
        ("task", T5, note("Loose end", u, "done = false\n")),
        (
            "book",
            B1,
            note(
                "Designing Data-Intensive Applications",
                "2026-10-05T09:00:00Z",
                &format!(
                    "author = \"Kleppmann\"\ntopics = [\"Tech\"]\n{}",
                    body("Covers event sourcing and stream processing.")
                ),
            ),
        ),
        (
            "article",
            A1,
            note(
                "Event sourcing explained",
                "2026-10-08T09:00:00Z",
                &body("Event sourcing stores every change as an event."),
            ),
        ),
        (
            "article",
            A2,
            note(
                "Gardening tips",
                "2026-10-09T09:00:00Z",
                &body("Compost and mulch."),
            ),
        ),
    ]
}

fn write(root: &Path, path: &str, text: &str) {
    let full = root.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    index: Index,
    schema: Schema,
}

impl Fixture {
    fn ctx(&self) -> Context<'_> {
        Context::new(&self.index, &self.schema).at(NOW)
    }
}

fn build(files: &[(String, String)], expect: usize) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    for (path, text) in files {
        write(vault.root(), path, text);
    }
    let mut index = Index::open(&vault).unwrap();
    let report = index.rebuild(&vault).unwrap();
    assert_eq!(report.failures, vec![], "fixture files must all be valid");
    assert_eq!(report.indexed, expect);
    let schema = Schema::load(&vault).unwrap_or_else(|e| panic!("{e:?}"));
    Fixture {
        _dir: dir,
        index,
        schema,
    }
}

fn fixture() -> Fixture {
    let mut files: Vec<(String, String)> = vec![
        ("taxonomies/status.wcl".into(), STATUS.into()),
        ("taxonomies/topic.wcl".into(), TOPIC.into()),
        ("types/project.wcl".into(), with_template(PROJECT)),
        ("types/task.wcl".into(), with_template(TASK)),
        ("types/book.wcl".into(), with_template(BOOK)),
        ("types/article.wcl".into(), with_template(ARTICLE)),
    ];
    for (ty, id, text) in notes() {
        files.push((format!("notes/{ty}/{id}.wcl"), text));
    }
    build(&files, notes().len())
}

fn ids(f: &Fixture, text: &str, this: Option<&str>) -> Result<Vec<String>, Error> {
    let q = query::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
    let mut ctx = f.ctx();
    if let Some(t) = this {
        ctx = ctx.with_this(t);
    }
    let page = query::run(&q, &ctx, &PageRequest::default())?;
    assert_eq!(page.next, None, "{text}: one page holds the fixture");
    Ok(page.rows.into_iter().map(|r| r.id).collect())
}

/// Whether the builder can show the query. Those it can must survive text to model to text.
#[derive(Clone, Copy, PartialEq)]
enum B {
    Model,
    Text,
}

/// `(query, this, expected ids in result order, builder)`. Every clause and every host function
/// appears at least once.
fn table() -> Vec<(&'static str, Option<&'static str>, Vec<&'static str>, B)> {
    use B::*;
    vec![
        // from
        ("from task", None, vec![T1, T2, T3, T4, T5], Model),
        (
            "from *",
            None,
            vec![A1, A2, B1, P1, P2, T1, T2, T3, T4, T5],
            Model,
        ),
        ("from book, article", None, vec![A1, A2, B1], Model),
        // where: bool fields, comparisons, && || ! and parentheses
        ("from task where done", None, vec![T2], Model),
        ("from task where !done", None, vec![T1, T3, T4, T5], Model),
        (
            "from task where done == false",
            None,
            vec![T1, T3, T4, T5],
            Model,
        ),
        ("from task where estimate >= 3", None, vec![T1, T3], Model),
        (
            "from task where estimate > 1 && estimate < 5",
            None,
            vec![T1],
            Model,
        ),
        ("from task where estimate <= 1", None, vec![T2], Model),
        ("from task where score <= 2.5", None, vec![T3], Model),
        (
            "from task where status == \"Done\" || estimate == 5",
            None,
            vec![T2, T3],
            Model,
        ),
        (
            "from task where status != \"Done\"",
            None,
            vec![T1, T3, T4, T5],
            Model,
        ),
        (
            "from task where !done && (estimate > 3 || has(tags, \"urgent\"))",
            None,
            vec![T1, T3],
            Model,
        ),
        ("from task where estimate * 2 > 5", None, vec![T1, T3], Text),
        ("from task where -estimate < -4", None, vec![T3], Text),
        // today(), now(), days()
        ("from task where due < today()", None, vec![T2], Model),
        ("from task where due == today()", None, vec![T4], Model),
        (
            "from task where due < today() + days(7)",
            None,
            vec![T1, T2, T4],
            Model,
        ),
        (
            "from task where due > today() - days(1)",
            None,
            vec![T1, T3, T4],
            Model,
        ),
        (
            "from task where due - today() == days(3)",
            None,
            vec![T1],
            Text,
        ),
        ("from task where starts_at > now()", None, vec![T1], Model),
        // A datetime with an offset compares as a time, not as text.
        (
            "from task where starts_at < \"2026-10-12T23:00:00Z\"",
            None,
            vec![T1],
            Model,
        ),
        (
            "from task where due >= \"2026-10-15\"",
            None,
            vec![T1, T3],
            Model,
        ),
        // A record member.
        (
            "from task where span.start < \"2026-10-14\"",
            None,
            vec![T1],
            Model,
        ),
        // has(), under()
        (
            "from task where has(tags, \"code\")",
            None,
            vec![T1, T3],
            Model,
        ),
        (
            "from task where under(status, \"Active\")",
            None,
            vec![T1, T3],
            Model,
        ),
        (
            "from task where under(status, \"Active/Blocked\")",
            None,
            vec![T3],
            Model,
        ),
        (
            "from * where under(topics, \"Tech\")",
            None,
            vec![B1, T1],
            Model,
        ),
        // any(), all() and closures
        (
            "from task where any(tags, |t| t == \"docs\" || t == \"urgent\")",
            None,
            vec![T1, T2],
            Text,
        ),
        (
            "from task where all(tags, |t| t == \"code\")",
            None,
            vec![T3, T4, T5],
            Text,
        ),
        // this, and a dot following a single reference
        (
            "from task where project == this",
            Some(P1),
            vec![T1, T2, T3],
            Model,
        ),
        (
            "from task where project == this && !done",
            Some(P1),
            vec![T1, T3],
            Model,
        ),
        (
            "from task where project.status == \"Inbox\"",
            None,
            vec![T4],
            Model,
        ),
        (
            "from task where under(project.status, \"Active\")",
            None,
            vec![T1, T2, T3],
            Model,
        ),
        (
            "from project where has(tasks, this)",
            Some(T4),
            vec![P2],
            Model,
        ),
        // linked_from()
        (
            "from project where any(linked_from(task, project), |t| !t.done)",
            None,
            vec![P1, P2],
            Text,
        ),
        (
            "from project where any(linked_from(task, project), |t| t.status == \"Done\")",
            None,
            vec![P1],
            Text,
        ),
        (
            "from project where all(linked_from(\"task\", \"project\"), |t| t.estimate > 0)",
            None,
            vec![P1],
            Text,
        ),
        // search
        ("from task search \"importer\"", None, vec![T1], Model),
        (
            "from book, article search \"event sourcing\" sort title",
            None,
            vec![B1, A1],
            Model,
        ),
        (
            "from book, article search \"event sourcing\" sort updated desc limit 20",
            None,
            vec![A1, B1],
            Model,
        ),
        // sort: missing values last, taxonomy fields in term order, through a reference
        ("from task sort due", None, vec![T2, T4, T1, T3, T5], Model),
        (
            "from task sort due asc",
            None,
            vec![T2, T4, T1, T3, T5],
            Model,
        ),
        (
            "from task sort due desc",
            None,
            vec![T3, T1, T4, T2, T5],
            Model,
        ),
        (
            "from task sort status",
            None,
            vec![T4, T1, T3, T2, T5],
            Model,
        ),
        (
            "from task sort project.title, title",
            None,
            vec![T4, T3, T1, T2, T5],
            Model,
        ),
        (
            "from task sort done desc, estimate desc",
            None,
            vec![T2, T3, T1, T4, T5],
            Model,
        ),
        // group by
        (
            "from task where !done group by status",
            None,
            vec![T4, T1, T3, T5],
            Model,
        ),
        (
            "from task where under(status, \"Active\") && due < today() + days(7) group by status",
            None,
            vec![T1],
            Model,
        ),
        // select
        (
            "from task where done select title, project.title",
            None,
            vec![T2],
            Model,
        ),
        // limit
        (
            "from task sort estimate desc limit 2",
            None,
            vec![T3, T1],
            Model,
        ),
        ("from task limit 0", None, vec![], Model),
    ]
}

#[test]
fn every_clause_and_host_function_returns_the_expected_notes() {
    let f = fixture();
    let mut failures = Vec::new();
    for (text, this, want, _) in table() {
        match ids(&f, text, this) {
            Ok(got) if got == want => {}
            Ok(got) => failures.push(format!("{text}\n  want {want:?}\n  got  {got:?}")),
            Err(e) => failures.push(format!("{text}\n  error: {e}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn text_to_model_to_text_reproduces_every_query_the_model_can_represent() {
    for (text, _, _, builder) in table() {
        assert_eq!(query::format(text).unwrap(), text, "the table is canonical");
        match (Model::from_text(text), builder) {
            (Ok(model), B::Model) => {
                assert_eq!(model.to_text().unwrap(), text);
                // The model is what the web client edits, so it survives JSON too.
                let json = serde_json::to_string(&model).unwrap();
                let back: Model = serde_json::from_str(&json).unwrap();
                assert_eq!(back, model, "{json}");
            }
            (Err(ModelError::Unrepresentable(_)), B::Text) => {}
            (got, _) => panic!("{text}: {got:?}"),
        }
    }
}

#[test]
fn under_matches_a_term_and_every_term_below_it() {
    let f = fixture();
    let active = ids(&f, "from task where under(status, \"Active\")", None).unwrap();
    // T1 is "Active/In progress" and T3 "Active/Blocked": both descendants of "Active".
    assert_eq!(active, vec![T1, T3]);
    let exact = ids(&f, "from project where under(status, \"Active\")", None).unwrap();
    assert_eq!(exact, vec![P1]);
    // A name that merely starts the same way is not below it.
    let none = ids(&f, "from task where under(status, \"Active/In\")", None);
    assert!(matches!(none, Err(Error::UnknownTerm { .. })), "{none:?}");
}

#[test]
fn group_by_reports_groups_in_term_order_with_counts() {
    let f = fixture();
    let q = query::parse("from task group by status").unwrap();
    let page = query::run(&q, &f.ctx(), &PageRequest::default()).unwrap();
    let groups: Vec<(serde_json::Value, usize)> = page
        .groups
        .into_iter()
        .map(|g| (g.value, g.count))
        .collect();
    assert_eq!(
        groups,
        vec![
            ("Inbox".into(), 1),
            ("Active/In progress".into(), 1),
            ("Active/Blocked".into(), 1),
            ("Done".into(), 1),
            (serde_json::Value::Null, 1),
        ]
    );
    assert_eq!(page.rows[0].group, Some("Inbox".into()));
}

#[test]
fn select_returns_stored_values_keyed_by_path() {
    let f = fixture();
    let q = query::parse("from task where done select title, due, project.title, project.status")
        .unwrap();
    let page = query::run(&q, &f.ctx(), &PageRequest::default()).unwrap();
    let row = &page.rows[0];
    assert_eq!(row.id, T2);
    assert_eq!(row.type_name, "task");
    assert_eq!(
        serde_json::Value::Object(row.fields.clone()),
        serde_json::json!({
            "title": "Write the docs",
            "due": "2026-10-01",
            "project.title": "Importer",
            "project.status": "Active",
        })
    );
}

#[test]
fn this_without_a_note_fails_clearly() {
    let f = fixture();
    let err = ids(&f, "from task where project == this", None).unwrap_err();
    assert!(matches!(err, Error::NoThis), "{err:?}");
    assert!(err.to_string().contains("not run from a note"), "{err}");
}

#[test]
fn unknown_names_fail_before_any_note_is_read() {
    let f = fixture();
    let cases = [
        ("from nothing", "there is no type \"nothing\""),
        (
            "from task where colour == \"red\"",
            "unknown field `colour`: task has no such field",
        ),
        ("from task sort colour", "unknown field `colour`"),
        (
            "from task where under(status, \"Paused\")",
            "taxonomy status has no term \"Paused\"",
        ),
        (
            "from project where any(linked_from(task, title), |t| t.done)",
            "not a reference field",
        ),
    ];
    for (text, want) in cases {
        let err = ids(&f, text, None).unwrap_err().to_string();
        assert!(err.contains(want), "{text}: {err}");
    }
}

#[test]
fn a_dot_on_a_list_says_to_use_any() {
    let f = fixture();
    let err = ids(&f, "from project where tasks.done", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("use any() or all()"), "{err}");
}

fn many(n: usize) -> Fixture {
    let mut files = vec![(
        "types/item.wcl".to_owned(),
        with_template("label = \"Item\"\n\nfield n {\n  kind = \"int\"\n}\n"),
    )];
    for i in 0..n {
        let id = format!("01JB8Z4K2MQ7V3X9T5R6W8{i:04}");
        // Many ties, so the order has to fall back to the id to be total.
        let text = format!(
            "title = \"Item {i}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\nn = {}\n",
            i % 7
        );
        files.push((format!("notes/item/{id}.wcl"), text));
    }
    build(&files, n)
}

fn page_through(f: &Fixture, text: &str, size: usize) -> (Vec<String>, usize) {
    let q = query::parse(text).unwrap();
    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = query::run(
            &q,
            &f.ctx(),
            &PageRequest {
                cursor: cursor.take(),
                size,
            },
        )
        .unwrap();
        pages += 1;
        assert!(page.rows.len() <= size);
        seen.extend(page.rows.into_iter().map(|r| r.id));
        match page.next {
            Some(c) => cursor = Some(c),
            None => return (seen, pages),
        }
    }
}

#[test]
fn paging_a_thousand_notes_in_pages_of_fifty_returns_each_exactly_once() {
    let f = many(1000);
    for text in [
        "from item",
        "from item sort n desc",
        "from item sort n group by n",
    ] {
        let (seen, pages) = page_through(&f, text, 50);
        assert_eq!(pages, 20, "{text}");
        assert_eq!(seen.len(), 1000, "{text}");
        let unique: HashSet<&String> = seen.iter().collect();
        assert_eq!(unique.len(), 1000, "{text}: a note came back twice");
        // The pages joined are the unpaged result, in order.
        let q = query::parse(text).unwrap();
        let all = query::run(&q, &f.ctx(), &PageRequest::default()).unwrap();
        assert_eq!(all.total, 1000);
        assert_eq!(all.rows.len(), 1000);
        let whole: Vec<String> = all.rows.into_iter().map(|r| r.id).collect();
        assert_eq!(seen, whole, "{text}");
    }
}

#[test]
fn limit_caps_the_whole_result_not_a_page() {
    let f = many(200);
    let (seen, pages) = page_through(&f, "from item sort n limit 120", 50);
    assert_eq!(seen.len(), 120);
    assert_eq!(pages, 3);
    let q = query::parse("from item sort n limit 120").unwrap();
    let first = query::run(
        &q,
        &f.ctx(),
        &PageRequest {
            cursor: None,
            size: 50,
        },
    )
    .unwrap();
    assert_eq!(first.total, 120);
}

#[test]
fn a_cursor_from_elsewhere_is_refused() {
    let f = fixture();
    let q = query::parse("from task").unwrap();
    for bad in ["zz", "7b7d", ""] {
        let r = query::run(
            &q,
            &f.ctx(),
            &PageRequest {
                cursor: Some(bad.to_owned()),
                size: 2,
            },
        );
        assert!(matches!(r, Err(Error::BadCursor)), "{bad}: {r:?}");
    }
}

#[test]
fn parse_errors_report_line_and_column() {
    let cases: [(&str, usize, usize, &str); 12] = [
        ("where done", 1, 1, "expected `from`"),
        ("from task\nwhere done &&", 2, 14, "expected an expression"),
        ("from task sort due where done", 1, 20, "out of order"),
        ("from task where foo(1)", 1, 17, "unknown function `foo`"),
        (
            "from task where title == \"abc",
            1,
            26,
            "unterminated string",
        ),
        (
            "from\n  task\n  where x == \"a{b}\"",
            3,
            16,
            "cannot interpolate",
        ),
        ("from task limit -1", 1, 17, "expected a whole number"),
        (
            "from task where days()",
            1,
            17,
            "`days` takes 1 argument, found 0",
        ),
        ("from task where any(tags, t)", 1, 27, "a closure"),
        (
            "from task where title == \"é\" && # ",
            1,
            33,
            "unexpected character `#`",
        ),
        ("from task where (done", 1, 22, "expected `)`"),
        (
            "from task where project.title()",
            1,
            30,
            "methods cannot be called",
        ),
    ];
    for (text, line, column, message) in cases {
        let e = query::parse(text).unwrap_err();
        assert_eq!((e.line, e.column), (line, column), "{text}: {e}");
        assert!(e.message.contains(message), "{text}: {e}");
        assert!(e.to_string().starts_with(&format!("{line}:{column}: ")));
    }
}

#[test]
fn format_canonicalises_layout_comments_and_parentheses() {
    let text = "from  task\n  where (done)   && ((a || b))  // open ones\n  sort due  desc ,title";
    assert_eq!(
        query::format(text).unwrap(),
        "from task where done && (a || b) sort due desc, title"
    );
    let strings = "from task where title == \"say \\\"hi\\\" {{x}}\\n\"";
    assert_eq!(query::format(strings).unwrap(), strings);
    // Precedence is wscript's: comparisons share a tier and associate to the left.
    assert_eq!(
        query::format("from task where a - (b - c) == (d == e)").unwrap(),
        "from task where a - (b - c) == (d == e)"
    );
    assert_eq!(
        query::format("from task where (a - b) - c").unwrap(),
        "from task where a - b - c"
    );
}

#[test]
fn the_model_reports_text_it_cannot_represent() {
    for text in [
        "from task where any(tags, |t| t == \"a\")",
        "from task where estimate * 2 > 5",
        "from task where 3 < estimate",
        "from task where !(done && estimate > 1)",
        "from task where due < today() + days(0)",
    ] {
        let r = Model::from_text(text);
        assert!(
            matches!(r, Err(ModelError::Unrepresentable(_))),
            "{text}: {r:?}"
        );
    }
    assert!(matches!(
        Model::from_text("from task where"),
        Err(ModelError::Parse(_))
    ));
}

#[test]
fn a_model_built_in_the_ui_becomes_text() {
    use query::{Condition, Group, Item, Join, Op, Operand, Order, Sort};
    let model = Model {
        from: vec!["task".into()],
        filter: Some(Group {
            join: Join::All,
            items: vec![
                Item::Condition(Condition {
                    field: "status".into(),
                    op: Op::Under,
                    value: Some(Operand::Str("Active".into())),
                }),
                Item::Group(Group {
                    join: Join::Any,
                    items: vec![
                        Item::Condition(Condition {
                            field: "due".into(),
                            op: Op::Lt,
                            value: Some(Operand::Today(7)),
                        }),
                        Item::Condition(Condition {
                            field: "done".into(),
                            op: Op::IsFalse,
                            value: None,
                        }),
                    ],
                }),
            ],
        }),
        search: None,
        sort: vec![Sort {
            field: "due".into(),
            order: Order::Desc,
        }],
        group_by: Some("status".into()),
        select: vec![],
        limit: Some(10),
    };
    let text = model.to_text().unwrap();
    assert_eq!(
        text,
        "from task where under(status, \"Active\") && (due < today() + days(7) || !done) \
         sort due desc group by status limit 10"
    );
    assert_eq!(Model::from_text(&text).unwrap(), model);
    let empty = Model {
        filter: Some(Group {
            join: Join::All,
            items: vec![],
        }),
        ..model
    };
    assert!(matches!(empty.to_text(), Err(ModelError::Invalid(_))));
}
