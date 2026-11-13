//! Narrow Settings edit. Never canonicalize the rest of a classifier definition.
use schema::Classifier;
use store::Change;
use wcl_lang::ast::Item;

use crate::{Error, Result, Writer};

impl Writer {
    /// Change only the top-level account field, under the writer lock. No note hooks run.
    pub fn set_classifier_account(&self, name: &str, account: &str) -> Result<()> {
        if !schema::is_name(name) {
            return Err(Error::BadRequest("Invalid classifier name".into()));
        }
        self.commit_checked(|w| {
            let path = format!("classifiers/{name}.wcl");
            let source = w.vault().read_text(&path)?;
            let text = account_source(name, &source, account)?;
            Ok::<_, Error>(vec![Change::ClassifierAccount { path, contents: text.into_bytes() }])
        })
    }
}

fn account_source(name: &str, source: &str, account: &str) -> Result<String> {
    Classifier::parse(name, source).map_err(Error::Invalid)?;
    let ast = store::parse_data_only(source, "classifier")?;
    let value = serde_json::to_string(account).expect("string");
    let field = format!("account = {value}");
    let mut result = source.to_owned();
    if let Some(field_ast) = ast.items.iter().find_map(|item| match item {
        Item::Field(f) if f.name == "account" => Some(f),
        _ => None,
    }) {
        result.replace_range(field_ast.span.start..field_ast.span.end, &field);
    } else {
        // Prepending leaves every existing byte (including a missing final newline) alone.
        result.insert_str(0, &format!("{field}\n"));
    }
    Classifier::parse(name, &result).map_err(Error::Invalid)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFINITION: &str = "# untouched\r\ntypes = [\"memo\"]\r\nmodel = \"jev\"\r\nfields = [\"title\"]\r\nquestion scam { type = \"noul\" instructions = \"account = fake\" target = \"scam\" }";

    #[test]
    fn preserves_every_byte_outside_account_field() {
        let source = format!("# before\naccount  = \"old\" # after\n{DEFINITION}");
        assert_eq!(
            account_source("scam", &source, "OpenRouter").unwrap(),
            format!("# before\naccount = \"OpenRouter\" # after\n{DEFINITION}")
        );
        assert_eq!(
            account_source("scam", DEFINITION, "OpenRouter").unwrap(),
            format!("account = \"OpenRouter\"\n{DEFINITION}")
        );
    }

    #[test]
    fn quotes_account_and_refuses_invalid_definition() {
        let text = account_source("scam", DEFINITION, "Router \"two\"").unwrap();
        assert_eq!(
            Classifier::parse("scam", &text).unwrap().account.as_deref(),
            Some("Router \"two\"")
        );
        assert!(account_source("scam", "broken", "OpenRouter").is_err());
    }
}
