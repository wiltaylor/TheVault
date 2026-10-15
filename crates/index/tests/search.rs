//! Full-text paging: top-k by score then id across several segments, cursors that land inside a
//! run of tied scores, and an index written before `id` was a fast field.

use std::collections::{BTreeSet, HashMap};

use index::{Hit, Index, Update};
use schema::{Doc, Schema};
use store::Vault;

const ITEM_TYPE: &str = r#"label = "Item"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn id(i: usize) -> String {
    format!("01JB8Z4K2MQ7V3X9T5R6W8{i:04}")
}

/// Titles that repeat, so many notes share a score, and vary, so scores differ. Every third note
/// does not match `apple`.
fn title(i: usize) -> String {
    if i % 3 == 2 {
        return format!("pear {}", i % 4);
    }
    let apples = vec!["apple"; i % 4 + 1].join(" ");
    let filler = vec!["pear"; i % 5].join(" ");
    format!("{apples} {filler}")
}

fn vault() -> (tempfile::TempDir, Vault, Schema) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    std::fs::create_dir_all(vault.root().join("types")).unwrap();
    std::fs::write(vault.root().join("types/item.wcl"), ITEM_TYPE).unwrap();
    let schema = Schema::load(&vault).unwrap();
    (dir, vault, schema)
}

fn put(schema: &Schema, i: usize) -> Update {
    let text = format!(
        "title   = \"{}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\n",
        title(i)
    );
    let file = format!("notes/item/{}.wcl", id(i));
    let report = schema
        .validate_note_source("item", &text, &file, &HashMap::new())
        .unwrap_or_else(|e| panic!("{e}"));
    Update::Put {
        id: id(i),
        type_name: "item".to_owned(),
        doc: Doc::parse(&text, &file).unwrap(),
        report,
    }
}

/// 120 items written in six commits, so the index has several segments, then two deleted.
fn index() -> (tempfile::TempDir, Index, BTreeSet<String>) {
    let (dir, vault, schema) = vault();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    for chunk in (0..120).collect::<Vec<_>>().chunks(20) {
        let updates: Vec<Update> = chunk.iter().map(|i| put(&schema, *i)).collect();
        index.update(&schema, &updates).unwrap();
    }
    index
        .update(
            &schema,
            &[Update::Delete { id: id(0) }, Update::Delete { id: id(4) }],
        )
        .unwrap();
    let matching = (0..120)
        .filter(|i| i % 3 != 2 && *i != 0 && *i != 4)
        .map(id)
        .collect();
    (dir, index, matching)
}

fn in_order(hits: &[Hit]) -> bool {
    hits.windows(2)
        .all(|w| w[0].score > w[1].score || (w[0].score == w[1].score && w[0].id < w[1].id))
}

fn page_through(index: &Index, text: &str, size: usize) -> Vec<Hit> {
    let mut seen: Vec<Hit> = Vec::new();
    loop {
        let after = seen.last().map(|h| (h.score, h.id.as_str()));
        let page = index.search_page(text, after, size).unwrap();
        assert!(page.hits.len() <= size);
        let done = page.hits.len() < size;
        seen.extend(page.hits);
        if done {
            return seen;
        }
    }
}

#[test]
fn every_hit_comes_back_by_score_then_id_with_ties_broken_by_id() {
    let (_dir, index, matching) = index();
    let all = index.search("apple", None).unwrap();
    let ids: BTreeSet<String> = all.iter().map(|h| h.id.clone()).collect();
    assert_eq!(ids, matching);
    assert_eq!(all.len(), matching.len(), "a hit came back twice");
    assert!(in_order(&all), "{all:#?}");
    // The fixture has runs of tied scores, so the id order is what is being tested.
    assert!(all.windows(2).any(|w| w[0].score == w[1].score));
    assert!(all.windows(2).any(|w| w[0].score > w[1].score));

    let top = index.search("apple", Some(5)).unwrap();
    assert_eq!(top, all[..5]);
}

#[test]
fn pages_joined_are_the_whole_result_and_report_every_match() {
    let (_dir, index, matching) = index();
    let all = index.search("apple", None).unwrap();
    for size in [1, 3, 7, 50, 1000] {
        assert_eq!(page_through(&index, "apple", size), all, "pages of {size}");
        let first = index.search_page("apple", None, size).unwrap();
        assert_eq!(first.total, matching.len(), "pages of {size}");
    }
    // A cursor after the last hit, and a page of nothing, still count every match.
    let last = all.last().unwrap();
    let past = index
        .search_page("apple", Some((last.score, &last.id)), 10)
        .unwrap();
    assert_eq!(past.hits, vec![]);
    assert_eq!(past.total, matching.len());
    assert_eq!(
        index.search_page("apple", None, 0).unwrap().total,
        matching.len()
    );
    // A cursor id that is not in the index still pages from the right place.
    let mid = &all[all.len() / 2];
    let between = format!("{}~", mid.id);
    let after = index
        .search_page("apple", Some((mid.score, &between)), 1000)
        .unwrap();
    assert_eq!(after.hits, all[all.len() / 2 + 1..]);
    assert_eq!(index.search_page("zeppelin", None, 10).unwrap().total, 0);
}

#[test]
fn an_index_written_without_the_id_fast_field_is_replaced_and_rebuilt() {
    use tantivy::schema::{STORED, STRING, TEXT};
    let (_dir, vault, _schema) = vault();
    let note = format!("notes/item/{}.wcl", id(1));
    std::fs::create_dir_all(vault.root().join("notes/item")).unwrap();
    std::fs::write(
        vault.root().join(&note),
        "title   = \"apple\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\n",
    )
    .unwrap();
    {
        let mut index = Index::open(&vault).unwrap();
        index.rebuild(&vault).unwrap();
        assert!(index.is_current(&vault).unwrap());
    }

    // Replace the full-text index with one in the schema before `id` was a fast field.
    let dir = vault.server_dir().join("search");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    {
        let mut b = tantivy::schema::Schema::builder();
        let id_f = b.add_text_field("id", STRING | STORED);
        b.add_text_field("type", STRING);
        let title_f = b.add_text_field("title", TEXT);
        b.add_text_field("body", TEXT);
        let old = tantivy::Index::create_in_dir(&dir, b.build()).unwrap();
        let mut w = old.writer_with_num_threads(1, 15_000_000).unwrap();
        w.add_document(tantivy::doc!(id_f => id(1), title_f => "apple"))
            .unwrap();
        w.commit().unwrap();
    }

    // It opens, empty, and the index says it is behind so the next open rebuilds it.
    let mut index = Index::open(&vault).unwrap();
    assert_eq!(index.search("apple", None).unwrap(), vec![]);
    assert!(!index.is_current(&vault).unwrap());
    index.rebuild(&vault).unwrap();
    assert!(index.is_current(&vault).unwrap());
    let hits = index.search("apple", None).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
        [id(1)]
    );
}

#[test]
fn search_pages_leave_archived_notes_out_include_them_or_read_only_them() {
    let (_dir, vault, schema) = vault();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let note = |i: usize, archived: bool| {
        let flag = if archived { "archived = true\n" } else { "" };
        let text = format!(
            "title   = \"plum {i}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\n{flag}"
        );
        let file = format!("notes/item/{}.wcl", id(i));
        let report = schema
            .validate_note_source("item", &text, &file, &HashMap::new())
            .unwrap();
        Update::Put {
            id: id(i),
            type_name: "item".to_owned(),
            doc: Doc::parse(&text, &file).unwrap(),
            report,
        }
    };
    index
        .update(&schema, &[note(1, false), note(2, true), note(3, false)])
        .unwrap();
    let ids = |index: &Index, archived: index::Archived| -> (Vec<String>, usize) {
        let page = index
            .search_page_where("plum", None, archived, None, 10)
            .unwrap();
        let mut ids: Vec<String> = page.hits.into_iter().map(|h| h.id).collect();
        ids.sort();
        (ids, page.total)
    };
    assert_eq!(
        ids(&index, index::Archived::Exclude),
        (vec![id(1), id(3)], 2)
    );
    assert_eq!(ids(&index, index::Archived::Only), (vec![id(2)], 1));
    assert_eq!(
        ids(&index, index::Archived::Include),
        (vec![id(1), id(2), id(3)], 3)
    );
    // The archive state scores nothing: a note scores the same whichever way it is read.
    let score = |archived: index::Archived| {
        index
            .search_page_where("plum", None, archived, None, 10)
            .unwrap()
            .hits
            .into_iter()
            .find(|h| h.id == id(2))
            .map(|h| h.score)
    };
    assert_eq!(
        score(index::Archived::Only),
        score(index::Archived::Include)
    );

    // Unarchiving a note puts it back among the default hits.
    index.update(&schema, &[note(2, false)]).unwrap();
    assert_eq!(ids(&index, index::Archived::Exclude).1, 3);
    assert_eq!(ids(&index, index::Archived::Only).1, 0);
}
