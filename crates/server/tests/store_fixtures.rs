//! The SQLite `RowSource` must give exactly the results of the reference in-memory one: run the shared ops
//! fixtures (fixtures/ops.json) through `SqliteStore::apply`.
use ops::{Change, Op, Row, RowChange};
use serde::Deserialize;
use server::store::{ApplyError, SqliteStore, Store};

#[derive(Deserialize)]
struct State {
    rows: Vec<Row>,
    changes: Vec<Change>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    state: State,
    op: Op,
    rows: Option<Vec<RowChange>>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct File {
    cases: Vec<Case>,
}

#[test]
fn ops_fixtures_through_sqlite() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/ops.json");
    let file: File = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("store-fixtures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (i, c) in file.cases.iter().enumerate() {
        let db = dir.join(format!("{i}.db"));
        let _ = std::fs::remove_file(&db);
        let store = SqliteStore::open(&db, Box::new(|_, _| {})).unwrap();
        store.create_owner("o1", 0).unwrap();
        // A second owner with the same ids must be invisible to o1.
        store.create_owner("o2", 0).unwrap();
        store.import("o1", &c.state.rows, &c.state.changes).unwrap_or_else(|e| panic!("{}: import: {e}", c.name));
        let max_seq = c.state.changes.iter().map(|c| c.seq).max().unwrap_or(0);
        match (store.apply("o1", &c.op, 0), &c.error) {
            (Err(ApplyError::Op(e)), Some(code)) => assert_eq!(e.code(), code, "{}", c.name),
            (Ok(a), None) => {
                assert_eq!(&a.change.rows, c.rows.as_ref().unwrap(), "{}", c.name);
                assert_eq!(a.change.seq, max_seq + 1, "{}: next seq", c.name);
                assert!(!a.duplicate);
                let again = store.apply("o1", &c.op, 0).unwrap();
                assert!(again.duplicate && again.change == a.change, "{}: idempotent by op_id", c.name);
                assert_eq!(store.changes_since("o1", max_seq, 10).unwrap(), vec![a.change], "{}", c.name);
                assert!(store.changes_since("o2", 0, 10).unwrap().is_empty(), "{}: owners are isolated", c.name);
            }
            (got, want) => panic!("{}: expected {want:?}, got {got:?}", c.name),
        }
    }
}
