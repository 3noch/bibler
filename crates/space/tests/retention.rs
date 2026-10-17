//! Retention: `prune` blanks old changes a batch at a time; their op ids still dedupe, they can't be replayed or
//! reverted, and a client that is too far behind gets a `reset` instead.
use ops::{Op, OpError, Row};
use rusqlite::Connection;
use space::{apply, changes_since, init, prune, pruned_through, reset, ApplyError};

fn open() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    init(&db).unwrap();
    db
}

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).unwrap()
}

fn tag(n: u32) -> Op {
    op(
        serde_json::json!({"op_id": format!("o{n}"), "kind": "tag_create", "id": format!("T{n}"), "name": format!("Tag {n}"), "color": "#123456", "now": 1}),
    )
}

/// Ops o1..=o`n`, op `i` applied at time `i * 10`.
fn space_with(n: u32) -> Connection {
    let db = open();
    for i in 1..=n {
        apply(&db, "me", &tag(i), i64::from(i) * 10).unwrap();
    }
    db
}

#[test]
fn prunes_the_oldest_changes_in_bounded_batches() {
    let db = space_with(10);
    assert_eq!(pruned_through(&db).unwrap(), 0);
    assert_eq!(prune(&db, 75, 3).unwrap(), 3, "at most a batch");
    assert_eq!(pruned_through(&db).unwrap(), 3);
    assert_eq!(prune(&db, 75, 3).unwrap(), 3);
    assert_eq!(prune(&db, 75, 3).unwrap(), 1, "stops at the first change that is too new (seq 8 at time 80)");
    assert_eq!(pruned_through(&db).unwrap(), 7);
    assert_eq!(prune(&db, 75, 3).unwrap(), 0);
    // Replay works from the pruning point on, and refuses to go back further.
    assert_eq!(changes_since(&db, 7, 100).unwrap().iter().map(|c| c.seq).collect::<Vec<_>>(), vec![8, 9, 10]);
    assert!(changes_since(&db, 6, 100).is_err());
}

#[test]
fn a_pruned_op_is_still_a_duplicate() {
    let db = space_with(3);
    prune(&db, 1000, 64).unwrap();
    let writes = db.total_changes();
    let again = apply(&db, "me", &tag(2), 2000).unwrap();
    assert!(again.duplicate);
    assert_eq!((again.change.seq, again.change.kind.as_str(), again.change.rows.len()), (2, "tag_create", 0));
    assert_eq!(db.total_changes(), writes, "nothing written");
}

#[test]
fn a_pruned_change_cannot_be_reverted() {
    let db = space_with(2);
    let undo =
        |id: &str, n: u32| op(serde_json::json!({"op_id": format!("u{n}"), "kind": "revert", "target": id, "now": 1}));
    prune(&db, 15, 64).unwrap(); // o1 only
    assert!(matches!(apply(&db, "me", &undo("o1", 1), 100), Err(ApplyError::Op(OpError::UnknownOp))));
    assert!(apply(&db, "me", &undo("o2", 2), 100).is_ok(), "o2 is still within retention");
}

#[test]
fn reset_sends_every_row_and_which_pending_ops_are_applied() {
    let db = space_with(2);
    apply(&db, "me", &op(serde_json::json!({"op_id": "d", "kind": "tag_delete", "id": "T1", "now": 5})), 30).unwrap();
    apply(&db, "other", &tag(9), 40).unwrap(); // another member's rows are not this owner's
    let r = reset(&db, "me", &["o2".into(), "never-sent".into(), "o9".into()]).unwrap();
    assert_eq!(r.seq, 4);
    let tags: Vec<_> = r
        .rows
        .iter()
        .map(|row| match row {
            Row::Tag(t) => (t.id.as_str(), t.deleted_at.is_some()),
            _ => panic!("only tags here"),
        })
        .collect();
    assert_eq!(tags, vec![("T1", true), ("T2", false)], "deleted rows too");
    assert_eq!(r.applied, vec!["o2".to_string()], "only this owner's applied ops");
}
