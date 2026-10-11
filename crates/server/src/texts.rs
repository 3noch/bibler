//! Known texts and their valid ordinal ranges, read from text-build output (`<dir>/<text_id>/manifest.json` + the
//! index it names). The server never trusts client ordinals: a tagging's ends must be real words.

use std::collections::HashMap;
use std::path::Path;

/// Per text: `(first ordinal, end ordinal)` of each book, sorted.
#[derive(Debug, Default)]
pub struct Texts {
    books: HashMap<String, Vec<(u32, u32)>>,
}

const BOOK_SHIFT: u32 = 24;

impl Texts {
    /// Load every `<text_id>/manifest.json` under `dir`.
    ///
    /// # Errors
    /// If `dir` is unreadable or an index is malformed.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let mut books = HashMap::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.filter_map(Result::ok) {
            let manifest_path = entry.path().join("manifest.json");
            let Ok(raw) = std::fs::read_to_string(&manifest_path) else { continue };
            let manifest: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
            let (Some(text_id), Some(index)) = (manifest["text_id"].as_str(), manifest["index"].as_str()) else {
                return Err(format!("{}: missing text_id/index", manifest_path.display()));
            };
            let bytes = std::fs::read(entry.path().join(index)).map_err(|e| format!("{index}: {e}"))?;
            books.insert(text_id.to_string(), book_ranges(&bytes).ok_or_else(|| format!("{index}: malformed index"))?);
        }
        Ok(Texts { books })
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
