//! A type's `list_page` block: the view, sort and page size of its list page (lists L1).

use schema::{ListPage, TypeDef};

const BASE: &str = r#"label = "Email"

field sent {
  kind = "datetime"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

fn with(block: &str) -> Result<TypeDef, Vec<String>> {
    TypeDef::parse("email", &format!("{BASE}\n{block}")).map_err(|e| {
        e.issues
            .iter()
            .map(|i| format!("{}: {}", i.at, i.message))
            .collect()
    })
}

#[test]
fn a_type_without_the_block_has_no_list_page_settings() {
    assert_eq!(TypeDef::parse("email", BASE).unwrap().list_page, None);
}

#[test]
fn the_list_page_settings_parse() {
    let t = with("list_page {\n  view      = \"details\"\n  sort      = \"sent desc\"\n  page_size = 50\n}\n")
        .unwrap();
    let lp = t.list_page.unwrap();
    assert_eq!(
        lp,
        ListPage {
            view: Some("details".to_owned()),
            sort: Some(("sent".to_owned(), true)),
            page_size: Some(50),
        }
    );
    assert_eq!(lp.sort_text().as_deref(), Some("sent desc"));
    assert!(t.warnings.is_empty());
}

#[test]
fn every_item_is_optional_and_a_sort_without_a_direction_is_ascending() {
    let lp = with("list_page {\n}\n").unwrap().list_page.unwrap();
    assert_eq!(lp, ListPage::default());
    for (sort, field, desc) in [
        ("title", "title", false),
        ("updated asc", "updated", false),
        ("created desc", "created", true),
        ("sent", "sent", false),
    ] {
        let lp = with(&format!("list_page {{\n  sort = \"{sort}\"\n}}\n"))
            .unwrap()
            .list_page
            .unwrap();
        assert_eq!(lp.sort, Some((field.to_owned(), desc)), "{sort}");
    }
    for view in ["cards", "list", "details"] {
        let lp = with(&format!("list_page {{\n  view = \"{view}\"\n}}\n"))
            .unwrap()
            .list_page
            .unwrap();
        assert_eq!(lp.view.as_deref(), Some(view));
    }
}

#[test]
fn a_bad_view_is_an_error() {
    let errors = with("list_page {\n  view = \"table\"\n}\n").unwrap_err();
    assert!(
        errors.iter().any(|e| e.starts_with("list_page.view")),
        "{errors:?}"
    );
}

#[test]
fn a_sort_by_an_unknown_field_is_an_error() {
    let errors = with("list_page {\n  sort = \"received desc\"\n}\n").unwrap_err();
    assert_eq!(
        errors,
        [
            r#"list_page.sort: names "received", which is not a field of the type, nor title, created or updated"#
        ]
    );
    let errors = with("list_page {\n  sort = \"sent down\"\n}\n").unwrap_err();
    assert!(errors[0].starts_with("list_page.sort"), "{errors:?}");
    let errors = with("list_page {\n  sort = \"\"\n}\n").unwrap_err();
    assert!(errors[0].starts_with("list_page.sort"), "{errors:?}");
}

#[test]
fn a_page_size_outside_10_to_500_is_an_error() {
    for size in ["9", "501", "0", "-5"] {
        let errors = with(&format!("list_page {{\n  page_size = {size}\n}}\n")).unwrap_err();
        assert!(
            errors.iter().any(|e| e.starts_with("list_page.page_size")),
            "{size}: {errors:?}"
        );
    }
    let errors = with("list_page {\n  page_size = \"50\"\n}\n").unwrap_err();
    assert!(
        errors.iter().any(|e| e.starts_with("list_page.page_size")),
        "{errors:?}"
    );
    for size in [10, 500] {
        let lp = with(&format!("list_page {{\n  page_size = {size}\n}}\n"))
            .unwrap()
            .list_page
            .unwrap();
        assert_eq!(lp.page_size, Some(size));
    }
}

#[test]
fn an_unknown_item_a_label_or_a_second_block_is_an_error() {
    assert!(with("list_page {\n  columns = 3\n}\n").is_err());
    assert!(with("list_page \"x\" {\n}\n").is_err());
    assert!(with("list_page {\n}\n\nlist_page {\n}\n").is_err());
}
