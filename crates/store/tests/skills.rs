//! Skills S1: `skills/` is an opaque tree of files, kept byte for byte, with limits and paths
//! confined to each skill's folder.

use store::{
    Change, Error, MAX_SKILL_BYTES, MAX_SKILL_FILES, Vault, is_skill_path, normalise_skill_path,
    skill_limits,
};
use tempfile::TempDir;

fn vault() -> (TempDir, Vault) {
    let dir = TempDir::new().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    (dir, vault)
}

#[test]
fn init_creates_the_skills_folder() {
    let (dir, _vault) = vault();
    assert!(dir.path().join("skills").is_dir());
}

#[test]
fn skill_files_are_committed_byte_for_byte_whatever_their_kind() {
    let (dir, vault) = vault();
    let binary: Vec<u8> = (0..=255u8).collect();
    // Not data-only and not canonical: a .wcl file under skills/ is the skill's own.
    let wcl = b"let x = 1 + 2\nfoo   =   bar\n".to_vec();
    vault
        .commit(vec![
            Change::write("skills/pdf/SKILL.md", "---\nname: pdf\n---\n"),
            Change::write("skills/pdf/assets/logo.bin", binary.clone()),
            Change::write("skills/pdf/scripts/run.py", "print('hi')\r\n"),
            Change::write("skills/pdf/config.wcl", wcl.clone()),
        ])
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("skills/pdf/assets/logo.bin")).unwrap(),
        binary
    );
    assert_eq!(
        std::fs::read(dir.path().join("skills/pdf/config.wcl")).unwrap(),
        wcl
    );
    assert_eq!(
        vault.read_skill_file("pdf", "scripts/run.py").unwrap(),
        b"print('hi')\r\n"
    );
    assert_eq!(vault.skill_folders().unwrap(), ["pdf"]);
    let tree = vault.skill_tree("pdf").unwrap();
    let paths: Vec<&str> = tree.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "SKILL.md",
            "assets/logo.bin",
            "config.wcl",
            "scripts/run.py"
        ]
    );
    assert_eq!(tree.files[1].size, 256);
    assert_eq!(tree.size(), tree.files.iter().map(|f| f.size).sum::<u64>());
}

#[test]
fn a_file_directly_under_skills_is_refused() {
    let (_dir, vault) = vault();
    let err = vault
        .commit(vec![Change::write("skills/SKILL.md", "x")])
        .unwrap_err();
    assert!(matches!(err, Error::InvalidPath { .. }), "{err}");
}

#[test]
fn paths_are_normalised_and_cannot_leave_the_skill() {
    assert_eq!(
        normalise_skill_path("./scripts//run.py").unwrap(),
        "scripts/run.py"
    );
    assert_eq!(
        normalise_skill_path("scripts\\win\\a.bat").unwrap(),
        "scripts/win/a.bat"
    );
    assert_eq!(normalise_skill_path("a/./b").unwrap(), "a/b");
    for bad in [
        "",
        ".",
        "/etc/passwd",
        "../other/SKILL.md",
        "a/../../x",
        "C:\\x",
        "a\0b",
    ] {
        assert!(normalise_skill_path(bad).is_err(), "{bad:?}");
    }
    assert!(normalise_skill_path(&"a".repeat(1025)).is_err());

    let (_dir, vault) = vault();
    assert_eq!(
        vault.skill_file_path("pdf", "a//b.txt").unwrap(),
        "skills/pdf/a/b.txt"
    );
    for folder in ["", ".hidden", "a/b", "..", "a\\b"] {
        assert!(vault.skill_file_path(folder, "x").is_err(), "{folder:?}");
    }
    assert!(is_skill_path("skills/pdf/SKILL.md"));
    assert!(!is_skill_path("skills/pdf"));
    assert!(!is_skill_path("agents/pdf.wcl"));
}

#[cfg(unix)]
#[test]
fn symlinks_are_listed_never_followed_and_refused_for_reads_and_writes() {
    let (dir, vault) = vault();
    vault
        .commit(vec![Change::write("skills/s/SKILL.md", "x")])
        .unwrap();
    std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("secret.txt"),
        dir.path().join("skills/s/link.txt"),
    )
    .unwrap();
    std::os::unix::fs::symlink(dir.path().join("notes"), dir.path().join("skills/s/dir")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("skills/s"), dir.path().join("skills/alias"))
        .unwrap();

    let tree = vault.skill_tree("s").unwrap();
    assert_eq!(tree.files.len(), 1);
    assert_eq!(tree.symlinks, ["dir", "link.txt"]);
    // A symlinked skill folder is not a skill.
    assert_eq!(vault.skill_folders().unwrap(), ["s"]);
    assert!(vault.skill_tree("alias").is_err());

    assert!(vault.read_skill_file("s", "link.txt").is_err());
    assert!(vault.skill_file_path("s", "dir/new.wcl").is_err());
}

#[test]
fn limits_count_files_and_bytes() {
    let ok: Vec<(String, u64)> = (0..MAX_SKILL_FILES).map(|i| (i.to_string(), 1)).collect();
    assert_eq!(skill_limits(ok.iter().map(|(p, s)| (p.as_str(), *s))), None);
    let many: Vec<(String, u64)> = (0..=MAX_SKILL_FILES).map(|i| (i.to_string(), 1)).collect();
    assert!(
        skill_limits(many.iter().map(|(p, s)| (p.as_str(), *s)))
            .unwrap()
            .contains("200 files")
    );
    assert_eq!(skill_limits([("a", MAX_SKILL_BYTES)]), None);
    assert!(
        skill_limits([("a", MAX_SKILL_BYTES), ("b", 1)])
            .unwrap()
            .contains("20 MiB")
    );
}

#[test]
fn pruning_removes_the_folders_a_delete_left_empty() {
    let (dir, vault) = vault();
    vault
        .commit(vec![
            Change::write("skills/s/SKILL.md", "x"),
            Change::write("skills/s/a/b/c.txt", "x"),
        ])
        .unwrap();
    vault
        .commit(vec![Change::delete("skills/s/a/b/c.txt")])
        .unwrap();
    vault.prune_skill_folder("s").unwrap();
    assert!(!dir.path().join("skills/s/a").exists());
    assert!(dir.path().join("skills/s/SKILL.md").is_file());
    vault
        .commit(vec![Change::delete("skills/s/SKILL.md")])
        .unwrap();
    vault.prune_skill_folder("s").unwrap();
    assert!(!dir.path().join("skills/s").exists());
    assert!(dir.path().join("skills").is_dir());
}
