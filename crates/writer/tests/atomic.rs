//! S3: a fault injected part way through the commit of a two-way link leaves neither side
//! changed, and a fault after the commit point leaves both changed once the vault reopens (D4).

use std::ops::ControlFlow;
use std::path::Path;

use state::{Author, State};
use store::{CommitStep, Vault};
use writer::{Error, Input, Writer};

const PROJECT: &str = r#"label = "Project"

field tasks {
  kind     = "reference"
  target   = "task"
  relation = "two_way"
  inverse  = "project"
  multiple = true
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TASK: &str = r#"label = "Task"

field project {
  kind     = "reference"
  target   = "project"
  relation = "two_way"
  inverse  = "tasks"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn wcl(text: &str) -> Input {
    Input::Wcl(text.to_owned())
}

fn file(root: &Path, type_name: &str, id: &str) -> String {
    std::fs::read_to_string(root.join(format!("notes/{type_name}/{id}.wcl"))).unwrap()
}

/// A vault with a project and an unlinked task. Returns the writer, the project id and the task id.
fn setup(root: &Path) -> (Writer, String, String) {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/project.wcl"), PROJECT).unwrap();
    std::fs::write(root.join("types/task.wcl"), TASK).unwrap();
    let w = open(root);
    let project = w
        .create("project", &wcl("title = \"Importer\"\n"), &author())
        .unwrap()
        .id;
    let task = w
        .create("task", &wcl("title = \"Parse\"\n"), &author())
        .unwrap()
        .id;
    (w, project, task)
}

/// Stop every commit at `stop`.
fn fail_at(w: &Writer, stop: CommitStep) {
    w.set_commit_hook(Some(Box::new(move |step| {
        if step == stop {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })));
}

#[test]
fn a_fault_before_the_commit_point_leaves_neither_side_changed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (w, project, task) = setup(root);
    let before = (file(root, "project", &project), file(root, "task", &task));
    let versions = (
        w.current_version(&project).unwrap(),
        w.current_version(&task).unwrap(),
    );
    let base = versions.1.clone().unwrap();
    let link = format!("title = \"Parse\"\nproject = \"{project}\"\n");

    // Both files are staged, then the commit stops short of writing its manifest.
    for stop in [CommitStep::Staged(0), CommitStep::Staged(1)] {
        fail_at(&w, stop);
        let err = w
            .update(&task, &wcl(&link), Some(&base), &author())
            .unwrap_err();
        assert!(
            matches!(err, Error::Store(store::Error::Interrupted)),
            "{err}"
        );
        assert_eq!(
            (file(root, "project", &project), file(root, "task", &task)),
            before,
            "a file changed after a fault at {stop:?}"
        );
        assert_eq!(
            (
                w.current_version(&project).unwrap(),
                w.current_version(&task).unwrap()
            ),
            versions
        );
    }

    // Reopening the vault, as a restarted server would, discards the half-made commit.
    drop(w);
    let w = open(root);
    assert_eq!(
        (file(root, "project", &project), file(root, "task", &task)),
        before
    );
    assert!(!root.join(".vault/txn").exists());

    // The same write without a fault links both sides.
    let saved = w
        .update(&task, &wcl(&link), Some(&base), &author())
        .unwrap();
    assert_eq!(saved.also.len(), 1);
    assert_eq!(saved.also[0].id, project);
    assert!(file(root, "task", &task).contains(&project));
    assert!(file(root, "project", &project).contains(&task));
}

#[test]
fn a_fault_after_the_commit_point_leaves_both_sides_changed_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (w, project, task) = setup(root);
    let base = w.current_version(&task).unwrap().unwrap();
    let link = format!("title = \"Parse\"\nproject = \"{project}\"\n");

    // The manifest is written and the first file applied, then the process "dies".
    fail_at(&w, CommitStep::Applied(0));
    w.update(&task, &wcl(&link), Some(&base), &author())
        .unwrap_err();
    drop(w);

    let w = open(root);
    assert!(file(root, "task", &task).contains(&project));
    assert!(file(root, "project", &project).contains(&task));
    assert!(!root.join(".vault/txn").exists());

    // state.db caught up with the files: each note's current version is its file's text.
    for (type_name, id) in [("task", &task), ("project", &project)] {
        let current = w.current_version(id).unwrap().unwrap();
        let version = w.version(id, &current).unwrap();
        assert_eq!(version.content, file(root, type_name, id), "{type_name}");
        assert_eq!(version.author, Author::Session("recovery".into()));
        assert_eq!(w.history(id).unwrap().len(), 2, "{type_name}");
    }
    assert_ne!(w.current_version(&task).unwrap().unwrap(), base);

    // The recovered version is a base a client can write against.
    let current = w.current_version(&task).unwrap().unwrap();
    w.update(
        &task,
        &wcl("title = \"Parse files\"\n"),
        Some(&current),
        &author(),
    )
    .unwrap();
}

#[test]
fn a_fault_after_the_commit_point_of_a_delete_trashes_the_note_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (w, _project, task) = setup(root);
    let base = w.current_version(&task).unwrap().unwrap();

    fail_at(&w, CommitStep::Applied(0));
    w.delete(&task, Some(&base), &author()).unwrap_err();
    drop(w);

    let w = open(root);
    assert!(!root.join(format!("notes/task/{task}.wcl")).exists());
    assert_eq!(w.current_version(&task).unwrap(), None);
    let trash = w.trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].note_id, task);
    assert_eq!(trash[0].version, base);
    assert_eq!(trash[0].deleted_by, Author::Session("recovery".into()));
}

#[test]
fn reopening_a_vault_with_nothing_to_recover_records_no_versions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (w, project, task) = setup(root);
    let base = w.current_version(&task).unwrap().unwrap();
    let link = format!("title = \"Parse\"\nproject = \"{project}\"\n");
    w.update(&task, &wcl(&link), Some(&base), &author())
        .unwrap();
    let before = (w.history(&project).unwrap(), w.history(&task).unwrap());
    drop(w);

    let w = open(root);
    assert_eq!(
        (w.history(&project).unwrap(), w.history(&task).unwrap()),
        before
    );
    assert!(w.trash().unwrap().is_empty());
}
