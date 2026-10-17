//! Signing in merges an anonymous owner into an account; in a group its comments get the account as author, as an
//! ordinary change (so clients receive it).
use ops::{Op, Row};
use rusqlite::Connection;
use space::{apply, apply_by, changes_since, init, reassign_author, ApplyError};

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).unwrap()
}

#[test]
fn comments_move_to_the_account_and_stay_editable() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    init(&db).unwrap();
    let g = "group";
    apply(&db, g, &op(serde_json::json!({"op_id": "t", "kind": "tag_create", "id": "T", "name": "Grace", "color": "#123456", "now": 1})), 1).unwrap();
    apply(&db, g, &op(serde_json::json!({"op_id": "p", "kind": "paint", "tag": "T", "text_id": "k", "start": 1, "end": 5, "new_id": "g1", "now": 1})), 1).unwrap();
    let comment = |id: &str| {
        op(
            serde_json::json!({"op_id": format!("c-{id}"), "kind": "comment_create", "id": id, "tagging": "g1", "body": "Note", "now": 1}),
        )
    };
    apply_by(&db, g, "anon", &comment("c1"), 2).unwrap();
    apply_by(&db, g, "bob", &comment("c2"), 2).unwrap();

    let change = reassign_author(&db, g, "anon", "acct", 3).unwrap().expect("a change");
    assert_eq!((change.seq, change.kind.as_str(), change.op_id.as_str()), (5, "reassign", "reassign-anon-5"));
    let authors: Vec<_> = change
        .rows
        .iter()
        .map(|rc| match (&rc.before, &rc.after) {
            (Some(Row::Comment(b)), Row::Comment(a)) => (a.id.as_str(), b.author.as_deref(), a.author.as_deref()),
            _ => panic!("comments only"),
        })
        .collect();
    assert_eq!(authors, [("c1", Some("anon"), Some("acct"))], "only anon's comments");
    assert_eq!(changes_since(&db, 4, 10).unwrap(), [change], "replayable like any change");
    assert_eq!(reassign_author(&db, g, "anon", "acct", 4).unwrap(), None, "nothing left to move");

    let edit = |by: &str, n: u32| {
        apply_by(
            &db,
            g,
            by,
            &op(
                serde_json::json!({"op_id": format!("e{n}"), "kind": "comment_update", "id": "c1", "body": format!("Edit {n}"), "now": 5}),
            ),
            5,
        )
    };
    assert!(matches!(edit("anon", 1), Err(ApplyError::Refused("not_author"))));
    assert!(edit("acct", 2).is_ok(), "the account may edit it now");
}
