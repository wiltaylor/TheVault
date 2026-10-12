//! D4: a commit changes every file or none of them, even when it is killed part way.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::Path;
use std::process::Command;

use store::{Change, CommitStep, Error, Vault};
use tempfile::TempDir;

const A: &str = "notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl";
const B: &str = "notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZB.wcl";
const C: &str = "notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZC.wcl";
const NEW: &str = "notes/board/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl";
const ASSET: &str = "assets/01JB9A0C3D4E5F6G7H8J9K0M1N/sketch.png";

/// Every file the commit touches, mapped to its contents (`None` when absent).
type Snapshot = BTreeMap<&'static str, Option<Vec<u8>>>;

fn snapshot(root: &Path) -> Snapshot {
    [A, B, C, NEW, ASSET]
        .into_iter()
        .map(|p| (p, std::fs::read(root.join(p)).ok()))
        .collect()
}

/// A vault holding A, B and C. Returns it with the snapshot before and after `changes()`.
fn seeded() -> (TempDir, Vault) {
    let dir = TempDir::new().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    vault
        .commit(vec![
            Change::write(A, "title = \"A\"\nrelated = [\"B\"]\n"),
            Change::write(B, "title = \"B\"\n"),
            Change::write(C, "title = \"C\"\n"),
        ])
        .unwrap();
    (dir, vault)
}

/// Update both sides of a link, delete a note, create a note and an asset.
fn changes() -> Vec<Change> {
    vec![
        Change::write(A, "title = \"A\"\nrelated = [\"B\", \"F\"]\n"),
        Change::write(B, "title = \"B\"\nrelated = [\"A\"]\n"),
        Change::delete(C),
        Change::write(NEW, "title = \"F\"\n"),
        Change::write(ASSET, vec![0x89, b'P', b'N', b'G', 0, 1, 2]),
    ]
}

fn expected() -> (Snapshot, Snapshot) {
    let (dir, vault) = seeded();
    let before = snapshot(dir.path());
    vault.commit(changes()).unwrap();
    let after = snapshot(dir.path());
    assert_ne!(before, after);
    assert!(after[C].is_none() && after[NEW].is_some() && after[ASSET].is_some());
    (before, after)
}

fn every_step() -> Vec<CommitStep> {
    let n = changes().len();
    let mut steps: Vec<CommitStep> = (0..n).map(CommitStep::Staged).collect();
    steps.push(CommitStep::Committed);
    steps.extend((0..n).map(CommitStep::Applied));
    steps
}

#[test]
fn a_commit_stopped_at_any_step_is_all_or_nothing_after_reopening() {
    let (before, after) = expected();
    for stop in every_step() {
        let (dir, vault) = seeded();
        let result = vault.commit_with_hook(changes(), &mut |step| {
            if step == stop {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert!(
            matches!(result, Err(Error::Interrupted)),
            "{stop:?}: {result:?}"
        );
        drop(vault);

        Vault::open(dir.path()).unwrap();
        let found = snapshot(dir.path());
        let want = if matches!(stop, CommitStep::Staged(_)) {
            &before
        } else {
            &after
        };
        assert_eq!(&found, want, "stopped at {stop:?}");
        assert!(
            !dir.path().join(".vault/txn").exists(),
            "{stop:?}: journal left behind"
        );
    }
}

/// Run inside a child process: commit and abort the process at the step named in the environment.
#[test]
fn crash_child() {
    let (Ok(root), Ok(step)) = (
        std::env::var("STORE_CRASH_ROOT"),
        std::env::var("STORE_CRASH_AT"),
    ) else {
        return;
    };
    let vault = Vault::open(Path::new(&root)).unwrap();
    let _ = vault.commit_with_hook(changes(), &mut |s| {
        if format!("{s:?}") == step {
            std::process::abort();
        }
        ControlFlow::Continue(())
    });
    panic!("the commit finished without reaching {step}");
}

#[test]
fn a_process_killed_between_file_writes_leaves_all_or_nothing() {
    let (before, after) = expected();
    let exe = std::env::current_exe().unwrap();
    for stop in [
        CommitStep::Staged(0),
        CommitStep::Staged(3),
        CommitStep::Committed,
        CommitStep::Applied(0),
        CommitStep::Applied(2),
    ] {
        let (dir, vault) = seeded();
        drop(vault);
        let status = Command::new(&exe)
            .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
            .env("STORE_CRASH_ROOT", dir.path())
            .env("STORE_CRASH_AT", format!("{stop:?}"))
            .status()
            .unwrap();
        assert!(!status.success(), "{stop:?}: the child was not killed");

        Vault::open(dir.path()).unwrap();
        let want = if matches!(stop, CommitStep::Staged(_)) {
            &before
        } else {
            &after
        };
        assert_eq!(&snapshot(dir.path()), want, "killed at {stop:?}");
    }
}

#[test]
fn a_commit_after_an_interrupted_one_finishes_it_first() {
    let (_, after) = expected();
    let (dir, vault) = seeded();
    let result = vault.commit_with_hook(changes(), &mut |step| match step {
        CommitStep::Applied(1) => ControlFlow::Break(()),
        _ => ControlFlow::Continue(()),
    });
    assert!(matches!(result, Err(Error::Interrupted)));
    vault.commit(vec![]).unwrap();
    assert_eq!(snapshot(dir.path()), after);
}

#[test]
fn a_rejected_change_leaves_every_file_alone() {
    let (dir, vault) = seeded();
    let before = snapshot(dir.path());
    for bad in [
        Change::delete("notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ.wcl"),
        Change::write(A, "x = 1"),
        Change::write("notes/specimen/x.txt", "x"),
        Change::write(".vault/state.db", "x"),
        Change::write("../escape.wcl", "x = 1"),
        Change::write(B, vec![0xff, 0xfe]),
    ] {
        let mut set = changes();
        set.push(bad.clone());
        assert!(vault.commit(set).is_err(), "{bad:?} was accepted");
        assert_eq!(snapshot(dir.path()), before, "{bad:?}");
    }
}
