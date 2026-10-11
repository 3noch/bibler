//! normalize: the pure core of tag painting/erasing. Rust twin of
//! `packages/scripture/src/normalize.ts`; both are verified against
//! `fixtures/normalize.json`.
//!
//! Contract:
//!  * Intervals are half-open `[start, end)` over word ordinals.
//!  * Per tag, taggings are disjoint and NON-adjacent (`a.end < b.start`).
//!  * ids are ASCII (ULIDs), so `str` ordering matches the TS implementation.
//!  * Callers may pass all of a tag's taggings or only those overlapping/abutting the
//!    interval; irrelevant ones are ignored.
//!  * Deterministic: no clocks, no RNG. Ids a result may need (`new_id`, `split_id`)
//!    are supplied by the client inside the intent.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tagging {
    pub id: String,
    pub tag: String,
    pub start: u32,
    pub end: u32,
    /// ms since epoch
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delete {
    pub id: String,
    /// Survivor id for merges; `None` for a plain deletion (erase).
    pub merged_into: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentMove {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Changeset {
    pub create: Vec<Tagging>,
    pub update: Vec<Update>,
    pub delete: Vec<Delete>,
    pub move_comments: Vec<CommentMove>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Intent {
    Paint {
        tag: String,
        start: u32,
        end: u32,
        new_id: String,
        now: i64,
    },
    Erase {
        tag: String,
        start: u32,
        end: u32,
        #[serde(default)]
        split_id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizeError {
    EmptyInterval,
    MissingSplitId,
}

impl NormalizeError {
    /// Stable code shared with the TS implementation and the fixtures.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            NormalizeError::EmptyInterval => "empty_interval",
            NormalizeError::MissingSplitId => "missing_split_id",
        }
    }
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for NormalizeError {}

fn finish(mut cs: Changeset) -> Changeset {
    cs.create.sort_by(|a, b| a.id.cmp(&b.id));
    cs.update.sort_by(|a, b| a.id.cmp(&b.id));
    cs.delete.sort_by(|a, b| a.id.cmp(&b.id));
    cs.move_comments.sort_by(|a, b| a.from.cmp(&b.from));
    cs
}

fn check_interval(start: u32, end: u32) -> Result<(), NormalizeError> {
    if start >= end {
        Err(NormalizeError::EmptyInterval)
    } else {
        Ok(())
    }
}

/// Canonical changeset for applying `intent` to `existing` (all of the tag's taggings, or just those overlapping or
/// abutting the interval).
///
/// # Errors
/// `EmptyInterval` if `start >= end`; `MissingSplitId` if an erase must split a tagging but has no `split_id`.
pub fn normalize(existing: &[Tagging], intent: &Intent) -> Result<Changeset, NormalizeError> {
    match intent {
        Intent::Paint { tag, start, end, new_id, now } => {
            check_interval(*start, *end)?;
            Ok(paint(existing, tag, *start, *end, new_id, *now))
        }
        Intent::Erase { tag, start, end, split_id } => {
            check_interval(*start, *end)?;
            erase(existing, tag, *start, *end, split_id.as_deref())
        }
    }
}

fn paint(existing: &[Tagging], tag: &str, start: u32, end: u32, new_id: &str, now: i64) -> Changeset {
    let mut cs = Changeset::default();

    // Overlapping OR abutting (half-open: t.end == start / t.start == end).
    let mut rel: Vec<&Tagging> = existing.iter().filter(|t| t.tag == tag && t.start <= end && t.end >= start).collect();
    rel.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));

    let Some((survivor, losers)) = rel.split_first() else {
        cs.create.push(Tagging { id: new_id.to_string(), tag: tag.to_string(), start, end, created_at: now });
        return cs;
    };

    let mut lo = start;
    let mut hi = end;
    for t in &rel {
        lo = lo.min(t.start);
        hi = hi.max(t.end);
    }

    if lo != survivor.start || hi != survivor.end {
        cs.update.push(Update { id: survivor.id.clone(), start: lo, end: hi });
    }
    for t in losers {
        cs.delete.push(Delete { id: t.id.clone(), merged_into: Some(survivor.id.clone()) });
        cs.move_comments.push(CommentMove { from: t.id.clone(), to: survivor.id.clone() });
    }
    finish(cs)
}

fn erase(
    existing: &[Tagging],
    tag: &str,
    start: u32,
    end: u32,
    split_id: Option<&str>,
) -> Result<Changeset, NormalizeError> {
    let mut cs = Changeset::default();

    // Strict overlap only: merely abutting taggings are untouched.
    for t in existing.iter().filter(|t| t.tag == tag && t.start < end && t.end > start) {
        let has_left = t.start < start;
        let has_right = t.end > end;
        match (has_left, has_right) {
            (true, true) => {
                // Left piece keeps id (and comments). Right piece is new, with the original age.
                let sid = split_id.ok_or(NormalizeError::MissingSplitId)?;
                cs.update.push(Update { id: t.id.clone(), start: t.start, end: start });
                cs.create.push(Tagging {
                    id: sid.to_string(),
                    tag: t.tag.clone(),
                    start: end,
                    end: t.end,
                    created_at: t.created_at,
                });
            }
            (true, false) => cs.update.push(Update { id: t.id.clone(), start: t.start, end: start }),
            (false, true) => cs.update.push(Update { id: t.id.clone(), start: end, end: t.end }),
            (false, false) => cs.delete.push(Delete { id: t.id.clone(), merged_into: None }),
        }
    }
    Ok(finish(cs))
}

/// Apply a changeset to a list of taggings. Returns a new vector.
#[must_use]
pub fn apply(state: &[Tagging], cs: &Changeset) -> Vec<Tagging> {
    let deleted: HashSet<&str> = cs.delete.iter().map(|d| d.id.as_str()).collect();
    let updates: HashMap<&str, &Update> = cs.update.iter().map(|u| (u.id.as_str(), u)).collect();

    let mut out = Vec::with_capacity(state.len() + cs.create.len());
    for t in state {
        if deleted.contains(t.id.as_str()) {
            continue;
        }
        match updates.get(t.id.as_str()) {
            Some(u) => out.push(Tagging { start: u.start, end: u.end, ..t.clone() }),
            None => out.push(t.clone()),
        }
    }
    out.extend(cs.create.iter().cloned());
    out
}
