//! Reading history: each member's own list of reading tracks in a space, kept outside the change log (no seq, no
//! undo, no replay). An entry is where a jump landed (`anchor`) and where scrolling from there got to (`last`).
//! Merging is conflict-free: each part keeps the later of the two (by time, then ordinal), and `gone` (dropped as a
//! duplicate or evicted, by any device) stays set, so replicas agree whatever order entries arrive in. Deciding
//! what to drop needs the text index, so clients do it; the server only merges, stores and bounds the rows.
//! Spec shared with `packages/scripture/src/history.ts`: `fixtures/history.json`.

use crate::{int, text, Db, Value};
use serde::{Deserialize, Serialize};

/// A place and when it was there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pos {
    pub ord: u32,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub anchor: Pos,
    pub last: Pos,
    #[serde(default)]
    pub gone: bool,
}

/// Rows kept per member and space (live and gone); clients show far fewer, so this only bounds storage.
pub const HISTORY_KEEP: usize = 200;

fn later(a: Pos, b: Pos) -> Pos {
    if (a.at, a.ord) >= (b.at, b.ord) {
        a
    } else {
        b
    }
}

/// The two versions of one entry (same id) combined.
#[must_use]
pub fn merge(a: &HistoryEntry, b: &HistoryEntry) -> HistoryEntry {
    HistoryEntry {
        id: a.id.clone(),
        anchor: later(a.anchor, b.anchor),
        last: later(a.last, b.last),
        gone: a.gone || b.gone,
    }
}

fn entry(r: &[Value]) -> Result<HistoryEntry, String> {
    let ord = |v: &Value| u32::try_from(int(v)?).map_err(|e| e.to_string());
    Ok(HistoryEntry {
        id: text(&r[0])?.to_string(),
        anchor: Pos { ord: ord(&r[1])?, at: int(&r[2])? },
        last: Pos { ord: ord(&r[3])?, at: int(&r[4])? },
        gone: int(&r[5])? != 0,
    })
}

const COLUMNS: &str = "id, anchor_ord, anchor_at, last_ord, last_at, gone";

/// `owner`'s entries in this space.
///
/// # Errors
/// On storage failure.
pub fn history(db: &impl Db, owner: &str) -> Result<Vec<HistoryEntry>, String> {
    let rows = db.query(&format!("SELECT {COLUMNS} FROM history WHERE owner_id = ? ORDER BY id"), &[owner.into()])?;
    rows.iter().map(|r| entry(r)).collect()
}

/// Merge `entries` into `owner`'s, keeping at most [`HISTORY_KEEP`] (the most recent). Returns the entries that
/// changed, as now stored (for `owner`'s other devices). Callers check ids and timestamps first.
///
/// # Errors
/// On storage failure.
pub fn put_history(db: &impl Db, owner: &str, entries: &[HistoryEntry]) -> Result<Vec<HistoryEntry>, String> {
    let mut changed = Vec::new();
    for e in entries {
        let stored = db.query(
            &format!("SELECT {COLUMNS} FROM history WHERE owner_id = ? AND id = ?"),
            &[owner.into(), (&e.id).into()],
        )?;
        let merged = match stored.first() {
            Some(r) => {
                let old = entry(r)?;
                let m = merge(&old, e);
                if m == old {
                    continue;
                }
                m
            }
            None => e.clone(),
        };
        db.exec(
            "INSERT OR REPLACE INTO history (owner_id, id, anchor_ord, anchor_at, last_ord, last_at, gone)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                owner.into(),
                (&merged.id).into(),
                merged.anchor.ord.into(),
                merged.anchor.at.into(),
                merged.last.ord.into(),
                merged.last.at.into(),
                i64::from(merged.gone).into(),
            ],
        )?;
        changed.push(merged);
    }
    if !changed.is_empty() {
        db.exec(
            "DELETE FROM history WHERE owner_id = ? AND id NOT IN (
               SELECT id FROM history WHERE owner_id = ? ORDER BY MAX(anchor_at, last_at) DESC, id DESC LIMIT ?)",
            &[owner.into(), owner.into(), u64::try_from(HISTORY_KEEP).unwrap_or(u64::MAX).into()],
        )?;
    }
    Ok(changed)
}

/// Delete `owner`'s history here (they left the group).
///
/// # Errors
/// On storage failure.
pub fn forget_history(db: &impl Db, owner: &str) -> Result<(), String> {
    db.exec("DELETE FROM history WHERE owner_id = ?", &[owner.into()])
}
