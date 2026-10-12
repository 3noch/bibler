//! The SQL `RowSource` must give exactly the results of the reference in-memory one: run the shared ops fixtures
//! (fixtures/ops.json) through `space::apply` on SQLite. Plus the multi-owner rules of a shared space.
use ops::{Change, Op, Row, RowChange};
use rusqlite::Connection;
use serde::Deserialize;
use space::{apply, apply_by, changes_since, import, init, members, ApplyError};

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
                // The server stamps new comments with their author; `effect` (the fixture) does not.
                let mut rows = a.change.rows.clone();
                for rc in &mut rows {
                    if let (None, Row::Comment(after)) = (&rc.before, &mut rc.after) {
                        assert_eq!(after.author.take().as_deref(), Some("o1"), "{}: author", c.name);
                    }
                }
                assert_eq!(&rows, c.rows.as_ref().unwrap(), "{}", c.name);
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

#[test]
fn group_members_share_rows_but_only_edit_their_own_comments() {
    let db = open();
    let g = "group";
    let ops = [
        (r##"{"op_id": "1", "kind": "tag_create", "id": "T", "name": "Grace", "color": "#123456", "now": 1}"##, "ann"),
        (
            r#"{"op_id": "2", "kind": "paint", "tag": "T", "text_id": "t", "start": 1, "end": 5, "new_id": "g1", "now": 2}"#,
            "bob",
        ),
        (r#"{"op_id": "3", "kind": "comment_create", "id": "c1", "tagging": "g1", "body": "Hi", "now": 3}"#, "ann"),
    ];
    for (o, by) in ops {
        apply_by(&db, g, by, &op(serde_json::from_str(o).unwrap()), 0).unwrap();
    }
    let created = &changes_since(&db, 2, 1).unwrap()[0].rows[0].after;
    assert!(matches!(created, Row::Comment(c) if c.author.as_deref() == Some("ann")));
    let edit = |op_id: &str| {
        op(serde_json::json!({"op_id": op_id, "kind": "comment_update", "id": "c1", "body": "Yo", "now": 4}))
    };
    let writes = db.total_changes();
    assert!(matches!(apply_by(&db, g, "bob", &edit("4"), 0), Err(ApplyError::Refused("not_author"))));
    assert_eq!(db.total_changes(), writes, "a refused op wrote");
    let del = op(serde_json::json!({"op_id": "5", "kind": "comment_delete", "id": "c1", "now": 5}));
    assert!(matches!(apply_by(&db, g, "bob", &del, 0), Err(ApplyError::Refused("not_author"))));
    // The author keeps their name on edits.
    let edited = apply_by(&db, g, "ann", &edit("6"), 0).unwrap().change;
    assert!(matches!(&edited.rows[0].after, Row::Comment(c) if c.author.as_deref() == Some("ann") && c.body == "Yo"));
    assert_eq!(members(&db).unwrap(), [g]);
}

#[test]
fn merge_copies_live_data_and_combines_same_named_tags() {
    use space::{export, merge};
    let (anon, account) = (open(), open());
    let ops = [
        serde_json::json!({"op_id": "1", "kind": "tag_create", "id": "AG", "name": "grace", "color": "#111111", "now": 1}),
        serde_json::json!({"op_id": "2", "kind": "tag_create", "id": "AL", "name": "Love", "color": "#222222", "now": 2}),
        serde_json::json!({"op_id": "3", "kind": "paint", "tag": "AG", "text_id": "t", "start": 10, "end": 20, "new_id": "g1", "now": 3}),
        serde_json::json!({"op_id": "4", "kind": "comment_create", "id": "c1", "tagging": "g1", "body": "kept", "now": 4}),
        serde_json::json!({"op_id": "5", "kind": "paint", "tag": "AL", "text_id": "t", "start": 50, "end": 60, "new_id": "g2", "now": 5}),
        serde_json::json!({"op_id": "6", "kind": "paint", "tag": "AL", "text_id": "t", "start": 70, "end": 80, "new_id": "g3", "now": 6}),
        serde_json::json!({"op_id": "7", "kind": "comment_create", "id": "c2", "tagging": "g3", "body": "hidden", "now": 7}),
        serde_json::json!({"op_id": "8", "kind": "erase", "tag": "AL", "text_id": "t", "start": 70, "end": 80, "split_id": null, "now": 8}),
        serde_json::json!({"op_id": "9", "kind": "palette_set", "tags": ["AG", "AL"], "now": 9}),
    ];
    for o in ops {
        apply(&anon, "anon", &op(o), 0).unwrap();
    }
    for o in [
        serde_json::json!({"op_id": "1", "kind": "tag_create", "id": "BG", "name": "Grace", "color": "#333333", "now": 1}),
        serde_json::json!({"op_id": "2", "kind": "paint", "tag": "BG", "text_id": "t", "start": 15, "end": 30, "new_id": "h1", "now": 0}),
        serde_json::json!({"op_id": "3", "kind": "palette_set", "tags": ["BG"], "now": 3}),
    ] {
        apply(&account, "me", &op(o), 0).unwrap();
    }
    let rows = export(&anon, "anon").unwrap();
    assert!(!rows.iter().any(|r| matches!(r, Row::Comment(c) if c.id == "c2")), "hidden comments stay behind");
    let changes = merge(&account, "me", "anon", &rows, 100).unwrap();
    assert_ne!(changes.len(), 0);
    let mine = export(&account, "me").unwrap();
    let names: Vec<_> =
        mine.iter().filter_map(|r| if let Row::Tag(t) = r { Some(t.name.as_str()) } else { None }).collect();
    assert_eq!(names, ["Grace", "Love"], "same-named tags are combined, others created");
    let spans: Vec<_> = mine
        .iter()
        .filter_map(|r| if let Row::Tagging(t) = r { Some((t.tag.as_str(), t.start, t.end)) } else { None })
        .collect();
    assert_eq!(spans, [("BG", 10, 30), ("AL", 50, 60)], "overlapping same-tag taggings merge");
    let comment = mine.iter().find_map(|r| if let Row::Comment(c) = r { Some(c) } else { None }).unwrap();
    assert_eq!((comment.body.as_str(), comment.created_at), ("kept", 4));
    assert!(matches!(mine.last(), Some(Row::Palette(p)) if p.tags == ["BG", "AL"]));
    // Idempotent.
    assert_eq!(merge(&account, "me", "anon", &rows, 200).unwrap().len(), 0);
    assert_eq!(export(&account, "me").unwrap(), mine);
}
