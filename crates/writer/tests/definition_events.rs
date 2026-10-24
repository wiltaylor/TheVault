//! Changes to the vault's definitions reach open clients: `definition.changed` on the live stream
//! for a write on the server, and a count a device's web client reads for a sync round's pull.

use state::{LiveEvent, State};
use store::Vault;
use writer::Writer;

const IDEA: &str =
    "label = \"Idea\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";
const SKILL: &str = "---\nname: pdf\ndescription: Read PDFs.\n---\nUse it.\n";

fn open(root: &std::path::Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

/// Every `DefinitionChanged` waiting on `sub`, as `(kind, name, deleted)`.
fn definitions(sub: &mut state::Subscription) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    while let Some(Ok((_, event))) = sub.try_recv() {
        if let LiveEvent::DefinitionChanged {
            kind,
            name,
            deleted,
        } = event
        {
            out.push((kind, name, deleted));
        }
    }
    out
}

#[test]
fn a_sync_round_pulling_definitions_is_announced_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let device = open(dir.path());
    let mut sub = device.state().events().subscribe();
    let before = device.definition_changes();

    let loaded = device
        .apply_remote_files(&[
            ("types/idea.wcl".to_owned(), Some(IDEA.as_bytes().to_vec())),
            (
                "agents/triage.wcl".to_owned(),
                Some(b"label = \"Triage\"\n".to_vec()),
            ),
            (
                "skills/pdf/SKILL.md".to_owned(),
                Some(SKILL.as_bytes().to_vec()),
            ),
        ])
        .unwrap();
    assert!(loaded);
    let mut got = definitions(&mut sub);
    got.sort();
    assert_eq!(
        got,
        [
            ("agent".to_owned(), "triage".to_owned(), false),
            ("skill".to_owned(), "pdf".to_owned(), false),
            ("type".to_owned(), "idea".to_owned(), false),
        ]
    );
    assert_eq!(device.definition_changes(), before + 3);
    // By the time the count moves, the type is in the schema the API answers from.
    assert!(device.schema().types().any(|t| t.name == "idea"));

    device
        .apply_remote_files(&[("agents/triage.wcl".to_owned(), None)])
        .unwrap();
    assert_eq!(
        definitions(&mut sub),
        [("agent".to_owned(), "triage".to_owned(), true)]
    );
    assert_eq!(device.definition_changes(), before + 4);

    // A write on the server says so the same way.
    device
        .put_type("idea", &IDEA.replace("Idea", "Big idea"))
        .unwrap();
    assert_eq!(
        definitions(&mut sub),
        [("type".to_owned(), "idea".to_owned(), false)]
    );
    assert_eq!(device.definition_changes(), before + 5);
}
