//! Which notes use each asset: from asset fields and from `image` and `asset` blocks at any
//! depth, kept in step with every write, delete and rebuild. Cleaning up deletes only what
//! nothing uses, a note in the trash counts as a user, and a deleted asset can be restored.

use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Input, Writer};

const DOC: &str = r#"label = "Doc"

field cover {
  kind = "asset"
}

field files {
  kind     = "asset"
  multiple = true
}

field body {
  kind = "content"
}

field board {
  kind = "canvas"
}

template default {
  tab "Main" {
    slot "title"
    slot "cover"
    slot "body"
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/doc.wcl"), DOC).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn png(w: &Writer, name: &str) -> String {
    w.put_asset(name, b"\x89PNG\r\n\x1a\nbytes".to_vec())
        .unwrap()
        .id
}

fn create(w: &Writer, wcl: String) -> String {
    w.create("doc", &Input::Wcl(wcl), &author()).unwrap().id
}

/// `(note, field, via)` for every use of `asset`.
fn uses(w: &Writer, asset: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<_> = w
        .asset_usage(asset)
        .unwrap()
        .notes
        .into_iter()
        .flat_map(|n| {
            n.how
                .into_iter()
                .map(move |h| (n.note.clone(), h.field, h.via))
        })
        .collect();
    out.sort();
    out
}

fn body_with(in_columns: &str, in_card: &str) -> String {
    format!(
        r#"
content body {{
  columns {{
    column {{
      image {{
        asset = "{in_columns}"
      }}
    }}

    column {{
      asset {{
        asset = "{in_columns}"
      }}
    }}
  }}
}}

canvas board {{
  viewport {{
    x    = 0.0
    y    = 0.0
    zoom = 1.0
  }}

  node card_one {{
    kind   = "card"
    x      = 0.0
    y      = 0.0
    width  = 200.0
    height = 100.0

    image {{
      asset = "{in_card}"
    }}
  }}
}}
"#
    )
}

#[test]
fn uses_come_from_fields_and_blocks_at_any_depth() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let cover = png(&w, "cover.png");
    let nested = png(&w, "nested.png");
    let card = png(&w, "card.png");
    let listed = png(&w, "listed.png");
    let lone = png(&w, "lone.png");

    let note = create(
        &w,
        format!(
            "title = \"Everything\"\ncover = \"{cover}\"\nfiles = [\"{listed}\", \"{cover}\"]\n{}",
            body_with(&nested, &card)
        ),
    );

    assert_eq!(
        uses(&w, &cover),
        vec![
            (note.clone(), "cover".into(), "field".into()),
            (note.clone(), "files".into(), "field".into()),
        ]
    );
    assert_eq!(
        uses(&w, &nested),
        vec![
            (note.clone(), "body".into(), "asset".into()),
            (note.clone(), "body".into(), "image".into()),
        ]
    );
    assert_eq!(
        uses(&w, &card),
        vec![(note.clone(), "board".into(), "image".into())]
    );
    assert_eq!(uses(&w, &listed).len(), 1);
    assert!(w.asset_usage(&lone).unwrap().is_unused());

    let usage = w.asset_usage(&cover).unwrap();
    assert_eq!(usage.notes[0].title, "Everything");
    assert_eq!(usage.notes[0].type_name, "doc");

    let mut counts: Vec<(String, usize)> = w
        .asset_summaries()
        .unwrap()
        .into_iter()
        .map(|s| (s.asset.filename.clone(), s.used_by()))
        .collect();
    counts.sort();
    assert_eq!(
        counts,
        vec![
            ("card.png".into(), 1),
            ("cover.png".into(), 1),
            ("listed.png".into(), 1),
            ("lone.png".into(), 0),
            ("nested.png".into(), 1),
        ]
    );
}

#[test]
fn uses_follow_writes_deletes_and_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let a = png(&w, "a.png");
    let b = png(&w, "b.png");

    let note = create(&w, format!("title = \"One\"\ncover = \"{a}\"\n"));
    let other = create(&w, format!("title = \"Two\"\n{}", body_with(&a, &b)));
    assert_eq!(w.asset_usage(&a).unwrap().notes.len(), 2);

    // An update moving the cover moves the use.
    let base = w.read(&note).unwrap().version;
    w.update(
        &note,
        &Input::Wcl(format!("title = \"One\"\ncover = \"{b}\"\n")),
        Some(&base),
        &author(),
    )
    .unwrap();
    assert_eq!(uses(&w, &a).len(), 2);
    assert!(uses(&w, &a).iter().all(|(n, _, _)| *n == other));
    assert!(uses(&w, &b).iter().any(|(n, _, _)| *n == note));

    // A rebuild comes to the same uses as the writes did.
    let before = w.read_index(|index, _| index.dump().unwrap());
    drop(w);
    let server = dir.path().join(".vault");
    std::fs::remove_file(server.join("index.db")).unwrap();
    std::fs::remove_dir_all(server.join("search")).unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = Writer::open(vault, state).unwrap();
    let after = w.read_index(|index, _| index.dump().unwrap());
    assert_eq!(before, after);
    assert!(after.contains(&format!("\"{other}\"\t\"{a}\"\t\"body\"\t\"image\"")));

    // A deleted note uses nothing live, but counts from the trash.
    let base = w.read(&other).unwrap().version;
    w.delete(&other, Some(&base), &author()).unwrap();
    let usage = w.asset_usage(&a).unwrap();
    assert!(usage.notes.is_empty());
    assert_eq!(usage.trashed.len(), 1);
    assert_eq!(usage.trashed[0].id, other);
    assert!(!usage.is_unused());
}

#[test]
fn a_note_in_the_trash_protects_its_assets() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let a = png(&w, "kept.png");
    let note = create(&w, format!("title = \"Gone\"\ncover = \"{a}\"\n"));
    let base = w.read(&note).unwrap().version;
    w.delete(&note, Some(&base), &author()).unwrap();

    let err = w.delete_asset(&a, true, &author()).unwrap_err();
    assert!(
        matches!(&err, writer::Error::InUse { code: "asset_in_trash", by, .. } if by == &vec![format!("trash/doc/{note}")]),
        "{err:?}"
    );
    assert!(w.cleanup_assets(None, true).unwrap().is_empty());
    assert!(w.cleanup_assets(None, false).unwrap().is_empty());
    assert!(w.asset_info(&a).is_ok());
    assert_eq!(w.trashed_title(&note).as_deref(), Some("Gone"));

    // Restored, the note uses it again as a live note.
    w.restore_from_trash(&note, &author()).unwrap();
    assert_eq!(w.asset_usage(&a).unwrap().notes.len(), 1);
}

#[test]
fn cleanup_deletes_only_unused_assets_and_they_can_be_restored() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let used = png(&w, "used.png");
    let spare = png(&w, "spare.png");
    let other = w.put_asset("notes.txt", b"hello".to_vec()).unwrap().id;
    create(&w, format!("title = \"Uses\"\ncover = \"{used}\"\n"));

    // A type file naming an asset keeps it too.
    let typed = png(&w, "typed.png");
    let mut doc = std::fs::read_to_string(dir.path().join("types/doc.wcl")).unwrap();
    doc.push_str(&format!(
        "\nfield desk {{\n  kind  = \"location\"\n  map   = \"image\"\n  image = \"{typed}\"\n}}\n"
    ));
    std::fs::write(dir.path().join("types/doc.wcl"), doc).unwrap();
    w.reload_schema().unwrap();
    assert_eq!(w.asset_usage(&typed).unwrap().types, vec!["doc".to_owned()]);

    let dry = w.cleanup_assets(None, true).unwrap();
    let mut names: Vec<_> = dry.iter().map(|a| a.filename.as_str()).collect();
    names.sort();
    assert_eq!(names, ["notes.txt", "spare.png"]);
    assert!(w.asset_info(&spare).is_ok(), "a dry run deletes nothing");

    // Narrowed to the ids a dry run listed, a used asset among them is still kept.
    let ids = vec![spare.clone(), used.clone()];
    let gone = w.cleanup_assets(Some(&ids), false).unwrap();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, spare);
    assert!(w.asset_info(&spare).is_err());
    assert!(!dir.path().join("assets").join(&spare).exists());
    assert!(w.asset_info(&used).is_ok());
    assert!(w.asset_info(&other).is_ok());

    let trash = w.asset_trash();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].asset.filename, "spare.png");
    assert_eq!(
        trash[0].purge_ms - trash[0].deleted_ms,
        writer::ASSET_TRASH_DAYS * 24 * 60 * 60 * 1000
    );

    let back = w.restore_asset(&spare).unwrap();
    assert_eq!(back.filename, "spare.png");
    assert_eq!(w.asset(&spare).unwrap().1, b"\x89PNG\r\n\x1a\nbytes");
    assert!(w.asset_trash().is_empty());
    assert!(w.restore_asset(&spare).is_err());

    // A single delete goes to the asset trash as well.
    w.delete_asset(&other, false, &author()).unwrap();
    assert_eq!(w.asset_trash()[0].asset.id, other);
    w.restore_asset(&other).unwrap();
    assert_eq!(w.asset(&other).unwrap().1, b"hello");
}

#[test]
fn a_rename_keeps_the_id_and_the_notes() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let a = png(&w, "old.png");
    let note = create(&w, format!("title = \"N\"\ncover = \"{a}\"\n"));
    let renamed = w.rename_asset(&a, "new name.png").unwrap();
    assert_eq!(renamed.path(), format!("assets/{a}/new name.png"));
    assert_eq!(w.asset_info(&a).unwrap().filename, "new name.png");
    assert!(!dir.path().join(format!("assets/{a}/old.png")).exists());
    assert_eq!(w.asset_usage(&a).unwrap().notes[0].note, note);
    assert!(w.rename_asset(&a, "a/b.png").is_err());
}
