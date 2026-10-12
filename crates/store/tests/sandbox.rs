//! A path or an import that points outside the vault cannot read the outside file.

use std::os::unix::fs::symlink;

use store::{Change, Error, Vault};
use tempfile::TempDir;

const SECRET: &str = "outside-secret-7f3a";

/// A vault next to a file outside it. Returns (temp dir, vault, path of the outside file).
fn setup() -> (TempDir, Vault, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let outside = dir.path().join("outside.wcl");
    std::fs::write(&outside, format!("secret = \"{SECRET}\"\n")).unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    (dir, vault, outside)
}

fn assert_no_secret(result: &store::Result<impl std::fmt::Debug>) {
    let text = format!("{result:?}");
    assert!(result.is_err(), "expected an error, got {text}");
    assert!(!text.contains(SECRET), "the outside file leaked: {text}");
}

#[test]
fn a_relative_or_absolute_path_cannot_leave_the_vault() {
    let (_dir, vault, outside) = setup();
    for path in [
        "../outside.wcl",
        "notes/../../outside.wcl",
        "notes/./x.wcl",
        outside.to_str().unwrap(),
        ".vault/state.db",
        "outside.wcl",
    ] {
        let result = vault.read(path);
        assert!(
            matches!(result, Err(Error::InvalidPath { .. })),
            "{path}: {result:?}"
        );
    }
}

#[test]
fn a_symlink_out_of_the_vault_cannot_be_read() {
    let (_dir, vault, outside) = setup();
    let root = vault.root().to_path_buf();
    symlink(&outside, root.join("notes/escape.wcl")).unwrap();
    symlink(outside.parent().unwrap(), root.join("notes/escape_dir")).unwrap();

    for path in ["notes/escape.wcl", "notes/escape_dir/outside.wcl"] {
        let result = vault.read(path);
        assert!(
            matches!(result, Err(Error::OutsideVault(_))),
            "{path}: {result:?}"
        );
        assert_no_secret(&result);
        assert_no_secret(&vault.read_wcl(path));
    }
}

#[test]
fn a_commit_cannot_write_through_a_symlink_out_of_the_vault() {
    let (dir, vault, outside) = setup();
    symlink(dir.path(), vault.root().join("notes/escape_dir")).unwrap();
    let result = vault.commit(vec![Change::write(
        "notes/escape_dir/outside.wcl",
        "secret = \"overwritten\"\n",
    )]);
    assert!(matches!(result, Err(Error::OutsideVault(_))), "{result:?}");
    assert!(std::fs::read_to_string(&outside).unwrap().contains(SECRET));
}

#[test]
fn the_loader_refuses_files_outside_the_vault_and_in_dot_vault() {
    let (_dir, vault, outside) = setup();
    let root = vault.root().to_path_buf();
    std::fs::write(root.join(".vault/state.db"), "private").unwrap();
    std::fs::write(root.join("types/inside.wcl"), "label = \"Inside\"\n").unwrap();
    symlink(&outside, root.join("types/escape.wcl")).unwrap();

    let loader = vault.loader();
    assert_eq!(
        loader(&root.join("types/inside.wcl")).unwrap(),
        "label = \"Inside\"\n"
    );
    for path in [
        outside.clone(),
        root.join("types/../../outside.wcl"),
        root.join("types/escape.wcl"),
        root.join(".vault/state.db"),
    ] {
        let err = loader(&path).unwrap_err();
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::PermissionDenied,
            "{}",
            path.display()
        );
    }
}

#[test]
fn an_import_resolved_through_the_loader_cannot_read_the_outside_file() {
    let (_dir, vault, outside) = setup();
    let env = wcl_lang::Environment::new();
    let open = |import: &str| {
        wcl_lang::Document::open_at_with_loader(
            &format!("import \"{import}\"\n"),
            "probe.wcl",
            Some(vault.root().join("notes")),
            &env,
            vault.loader(),
        )
    };
    // An import inside the vault resolves, so the refusals below are not vacuous.
    std::fs::write(vault.root().join("types/inside.wcl"), "x = 1\n").unwrap();
    let doc = open("../types/inside.wcl").unwrap();
    assert_eq!(
        doc.imported_paths(),
        [vault.root().join("types/inside.wcl").as_path()]
    );
    // The data-only check stops imports first; this proves the loader holds even without it.
    for import in ["../../outside.wcl", outside.to_str().unwrap()] {
        let text = match open(import) {
            Ok(_) => panic!("{import}: the import was loaded"),
            Err(e) => format!("{e:?}"),
        };
        assert!(text.contains("outside the vault"), "{import}: {text}");
        assert!(
            !text.contains(SECRET),
            "{import}: the outside file leaked: {text}"
        );
    }
}
