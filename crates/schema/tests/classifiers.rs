use schema::Classifier;

const CONFIG: &str = r#"
types = ["memo"]
where = "status == \"incoming\""
account = "router"
model = "typesafe/jev-1.13"
fields = ["title", "body"]
question scam {
  type = "noul"
  instructions = "Is this a scam?"
  target = "scam_probability"
}
notify {
  question = "scam"
  threshold = 0.8
  title = "Possible scam"
}
"#;

#[test]
fn reads_classifier() {
    let c = Classifier::parse("incoming", CONFIG).unwrap();
    assert_eq!(c.types, ["memo"]);
    assert_eq!(c.fields, ["title", "body"]);
    assert_eq!(c.questions[0].target, "scam_probability");
    assert_eq!(c.notify.unwrap().threshold, 0.8);
}

#[test]
fn rejects_invalid_classifier_definitions() {
    for source in [
        CONFIG.replace("0.8", "1.8"),
        CONFIG.replace("scam_probability", "title"),
        CONFIG.replace("type = \"noul\"", "type = \"unknown\""),
        CONFIG.replace("question = \"scam\"", "question = \"absent\""),
        CONFIG.replace("status ==", "status ==== "),
        format!("{CONFIG}\nunknown = true\n"),
        format!("{CONFIG}\nmodel = \"duplicate\"\n"),
        CONFIG.replace("fields = [\"title\", \"body\"]", "fields = []"),
        CONFIG.replace("instructions = \"Is this a scam?\"", "instructions = 12"),
    ] {
        assert!(Classifier::parse("incoming", &source).is_err(), "{source}");
    }
}

#[test]
fn rejects_executable_wcl_and_duplicate_targets() {
    assert!(Classifier::parse("incoming", "import \"elsewhere.wcl\"").is_err());
    assert!(Classifier::parse("../incoming", CONFIG).is_err());
    let source = format!(
        "{CONFIG}\nquestion second {{ type = \"noul\" instructions = \"Test\" target = \"scam_probability\" }}"
    );
    assert!(Classifier::parse("incoming", &source).is_err());
}
