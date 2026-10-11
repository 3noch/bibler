//! ops: what a client op does to a user's rows. Pure and deterministic, so the client's optimistic result equals
//! the server's canonical one. Rust twin of `packages/scripture/src/ops.ts`; both are verified against
//! `fixtures/ops.json`.
//!
//! * An [`Op`] (client-supplied `op_id` + [`Intent`]) becomes a [`Change`]: a list of row `{before, after}` pairs.
//!   Deletes are soft (`deleted_at`), so every change is reversible.
//! * Undo/redo is [`Intent::Revert`]: apply a past change's `before` rows, only if those rows are still exactly as
//!   that change left them and the per-tag disjointness invariant still holds; otherwise `conflict`.
//! * Rows are read through [`RowSource`] (SQLite transaction on the server, in-memory maps on the client), always
//!   scoped to one owner. Ids and timestamps come from the op, never from clocks or RNG here.

use normalize::{normalize, NormalizeError, Tagging};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const MAX_PALETTE: usize = 9;
pub const MAX_TAG_NAME: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    pub color: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaggingRow {
    pub id: String,
    pub tag: String,
    pub text_id: String,
    pub start: u32,
    pub end: u32,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
    pub merged_into: Option<String>,
}

/// The owner's palette (one row per owner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteRow {
    pub tags: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "table", rename_all = "lowercase")]
pub enum Row {
    Tag(TagRow),
    Tagging(TaggingRow),
    Palette(PaletteRow),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Table {
    Tag,
    Tagging,
    Palette,
}

pub const PALETTE_ID: &str = "palette";

impl Row {
    #[must_use]
    pub fn key(&self) -> (Table, &str) {
        match self {
            Row::Tag(t) => (Table::Tag, &t.id),
            Row::Tagging(t) => (Table::Tagging, &t.id),
            Row::Palette(_) => (Table::Palette, PALETTE_ID),
        }
    }

    /// The row soft-deleted at `now` (what reverting its creation leaves behind).
    #[must_use]
    pub fn tombstone(&self, now: i64) -> Row {
        match self {
            Row::Tag(t) => Row::Tag(TagRow { deleted_at: Some(now), updated_at: now, ..t.clone() }),
            Row::Tagging(t) => Row::Tagging(TaggingRow { deleted_at: Some(now), ..t.clone() }),
            Row::Palette(_) => Row::Palette(PaletteRow { tags: vec![], updated_at: now }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowChange {
    /// `None` = the row did not exist.
    pub before: Option<Row>,
    pub after: Row,
}

/// The canonical effect of one op. `seq` is assigned by the server (0 while optimistic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    #[serde(default)]
    pub seq: u64,
    pub op_id: String,
    pub kind: String,
    /// Sorted by (table, id); may be empty (a no-op still gets a seq, confirming the op).
    pub rows: Vec<RowChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Intent {
    Paint { tag: String, text_id: String, start: u32, end: u32, new_id: String, now: i64 },
    Erase { tag: String, text_id: String, start: u32, end: u32, split_id: Option<String>, now: i64 },
    TagCreate { id: String, name: String, color: String, now: i64 },
    TagUpdate { id: String, name: Option<String>, color: Option<String>, now: i64 },
    TagDelete { id: String, now: i64 },
    PaletteSet { tags: Vec<String>, now: i64 },
    Revert { target: String, now: i64 },
}

impl Intent {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Intent::Paint { .. } => "paint",
            Intent::Erase { .. } => "erase",
            Intent::TagCreate { .. } => "tag_create",
            Intent::TagUpdate { .. } => "tag_update",
            Intent::TagDelete { .. } => "tag_delete",
            Intent::PaletteSet { .. } => "palette_set",
            Intent::Revert { .. } => "revert",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub op_id: String,
    #[serde(flatten)]
    pub intent: Intent,
}

/// Stable error codes shared with the TS implementation and the fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpError {
    EmptyInterval,
    MissingSplitId,
    UnknownTag,
    TagDeleted,
    IdExists,
    BadName,
    NameTaken,
    BadColor,
    BadPalette,
    UnknownOp,
    Conflict,
}

impl OpError {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            OpError::EmptyInterval => "empty_interval",
            OpError::MissingSplitId => "missing_split_id",
            OpError::UnknownTag => "unknown_tag",
            OpError::TagDeleted => "tag_deleted",
            OpError::IdExists => "id_exists",
            OpError::BadName => "bad_name",
            OpError::NameTaken => "name_taken",
            OpError::BadColor => "bad_color",
            OpError::BadPalette => "bad_palette",
            OpError::UnknownOp => "unknown_op",
            OpError::Conflict => "conflict",
        }
    }
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for OpError {}

impl From<NormalizeError> for OpError {
    fn from(e: NormalizeError) -> Self {
        match e {
            NormalizeError::EmptyInterval => OpError::EmptyInterval,
            NormalizeError::MissingSplitId => OpError::MissingSplitId,
        }
    }
}

/// One owner's rows, as `effect` needs them. Deleted rows are returned by the id lookups (so ids are never reused
/// and reverts can restore them) but never by the list queries.
pub trait RowSource {
    fn tag(&self, id: &str) -> Option<TagRow>;
    fn tagging(&self, id: &str) -> Option<TaggingRow>;
    fn palette(&self) -> Option<PaletteRow>;
    /// Live tag whose name equals `name` case-insensitively (ASCII + Unicode lowercase).
    fn live_tag_named(&self, name: &str) -> Option<TagRow>;
    /// Live taggings of `tag` in `text_id` overlapping or abutting `[start, end]`.
    fn taggings_near(&self, tag: &str, text_id: &str, start: u32, end: u32) -> Vec<TaggingRow>;
    /// All live taggings of `tag`.
    fn live_taggings_of_tag(&self, tag: &str) -> Vec<TaggingRow>;
    /// A previously recorded change, by its op id.
    fn change(&self, op_id: &str) -> Option<Change>;
}

fn current(src: &dyn RowSource, table: Table, id: &str) -> Option<Row> {
    match table {
        Table::Tag => src.tag(id).map(Row::Tag),
        Table::Tagging => src.tagging(id).map(Row::Tagging),
        Table::Palette => src.palette().map(Row::Palette),
    }
}

fn live_tag(src: &dyn RowSource, id: &str) -> Result<TagRow, OpError> {
    let tag = src.tag(id).ok_or(OpError::UnknownTag)?;
    if tag.deleted_at.is_some() {
        return Err(OpError::TagDeleted);
    }
    Ok(tag)
}

fn check_name(name: &str) -> Result<String, OpError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_TAG_NAME {
        return Err(OpError::BadName);
    }
    Ok(name.to_string())
}

fn check_color(color: &str) -> Result<(), OpError> {
    let hex = color.strip_prefix('#').ok_or(OpError::BadColor)?;
    if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(OpError::BadColor)
    }
}

fn to_tagging(t: &TaggingRow) -> Tagging {
    Tagging { id: t.id.clone(), tag: t.tag.clone(), start: t.start, end: t.end, created_at: t.created_at }
}

/// Compute the change an op makes. `seq` is left 0.
///
/// # Errors
/// A stable [`OpError`] when the op is invalid against `src` (see the variants); nothing is changed then.
pub fn effect(src: &dyn RowSource, op: &Op) -> Result<Change, OpError> {
    let rows = match &op.intent {
        Intent::Paint { tag, text_id, start, end, new_id, now } => {
            live_tag(src, tag)?;
            if src.tagging(new_id).is_some() {
                return Err(OpError::IdExists);
            }
            let intent = normalize::Intent::Paint {
                tag: tag.clone(),
                start: *start,
                end: *end,
                new_id: new_id.clone(),
                now: *now,
            };
            tagging_rows(src, tag, text_id, *start, *end, &intent, *now)?
        }
        Intent::Erase { tag, text_id, start, end, split_id, now } => {
            live_tag(src, tag)?;
            if split_id.as_deref().is_some_and(|id| src.tagging(id).is_some()) {
                return Err(OpError::IdExists);
            }
            let intent =
                normalize::Intent::Erase { tag: tag.clone(), start: *start, end: *end, split_id: split_id.clone() };
            tagging_rows(src, tag, text_id, *start, *end, &intent, *now)?
        }
        Intent::TagCreate { id, name, color, now } => {
            let name = check_name(name)?;
            check_color(color)?;
            if src.tag(id).is_some() {
                return Err(OpError::IdExists);
            }
            if src.live_tag_named(&name).is_some() {
                return Err(OpError::NameTaken);
            }
            let row = TagRow {
                id: id.clone(),
                name,
                color: color.clone(),
                created_at: *now,
                updated_at: *now,
                deleted_at: None,
            };
            vec![RowChange { before: None, after: Row::Tag(row) }]
        }
        Intent::TagUpdate { id, name, color, now } => {
            let before = live_tag(src, id)?;
            let mut after = before.clone();
            if let Some(name) = name {
                let name = check_name(name)?;
                if src.live_tag_named(&name).is_some_and(|t| t.id != *id) {
                    return Err(OpError::NameTaken);
                }
                after.name = name;
            }
            if let Some(color) = color {
                check_color(color)?;
                after.color.clone_from(color);
            }
            if after == before {
                vec![]
            } else {
                after.updated_at = *now;
                vec![RowChange { before: Some(Row::Tag(before)), after: Row::Tag(after) }]
            }
        }
        Intent::TagDelete { id, now } => tag_delete_rows(src, id, *now)?,
        Intent::PaletteSet { tags, now } => palette_rows(src, tags, *now)?,
        Intent::Revert { target, now } => revert_rows(src, target, *now)?,
    };
    Ok(Change { seq: 0, op_id: op.op_id.clone(), kind: op.intent.kind().to_string(), rows: sorted(rows) })
}

fn tag_delete_rows(src: &dyn RowSource, id: &str, now: i64) -> Result<Vec<RowChange>, OpError> {
    let tag = live_tag(src, id)?;
    let mut rows = vec![RowChange {
        after: Row::Tag(TagRow { deleted_at: Some(now), updated_at: now, ..tag.clone() }),
        before: Some(Row::Tag(tag)),
    }];
    for t in src.live_taggings_of_tag(id) {
        rows.push(RowChange {
            after: Row::Tagging(TaggingRow { deleted_at: Some(now), ..t.clone() }),
            before: Some(Row::Tagging(t)),
        });
    }
    if let Some(p) = src.palette().filter(|p| p.tags.iter().any(|t| t == id)) {
        let tags = p.tags.iter().filter(|t| *t != id).cloned().collect();
        rows.push(RowChange {
            after: Row::Palette(PaletteRow { tags, updated_at: now }),
            before: Some(Row::Palette(p)),
        });
    }
    Ok(rows)
}

fn palette_rows(src: &dyn RowSource, tags: &[String], now: i64) -> Result<Vec<RowChange>, OpError> {
    let unique: HashSet<&String> = tags.iter().collect();
    if tags.len() > MAX_PALETTE || unique.len() != tags.len() {
        return Err(OpError::BadPalette);
    }
    for t in tags {
        live_tag(src, t)?;
    }
    let before = src.palette();
    if before.as_ref().is_some_and(|p| p.tags == tags) {
        return Ok(vec![]);
    }
    Ok(vec![RowChange {
        before: before.map(Row::Palette),
        after: Row::Palette(PaletteRow { tags: tags.to_vec(), updated_at: now }),
    }])
}

fn tagging_rows(
    src: &dyn RowSource,
    tag: &str,
    text_id: &str,
    start: u32,
    end: u32,
    intent: &normalize::Intent,
    now: i64,
) -> Result<Vec<RowChange>, OpError> {
    let near = src.taggings_near(tag, text_id, start, end);
    let cs = normalize(&near.iter().map(to_tagging).collect::<Vec<_>>(), intent)?;
    let by_id: BTreeMap<&str, &TaggingRow> = near.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut rows = vec![];
    for c in cs.create {
        let row = TaggingRow {
            id: c.id,
            tag: c.tag,
            text_id: text_id.to_string(),
            start: c.start,
            end: c.end,
            created_at: c.created_at,
            deleted_at: None,
            merged_into: None,
        };
        rows.push(RowChange { before: None, after: Row::Tagging(row) });
    }
    for u in cs.update {
        let before = by_id[u.id.as_str()];
        let after = TaggingRow { start: u.start, end: u.end, ..before.clone() };
        rows.push(RowChange { before: Some(Row::Tagging(before.clone())), after: Row::Tagging(after) });
    }
    for d in cs.delete {
        let before = by_id[d.id.as_str()];
        let after = TaggingRow { deleted_at: Some(now), merged_into: d.merged_into, ..before.clone() };
        rows.push(RowChange { before: Some(Row::Tagging(before.clone())), after: Row::Tagging(after) });
    }
    Ok(rows)
}

fn revert_rows(src: &dyn RowSource, target: &str, now: i64) -> Result<Vec<RowChange>, OpError> {
    let change = src.change(target).ok_or(OpError::UnknownOp)?;
    let mut rows = vec![];
    for rc in &change.rows {
        let (table, id) = rc.after.key();
        let now_row = current(src, table, id).ok_or(OpError::Conflict)?;
        if now_row != rc.after {
            return Err(OpError::Conflict); // changed since: reverting would clobber that
        }
        let after = rc.before.clone().unwrap_or_else(|| rc.after.tombstone(now));
        rows.push(RowChange { before: Some(now_row), after });
    }
    check_revert_invariants(src, &change, &rows)?;
    Ok(rows)
}

/// A revert must leave the same state a sequence of ordinary ops could: per-tag disjoint, non-adjacent taggings;
/// no live tagging of a deleted tag; unique live tag names; a palette of live tags.
fn check_revert_invariants(src: &dyn RowSource, target: &Change, rows: &[RowChange]) -> Result<(), OpError> {
    let touched: HashSet<(Table, &str)> = target.rows.iter().map(|rc| rc.after.key()).collect();
    let result_tag = |id: &str| -> Option<TagRow> {
        rows.iter()
            .find_map(|rc| match &rc.after {
                Row::Tag(t) if t.id == id => Some(t.clone()),
                _ => None,
            })
            .or_else(|| src.tag(id))
    };
    let live_in_result = |t: &TaggingRow| -> bool {
        rows.iter()
            .find_map(|rc| match &rc.after {
                Row::Tagging(r) if r.id == t.id => Some(r.deleted_at.is_none()),
                _ => None,
            })
            .unwrap_or(t.deleted_at.is_none())
    };
    let live_taggings: Vec<&TaggingRow> = rows
        .iter()
        .filter_map(|rc| match &rc.after {
            Row::Tagging(t) if t.deleted_at.is_none() => Some(t),
            _ => None,
        })
        .collect();
    for t in &live_taggings {
        if result_tag(&t.tag).is_none_or(|tag| tag.deleted_at.is_some()) {
            return Err(OpError::Conflict);
        }
        // Neighbors outside this change must not overlap or abut the restored tagging. (Restored taggings cannot
        // clash with each other: they are `before` rows of one change, valid together, and unchanged since.)
        let clash = src
            .taggings_near(&t.tag, &t.text_id, t.start, t.end)
            .iter()
            .any(|n| !touched.contains(&(Table::Tagging, n.id.as_str())) && live_in_result(n));
        if clash {
            return Err(OpError::Conflict);
        }
    }
    let result_palette = rows
        .iter()
        .find_map(|rc| match &rc.after {
            Row::Palette(p) => Some(p.clone()),
            _ => None,
        })
        .or_else(|| src.palette());
    for rc in rows {
        match &rc.after {
            Row::Tag(tag) if tag.deleted_at.is_some() => {
                let orphaned = src.live_taggings_of_tag(&tag.id).iter().any(&live_in_result);
                let in_palette = result_palette.as_ref().is_some_and(|p| p.tags.contains(&tag.id));
                if orphaned || in_palette {
                    return Err(OpError::Conflict);
                }
            }
            Row::Tag(tag) => {
                if src
                    .live_tag_named(&tag.name)
                    .is_some_and(|o| o.id != tag.id && result_tag(&o.id).is_some_and(|r| r.deleted_at.is_none()))
                {
                    return Err(OpError::NameTaken);
                }
            }
            Row::Palette(p) => {
                if p.tags.iter().any(|id| result_tag(id).is_none_or(|t| t.deleted_at.is_some())) {
                    return Err(OpError::Conflict);
                }
            }
            Row::Tagging(_) => {}
        }
    }
    Ok(())
}

fn sorted(mut rows: Vec<RowChange>) -> Vec<RowChange> {
    rows.sort_by(|a, b| a.after.key().cmp(&b.after.key()));
    rows
}
