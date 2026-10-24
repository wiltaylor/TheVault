//! Skills S1, S2, S3, S7 in the writer: skills listed with their validity, `SKILL.md` checked
//! before a write, files confined to the skill and within its limits, zip import and export, the
//! change feed, and one `skill.change` entry per write.

use std::io::{Read, Write};

use state::{ActivityFilter, Author, ChangeKey, State};
use store::Vault;
use writer::{Error, Writer, acting_as};

const PDF: &str = "---\nname: pdf\ndescription: Fills PDF forms. Use when a PDF form arrives.\nlicense: MIT\nmetadata:\n  author: me\nextra-key: kept\n---\n\n# PDF\n\nRun scripts/fill.py.\n";

fn open(root: &std::path::Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (path, bytes) in files {
        if path.ends_with('/') {
            z.add_directory(path.trim_end_matches('/'), opts).unwrap();
        } else {
            z.start_file(*path, opts).unwrap();
            z.write_all(bytes).unwrap();
        }
    }
    z.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index(i).unwrap();
            let mut out = Vec::new();
            f.read_to_end(&mut out).unwrap();
            (f.name().unwrap().into_owned(), out)
        })
        .collect()
}

fn skill_entries(w: &Writer) -> Vec<(String, String, String)> {
    let mut items = w
        .state()
        .activity(&ActivityFilter {
            kind: Some("skill".into()),
            ..Default::default()
        })
        .unwrap()
        .items;
    items.reverse();
    items
        .into_iter()
        .map(|a| {
            assert_eq!(a.category, "definitions");
            assert_eq!(a.subject_kind.as_deref(), Some("skill"));
            (a.kind, a.actor, a.summary)
        })
        .collect()
}

#[test]
fn a_skill_is_written_read_and_listed_with_its_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    assert!(w.skills().unwrap().is_empty());
    let (skill, created) = w.put_skill_md("pdf", PDF).unwrap();
    assert!(created);
    assert!(skill.valid(), "{:?}", skill.issues);
    assert_eq!(skill.md.license.as_deref(), Some("MIT"));
    assert_eq!(skill.md.metadata["author"], "me");
    assert_eq!(skill.md.extra["extra-key"], "kept");
    // The unknown key is a warning, not an error.
    assert_eq!(skill.issues.len(), 1);
    assert!(skill.issues[0].is_warning());
    assert_eq!(skill.md.body, "\n# PDF\n\nRun scripts/fill.py.\n");
    // Stored exactly as sent.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("skills/pdf/SKILL.md")).unwrap(),
        PDF
    );

    let png = [0x89u8, b'P', b'N', b'G', 0, 1, 2, 255];
    let f = w.put_skill_file("pdf", "assets/logo.png", &png).unwrap();
    assert_eq!(
        (f.path.as_str(), f.size, f.media_type, f.text),
        ("assets/logo.png", 8, "image/png", false)
    );
    w.put_skill_file("pdf", "./scripts//fill.py", b"print(1)\n")
        .unwrap();
    assert_eq!(w.skill_file("pdf", "assets/logo.png").unwrap(), png);
    assert_eq!(
        w.skill_text("pdf", "scripts/fill.py").unwrap(),
        "print(1)\n"
    );
    assert!(w.skill_text("pdf", "assets/logo.png").is_err());

    let skill = w.skill("pdf").unwrap();
    let files: Vec<(&str, bool)> = skill
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.text))
        .collect();
    assert_eq!(
        files,
        [
            ("SKILL.md", true),
            ("assets/logo.png", false),
            ("scripts/fill.py", true)
        ]
    );
    assert_eq!(skill.size, PDF.len() as u64 + 8 + 9);
    let list = w.skills().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(w.valid_skills().unwrap().len(), 1);

    let (_, created) = w.put_skill_md("pdf", PDF).unwrap();
    assert!(!created);
}

#[test]
fn an_invalid_skill_md_is_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    // The name must equal the folder.
    let err = w
        .put_skill_md("pdf-tools", "---\nname: pdf\ndescription: d\n---\n")
        .unwrap_err();
    let Error::Invalid(inv) = err else {
        panic!("{err}")
    };
    assert_eq!(inv.file, "skills/pdf-tools/SKILL.md");
    assert_eq!(inv.issues[0].at, "name");
    assert!(w.put_skill_md("pdf", "# no frontmatter\n").is_err());
    assert!(matches!(
        w.put_skill_md("PDF", "---\nname: PDF\ndescription: d\n---\n"),
        Err(Error::BadRequest(_))
    ));
    assert!(matches!(
        w.put_skill_md("import", "---\nname: import\ndescription: d\n---\n"),
        Err(Error::BadRequest(_))
    ));
    assert!(!dir.path().join("skills/pdf-tools").exists());
    assert!(!dir.path().join("skills/pdf").exists());
    // A file needs an existing skill.
    assert!(matches!(
        w.put_skill_file("pdf", "a.txt", b"x"),
        Err(Error::Missing(_))
    ));
}

#[test]
fn a_skill_on_disk_that_fails_is_listed_with_its_errors() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    std::fs::create_dir_all(dir.path().join("skills/Bad Name")).unwrap();
    std::fs::write(
        dir.path().join("skills/Bad Name/SKILL.md"),
        "---\nname: bad-name\ndescription: d\n---\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("skills/no-md/scripts")).unwrap();
    std::fs::write(dir.path().join("skills/no-md/scripts/a.sh"), "x").unwrap();
    // Left empty by a sync: not a skill.
    std::fs::create_dir_all(dir.path().join("skills/empty/sub")).unwrap();
    let list = w.skills().unwrap();
    let names: Vec<(&str, bool)> = list.iter().map(|s| (s.name.as_str(), s.valid())).collect();
    assert_eq!(names, [("Bad Name", false), ("no-md", false)]);
    assert!(list[0].issues.iter().any(|i| i.at == "name"));
    assert!(list[1].issues.iter().any(|i| i.at == "SKILL.md"));
    assert!(w.valid_skills().unwrap().is_empty());
    assert!(matches!(w.skill("empty"), Err(Error::Missing(_))));
    assert!(matches!(w.skill("missing"), Err(Error::Missing(_))));
}

#[test]
fn paths_stay_inside_the_skill_and_the_limits_hold() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.put_skill_md("pdf", PDF).unwrap();
    for bad in ["../agents/x.wcl", "/etc/passwd", "a/../../b", ""] {
        assert!(
            matches!(
                w.put_skill_file("pdf", bad, b"x"),
                Err(Error::BadRequest(_))
            ),
            "{bad}"
        );
        assert!(w.skill_file("pdf", bad).is_err(), "{bad}");
    }
    assert!(!dir.path().join("agents/x.wcl").exists());
    w.put_skill_file("pdf", "a/b.txt", b"x").unwrap();
    // A file cannot hold a file, and a folder cannot be overwritten by one.
    assert!(w.put_skill_file("pdf", "a/b.txt/c", b"x").is_err());
    assert!(w.put_skill_file("pdf", "a", b"x").is_err());
    // Twenty MiB at most.
    let big = vec![0u8; store::MAX_SKILL_BYTES as usize];
    assert!(matches!(
        w.put_skill_file("pdf", "big.bin", &big),
        Err(Error::TooLarge(_))
    ));
    // Two hundred files at most: SKILL.md and a/b.txt are two.
    for i in 0..198 {
        w.put_skill_file("pdf", &format!("many/{i}.txt"), b"x")
            .unwrap();
    }
    assert!(matches!(
        w.put_skill_file("pdf", "one-more.txt", b"x"),
        Err(Error::TooLarge(_))
    ));
    // Replacing a file adds none.
    w.put_skill_file("pdf", "many/0.txt", b"y").unwrap();
}

#[test]
fn deleting_files_folders_and_the_skill() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.put_skill_md("pdf", PDF).unwrap();
    w.put_skill_file("pdf", "scripts/a.py", b"a").unwrap();
    w.put_skill_file("pdf", "scripts/b.py", b"b").unwrap();
    w.put_skill_file("pdf", "notes.txt", b"n").unwrap();
    assert!(matches!(
        w.delete_skill_file("pdf", "SKILL.md"),
        Err(Error::BadRequest(_))
    ));
    assert!(matches!(
        w.delete_skill_file("pdf", "nope.txt"),
        Err(Error::Missing(_))
    ));
    assert_eq!(
        w.delete_skill_file("pdf", "notes.txt").unwrap(),
        ["notes.txt"]
    );
    assert_eq!(
        w.delete_skill_file("pdf", "scripts").unwrap(),
        ["scripts/a.py", "scripts/b.py"]
    );
    assert!(!dir.path().join("skills/pdf/scripts").exists());
    w.delete_skill("pdf").unwrap();
    assert!(!dir.path().join("skills/pdf").exists());
    assert!(matches!(w.delete_skill("pdf"), Err(Error::Missing(_))));
}

#[test]
fn a_zip_imports_from_a_folder_or_its_root_and_exports_back() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let binary: Vec<u8> = (0..=255u8).collect();
    // The folder in the zip is named otherwise: the frontmatter names the skill.
    let folder = zip(&[
        ("pdf-v2/", b""),
        ("pdf-v2/SKILL.md", PDF.as_bytes()),
        ("pdf-v2/scripts/fill.py", b"print(1)\n"),
        ("pdf-v2/assets/logo.bin", &binary),
        ("__MACOSX/pdf-v2/._SKILL.md", b"junk"),
        ("pdf-v2/.DS_Store", b"junk"),
    ]);
    let (skill, replaced) = w.import_skill_zip(&folder, false).unwrap();
    assert!(!replaced);
    assert_eq!(skill.name, "pdf");
    let paths: Vec<&str> = skill.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["SKILL.md", "assets/logo.bin", "scripts/fill.py"]);
    assert_eq!(w.skill_file("pdf", "assets/logo.bin").unwrap(), binary);

    // An existing name is refused unless replace.
    let root = zip(&[
        ("SKILL.md", PDF.as_bytes()),
        ("references/REFERENCE.md", b"# Ref\n"),
    ]);
    let err = w.import_skill_zip(&root, false).unwrap_err();
    assert!(
        matches!(&err, Error::Exists { code: "skill_exists", name, .. } if name == "pdf"),
        "{err}"
    );
    let (skill, replaced) = w.import_skill_zip(&root, true).unwrap();
    assert!(replaced);
    let paths: Vec<&str> = skill.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["SKILL.md", "references/REFERENCE.md"]);
    assert!(!dir.path().join("skills/pdf/scripts").exists());

    // Export is one folder named by the skill.
    let out = unzip(&w.export_skill_zip("pdf").unwrap());
    assert_eq!(
        out,
        [
            ("pdf/SKILL.md".to_owned(), PDF.as_bytes().to_vec()),
            (
                "pdf/references/REFERENCE.md".to_owned(),
                b"# Ref\n".to_vec()
            ),
        ]
    );
    // And it imports again as it was.
    w.delete_skill("pdf").unwrap();
    let (again, _) = w
        .import_skill_zip(&w_export_roundtrip(&out), false)
        .unwrap();
    assert_eq!(again.files.len(), 2);
}

fn w_export_roundtrip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    zip(&refs)
}

#[test]
fn an_invalid_zip_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let cases: Vec<Vec<u8>> = vec![
        b"not a zip".to_vec(),
        // No SKILL.md.
        zip(&[("pdf/readme.md", b"x")]),
        // Two top-level folders.
        zip(&[
            ("a/SKILL.md", PDF.as_bytes()),
            ("b/SKILL.md", PDF.as_bytes()),
        ]),
        // A path that leaves the folder.
        zip(&[("SKILL.md", PDF.as_bytes()), ("../escape.txt", b"x")]),
        // Frontmatter that fails.
        zip(&[
            ("pdf/SKILL.md", b"---\nname: pdf\n---\n"),
            ("pdf/a.txt", b"x"),
        ]),
        zip(&[("SKILL.md", b"---\nname: Bad\ndescription: d\n---\n")]),
        zip(&[("SKILL.md", b"---\ndescription: d\n---\n")]),
    ];
    for (n, bytes) in cases.iter().enumerate() {
        let err = w.import_skill_zip(bytes, true).unwrap_err();
        assert!(
            matches!(err, Error::BadRequest(_) | Error::Invalid(_)),
            "case {n}: {err}"
        );
    }
    assert!(w.skills().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(dir.path().join("skills"))
            .unwrap()
            .count(),
        0
    );
    assert!(!dir.path().join("escape.txt").exists());
    // Over the file limit.
    let many: Vec<(String, Vec<u8>)> = std::iter::once(("SKILL.md".to_owned(), PDF.into()))
        .chain((0..200).map(|i| (format!("f/{i}"), b"x".to_vec())))
        .collect();
    let err = w
        .import_skill_zip(&w_export_roundtrip(&many), false)
        .unwrap_err();
    assert!(matches!(err, Error::TooLarge(_)), "{err}");
}

#[test]
fn skill_files_reach_the_change_feed_as_kind_skill() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.put_skill_md("pdf", PDF).unwrap();
    w.put_skill_file("pdf", "assets/a.bin", &[0, 255]).unwrap();
    w.delete_skill_file("pdf", "assets/a.bin").unwrap();
    let page = w.state().changes_since(0, 100).unwrap();
    let files: Vec<(String, bool)> = page
        .changes
        .iter()
        .filter_map(|c| match &c.key {
            ChangeKey::File(p) if p.starts_with("skills/") => Some((p.clone(), c.present)),
            _ => None,
        })
        .collect();
    assert_eq!(
        files,
        [
            ("skills/pdf/SKILL.md".to_owned(), true),
            ("skills/pdf/assets/a.bin".to_owned(), false),
        ]
    );
    assert_eq!(
        writer::synced_file("skills/pdf/assets/a.bin"),
        Some(("skill", "pdf"))
    );
    w.put_skill_file("pdf", "assets/a.bin", &[0, 255]).unwrap();
    assert_eq!(
        w.read_synced_bytes("skills/pdf/assets/a.bin").unwrap(),
        Some(vec![0, 255])
    );

    // A device applies the feed byte for byte, and a delete takes the emptied folders with it.
    let device_dir = tempfile::tempdir().unwrap();
    let device = open(device_dir.path());
    device
        .apply_remote_files(&[
            (
                "skills/pdf/SKILL.md".to_owned(),
                Some(PDF.as_bytes().to_vec()),
            ),
            ("skills/pdf/assets/a.bin".to_owned(), Some(vec![0, 255])),
        ])
        .unwrap();
    assert_eq!(device.skill_file("pdf", "assets/a.bin").unwrap(), [0, 255]);
    assert!(device.skill("pdf").unwrap().valid());
    device
        .apply_remote_files(&[
            ("skills/pdf/SKILL.md".to_owned(), None),
            ("skills/pdf/assets/a.bin".to_owned(), None),
        ])
        .unwrap();
    assert!(!device_dir.path().join("skills/pdf").exists());
}

#[test]
fn every_write_records_one_skill_change() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let me = Author::Session("owner".into());
    {
        let _acting = acting_as(&me);
        w.put_skill_md("pdf", PDF).unwrap();
        w.put_skill_md("pdf", PDF).unwrap();
        w.put_skill_file("pdf", "a/b.txt", b"x").unwrap();
        w.delete_skill_file("pdf", "a").unwrap();
        w.export_skill_zip("pdf").unwrap();
        let z = w.export_skill_zip("pdf").unwrap();
        w.import_skill_zip(&z, true).unwrap();
        w.delete_skill("pdf").unwrap();
        w.import_skill_zip(&z, false).unwrap();
        // A refused write records nothing.
        assert!(w.put_skill_md("pdf", "nope").is_err());
    }
    let got = skill_entries(&w);
    let summaries: Vec<&str> = got.iter().map(|(_, _, s)| s.as_str()).collect();
    assert_eq!(
        summaries,
        [
            "Created skill pdf",
            "Changed skill pdf",
            "Changed skill pdf: wrote a/b.txt",
            "Changed skill pdf: deleted a",
            "Replaced skill pdf from a zip",
            "Deleted skill pdf",
            "Imported skill pdf from a zip",
        ]
    );
    assert!(
        got.iter()
            .all(|(k, actor, _)| k == "skill.change" && actor == "owner")
    );
    // The commit's own definition entries leave skill files out.
    let defs = w
        .state()
        .activity(&ActivityFilter {
            category: Some("definitions".into()),
            ..Default::default()
        })
        .unwrap()
        .items;
    assert!(defs.iter().all(|a| a.kind == "skill.change"), "{defs:?}");
}
