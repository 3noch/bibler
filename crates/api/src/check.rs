//! Checks on an op before it reaches `space::apply`: id shapes and that tagging ends are real words. The server
//! never trusts client ordinals.

use ops::{Intent, Op};
use std::collections::HashMap;

/// Known texts: per text, `(first ordinal, end ordinal)` of each book, sorted.
#[derive(Debug, Default, Clone)]
pub struct Texts {
    books: HashMap<String, Vec<(u32, u32)>>,
}

const BOOK_SHIFT: u32 = 24;

impl Texts {
    /// Add a text from its BTIX index (text-build output).
    ///
    /// # Errors
    /// If the index is malformed.
    pub fn add_index(&mut self, text_id: &str, index: &[u8]) -> Result<(), String> {
        let ranges = book_ranges(index).ok_or_else(|| format!("{text_id}: malformed index"))?;
        self.books.insert(text_id.to_string(), ranges);
        Ok(())
    }

    /// Check a tagging interval: a known text, `start < end`, and both `start` and `end - 1` are words.
    ///
    /// # Errors
    /// A human-readable reason.
    pub fn check(&self, text_id: &str, start: u32, end: u32) -> Result<(), String> {
        let ranges = self.books.get(text_id).ok_or_else(|| format!("unknown text {text_id}"))?;
        let is_word = |o: u32| ranges.iter().any(|&(s, e)| s <= o && o < e);
        if start < end && is_word(start) && is_word(end - 1) {
            Ok(())
        } else {
            Err(format!("[{start}, {end}) is not a range of words in {text_id}"))
        }
    }
}

/// Parse a BTIX v2 index far enough to get each book's ordinal range.
fn book_ranges(b: &[u8]) -> Option<Vec<(u32, u32)>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    if b.get(0..4)? != b"BTIX" || u32_at(4)? != 2 {
        return None;
    }
    let [books, chapters, slots] = [u32_at(8)?, u32_at(12)?, u32_at(16)?].map(|n| n as usize);
    let book_end_at = 24 + 4 * (slots + 1);
    let book_id_at = book_end_at + 4 * books + 4 * (chapters + 1) + 2 * slots + 2 * (books + 1) + chapters;
    (0..books)
        .map(|i| {
            let id = u32::from(*b.get(book_id_at + i)?);
            Some((id << BOOK_SHIFT, u32_at(book_end_at + 4 * i)?))
        })
        .collect()
}

/// Ids are client-generated (ULIDs): bound their shape so they cannot be abused as storage.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Check an op's ids and ordinals. Returns `(code, message)` for a `reject`.
///
/// # Errors
/// `bad_id` or `bad_text`.
pub fn check_op(texts: &Texts, op: &Op) -> Result<(), (&'static str, String)> {
    let ids: Vec<&str> = match &op.intent {
        Intent::Paint { tag, new_id, .. } => vec![tag, new_id],
        Intent::Erase { tag, split_id, .. } => {
            [Some(tag.as_str()), split_id.as_deref()].into_iter().flatten().collect()
        }
        Intent::TagCreate { id, .. }
        | Intent::TagUpdate { id, .. }
        | Intent::TagDelete { id, .. }
        | Intent::CommentUpdate { id, .. }
        | Intent::CommentDelete { id, .. } => vec![id],
        Intent::CommentCreate { id, tagging, .. } => vec![id, tagging],
        Intent::PaletteSet { tags, .. } => tags.iter().map(String::as_str).collect(),
        Intent::Revert { target, .. } => vec![target],
    };
    if !valid_id(&op.op_id) || !ids.into_iter().all(valid_id) {
        return Err(("bad_id", "ids must be 1-64 characters of [0-9A-Za-z_-]".into()));
    }
    if let Intent::Paint { text_id, start, end, .. } | Intent::Erase { text_id, start, end, .. } = &op.intent {
        texts.check(text_id, *start, *end).map_err(|m| ("bad_text", m))?;
    }
    Ok(())
}
