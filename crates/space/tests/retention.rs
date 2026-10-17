//! Retention: `prune` blanks old changes a batch at a time; their op ids still dedupe, they can't be replayed or
//! reverted, and a client that is too far behind gets a `reset` instead.
use ops::{Op, OpError, Row};
use rusqlite::Connection;
use space::{
    apply, changes_since, init, needs_reset, prune, pruned_through, reset, ApplyError, ReplayLimit, REPLAY_LIMIT,
};

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

#[test]
fn a_client_that_cannot_be_replayed_to_needs_a_reset() {
    let db = space_with(5);
    let needs = |after, last_op| needs_reset(&db, after, last_op, REPLAY_LIMIT).unwrap();
    assert!(!needs(0, None), "a new client replays everything");
    assert!(!needs(3, Some("o3")), "the same change at its seq");
    assert!(!needs(3, None), "an older client that doesn't say");
    assert!(!needs(5, Some("o5")), "up to date");
    // The space was restored from a backup older than what the client has seen.
    assert!(needs(6, Some("o6")), "ahead of the space");
    assert!(needs(6, None), "ahead of the space, even unsaid");
    assert!(needs(3, Some("other")), "a different change at its seq: the space went back, then on");
    prune(&db, 35, 64).unwrap();
    assert!(needs(2, Some("o2")), "pruned");
    assert!(!needs(3, Some("o3")), "the pruning point itself still replays");
    let r = reset(&db, "me", &[]).unwrap();
    assert_eq!((r.seq, r.last_op.as_deref()), (5, Some("o5")));
    assert_eq!(reset(&open(), "me", &[]).unwrap().last_op, None, "an empty space");
}

#[test]
fn a_replay_past_the_limit_is_a_reset_instead() {
    let db = space_with(5);
    let bytes: i64 =
        db.query_row("SELECT SUM(length(CAST(payload AS BLOB))) FROM changes WHERE seq > 1", [], |r| r.get(0)).unwrap();
    let bytes = u64::try_from(bytes).unwrap();
    let needs = |after, changes, bytes| needs_reset(&db, after, None, ReplayLimit { changes, bytes }).unwrap();
    assert!(!needs(1, 4, bytes), "exactly at both limits");
    assert!(needs(1, 3, bytes), "one change too many");
    assert!(needs(1, 4, bytes - 1), "one byte too many");
    assert!(!needs(2, 3, bytes), "closer, so less to send");
}
