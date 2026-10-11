use normalize::{normalize, Changeset, Intent, Tagging};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    existing: Vec<Tagging>,
    intent: Intent,
    #[serde(default)]
    expected: Option<Changeset>,
    #[serde(default)]
    expect_error: Option<String>,
}

#[derive(Deserialize)]
struct File {
    cases: Vec<Case>,
}

#[test]
fn shared_fixtures() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/normalize.json");
    let raw = std::fs::read_to_string(path).expect("read fixtures");
    let file: File = serde_json::from_str(&raw).expect("parse fixtures");
    assert!(!file.cases.is_empty());

    for c in file.cases {
        let got = normalize(&c.existing, &c.intent);
        match (&c.expect_error, got) {
            (Some(code), Err(e)) => assert_eq!(e.code(), code.as_str(), "{}", c.name),
            (Some(code), Ok(_)) => panic!("{}: expected error {code}", c.name),
            (None, Ok(cs)) => assert_eq!(cs, c.expected.clone().unwrap_or_default(), "{}", c.name),
            (None, Err(e)) => panic!("{}: unexpected error {e}", c.name),
        }
    }
}
