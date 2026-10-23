//! Reading history: the shared merge fixtures (fixtures/history.json), and storage (merge on put, the row bound,
//! forgetting a member).

use space::{forget_history, history, merge_history, put_history, HistoryEntry, Pos, HISTORY_KEEP};

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    a: HistoryEntry,
    b: HistoryEntry,
    out: HistoryEntry,
}

#[test]
fn merge_fixtures() {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/history.json")).unwrap();
    let cases: Vec<Case> =
        serde_json::from_value(serde_json::from_str::<serde_json::Value>(&raw).unwrap()["cases"].clone()).unwrap();
    assert!(cases.len() >= 5);
    for c in &cases {
        assert_eq!(merge_history(&c.a, &c.b), c.out, "{}", c.name);
        assert_eq!(merge_history(&c.b, &c.a), c.out, "{} (other way)", c.name);
    }
}

fn db() -> rusqlite::Connection {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    space::init(&db).unwrap();
    db
}

fn e(id: &str, ord: u32, at: i64) -> HistoryEntry {
    HistoryEntry { id: id.into(), anchor: Pos { ord, at }, last: Pos { ord, at }, gone: false }
}

#[test]
fn put_merges_and_returns_what_changed() {
    let db = db();
    assert_eq!(put_history(&db, "me", &[e("a", 10, 1)]).unwrap(), vec![e("a", 10, 1)]);
    let mut scrolled = e("a", 10, 1);
    scrolled.last = Pos { ord: 50, at: 5 };
    assert_eq!(put_history(&db, "me", &[scrolled.clone()]).unwrap(), vec![scrolled.clone()]);
    assert!(put_history(&db, "me", &[e("a", 10, 1)]).unwrap().is_empty(), "an older copy changes nothing");
    assert_eq!(history(&db, "me").unwrap(), vec![scrolled]);
    assert!(history(&db, "you").unwrap().is_empty(), "per member");
}

#[test]
fn rows_are_bounded_newest_kept() {
    let db = db();
    let all: Vec<_> = (0..HISTORY_KEEP + 5).map(|i| e(&format!("h{i:04}"), 1, i64::try_from(i).unwrap())).collect();
    put_history(&db, "me", &all).unwrap();
    let kept = history(&db, "me").unwrap();
    assert_eq!(kept.len(), HISTORY_KEEP);
    assert_eq!(kept[0].id, "h0005", "the oldest went");
    put_history(&db, "other", &[e("x", 1, 0)]).unwrap();
    assert_eq!(history(&db, "me").unwrap().len(), HISTORY_KEEP, "another member's rows don't count");
    forget_history(&db, "me").unwrap();
    assert_eq!(history(&db, "me").unwrap(), []);
    assert_eq!(history(&db, "other").unwrap().len(), 1);
}
