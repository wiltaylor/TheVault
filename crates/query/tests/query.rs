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
/// An archived task, which no query reads unless it asks for archived notes.
const T6: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2T6";
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
            "task",
            T6,
            note(
                "Retired idea",
                u,
                "archived = true\ndone = false\nestimate = 2\n",
            ),
        ),
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
        // contains and empty
        (
            "from * where contains(title, \"THE\")",
            None,
            vec![T1, T2, T3],
            Text,
        ),
        (
            "from task where contains(tags, \"od\")",
            None,
            vec![T1, T3],
            Text,
        ),
        (
            "from book where contains(author, \"klepp\")",
            None,
            vec![B1],
            Text,
        ),
        ("from task where empty(estimate)", None, vec![T4, T5], Text),
        ("from task where !empty(tags)", None, vec![T1, T2, T3], Text),
        ("from task where empty(project)", None, vec![T5], Text),
        (
            "from task where empty(span)",
            None,
            vec![T2, T3, T4, T5],
            Text,
        ),
        (
            "from task where empty(topics)",
            None,
            vec![T2, T3, T5],
            Text,
        ),
        (
            "from task where !empty(status) && contains(title, \"i\") sort title",
            None,
            vec![T3, T1, T2],
            Text,
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
        // archived notes
        ("from task where archived", None, vec![T6], Model),
        (
            "from * where !archived && estimate == 2",
            None,
            vec![],
            Model,
        ),
        (
            "from task include archived where !done sort title",
            None,
            vec![T3, T5, T4, T6, T1],
            Model,
        ),
        (
            "from task where archived || done sort title",
            None,
            vec![T6, T2],
            Model,
        ),
        ("from * search \"retired\"", None, vec![], Model),
        (
            "from * include archived search \"retired\"",
            None,
            vec![T6],
            Model,
        ),
        (
            "from task where archived search \"idea\"",
            None,
            vec![T6],
            Model,
        ),
        (
            "from task where any(tags, |archived| archived == \"docs\")",
            None,
            vec![T2],
            Text,
        ),
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

/// The model exactly as the web builder sends it for a type, two conditions, a sort and a limit.
#[test]
fn a_model_built_in_the_web_builder_prints_the_expected_text() {
    let model: Model = serde_json::from_value(serde_json::json!({
        "from": ["task"],
        "filter": {
            "join": "all",
            "items": [
                { "condition": { "field": "status", "op": "eq", "value": { "kind": "str", "value": "todo" } } },
                { "condition": { "field": "estimate", "op": "gt", "value": { "kind": "int", "value": 2 } } },
            ],
        },
        "search": null,
        "sort": [{ "field": "title", "order": "desc" }],
        "group_by": null,
        "select": [],
        "limit": 10,
    }))
    .unwrap();
    assert_eq!(
        model.to_text().unwrap(),
        "from task where status == \"todo\" && estimate > 2 sort title desc limit 10"
    );
}

/// An agent may write any spacing. The builder still shows such a query; the web client keeps
/// the text as written until the model is edited.
#[test]
fn agent_spacing_still_opens_in_the_builder() {
    let text = "from   task\nwhere status==\"todo\"  &&estimate>2\n  sort title desc limit 10";
    let model = Model::from_text(text).unwrap();
    assert_eq!(
        model.to_text().unwrap(),
        "from task where status == \"todo\" && estimate > 2 sort title desc limit 10"
    );
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

/// The shapes of query the web Search page writes from its filters (web/src/search).
#[test]
fn the_search_page_query_shapes_run() {
    let f = fixture();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        (
            "from task where span.start <= \"2026-10-14\" && span.end >= \"2026-10-14\"",
            vec![T1],
        ),
        ("from task where span.end > \"2026-10-19\"", vec![T1]),
        ("from task where span.start < \"2026-10-13\"", vec![]),
        (
            "from task where starts_at >= \"2026-10-13T00:00:00+11:00\" && starts_at < \"2026-10-14T00:00:00+11:00\"",
            vec![T1],
        ),
        (
            "from task where created >= now() - days(30) sort updated desc",
            vec![T1, T2, T3, T4, T5],
        ),
        (
            "from * where created >= \"2026-10-01T00:00:00Z\" && created < \"2026-10-02T00:00:00Z\" && type == \"book\"",
            vec![B1],
        ),
        (
            "from task where due >= today() - days(7) && due <= today()",
            vec![T4],
        ),
        (
            "from task, project where type == \"task\" && done && type == \"project\"",
            vec![],
        ),
        ("from task where any(tags, |v| v == \"docs\")", vec![T2]),
        (
            "from task where (status == \"Done\" || status == \"Inbox\") && !empty(project)",
            vec![T2, T4],
        ),
        ("from * search \"compost\" sort title asc", vec![A2, T4]),
    ];
    for (text, want) in cases {
        assert_eq!(ids(&f, text, None).unwrap(), want, "{text}");
    }
}

#[test]
fn contains_reads_only_text() {
    let f = fixture();
    let err = ids(&f, "from task where contains(estimate, \"3\")", None).unwrap_err();
    assert!(err.to_string().contains("contains() reads text"), "{err}");
    let err = ids(&f, "from task where contains(title, 3)", None).unwrap_err();
    assert!(
        err.to_string().contains("contains() looks for text"),
        "{err}"
    );
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
    let cases: [(&str, usize, usize, &str); 13] = [
        (
            "from task include where done",
            1,
            19,
            "expected `archived` after `include`",
        ),
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
    assert_eq!(
        query::format("from task  include   archived where done").unwrap(),
        "from task include archived where done"
    );
    // `include` is a word only after the type list, so a field may still be called that.
    assert_eq!(
        query::format("from task where include sort include").unwrap(),
        "from task where include sort include"
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
        include_archived: false,
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

/// Every page of `text`, from [`query::run`] or from the note-by-note [`query::run_scan`].
fn every_page(
    f: &Fixture,
    text: &str,
    this: Option<&str>,
    size: usize,
    scan: bool,
) -> Result<Vec<query::Page>, Error> {
    let q = query::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
    let mut ctx = f.ctx();
    if let Some(t) = this {
        ctx = ctx.with_this(t);
    }
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let req = PageRequest {
            cursor: cursor.take(),
            size,
        };
        let page = if scan {
            query::run_scan(&q, &ctx, &req)?
        } else {
            query::run(&q, &ctx, &req)?
        };
        let next = page.next.clone();
        pages.push(page);
        match next {
            Some(c) if pages.len() < 100 => cursor = Some(c),
            _ => return Ok(pages),
        }
    }
}

/// Pages compared on everything but the cursor text, which each way of running encodes itself.
fn same_pages(a: &[query::Page], b: &[query::Page]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.rows == y.rows
                && x.total == y.total
                && x.groups == y.groups
                && x.next.is_some() == y.next.is_some()
        })
}

/// Queries beyond the table that probe the planner's edges: builtins, negation, flipped sides,
/// mixed sort directions, missing values, limits and groups.
fn edge_queries() -> Vec<(&'static str, Option<&'static str>)> {
    vec![
        ("from * sort created desc", None),
        ("from * sort updated", None),
        ("from * sort title desc", None),
        ("from * sort type, title", None),
        ("from * group by type", None),
        ("from * where title == \"Loose end\"", None),
        ("from * where title < \"G\" sort title", None),
        (
            "from * where updated > \"2026-10-04\" sort updated desc",
            None,
        ),
        ("from * where id == \"01JB8Z4K2MQ7V3X9T5R6W8Y2T3\"", None),
        ("from task where 3 < estimate", None),
        ("from task where estimate == 3.0", None),
        ("from task where !(done || estimate > 2)", None),
        (
            "from task where !(estimate >= 1 && has(tags, \"code\"))",
            None,
        ),
        (
            "from task where due < \"2026-10-15\" || has(topics, \"Life\")",
            None,
        ),
        ("from task where starts_at >= \"2026-10-12\"", None),
        ("from task where due == \"2026-10-12T00:00:00Z\"", None),
        ("from task where project != this", Some(P1)),
        (
            "from task where !under(status, \"Active\") sort status desc",
            None,
        ),
        ("from task where !has(tags, \"code\") sort title", None),
        ("from task sort status desc, due desc", None),
        ("from task sort score desc, due", None),
        ("from task sort project", None),
        ("from task sort title limit 3", None),
        ("from task group by project", None),
        ("from task sort due desc group by done", None),
        ("from task group by status limit 3", None),
        (
            "from task where status != \"Done\" sort due desc limit 3",
            None,
        ),
        (
            "from project where has(tasks, \"01JB8Z4K2MQ7V3X9T5R6W8Y2T4\")",
            None,
        ),
        (
            "from * where under(topics, \"Tech\") sort title desc limit 1",
            None,
        ),
        ("from task where done == true", None),
        ("from task where true", None),
        ("from task where !false && estimate * 1 > 0", None),
        ("from task where estimate > \"3\"", None),
        ("from task where tags == \"code\"", None),
        ("from task where !done sort updated desc", None),
        (
            "from task where has(tags, \"code\") sort created desc",
            None,
        ),
        ("from task where estimate > 0", None),
        ("from * where updated > \"2026-10-01\"", None),
        // contains() and empty() in SQL
        ("from * where contains(title, \"\")", None),
        ("from task where contains(status, \"PROG\")", None),
        (
            "from * where contains(topics, \"rust\") sort title desc",
            None,
        ),
        ("from task where !contains(tags, \"co\")", None),
        ("from * where contains(type, \"as\")", None),
        (
            "from task where contains(title, \"the\") || empty(due) sort due desc",
            None,
        ),
        ("from task where empty(due) && !empty(status)", None),
        ("from task where empty(title) || !empty(title)", None),
        ("from * where empty(type) || empty(id)", None),
        ("from task where !empty(type) && !empty(done)", None),
        ("from project where empty(tasks)", None),
        ("from project where !empty(tasks) sort title", None),
        ("from task where !empty(span) sort due", None),
        ("from task where !empty(estimate) group by status", None),
        (
            "from task where !empty(topics) && contains(title, \"t\")",
            None,
        ),
        ("from task where empty(created)", None),
        ("from task where contains(estimate, \"3\")", None),
        ("from task where contains(title, 3)", None),
        // Date range ends
        ("from task where span.end > \"2026-10-19\"", None),
        ("from task where span.start != \"2026-10-13\"", None),
        ("from task where !(span.start < \"2026-10-14\")", None),
        ("from task where \"2026-10-14\" > span.start", None),
        ("from task where span.foo == \"2026-10-13\"", None),
        // search with a filter, a sort, a group or a limit
        ("from task search \"the\"", None),
        ("from task where !done search \"the\"", None),
        ("from * where !done search \"the\" sort due desc", None),
        (
            "from * where contains(title, \"o\") search \"compost tomatoes\" sort updated desc",
            None,
        ),
        ("from book, article search \"event\" group by type", None),
        ("from * search \"event\" limit 1", None),
        ("from * where !empty(topics) search \"ev\" sort title", None),
        ("from task search \"im\"", None),
        ("from * search \"e\"", None),
        ("from * where done search \"nothing at all\"", None),
        // archived notes
        ("from task include archived", None),
        ("from task include archived sort archived, title", None),
        ("from task include archived group by archived", None),
        ("from task where archived != false", None),
        ("from task where archived == true && !done", None),
        ("from * where !archived sort updated desc", None),
        ("from task where empty(archived)", None),
        ("from task include archived select archived, title", None),
        ("from task where project.archived", None),
        (
            "from task include archived search \"idea\" sort title",
            None,
        ),
    ]
}

#[test]
fn the_sql_plan_and_the_note_by_note_scan_agree() {
    let f = fixture();
    let mut cases: Vec<(&str, Option<&str>)> = table()
        .into_iter()
        .map(|(t, this, _, _)| (t, this))
        .collect();
    cases.extend(edge_queries());
    let mut failures = Vec::new();
    for (text, this) in cases {
        for size in [0, 1, 2, 3] {
            let planned = every_page(&f, text, this, size, false);
            let scanned = every_page(&f, text, this, size, true);
            match (planned, scanned) {
                (Ok(a), Ok(b)) if same_pages(&a, &b) => {}
                (Err(a), Err(b)) if a.to_string() == b.to_string() => {}
                (a, b) => failures.push(format!(
                    "{text} (pages of {size})\n  planned {a:?}\n  scanned {b:?}"
                )),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn limits_cursors_and_groups_agree_over_many_notes() {
    let f = many(200);
    for text in [
        "from item sort n limit 120",
        "from item sort n desc limit 37",
        "from item where n >= 3 group by n limit 50",
        "from item where n != 4 sort n desc, title",
        "from item sort updated desc",
    ] {
        for size in [7, 50] {
            let a = every_page(&f, text, None, size, false).unwrap();
            let b = every_page(&f, text, None, size, true).unwrap();
            assert!(same_pages(&a, &b), "{text} (pages of {size})");
        }
    }
}

/// A note may hold the same value twice in a list, and several values below one term. It is
/// still one result.
#[test]
fn a_note_matching_several_list_items_is_one_result() {
    let mut files = vec![
        ("taxonomies/topic.wcl".to_owned(), TOPIC.to_owned()),
        (
            "types/item.wcl".to_owned(),
            with_template(
                "label = \"Item\"\n\nfield tags {\n  kind  = \"list\"\n  items = \"string\"\n}\n\n\
                 field topics {\n  kind     = \"taxonomy\"\n  taxonomy = \"topic\"\n  multiple = true\n}\n",
            ),
        ),
    ];
    let lists = [
        "tags = [\"a\", \"a\", \"b\"]\ntopics = [\"Tech\", \"Tech/Rust\"]\n",
        "tags = [\"b\"]\ntopics = [\"Life\", \"Tech/Rust\"]\n",
        "tags = [\"a\"]\ntopics = [\"Life\"]\n",
        "",
    ];
    for (i, fields) in lists.iter().enumerate() {
        let id = format!("01JB8Z4K2MQ7V3X9T5R6W8Y2M{i}");
        files.push((
            format!("notes/item/{id}.wcl"),
            note(&format!("Item {i}"), "2026-10-02T09:00:00Z", fields),
        ));
    }
    let f = build(&files, lists.len());
    for text in [
        "from item where has(tags, \"a\")",
        "from item where under(topics, \"Tech\") sort title desc",
        "from item where has(tags, \"a\") && under(topics, \"Tech\")",
        "from item where under(topics, \"Tech\") group by title",
        "from item where has(tags, \"b\") sort updated desc",
        "from item where !has(tags, \"a\")",
    ] {
        for size in [0, 1, 2] {
            let a = every_page(&f, text, None, size, false).unwrap();
            let b = every_page(&f, text, None, size, true).unwrap();
            assert!(
                same_pages(&a, &b),
                "{text} (pages of {size})\n{a:#?}\n{b:#?}"
            );
        }
    }
    let ids = ids(&f, "from item where has(tags, \"a\")", None).unwrap();
    assert_eq!(ids.len(), 2, "{ids:?}");
}

/// A `date_or_datetime` range holds dates on some notes and datetimes on others. Filters and
/// sorts read both forms on one time line, a date as the start of its day in UTC, and the SQL
/// plan agrees with the note-by-note scan (google-sync D15).
#[test]
fn a_date_or_datetime_range_filters_and_sorts_across_both_forms() {
    let mut files = vec![(
        "types/event.wcl".to_owned(),
        with_template(
            "label = \"Event\"\n\nfield when {\n  kind = \"date_range\"\n  of   = \"date_or_datetime\"\n}\n",
        ),
    )];
    // (title, when): two all-day ranges and two timed ones, interleaved in time.
    let events = [
        ("Offsite", "{ start: \"2026-10-14\", end: \"2026-10-15\" }"),
        (
            "Standup",
            "{ start: \"2026-10-14T09:00:00Z\", end: \"2026-10-14T09:15:00Z\" }",
        ),
        (
            "Late call",
            "{ start: \"2026-10-13T23:00:00Z\", end: \"2026-10-14T00:30:00Z\" }",
        ),
        ("Holiday", "{ start: \"2026-10-16\", end: \"2026-10-16\" }"),
    ];
    for (i, (title, when)) in events.iter().enumerate() {
        let id = format!("01JB8Z4K2MQ7V3X9T5R6W8Y2E{i}");
        files.push((
            format!("notes/event/{id}.wcl"),
            note(title, "2026-10-02T09:00:00Z", &format!("when = {when}\n")),
        ));
    }
    let f = build(&files, events.len());
    let titles = |text: &str| -> Vec<String> {
        let q = query::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        let page = query::run(&q, &f.ctx(), &PageRequest::default()).unwrap();
        let by_id: std::collections::HashMap<String, &str> = (0..events.len())
            .map(|i| (format!("01JB8Z4K2MQ7V3X9T5R6W8Y2E{i}"), events[i].0))
            .collect();
        page.rows.iter().map(|r| by_id[&r.id].to_owned()).collect()
    };
    let cases: [(&str, &[&str]); 7] = [
        // The date 2026-10-14 is 2026-10-14T00:00:00Z, after the late call starts.
        (
            "from event sort when.start",
            &["Late call", "Offsite", "Standup", "Holiday"],
        ),
        (
            "from event sort when.start desc",
            &["Holiday", "Standup", "Offsite", "Late call"],
        ),
        (
            "from event where when.start >= \"2026-10-14\" sort when.start",
            &["Offsite", "Standup", "Holiday"],
        ),
        (
            "from event where when.start < \"2026-10-14T09:00:00Z\" sort when.start",
            &["Late call", "Offsite"],
        ),
        (
            "from event where when.end > \"2026-10-14T00:00:00Z\" && when.start < \"2026-10-15\" sort when.start",
            &["Late call", "Offsite", "Standup"],
        ),
        (
            "from event where when.end >= \"2026-10-16\" sort title",
            &["Holiday"],
        ),
        (
            "from event where !empty(when) sort title",
            &["Holiday", "Late call", "Offsite", "Standup"],
        ),
    ];
    for (text, want) in cases {
        assert_eq!(titles(text), want, "{text}");
        for size in [0, 1, 2] {
            let a = every_page(&f, text, None, size, false).unwrap();
            let b = every_page(&f, text, None, size, true).unwrap();
            assert!(
                same_pages(&a, &b),
                "{text} (pages of {size})\n{a:#?}\n{b:#?}"
            );
        }
    }
}

/// The shapes the S6 benchmark times, against this fixture's fields, must run as SQL: run note
/// by note they read every note of their types (D33).
#[test]
fn the_benchmarked_query_shapes_run_as_sql() {
    let f = fixture();
    for text in [
        "from task where status == \"Active/In progress\" sort due asc",
        "from task where under(status, \"Active\") && due < \"2026-01-01\" sort due asc",
        "from task where status == \"Inbox\" && !done sort updated desc",
        "from task where project == \"01JB8Z4K2MQ7V3X9T5R6W8Y2P1\" sort due asc",
        "from book where author == \"Kleppmann\" sort title asc",
        "from book where has(topics, \"Tech\") sort updated desc",
        "from task where under(topics, \"Tech\") && score >= 4.0 sort score desc",
        "from * sort updated desc",
        "from task where estimate > 8 && !done group by status",
        "from task where project == this && due < today() + days(7)",
        // contains(), empty(), date range ends and search with a filter or a sort.
        "from task where contains(title, \"ship\")",
        "from task where contains(tags, \"od\") && !done sort due asc",
        "from * where contains(title, \"the\") && type == \"task\"",
        "from task where empty(estimate)",
        "from task where !empty(tags) sort title",
        "from task where empty(span) || empty(topics)",
        "from task where span.start <= \"2026-10-14\" && span.end >= \"2026-10-14\"",
        "from task search \"importer\"",
        "from * where !done search \"tomatoes\" sort updated desc",
        "from book, article where !empty(topics) search \"event\" group by type",
        // The archive rule and conditions on `archived` read the notes table's column.
        "from task where archived sort updated desc",
        "from * where archived || status == \"Inbox\"",
        "from task include archived where !done sort title",
        "from task where archived == true && estimate > 1",
    ] {
        let q = query::parse(text).unwrap();
        let ctx = f.ctx().with_this(P1);
        assert!(
            query::is_planned(&q, &ctx).unwrap(),
            "{text} runs note by note"
        );
    }
    // Paths through a reference, closures, empty() on a builtin date and contains() on a field
    // that is not text, which is an error, still run note by note.
    for text in [
        "from task where project.status == \"Inbox\"",
        "from task sort project.title",
        "from task where any(tags, |t| t == \"docs\")",
        "from task where empty(created)",
        "from task where contains(project.title, \"imp\")",
        "from task where contains(estimate, \"3\")",
        "from task where !done search \"importer\" sort project.title",
    ] {
        let q = query::parse(text).unwrap();
        assert!(
            !query::is_planned(&q, &f.ctx()).unwrap(),
            "{text} runs as SQL"
        );
    }
}

/// Items whose titles repeat `apple` a varying number of times, so scores both tie and differ.
fn searchable(n: usize) -> Fixture {
    let mut files = vec![(
        "types/item.wcl".to_owned(),
        with_template("label = \"Item\"\n\nfield n {\n  kind = \"int\"\n}\n"),
    )];
    for i in 0..n {
        let id = format!("01JB8Z4K2MQ7V3X9T5R6W8{i:04}");
        let title = if i % 3 == 2 {
            "pear".to_owned()
        } else {
            format!(
                "{} {}",
                vec!["apple"; i % 4 + 1].join(" "),
                vec!["pear"; i % 5].join(" ")
            )
        };
        let text = format!(
            "title = \"{title}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\nn = {}\n",
            i % 7
        );
        files.push((format!("notes/item/{id}.wcl"), text));
    }
    build(&files, n)
}

#[test]
fn a_plain_search_pages_in_the_order_the_general_runner_gives() {
    let f = searchable(300);
    // `from item` takes the general path, which reads every note and sorts them itself.
    let q = query::parse("from item search \"apple\"").unwrap();
    let whole = query::run(&q, &f.ctx(), &PageRequest::default()).unwrap();
    assert_eq!(whole.total, 200);
    let expected: Vec<String> = whole.rows.iter().map(|r| r.id.clone()).collect();

    for size in [1, 7, 50, 1000] {
        let (seen, pages) = page_through(&f, "from * search \"apple\"", size);
        assert_eq!(seen, expected, "pages of {size}");
        assert_eq!(pages, 200usize.div_ceil(size), "pages of {size}");
    }
    let q = query::parse("from * search \"apple\" select n").unwrap();
    let first = query::run(
        &q,
        &f.ctx(),
        &PageRequest {
            cursor: None,
            size: 10,
        },
    )
    .unwrap();
    assert_eq!(first.total, 200);
    assert_eq!(first.rows.len(), 10);
    assert_eq!(first.rows[0].id, expected[0]);
    assert_eq!(first.rows[0].fields.keys().collect::<Vec<_>>(), ["n"]);
    // The general path's cursor and the plain search's cursor are the same shape.
    let q = query::parse("from * search \"apple\"").unwrap();
    let general = query::parse("from item search \"apple\"").unwrap();
    let g = query::run(
        &general,
        &f.ctx(),
        &PageRequest {
            cursor: None,
            size: 10,
        },
    )
    .unwrap();
    let rest = query::run(
        &q,
        &f.ctx(),
        &PageRequest {
            cursor: g.next,
            size: 1000,
        },
    )
    .unwrap();
    assert_eq!(
        rest.rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        expected[10..]
    );
    // A cursor of another query shape is refused.
    let sorted = query::parse("from item sort n").unwrap();
    let s = query::run(
        &sorted,
        &f.ctx(),
        &PageRequest {
            cursor: None,
            size: 10,
        },
    )
    .unwrap();
    let r = query::run(
        &q,
        &f.ctx(),
        &PageRequest {
            cursor: s.next,
            size: 10,
        },
    );
    assert!(matches!(r, Err(Error::BadCursor)), "{r:?}");
}

/// Text fields whose values need Unicode case folding and white space rules: a Kelvin sign
/// lowercases to `k`, `İ` to two characters, and an em space is blank.
fn unicode_fixture() -> Fixture {
    let ty = "label = \"Item\"\n\nfield label {\n  kind = \"string\"\n}\n\n\
              field aliases {\n  kind  = \"list\"\n  items = \"string\"\n}\n\n\
              field size {\n  kind    = \"select\"\n  options = [\"Small\", \"LARGE\"]\n}\n\n\
              field site {\n  kind = \"url\"\n}\n\n\
              field count {\n  kind = \"int\"\n}\n";
    let mut files = vec![("types/item.wcl".to_owned(), with_template(ty))];
    let items = [
        "label = \"\u{212A}elvin\"\naliases = [\"Straße\", \"ΣΑΣ\"]\nsize = \"LARGE\"\n",
        "label = \"İstanbul\"\naliases = []\ncount = 0\n",
        "label = \"\u{2003}\"\naliases = [\"  \"]\nsite = \"https://example.com/A\"\n",
        "label = \"  \"\nsize = \"Small\"\n",
        "label = \"kelvin\"\naliases = [\"\"]\n",
        "",
    ];
    for (i, fields) in items.iter().enumerate() {
        let id = format!("01JB8Z4K2MQ7V3X9T5R6W8Y2V{i}");
        files.push((
            format!("notes/item/{id}.wcl"),
            note(&format!("Item {i}"), "2026-10-02T09:00:00Z", fields),
        ));
    }
    build(&files, items.len())
}

#[test]
fn contains_and_empty_in_sql_return_what_the_evaluator_returns() {
    let f = unicode_fixture();
    let u = |i: usize| format!("01JB8Z4K2MQ7V3X9T5R6W8Y2V{i}");
    let cases = [
        "from item where contains(label, \"k\")",
        "from item where contains(label, \"KELVIN\")",
        "from item where contains(label, \"i̇st\")",
        "from item where contains(label, \"İST\")",
        "from item where contains(label, \"\")",
        "from item where contains(aliases, \"STRASSE\")",
        "from item where contains(aliases, \"straße\")",
        "from item where contains(aliases, \"σας\")",
        "from item where contains(aliases, \"\")",
        "from item where !contains(aliases, \"a\")",
        "from item where contains(size, \"arg\")",
        "from item where contains(site, \"/a\")",
        "from item where contains(title, \"ITEM 3\")",
        "from item where empty(label)",
        "from item where !empty(label) sort label",
        "from item where empty(aliases)",
        "from item where !empty(aliases)",
        "from item where empty(size) || empty(site)",
        "from item where empty(count)",
        "from item where !empty(count) || contains(label, \"\u{212A}\")",
    ];
    let mut failures = Vec::new();
    for text in cases {
        let q = query::parse(text).unwrap();
        assert!(query::is_planned(&q, &f.ctx()).unwrap(), "{text}");
        for size in [0, 1, 2] {
            let a = every_page(&f, text, None, size, false);
            let b = every_page(&f, text, None, size, true);
            match (a, b) {
                (Ok(a), Ok(b)) if same_pages(&a, &b) => {}
                (a, b) => failures.push(format!(
                    "{text} (pages of {size})\n  sql  {a:?}\n  scan {b:?}"
                )),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
    // The Kelvin sign is a `k` once lowercased, which SQLite's own LIKE and lower() miss.
    assert_eq!(
        ids(&f, "from item where contains(label, \"kel\")", None).unwrap(),
        vec![u(0), u(4)]
    );
    // An em space is blank; an empty list is empty, and a list of a blank item is not.
    assert_eq!(
        ids(&f, "from item where empty(label)", None).unwrap(),
        vec![u(2), u(3), u(5)]
    );
    assert_eq!(
        ids(&f, "from item where empty(aliases)", None).unwrap(),
        vec![u(1), u(3), u(5)]
    );
}

/// Every page of a search with a filter and a sort, read by id and read through the driving
/// condition, holds each match once, in the order the evaluator gives.
#[test]
fn search_with_a_filter_and_a_sort_pages_each_match_exactly_once() {
    let f = searchable(300);
    let cases = [
        "from item where n >= 2 search \"apple\" sort n desc",
        "from item where n >= 2 search \"apple\"",
        "from item where n != 3 search \"apple\"",
        "from item where n == 1 search \"apple pear\" sort title",
        "from item where n < 5 search \"apple\" sort n group by n limit 40",
        "from item where n > 0 && contains(title, \"pear\") search \"app\" sort updated desc",
        "from item search \"apple\" sort n",
    ];
    for drive in [usize::MAX, 0] {
        query::set_hits_drive(drive);
        for text in cases {
            let q = query::parse(text).unwrap();
            assert!(query::is_planned(&q, &f.ctx()).unwrap(), "{text}");
            let whole = query::run(&q, &f.ctx(), &PageRequest::default()).unwrap();
            assert!(whole.total > 0, "{text}");
            for size in [3, 7, 50] {
                let a = every_page(&f, text, None, size, false).unwrap();
                let b = every_page(&f, text, None, size, true).unwrap();
                assert!(
                    same_pages(&a, &b),
                    "{text} (pages of {size}, drive {drive})"
                );
                let seen: Vec<String> = a
                    .iter()
                    .flat_map(|p| p.rows.iter().map(|r| r.id.clone()))
                    .collect();
                let unique: HashSet<&String> = seen.iter().collect();
                assert_eq!(seen.len(), whole.total, "{text} (pages of {size})");
                assert_eq!(unique.len(), seen.len(), "{text}: a note came back twice");
                let all: Vec<String> = whole.rows.iter().map(|r| r.id.clone()).collect();
                assert_eq!(seen, all, "{text} (pages of {size})");
            }
        }
    }
    query::set_hits_drive(50_000);
}
