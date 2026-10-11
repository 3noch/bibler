mod common;

use common::Mem;
use ops::{effect, Op, RowChange};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    state: Mem,
    op: Op,
    rows: Option<Vec<RowChange>>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct File {
    cases: Vec<Case>,
}

#[test]
fn shared_fixtures() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/ops.json");
    let file: File = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(file.cases.len() >= 40);
    for c in file.cases {
        let got = effect(&c.state, &c.op);
        match (c.error, got) {
            (Some(code), Err(e)) => assert_eq!(e.code(), code, "{}", c.name),
            (Some(code), Ok(ch)) => panic!("{}: expected {code}, got {ch:?}", c.name),
            (None, Ok(ch)) => {
                assert_eq!(ch.rows, c.rows.unwrap(), "{}", c.name);
                assert_eq!((ch.seq, ch.op_id.as_str(), ch.kind.as_str()), (0, "op", c.op.intent.kind()), "{}", c.name);
            }
            (None, Err(e)) => panic!("{}: unexpected {e}", c.name),
        }
    }
}
