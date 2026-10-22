//! The field names in templates' query views: a rename in a type change renames them, in the
//! type's own templates and in other types' templates that query it, and saving a template
//! refuses a query view that names a field no queried type has (ticket 120).

use std::collections::BTreeMap;
use std::path::Path;

use state::{Author, State};
use store::Vault;
use writer::{Error, Input, TypeChange, Writer};

const INVOICE: &str = r#"label = "Invoice"

field amount {
  kind = "int"
}

field paid {
  kind = "bool"
}

field client {
  kind   = "reference"
  target = "client"
}

template default {
  tab "Main" {
    slot "amount"
    query_view {
      query = "from invoice where !paid && amount > this.amount sort amount desc"

      view table {
        kind    = "table"
        columns = ["title", "amount", "client.amount"]
      }
    }
  }
}
"#;

const CLIENT: &str = r#"label = "Client"

field amount {
  kind = "int"
}

template default {
  tab "Main" {
    slot "amount"
    query_view {
      query = "from invoice where client == this && amount > this.amount sort amount"

      view table {
        kind    = "table"
        columns = ["title", "amount"]
      }
    }
    query_view {
      query = "from client sort amount"

      view table {
        kind    = "table"
        columns = ["amount"]
      }
    }
  }
}
"#;

fn open(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/invoice.wcl"), INVOICE).unwrap();
    std::fs::write(root.join("types/client.wcl"), CLIENT).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn author() -> Author {
    Author::Session("test".into())
}

fn read(root: &Path, file: &str) -> String {
    std::fs::read_to_string(root.join(file)).unwrap()
}

#[test]
fn a_field_rename_renames_the_field_in_template_query_views_that_read_the_type() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    w.create(
        "invoice",
        &Input::Wcl("title = \"March\"\namount = 5\n".into()),
        &author(),
    )
    .unwrap();

    // The submitted file renames the field and its slot, and leaves the query view as it was.
    let wcl = INVOICE
        .replace("field amount {", "field total {")
        .replace("slot \"amount\"", "slot \"total\"");
    let change = TypeChange {
        wcl,
        renames: BTreeMap::from([("amount".to_owned(), "total".to_owned())]),
        migration: None,
    };
    let plan = w.plan_type_change("invoice", &change).unwrap();
    assert_eq!(plan.retemplated, ["client"]);
    assert!(
        plan.wcl
            .contains("query = \"from invoice where !paid && total > this.total sort total desc\""),
        "{}",
        plan.wcl
    );
    // A member read through a reference is another type's field.
    assert!(
        plan.wcl
            .contains("columns = [\"title\", \"total\", \"client.amount\"]"),
        "{}",
        plan.wcl
    );
    // Planning writes nothing.
    assert!(read(root, "types/client.wcl").contains("sort amount\""));

    w.apply_type_change("invoice", &change, &plan.token, &author())
        .unwrap();
    let invoice = read(root, "types/invoice.wcl");
    assert!(!invoice.contains("\"amount\""), "{invoice}");
    let client = read(root, "types/client.wcl");
    // The client's view of invoices follows; `this` there is a client, whose own `amount` stays,
    // and so does the client's view of clients.
    assert!(
        client.contains(
            "query = \"from invoice where client == this && total > this.amount sort total\""
        ),
        "{client}"
    );
    assert!(
        client.contains("columns = [\"title\", \"total\"]"),
        "{client}"
    );
    assert!(
        client.contains("query = \"from client sort amount\""),
        "{client}"
    );
    assert!(client.contains("columns = [\"amount\"]"), "{client}");
    assert_eq!(w.schema().type_def("client").unwrap().templates.len(), 1);
}

#[test]
fn a_type_change_without_renames_leaves_other_types_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    let change = TypeChange {
        wcl: INVOICE.replace("label = \"Invoice\"", "label = \"Bill\""),
        renames: BTreeMap::new(),
        migration: None,
    };
    let plan = w.plan_type_change("invoice", &change).unwrap();
    assert!(plan.retemplated.is_empty());
    w.apply_type_change("invoice", &change, &plan.token, &author())
        .unwrap();
    assert_eq!(read(root, "types/client.wcl"), CLIENT);
}

fn card(query: &str, view: &str) -> String {
    format!(
        "template card {{\n  tab \"Card\" {{\n    slot \"title\"\n    query_view {{\n      query = {query:?}\n\n      view v {{\n{view}\n      }}\n    }}\n  }}\n}}\n"
    )
}

fn refused(w: &Writer, source: &str) -> String {
    match w.put_template("invoice", "card", source) {
        Err(e @ Error::InvalidType(_)) => e.to_string(),
        other => panic!("expected the template to be refused, got {other:?}"),
    }
}

#[test]
fn template_put_refuses_query_view_fields_no_queried_type_has() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let table = "        kind    = \"table\"\n        columns = [\"title\", \"amount\"]";

    let err = refused(
        &w,
        &card(
            "from invoice",
            "        kind    = \"table\"\n        columns = [\"title\", \"nonexistent\"]",
        ),
    );
    assert!(
        err.contains("view \"v\" columns names `nonexistent`, which is not a field of invoice"),
        "{err}"
    );
    assert!(
        err.contains("template card > tab \"Card\" > query_view#2"),
        "{err}"
    );

    let err = refused(&w, &card("from invoice sort nope desc", table));
    assert!(
        err.contains("the query's sort names `nope`, which is not a field of invoice"),
        "{err}"
    );
    let err = refused(&w, &card("from invoice where nonexistent == 1", table));
    assert!(
        err.contains(
            "the query's where clause names `nonexistent`, which is not a field of invoice"
        ),
        "{err}"
    );
    let err = refused(
        &w,
        &card(
            "from invoice",
            "        kind = \"calendar\"\n        date = \"due\"",
        ),
    );
    assert!(err.contains("view \"v\" date names `due`"), "{err}");
    // Nothing was written.
    assert!(
        w.schema()
            .type_def("invoice")
            .unwrap()
            .template("card")
            .is_none()
    );

    // A closure parameter is not a field, so only the list it reads is reported.
    let err = refused(
        &w,
        &card("from invoice where any(tags, |t| t == \"x\")", table),
    );
    assert!(
        err.contains("names `tags`") && !err.contains("`t`"),
        "{err}"
    );

    // Fields of any listed type, built-ins, `this` and paths through a reference are fine, and
    // `from *` is not checked.
    for (query, view) in [
        ("from invoice sort amount desc", table),
        ("from invoice, client where amount > 1 sort updated", table),
        (
            "from invoice where id == this.id && client.amount > 1",
            table,
        ),
        (
            "from invoice",
            "        kind    = \"table\"\n        columns = [\"client.anything\"]",
        ),
        ("from * where whatever == 1", "        kind = \"list\""),
    ] {
        w.put_template("invoice", "card", &card(query, view))
            .unwrap_or_else(|e| panic!("{query}: {e}"));
    }
}

#[test]
fn a_note_restored_from_the_trash_keeps_its_computed_values() {
    // Found with the event type's computed `calendar_name` (ticket 103): restoring a note whose
    // computed field a task or handler wrote was refused as a client writing it.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(
        root.join("types/trip.wcl"),
        "label = \"Trip\"\n\nfield leg {\n  kind     = \"string\"\n  computed = true\n}\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n",
    )
    .unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = Writer::open(vault, state).unwrap();
    let task = Author::Task("sync".into());
    let made = w
        .create(
            "trip",
            &Input::Wcl("title = \"Coast\"\nleg = \"north\"\n".into()),
            &task,
        )
        .unwrap();
    // A client still cannot write it.
    assert!(
        w.create(
            "trip",
            &Input::Wcl("title = \"x\"\nleg = \"south\"\n".into()),
            &author()
        )
        .is_err()
    );
    w.delete(&made.id, Some(&made.version), &author()).unwrap();
    let back = w.restore_from_trash(&made.id, &author()).unwrap();
    assert!(back.wcl.contains("\"north\""), "{}", back.wcl);
}
