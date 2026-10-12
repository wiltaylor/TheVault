//! Opening the write path rebuilds the index only when it is behind the vault's commits: a
//! current index is trusted as it is, and a stale or missing one is detected and rebuilt.

use std::ops::ControlFlow;
use std::path::Path;

use state::{Author, State};
use store::{Change, CommitStep, Vault};
use writer::{Input, Writer};

const TASK: &str = r#"label = "Task"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const STRAY: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZS";

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn create(w: &Writer, title: &str) -> String {
    w.create(
        "task",
        &Input::Wcl(format!("title = \"{title}\"\n")),
        &author(),
    )
    .unwrap()
    .id
}

/// The ids in the index, sorted.
fn indexed(w: &Writer) -> Vec<String> {
    w.read_index(|index, _| {
        let mut stmt = index
            .connection()
            .prepare("SELECT id FROM notes ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    })
}

fn searched(w: &Writer, text: &str) -> Vec<String> {
    w.read_index(|index, _| {
        index
            .search(text, None)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect()
    })
}

fn is_current(w: &Writer) -> bool {
    w.read_index(|index, _| index.is_current(w.vault()).unwrap())
}

/// A vault with the task type and two tasks, its writer closed. Returns the task ids, sorted.
fn setup(root: &Path) -> Vec<String> {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/task.wcl"), TASK).unwrap();
    let w = open(root);
    let mut ids = vec![create(&w, "Parse"), create(&w, "Render")];
    // A script and an asset change no note, and keep the index current all the same.
    w.put_script("noop", "fn main() {}\n").unwrap();
    w.put_asset("a.png", vec![1, 2, 3]).unwrap();
    assert!(is_current(&w));
    ids.sort();
    ids
}

/// A note file written behind the server's back (D28).
fn stray_note(root: &Path) {
    std::fs::write(
        root.join(format!("notes/task/{STRAY}.wcl")),
        "title = \"Stray\"\n",
    )
    .unwrap();
}

#[test]
fn a_current_index_is_not_rebuilt_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ids = setup(root);
    // A rebuild would index this file, so its absence shows the index was trusted.
    stray_note(root);
    let w = open(root);
    assert!(is_current(&w));
    assert_eq!(indexed(&w), ids);
    assert_eq!(searched(&w, "parse").len(), 1);
}

#[test]
fn an_index_behind_a_commit_that_stopped_after_the_commit_point_is_rebuilt_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut ids = setup(root);
    let w = open(root);
    w.set_commit_hook(Some(Box::new(|step| {
        if step == CommitStep::Committed {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })));
    assert!(
        w.create(
            "task",
            &Input::Wcl("title = \"Zeppelin\"\n".into()),
            &author()
        )
        .is_err()
    );
    drop(w);

    let w = open(root);
    assert!(is_current(&w));
    let found = indexed(&w);
    assert_eq!(found.len(), 3, "{found:?}");
    let new = found.iter().find(|id| !ids.contains(id)).unwrap().clone();
    assert_eq!(searched(&w, "zeppelin"), vec![new.clone()]);
    assert!(w.read(&new).is_ok());
    ids.push(new);
    ids.sort();
    assert_eq!(found, ids);
}

#[test]
fn an_index_behind_a_commit_made_without_the_writer_is_rebuilt_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut ids = setup(root);
    let vault = Vault::open(root).unwrap();
    vault
        .commit(vec![Change::write(
            format!("notes/task/{STRAY}.wcl"),
            "title = \"Stray\"\n",
        )])
        .unwrap();
    drop(vault);

    let w = open(root);
    assert!(is_current(&w));
    ids.push(STRAY.to_owned());
    ids.sort();
    assert_eq!(indexed(&w), ids);
}

#[test]
fn a_deleted_index_is_rebuilt_on_open() {
    for gone in [
        &["index.db", "index.db-wal", "index.db-shm"][..],
        &["search"],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ids = setup(root);
        stray_note(root);
        for name in gone {
            let path = root.join(".vault").join(name);
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
        let w = open(root);
        assert!(is_current(&w), "{gone:?}");
        let mut want = ids.clone();
        want.push(STRAY.to_owned());
        want.sort();
        assert_eq!(indexed(&w), want, "{gone:?}");
        assert_eq!(searched(&w, "stray"), vec![STRAY.to_owned()], "{gone:?}");
    }
}
