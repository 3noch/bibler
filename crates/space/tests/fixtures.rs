//! The SQL `RowSource` must give exactly the results of the reference in-memory one: run the shared ops fixtures
//! (fixtures/ops.json) through `space::apply` on SQLite. Plus the multi-owner rules of a shared space.
use ops::{Change, Op, Row, RowChange};
use rusqlite::Connection;
use serde::Deserialize;
use space::{apply, changes_since, import, init, members, ApplyError};

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

fn open() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    init(&db).unwrap();
    db
}

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).unwrap()
}

#[test]
fn ops_fixtures_through_sqlite() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/ops.json");
    let file: File = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for c in &file.cases {
        let db = open();
        init(&db).unwrap(); // idempotent
        import(&db, "o1", &c.state.rows, &c.state.changes).unwrap_or_else(|e| panic!("{}: import: {e}", c.name));
        let max_seq = c.state.changes.iter().map(|c| c.seq).max().unwrap_or(0);
        // No transaction: Durable Objects have no rollback, so a rejected op must not have written anything.
        let writes = db.total_changes();
        match (apply(&db, "o1", &c.op, 0), &c.error) {
            (Err(ApplyError::Op(e)), Some(code)) => {
                assert_eq!(e.code(), code, "{}", c.name);
                assert_eq!(db.total_changes(), writes, "{}: a rejected op wrote", c.name);
            }
            (Ok(a), None) => {
                assert_eq!(&a.change.rows, c.rows.as_ref().unwrap(), "{}", c.name);
                assert_eq!(a.change.seq, max_seq + 1, "{}: next seq", c.name);
                assert!(!a.duplicate);
                let again = apply(&db, "o1", &c.op, 0).unwrap();
                assert!(again.duplicate && again.change == a.change, "{}: idempotent by op_id", c.name);
                assert_eq!(changes_since(&db, max_seq, 10).unwrap(), vec![a.change], "{}", c.name);
                // Another member of the same space sees none of o1's rows.
                let mut other = c.op.clone();
                other.op_id = format!("{}-o2", other.op_id);
                if let Ok(b) = apply(&db, "o2", &other, 0) {
                    assert!(b.change.rows.iter().all(|r| r.before.is_none()), "{}: o2 changed o1's rows", c.name);
                }
            }
            (got, want) => panic!("{}: expected {want:?}, got {got:?}", c.name),
        }
    }
}

#[test]
fn a_shared_space_has_one_seq_and_owner_scoped_rows() {
    let db = open();
    let tag = |op_id: &str, id: &str, name: &str| {
        op(
            serde_json::json!({"op_id": op_id, "kind": "tag_create", "id": id, "name": name, "color": "#123456", "now": 1}),
        )
    };
    assert_eq!(apply(&db, "alice", &tag("op1", "A", "Grace"), 1).unwrap().change.seq, 1);
    // Same op id and same tag name from another member: both fine (op ids and names are per owner).
    assert_eq!(apply(&db, "bob", &tag("op1", "B", "Grace"), 2).unwrap().change.seq, 2);
    assert_eq!(apply(&db, "alice", &tag("op2", "A2", "Love"), 3).unwrap().change.seq, 3);
    assert_eq!(changes_since(&db, 0, 10).unwrap().iter().map(|c| c.seq).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(changes_since(&db, 2, 10).unwrap()[0].op_id, "op2");
    // Ids are unique within the space, and other members' rows and ops are invisible.
    assert!(
        matches!(apply(&db, "bob", &tag("op3", "A", "Mine"), 4), Err(ApplyError::Op(e)) if e.code() == "id_exists")
    );
    let paint = op(
        serde_json::json!({"op_id": "p", "kind": "paint", "tag": "A", "text_id": "t", "start": 1, "end": 2, "new_id": "g", "now": 5}),
    );
    assert!(matches!(apply(&db, "bob", &paint, 5), Err(ApplyError::Op(e)) if e.code() == "unknown_tag"));
    let revert = op(serde_json::json!({"op_id": "r", "kind": "revert", "target": "op2", "now": 6}));
    assert!(matches!(apply(&db, "bob", &revert, 6), Err(ApplyError::Op(e)) if e.code() == "unknown_op"));
    // A failed op wrote nothing, not even the membership of a newcomer.
    assert!(apply(&db, "carol", &paint, 7).is_err());
    assert_eq!(changes_since(&db, 0, 10).unwrap().len(), 3);
    assert_eq!(members(&db).unwrap(), ["alice", "bob"]);
}
