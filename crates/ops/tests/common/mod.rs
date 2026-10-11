#![allow(dead_code)] // each test binary uses a different subset

use ops::{Change, CommentRow, PaletteRow, Row, RowSource, TagRow, TaggingRow};
use serde::Deserialize;

/// In-memory rows for one owner; the reference semantics every `RowSource` must match.
#[derive(Default, Deserialize)]
pub struct Mem {
    pub rows: Vec<Row>,
    pub changes: Vec<Change>,
}

impl RowSource for Mem {
    fn tag(&self, id: &str) -> Option<TagRow> {
        self.rows.iter().find_map(|r| match r {
            Row::Tag(t) if t.id == id => Some(t.clone()),
            _ => None,
        })
    }
    fn tagging(&self, id: &str) -> Option<TaggingRow> {
        self.rows.iter().find_map(|r| match r {
            Row::Tagging(t) if t.id == id => Some(t.clone()),
            _ => None,
        })
    }
    fn palette(&self) -> Option<PaletteRow> {
        self.rows.iter().find_map(|r| match r {
            Row::Palette(p) => Some(p.clone()),
            _ => None,
        })
    }
    fn comment(&self, id: &str) -> Option<CommentRow> {
        self.rows.iter().find_map(|r| match r {
            Row::Comment(c) if c.id == id => Some(c.clone()),
            _ => None,
        })
    }
    fn live_tag_named(&self, name: &str) -> Option<TagRow> {
        self.rows.iter().find_map(|r| match r {
            Row::Tag(t) if t.deleted_at.is_none() && t.name.to_lowercase() == name.to_lowercase() => Some(t.clone()),
            _ => None,
        })
    }
    fn taggings_near(&self, tag: &str, text_id: &str, start: u32, end: u32) -> Vec<TaggingRow> {
        self.live_taggings_of_tag(tag)
            .into_iter()
            .filter(|t| t.text_id == text_id && t.start <= end && t.end >= start)
            .collect()
    }
    fn live_taggings_of_tag(&self, tag: &str) -> Vec<TaggingRow> {
        self.rows
            .iter()
            .filter_map(|r| match r {
                Row::Tagging(t) if t.tag == tag && t.deleted_at.is_none() => Some(t.clone()),
                _ => None,
            })
            .collect()
    }
    fn change(&self, op_id: &str) -> Option<Change> {
        self.changes.iter().find(|c| c.op_id == op_id).cloned()
    }
}

impl Mem {
    /// Record a change: upsert its `after` rows.
    pub fn apply(&mut self, mut change: Change) {
        for rc in &change.rows {
            let key = rc.after.key();
            match self.rows.iter_mut().find(|r| r.key() == key) {
                Some(r) => *r = rc.after.clone(),
                None => self.rows.push(rc.after.clone()),
            }
        }
        change.seq = self.changes.len() as u64 + 1;
        self.changes.push(change);
    }

    pub fn row(&self, key: (ops::Table, &str)) -> Option<&Row> {
        self.rows.iter().find(|r| r.key() == key)
    }
}
