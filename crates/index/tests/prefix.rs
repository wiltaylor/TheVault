//! Search as you type: the last word of the text also matches as a prefix, earlier words and a
//! word followed by a space match whole words, and a one-letter last word or a prefix that starts
//! many words stays cheap.

use std::collections::HashMap;

use index::{Index, Update};
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

fn put(schema: &Schema, i: usize, title: &str) -> Update {
    let text = format!(
        "title   = \"{title}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\n"
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

/// An index of one item per title, item `i` holding `titles[i]`.
fn index(titles: &[String]) -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    std::fs::create_dir_all(vault.root().join("types")).unwrap();
    std::fs::write(vault.root().join("types/item.wcl"), ITEM_TYPE).unwrap();
    let schema = Schema::load(&vault).unwrap();
    let mut index = Index::open(&vault).unwrap();
    index.rebuild(&vault).unwrap();
    let updates: Vec<Update> = titles
        .iter()
        .enumerate()
        .map(|(i, t)| put(&schema, i, t))
        .collect();
    // Two commits, so the words are spread over two segments.
    let (a, b) = updates.split_at(updates.len() / 2);
    index.update(&schema, a).unwrap();
    index.update(&schema, b).unwrap();
    (dir, index)
}

/// The items `text` finds, as their numbers, in id order.
fn found(index: &Index, text: &str) -> Vec<usize> {
    let mut ids: Vec<usize> = index
        .search(text, None)
        .unwrap()
        .into_iter()
        .map(|h| h.id[h.id.len() - 4..].parse().unwrap())
        .collect();
    ids.sort();
    ids
}

fn titles(list: &[&str]) -> Vec<String> {
    list.iter().map(|t| (*t).to_owned()).collect()
}

#[test]
fn the_last_word_matches_as_a_prefix_and_earlier_words_whole() {
    let (_dir, index) = index(&titles(&[
        "Meeting notes",       // 0
        "meet the team",       // 1
        "Meetup plan",         // 2
        "team meeting agenda", // 3
        "met before",          // 4
        "a note about meat",   // 5
        "meet notes",          // 6
    ]));
    // Typing `meet` finds every word it starts, in any case.
    assert_eq!(found(&index, "meet"), vec![0, 1, 2, 3, 6]);
    assert_eq!(found(&index, "MEE"), vec![0, 1, 2, 3, 6]);
    // A space after it says the word is complete.
    assert_eq!(found(&index, "meet "), vec![1, 6]);
    // Earlier words match whole words only: `meet` is not `meeting`.
    assert_eq!(found(&index, "meet no"), vec![6]);
    assert_eq!(found(&index, "meeting no"), vec![0]);
    assert_eq!(found(&index, "team meet"), vec![1, 3]);
    assert_eq!(found(&index, "team meet "), vec![1]);
    // A prefix that starts no word finds nothing, and so does a word that is not there.
    assert!(found(&index, "meetx").is_empty());
    assert!(found(&index, "zebra ").is_empty());
    // A quoted phrase is left to the query syntax.
    assert_eq!(found(&index, "\"meet the"), vec![1]);
}

#[test]
fn a_one_letter_last_word_matches_only_that_word() {
    let (_dir, index) = index(&titles(&["a plan", "apple pie", "an apple", "b"]));
    assert_eq!(found(&index, "a"), vec![0]);
    assert_eq!(found(&index, "b"), vec![3]);
    // Two letters are enough for a prefix.
    assert_eq!(found(&index, "ap"), vec![1, 2]);
    assert!(found(&index, "pie a").is_empty());
}

#[test]
fn a_prefix_keeps_relevance_ranking() {
    let (_dir, index) = index(&titles(&[
        "planning",                            // 0
        "planning planning planning planning", // 1
        "plan for the planning of plans",      // 2
    ]));
    let hits = index.search("plann", None).unwrap();
    assert_eq!(hits.len(), 3);
    // The item that says `planning` most often, in the shortest title, ranks first.
    assert_eq!(hits[0].id, id(1));
    assert!(hits[0].score > hits[2].score, "{hits:?}");
    // A page and the whole search agree on scores, so cursors work as before.
    let page = index.search_page("plann", None, 2).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.hits, hits[..2]);
}

#[test]
fn a_prefix_stands_for_at_most_fifty_words_and_always_the_word_itself() {
    // 60 words starting `zq`, word `k` in `k + 2` items, so the rarest are dropped first, and
    // `zq` itself in one item only.
    let mut list = vec!["zq".to_owned()];
    for k in 0..60 {
        for _ in 0..k + 2 {
            list.push(format!("zq{k:02}x"));
        }
    }
    let (_dir, index) = index(&list);
    let hits = found(&index, "zq");
    // The word itself, and the 49 most frequent: words 11 to 59.
    let mut want = vec![0];
    let mut at = 1;
    for k in 0..60 {
        if k >= 11 {
            want.extend(at..at + k + 2);
        }
        at += k + 2;
    }
    assert_eq!(hits, want);
    // A whole word is never cut.
    assert_eq!(found(&index, "zq05x ").len(), 7);
}
