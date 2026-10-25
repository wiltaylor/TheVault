//! The index against a fixture vault on disk: a rebuild, the same notes applied one commit at a
//! time, invalid files, full-text search, and a rebuild after deleting both indexes.

use std::collections::HashMap;
use std::path::Path;

use index::{Index, Update};
use schema::{Doc, Schema};
use store::Vault;

const PROJECT: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2Z0";
const TASK_A: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const TASK_B: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const TASK_C: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";
const SCRATCH: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZD";
const ASSET: &str = "01JB9A0C3D4E5F6G7H8J9K0M1N";

const STATUS: &str = r#"label = "Status"

term "Inbox"

term "Active" {
  term "In progress"

  term "Blocked"
}

term "Done"
"#;

const PROJECT_TYPE: &str = r#"label = "Project"

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

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TASK_TYPE: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

field score {
  kind = "float"
}

field done {
  kind = "bool"
}

field due {
  kind = "date"
}

field starts_at {
  kind = "datetime"
}

field status {
  kind     = "taxonomy"
  taxonomy = "status"
}

field project {
  kind      = "reference"
  target    = "project"
  relation  = "two_way"
  inverse   = "tasks"
  on_delete = "cascade"
}

field cover {
  kind = "asset"
}

field place {
  kind = "location"
  map  = "geo"
}

field aliases {
  kind  = "list"
  items = "string"
}

field extra {
  kind   = "dict"
  values = "string"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn project_note() -> String {
    format!(
        r#"title   = "Importer"
created = "2026-10-01T09:00:00Z"
updated = "2026-10-02T09:00:00Z"
status  = "Active"
tasks   = ["{TASK_A}", "{TASK_B}", "{TASK_C}"]

content body {{
  paragraph {{
    text = "The importer reads exports from other apps."
  }}
}}
"#
    )
}

fn task_a() -> String {
    format!(
        r#"title     = "Ship the importer"
created   = "2026-10-12T09:15:00Z"
updated   = "2026-10-12T10:42:07Z"
estimate  = 3
score     = 4
done      = false
due       = "2026-11-01"
starts_at = "2026-10-20T09:30:00+11:00"
status    = "Active/In progress"
project   = "{PROJECT}"
cover     = "{ASSET}"
place     = {{ lat: -37.8136, lng: 144.9631 }}
aliases   = ["importer", "capacities import"]

dict extra {{
  owner = "Wil"
}}

content body {{
  heading {{
    level = 2
    text  = "Plan"
  }}

  paragraph {{
    text = "Ride the **zeppelin** to [[{PROJECT}|the project]] and back to [[{TASK_B}]]."
  }}

  list {{
    style = "todo"

    item {{
      text = "Pack the quokka"

      item {{
        text = "Nested mention of [[{TASK_C}]]"
      }}
    }}
  }}

  code {{
    source = "fn wombat() {{}}"
  }}

  canvas {{
    viewport {{
      x    = 0.0
      y    = 0.0
      zoom = 1.0
    }}

    node idea {{
      kind   = "text"
      x      = 0.0
      y      = 0.0
      width  = 100.0
      height = 100.0
      text   = "Canvas thought about [[{PROJECT}]]"
    }}
  }}
}}
"#
    )
}

fn task_b() -> String {
    format!(
        r#"title     = "Write the docs"
created   = "2026-10-12T09:15:00Z"
updated   = "2026-10-12T10:42:07Z"
done      = true
due       = "2026-10-15"
starts_at = "2026-10-19T23:00:00Z"
status    = "Done"
project   = "{PROJECT}"
"#
    )
}

fn task_c() -> String {
    format!(
        r#"title   = "Fix the blocker"
created = "2026-10-12T09:15:00Z"
updated = "2026-10-12T10:42:07Z"
done    = false
due     = "2026-10-20"
status  = "Active/Blocked"
project = "{PROJECT}"
"#
    )
}

/// `(type, id, source)` of every fixture note.
fn notes() -> Vec<(&'static str, &'static str, String)> {
    vec![
        ("project", PROJECT, project_note()),
        ("task", TASK_A, task_a()),
        ("task", TASK_B, task_b()),
        ("task", TASK_C, task_c()),
    ]
}

fn write(root: &Path, path: &str, text: &str) {
    let full = root.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn fixture() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    let root = vault.root().to_owned();
    write(&root, "taxonomies/status.wcl", STATUS);
    write(&root, "types/project.wcl", PROJECT_TYPE);
    write(&root, "types/task.wcl", TASK_TYPE);
    std::fs::create_dir_all(root.join("assets").join(ASSET)).unwrap();
    for (ty, id, text) in notes() {
        write(&root, &format!("notes/{ty}/{id}.wcl"), &text);
    }
    (dir, vault)
}

fn put(schema: &Schema, ty: &str, id: &str, text: &str) -> Update {
    let lookup: HashMap<String, String> = notes()
        .into_iter()
        .map(|(t, i, _)| (i.to_owned(), t.to_owned()))
        .chain([(SCRATCH.to_owned(), "task".to_owned())])
        .collect();
    let file = format!("notes/{ty}/{id}.wcl");
    let report = schema
        .validate_note_source(ty, text, &file, &lookup)
        .unwrap_or_else(|e| panic!("{e}"));
    Update::Put {
        id: id.to_owned(),
        type_name: ty.to_owned(),
        doc: Doc::parse(text, &file).unwrap(),
        report,
    }
}

/// Query results the `query` component will build on: one per table and value kind.
fn query_results(index: &Index) -> Vec<Vec<String>> {
    let queries = [
        // under(status, "Active")
        "SELECT v.note FROM field_values v JOIN term_ancestors a
           ON a.taxonomy = 'status' AND a.path = v.str
         WHERE v.field = 'status' AND a.ancestor = 'Active' ORDER BY v.note",
        // Backlinks to the project, from fields and mentions.
        "SELECT src || ' ' || via || ' ' || field FROM links WHERE dst = '01JB8Z4K2MQ7V3X9T5R6W8Y2Z0'
         ORDER BY 1",
        // due < 2026-10-21, as a date comparison.
        "SELECT note FROM field_values WHERE field = 'due' AND str < '2026-10-21' ORDER BY note",
        // starts_at in time order across offsets.
        "SELECT note FROM field_values WHERE field = 'starts_at' ORDER BY num",
        // !done
        "SELECT note FROM field_values WHERE field = 'done' AND num = 0 ORDER BY note",
        "SELECT id || ' ' || type || ' ' || title FROM notes ORDER BY id",
    ];
    let mut out: Vec<Vec<String>> = queries
        .iter()
        .map(|q| {
            let mut stmt = index.connection().prepare(q).unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        })
        .collect();
    for text in ["zeppelin", "importer", "quokka", "docs"] {
        out.push(
            index
                .search(text, None)
                .unwrap()
                .into_iter()
                .map(|h| h.id)
                .collect(),
        );
    }
    out
}

#[test]
fn a_rebuild_equals_the_same_notes_indexed_incrementally() {
    let (_dir, vault) = fixture();
    let mut rebuilt = Index::open(&vault).unwrap();
    let report = rebuilt.rebuild(&vault).unwrap();
    assert_eq!(report.failures, vec![], "{report:?}");
    assert_eq!(report.indexed, 4);

    let other = tempfile::tempdir().unwrap();
    let mut incremental =
        Index::open_at(&other.path().join("index.db"), &other.path().join("search")).unwrap();
    let schema = Schema::load(&vault).unwrap();
    incremental.set_schema(&schema).unwrap();
    // Notes arrive one commit at a time in another order, one is edited and put back, and a note
    // that is later deleted comes and goes.
    let scratch = format!(
        "title = \"Scratch\"\ncreated = \"2026-10-12T09:15:00Z\"\nupdated = \"2026-10-12T09:15:00Z\"\n\ncontent body {{\n  paragraph {{\n    text = \"zeppelin [[{PROJECT}]]\"\n  }}\n}}\n"
    );
    incremental
        .update(&schema, &[put(&schema, "task", SCRATCH, &scratch)])
        .unwrap();
    for (ty, id, text) in notes().into_iter().rev() {
        incremental
            .update(&schema, &[put(&schema, ty, id, &text)])
            .unwrap();
    }
    let edited = task_b().replace("Write the docs", "Write the zeppelin docs");
    incremental
        .update(&schema, &[put(&schema, "task", TASK_B, &edited)])
        .unwrap();
    incremental
        .update(
            &schema,
            &[
                put(&schema, "task", TASK_B, &task_b()),
                Update::Delete {
                    id: SCRATCH.to_owned(),
                },
            ],
        )
        .unwrap();

    let dump = rebuilt.dump().unwrap();
    assert_eq!(dump, incremental.dump().unwrap());
    assert_eq!(query_results(&rebuilt), query_results(&incremental));

    // The dump holds what the query language needs.
    for expected in [
        format!("\"{TASK_A}\"\t\"field\"\t\"project\"\t0\t\"{PROJECT}\""),
        format!("\"{TASK_A}\"\t\"mention\"\t\"body\"\t0\t\"{PROJECT}\""),
        format!("\"{TASK_A}\"\t\"mention\"\t\"body\"\t1\t\"{TASK_B}\""),
        format!("\"{TASK_A}\"\t\"mention\"\t\"body\"\t2\t\"{TASK_C}\""),
        format!("\"{TASK_A}\"\t\"mention\"\t\"body\"\t3\t\"{PROJECT}\""),
        format!("\"{TASK_A}\"\t\"place\"\t0\t\"lat\"\t\"location\"\tNULL\t-37.8136"),
        format!("\"{TASK_A}\"\t\"extra\"\t0\t\"owner\"\t\"string\"\t\"Wil\"\tNULL"),
        format!("\"{TASK_A}\"\t\"score\"\t0\t\"\"\t\"float\"\tNULL\t4.0"),
        format!("\"{TASK_A}\"\t\"estimate\"\t0\t\"\"\t\"int\"\tNULL\t3"),
        format!("\"{TASK_A}\"\t\"aliases\"\t1\t\"\"\t\"string\"\t\"capacities import\"\tNULL"),
        "\"status\"\t\"Active\"\t\"Active/Blocked\"".to_owned(),
        "\"status\"\t\"Active/In progress\"\t\"In progress\"\t\"Active\"\t1\t2".to_owned(),
    ] {
        assert!(dump.contains(&expected), "missing {expected}\n{dump}");
    }
}

#[test]
fn an_invalid_file_is_reported_by_path_and_every_other_file_is_indexed() {
    let (_dir, vault) = fixture();
    let root = vault.root().to_owned();
    let expression = "notes/task/01JB8Z4K2MQ7V3X9T5R6W8Y2ZE.wcl";
    write(
        &root,
        expression,
        "title = \"Sum\"\ncreated = \"2026-10-12T09:15:00Z\"\nupdated = \"2026-10-12T09:15:00Z\"\nestimate = 1 + 2\n",
    );
    let untitled = "notes/task/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl";
    write(
        &root,
        untitled,
        "created = \"2026-10-12T09:15:00Z\"\nupdated = \"2026-10-12T09:15:00Z\"\ndue = \"2026-13-01\"\n",
    );
    let misnamed = "notes/task/not-a-ulid.wcl";
    write(&root, misnamed, &task_c());
    let unknown_type = "notes/ghost/01JB8Z4K2MQ7V3X9T5R6W8Y2ZG.wcl";
    write(&root, unknown_type, &task_c());
    let bad_taxonomy = "taxonomies/broken.wcl";
    write(&root, bad_taxonomy, "description = \"no label\"\n");

    let mut index = Index::open(&vault).unwrap();
    let report = index.rebuild(&vault).unwrap();

    let by_path: HashMap<&str, &str> = report
        .failures
        .iter()
        .map(|f| (f.path.as_str(), f.diagnostic.as_str()))
        .collect();
    assert_eq!(by_path.len(), 5, "{report:#?}");
    assert!(by_path[expression].contains("estimate"), "{report:#?}");
    assert!(
        by_path[untitled].contains("title: is required"),
        "{report:#?}"
    );
    assert!(by_path[untitled].contains("due"), "{report:#?}");
    assert!(by_path[misnamed].contains("not a note file"), "{report:#?}");
    assert!(
        by_path[unknown_type].contains("no type \"ghost\""),
        "{report:#?}"
    );
    assert!(by_path[bad_taxonomy].contains("label"), "{report:#?}");
    assert_eq!(index.failures().unwrap(), report.failures);

    assert_eq!(report.indexed, 4);
    let ids: Vec<String> = index
        .connection()
        .prepare("SELECT id FROM notes ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, vec![PROJECT, TASK_A, TASK_B, TASK_C]);
}

#[test]
fn a_failing_type_is_reported_with_the_types_and_notes_that_depend_on_it() {
    let (_dir, vault) = fixture();
    // The project type names a taxonomy that does not exist, so the schema check refuses it.
    write(
        vault.root(),
        "types/project.wcl",
        &PROJECT_TYPE.replace("taxonomy = \"status\"", "taxonomy = \"missing\""),
    );
    let mut index = Index::open(&vault).unwrap();
    let report = index.rebuild(&vault).unwrap();
    let paths: Vec<&str> = report.failures.iter().map(|f| f.path.as_str()).collect();
    // The task type's `project` field targets the dropped type, so it fails in turn, and every
    // note with it. Each failure is still reported by path.
    let project_note = format!("notes/project/{PROJECT}.wcl");
    let task_a = format!("notes/task/{TASK_A}.wcl");
    for expected in [
        "types/project.wcl",
        "types/task.wcl",
        &project_note,
        &task_a,
    ] {
        assert!(paths.contains(&expected), "{expected}: {report:#?}");
    }
    assert_eq!(report.indexed, 0);
    assert_eq!(report.failures.len(), 6, "{report:#?}");
}

#[test]
fn a_full_text_search_finds_a_word_in_a_paragraph_block() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();

    let ids = |index: &Index, q: &str| -> Vec<String> {
        index
            .search(q, None)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect()
    };
    assert_eq!(ids(&index, "zeppelin"), vec![TASK_A]);
    assert_eq!(ids(&index, "Zeppelin"), vec![TASK_A]);
    // A nested list item, a code block, an old canvas text node (read as a card) and a dict value.
    assert_eq!(ids(&index, "quokka"), vec![TASK_A]);
    assert_eq!(ids(&index, "wombat"), vec![TASK_A]);
    assert_eq!(ids(&index, "canvas thought"), vec![TASK_A]);
    assert_eq!(ids(&index, "Wil"), vec![TASK_A]);
    // Titles and the project's own paragraph.
    assert_eq!(ids(&index, "docs"), vec![TASK_B]);
    assert_eq!(ids(&index, "exports"), vec![PROJECT]);
    assert!(ids(&index, "nowhere").is_empty());
    assert_eq!(index.search("importer", Some(1)).unwrap().len(), 1);
    // Search syntax a user might type does not fail.
    assert!(index.search("\"unclosed (", None).is_ok());

    // An update replaces the note's text, and a delete removes it.
    let schema = Schema::load(&vault).unwrap();
    let text = task_a().replace("zeppelin", "airship");
    index
        .update(&schema, &[put(&schema, "task", TASK_A, &text)])
        .unwrap();
    assert!(ids(&index, "zeppelin").is_empty());
    assert_eq!(ids(&index, "airship"), vec![TASK_A]);
    index
        .update(
            &schema,
            &[Update::Delete {
                id: TASK_A.to_owned(),
            }],
        )
        .unwrap();
    assert!(ids(&index, "airship").is_empty());
    let left: i64 = index
        .connection()
        .query_row(
            "SELECT (SELECT count(*) FROM field_values WHERE note = ?1) + (SELECT count(*) FROM links WHERE src = ?1)",
            [TASK_A],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn deleting_both_indexes_and_rebuilding_restores_identical_query_results() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let before = query_results(&index);
    let dump = index.dump().unwrap();
    assert_eq!(before[0], vec![PROJECT, TASK_A, TASK_C], "under Active");
    assert_eq!(before[3], vec![TASK_A, TASK_B], "starts_at in time order");
    assert_eq!(before[6], vec![TASK_A], "search zeppelin");
    drop(index);

    let server = vault.server_dir();
    for name in ["index.db", "index.db-wal", "index.db-shm"] {
        let _ = std::fs::remove_file(server.join(name));
    }
    std::fs::remove_dir_all(server.join("search")).unwrap();
    assert!(!server.join("index.db").exists());

    let mut index = Index::open(&vault).unwrap();
    assert!(index.search("zeppelin", None).unwrap().is_empty());
    index.rebuild(&vault).unwrap();
    assert_eq!(query_results(&index), before);
    assert_eq!(index.dump().unwrap(), dump);
}

#[test]
fn a_second_rebuild_replaces_the_first() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let first = index.dump().unwrap();
    std::fs::remove_file(vault.root().join(format!("notes/task/{TASK_C}.wcl"))).unwrap();
    // The project still lists the deleted task, so it now fails validation.
    let report = index.rebuild(&vault).unwrap();
    assert_eq!(report.indexed, 2, "{report:#?}");
    assert_eq!(report.failures.len(), 1, "{report:#?}");
    assert_eq!(
        report.failures[0].path,
        format!("notes/project/{PROJECT}.wcl")
    );
    assert_ne!(index.dump().unwrap(), first);
    assert!(index.search("blocker", None).unwrap().is_empty());
}

#[test]
fn a_rebuild_makes_the_index_current_with_the_vault_generation() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    assert_eq!(index.generation().unwrap(), None, "a new index is behind");
    assert!(!index.is_current(&vault).unwrap());
    index.rebuild(&vault).unwrap();
    assert_eq!(index.generation().unwrap(), Some(0));
    assert!(index.is_current(&vault).unwrap());
    drop(index);
    assert!(Index::open(&vault).unwrap().is_current(&vault).unwrap());
}

#[test]
fn the_index_advances_one_commit_at_a_time_and_a_missed_commit_leaves_it_behind() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let schema = Schema::load(&vault).unwrap();

    let text = notes()[1].2.replace("Importer", "Importer two");
    let path = format!("notes/task/{TASK_A}.wcl");
    let g = vault
        .commit(vec![store::Change::write(&path, text.as_str())])
        .unwrap();
    assert!(!index.is_current(&vault).unwrap(), "committed, not applied");
    index
        .update(&schema, &[put(&schema, "task", TASK_A, &text)])
        .unwrap();
    index.advance(g).unwrap();
    assert!(index.is_current(&vault).unwrap());

    // A commit the index never hears of, then one it does: it stays behind.
    vault
        .commit(vec![store::Change::write(&path, notes()[1].2.as_str())])
        .unwrap();
    let g = vault.commit(vec![]).unwrap();
    index.advance(g).unwrap();
    assert_eq!(index.generation().unwrap(), Some(g - 2));
    assert!(!index.is_current(&vault).unwrap());

    index.rebuild(&vault).unwrap();
    assert!(index.is_current(&vault).unwrap());
}

#[test]
fn an_index_whose_full_text_half_is_gone_is_behind() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    drop(index);
    std::fs::remove_dir_all(vault.server_dir().join("search")).unwrap();
    let index = Index::open(&vault).unwrap();
    assert!(!index.is_current(&vault).unwrap());
}

#[test]
fn text_and_mentions_inside_columns_are_indexed() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let schema = Schema::load(&vault).unwrap();
    let text = format!(
        r#"title   = "Laid out"
created = "2026-10-01T09:00:00Z"
updated = "2026-10-01T09:00:00Z"

content body {{
  columns {{
    column {{
      width = 2
      paragraph {{
        text = "A narwhal beside [[{PROJECT}|the project]]."
      }}
    }}

    column {{
      width = 1
      list {{
        style = "bullet"

        item {{
          text = "An axolotl, see [[{TASK_B}]]"
        }}
      }}

      code {{
        source = "let pangolin = 1;"
      }}
    }}
  }}
}}
"#
    );
    index
        .update(&schema, &[put(&schema, "task", SCRATCH, &text)])
        .unwrap();
    for word in ["narwhal", "axolotl", "pangolin"] {
        let ids: Vec<String> = index
            .search(word, None)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids, vec![SCRATCH], "{word}");
    }
    let mut stmt = index
        .connection()
        .prepare("SELECT via || ' ' || field || ' ' || dst FROM links WHERE src = ?1 ORDER BY pos")
        .unwrap();
    let links: Vec<String> = stmt
        .query_map([SCRATCH], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        links,
        vec![
            format!("mention body {PROJECT}"),
            format!("mention body {TASK_B}")
        ]
    );
}

#[test]
fn text_and_mentions_inside_toggles_and_admonitions_are_indexed() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let schema = Schema::load(&vault).unwrap();
    let text = format!(
        r#"title   = "Folded"
created = "2026-10-01T09:00:00Z"
updated = "2026-10-01T09:00:00Z"

content body {{
  toggle {{
    level = 2
    text  = "A bilby summary for [[{PROJECT}|the project]]"

    paragraph {{
      text = "A narwhal hidden inside"
    }}
  }}

  admonition {{
    kind  = "warning"
    title = "Beware the echidna, see [[{TASK_B}]]"

    toggle {{
      text = "Nested"

      code {{
        source = "let pangolin = 1;"
      }}
    }}
  }}
}}
"#
    );
    index
        .update(&schema, &[put(&schema, "task", SCRATCH, &text)])
        .unwrap();
    for word in ["bilby", "narwhal", "echidna", "pangolin"] {
        let ids: Vec<String> = index
            .search(word, None)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids, vec![SCRATCH], "{word}");
    }
    let mut stmt = index
        .connection()
        .prepare("SELECT via || ' ' || field || ' ' || dst FROM links WHERE src = ?1 ORDER BY pos")
        .unwrap();
    let links: Vec<String> = stmt
        .query_map([SCRATCH], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // The mention in the toggle's summary and the one in the admonition's title are backlinks.
    assert_eq!(
        links,
        vec![
            format!("mention body {PROJECT}"),
            format!("mention body {TASK_B}")
        ]
    );
}

#[test]
fn a_card_block_links_to_its_note_as_a_mention_does() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let schema = Schema::load(&vault).unwrap();
    let text = format!(
        r#"title   = "Carded"
created = "2026-10-01T09:00:00Z"
updated = "2026-10-01T09:00:00Z"

content body {{
  card {{
    note = "{PROJECT}"
  }}

  paragraph {{
    text = "Then [[{TASK_C}]]"
  }}

  toggle {{
    text = "More"

    card {{
      note = "{TASK_B}"
    }}
  }}
}}
"#
    );
    index
        .update(&schema, &[put(&schema, "task", SCRATCH, &text)])
        .unwrap();
    let mut stmt = index
        .connection()
        .prepare("SELECT via || ' ' || field || ' ' || dst FROM links WHERE src = ?1 ORDER BY pos")
        .unwrap();
    let links: Vec<String> = stmt
        .query_map([SCRATCH], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // In content order, nested cards too, so each target's backlinks list this note.
    assert_eq!(
        links,
        vec![
            format!("mention body {PROJECT}"),
            format!("mention body {TASK_C}"),
            format!("mention body {TASK_B}")
        ]
    );
}

#[test]
fn placeholder_links_are_searched_but_link_nowhere() {
    let (_dir, vault) = fixture();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let schema = Schema::load(&vault).unwrap();
    let text = format!(
        r#"title   = "Plans"
created = "2026-10-01T09:00:00Z"
updated = "2026-10-01T09:00:00Z"

content body {{
  paragraph {{
    text = "Next: [[+task|Fix the capybara pump]], then [[+project|[[{PROJECT}]] and [[{TASK_B}]]"
  }}
}}
"#
    );
    index
        .update(&schema, &[put(&schema, "task", SCRATCH, &text)])
        .unwrap();
    let ids: Vec<String> = index
        .search("capybara", None)
        .unwrap()
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(ids, vec![SCRATCH]);
    let mut stmt = index
        .connection()
        .prepare("SELECT via || ' ' || dst FROM links WHERE src = ?1 ORDER BY pos")
        .unwrap();
    let links: Vec<String> = stmt
        .query_map([SCRATCH], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // The second placeholder's title is `[[` and the project's id; only the real mention links.
    assert_eq!(links, vec![format!("mention {TASK_B}")]);
}

#[test]
fn a_placeholder_naming_no_type_is_invalid() {
    let (_dir, vault) = fixture();
    let schema = Schema::load(&vault).unwrap();
    let text = r#"title = "Plans"

content body {
  list {
    style = "bullet"

    item {
      text = "Do [[+chore|Sweep]] soon"
    }
  }
}
"#;
    let err = schema
        .validate_note_source("task", text, "notes/task/x.wcl", &HashMap::new())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("> list#1 > item#1.text")
            && err.contains(
                "the placeholder link titled \"Sweep\" names type \"chore\", which does not exist"
            ),
        "{err}"
    );
    let ok = text.replace("+chore", "+task");
    schema
        .validate_note_source("task", &ok, "notes/task/x.wcl", &HashMap::new())
        .unwrap();
}
