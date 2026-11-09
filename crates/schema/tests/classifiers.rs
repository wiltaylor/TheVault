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
fn notification_requires_a_probability_not_confidence() {
    let choice = CONFIG.replace(
        "type = \"noul\"",
        "type = \"choice\"\n criteria = { scam: \"Scam\", personal: \"Personal\" }",
    );
    assert!(Classifier::parse("incoming", &choice).is_err());
    let choice = choice.replace("threshold = 0.8", "threshold = 0.8\n option = \"scam\"");
    assert_eq!(
        Classifier::parse("incoming", &choice)
            .unwrap()
            .notify
            .unwrap()
            .option
            .as_deref(),
        Some("scam")
    );
    assert!(
        Classifier::parse(
            "incoming",
            &choice.replace("option = \"scam\"", "option = \"unknown\"")
        )
        .is_err()
    );
    assert!(
        Classifier::parse(
            "incoming",
            &CONFIG.replace("threshold = 0.8", "threshold = 0.8\n option = \"scam\"")
        )
        .is_err()
    );
    assert!(
        Classifier::parse(
            "incoming",
            &CONFIG.replace(
                "type = \"noul\"",
                "type = \"score\"\n criteria = [\"Low\", \"High\"]"
            )
        )
        .is_err()
    );
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
