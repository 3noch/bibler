//! Checks on an op before it reaches `space::apply`: id shapes and that tagging ends are real words. The server
//! never trusts client ordinals.

use crate::pages::{Limits, PageError, PagePart};
use ops::{Intent, Op};
use std::collections::HashMap;

/// Known texts, each from its BTIX index, and the limits of those a license limits (`crate::pages`).
#[derive(Debug, Default, Clone)]
pub struct Texts {
    texts: HashMap<String, Index>,
    limits: HashMap<String, Limits>,
}

/// What `check` needs from an index: each book's slots, and each slot's first ordinal and word count.
#[derive(Debug, Clone)]
struct Index {
    /// Per book: (id, first slot, end slot, end ordinal).
    books: Vec<(u32, usize, usize, u32)>,
    verse_start: Vec<u32>,
    slot_words: Vec<u8>,
}

const BOOK_SHIFT: u32 = 24;

impl Texts {
    /// Add a text from its BTIX index (text-build output).
    ///
    /// # Errors
    /// If the index is malformed.
    pub fn add_index(&mut self, text_id: &str, index: &[u8]) -> Result<(), String> {
        let index = parse(index).ok_or_else(|| format!("{text_id}: malformed index"))?;
        self.texts.insert(text_id.to_string(), index);
        Ok(())
    }

    /// Mark a text as limited by its license: read a page at a time (`page`), never served whole.
    pub fn set_limits(&mut self, text_id: &str, limits: Limits) {
        self.limits.insert(text_id.to_string(), limits);
    }

    /// The text's limits, if its license limits it.
    #[must_use]
    pub fn limits(&self, text_id: &str) -> Option<Limits> {
        self.limits.get(text_id).copied()
    }

    /// The parts of each book that slots `from..=to` of a limited text cover, if they may be shown at once: within
    /// the cap of every book they touch and of the whole (the client's `fits`, packages/scripture limits.ts).
    ///
    /// # Errors
    /// Unknown or not limited text, a bad range, or more than its limits allow.
    pub fn page(&self, text_id: &str, from: usize, to: usize) -> Result<Vec<PagePart>, PageError> {
        let index = self.texts.get(text_id).ok_or(PageError::UnknownText)?;
        let limits = self.limits(text_id).ok_or(PageError::NotLimited)?;
        if from > to || to >= index.slot_words.len() {
            return Err(PageError::BadRange);
        }
        let verses = |lo: usize, hi: usize| index.slot_words[lo..hi].iter().filter(|&&w| w > 0).count();
        let mut parts = vec![];
        let mut total = 0;
        for (book, &(_, lo, hi, end)) in index.books.iter().enumerate() {
            let (a, b) = (from.max(lo), to.min(hi - 1));
            if a > b {
                continue;
            }
            let n = verses(a, b + 1);
            let share = verses(lo, hi) * limits.book_percent as usize / 100;
            if n > share.min(limits.verses as usize) {
                return Err(PageError::OverLimits);
            }
            total += n;
            let words = |x: usize, y: usize| index.slot_words[x..y].iter().map(|&w| usize::from(w)).sum::<usize>();
            parts.push(PagePart {
                book,
                from_slot: a,
                to_slot: b,
                first_word: words(lo, a),
                words: words(a, b + 1),
                start_ord: index.verse_start[a],
                end_ord: if b + 1 == hi { end } else { index.verse_start[b + 1] },
            });
        }
        if total > limits.verses as usize {
            return Err(PageError::OverLimits);
        }
        Ok(parts)
    }

    /// Check a tagging interval: a known text, `start < end`, `start` is a word, and `end` is a word of the same
    /// book or that book's end. (Intervals tile the ordinal line: an interval ends at the word after its last word,
    /// so the gaps between verses belong to the word before them; see docs/text-format.md. Taggings never span
    /// books: erasing part of one that did would leave pieces starting or ending between books, on no word.)
    ///
    /// # Errors
    /// A human-readable reason.
    pub fn check(&self, text_id: &str, start: u32, end: u32) -> Result<(), String> {
        let index = self.texts.get(text_id).ok_or_else(|| format!("unknown text {text_id}"))?;
        let same_book = |o: u32| o >> BOOK_SHIFT == start >> BOOK_SHIFT;
        let book_end = index.books.iter().find(|b| b.0 == start >> BOOK_SHIFT).map(|b| b.3);
        if start < end && index.is_word(start) && ((index.is_word(end) && same_book(end)) || book_end == Some(end)) {
            Ok(())
        } else {
            Err(format!("[{start}, {end}) is not a range of words in {text_id}"))
        }
    }
}

impl Index {
    fn is_word(&self, o: u32) -> bool {
        let Some(&(_, lo, hi, end)) = self.books.iter().find(|b| b.0 == o >> BOOK_SHIFT) else { return false };
        if o < self.verse_start[lo] || o >= end {
            return false;
        }
        let slot = lo + self.verse_start[lo..hi].partition_point(|&s| s <= o) - 1;
        o - self.verse_start[slot] < u32::from(self.slot_words[slot])
    }
}

/// Parse a BTIX v3 index (layout: docs/text-format.md) far enough for `check`.
fn parse(b: &[u8]) -> Option<Index> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let u16_at = |i: usize| Some(usize::from(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?)));
    if b.get(0..4)? != b"BTIX" || u32_at(4)? != 3 {
        return None;
    }
    let [books, chapters, slots] = [u32_at(8)?, u32_at(12)?, u32_at(16)?].map(|n| n as usize);
    let verse_start: Vec<u32> = (0..=slots).map(|i| u32_at(24 + 4 * i)).collect::<Option<_>>()?;
    let book_end_at = 24 + 4 * (slots + 1);
    let first_verse_at = book_end_at + 4 * books;
    let first_chapter_at = first_verse_at + 4 * (chapters + 1) + 2 * slots;
    let book_id_at = first_chapter_at + 2 * (books + 1) + chapters;
    let slot_words = b.get(book_id_at + books..book_id_at + books + slots)?.to_vec();
    if b.len() != book_id_at + books + slots {
        return None;
    }
    let first_slot = |book: usize| Some(u32_at(first_verse_at + 4 * u16_at(first_chapter_at + 2 * book)?)? as usize);
    let books = (0..books)
        .map(|i| {
            let (lo, hi) = (first_slot(i)?, first_slot(i + 1)?);
            (lo < hi && hi <= slots).then_some(())?;
            Some((u32::from(*b.get(book_id_at + i)?), lo, hi, u32_at(book_end_at + 4 * i)?))
        })
        .collect::<Option<_>>()?;
    Some(Index { books, verse_start, slot_words })
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

#[cfg(test)]
mod tests {
    use super::Texts;

    /// Book 2, chapter 1: an empty title slot, 1:1 with 3 words, 1:2 with 2 words. Book 3, chapter 1: an empty
    /// title slot, 1:1 with 2 words.
    fn texts() -> Texts {
        let at = |b: u32, v: u32, w: u32| b << 24 | 1 << 16 | v << 8 | w;
        let mut b = b"BTIX".to_vec();
        let u32s = |b: &mut Vec<u8>, xs: &[u32]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
        let u16s = |b: &mut Vec<u8>, xs: &[u16]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
        u32s(&mut b, &[3, 2, 2, 5, 7]); // version, books, chapters, slots, tokens
        u32s(&mut b, &[at(2, 1, 0), at(2, 1, 0), at(2, 2, 0), at(3, 1, 0), at(3, 1, 0), at(3, 1, 2)]); // verseStart
        u32s(&mut b, &[at(2, 2, 2), at(3, 1, 2)]); // bookEnd
        u32s(&mut b, &[0, 3, 5]); // chapterFirstVerse
        u16s(&mut b, &[0, 0, 0, 1, 1]); // chapterOfVerse
        u16s(&mut b, &[0, 1, 2]); // bookFirstChapter
        b.extend_from_slice(&[0, 1, 2, 3]); // bookOfChapter, bookId
        b.extend_from_slice(&[0, 3, 2, 0, 2]); // slotWords
        let mut t = Texts::default();
        t.add_index("t", &b).unwrap();
        t
    }

    #[test]
    fn intervals_end_at_the_next_word_or_the_book_end() {
        let t = texts();
        let at = |v: u32, w: u32| 2u32 << 24 | 1 << 16 | v << 8 | w;
        let next = |w: u32| 3u32 << 24 | 1 << 16 | 1 << 8 | w; // book 3, 1:1
        let ok = |s, e| t.check("t", s, e).is_ok();
        assert!(ok(at(1, 0), at(1, 1)));
        assert!(ok(at(1, 1), at(2, 0)), "the end of 1:1 is the first word of 1:2");
        assert!(ok(at(1, 0), at(2, 2)), "the book's end");
        assert!(!ok(at(1, 0), at(1, 3)), "a gap is never an end");
        assert!(!ok(at(1, 3), at(2, 1)), "or a start");
        assert!(!ok(at(0, 0), at(1, 1)), "the empty title has no words");
        assert!(!ok(at(2, 0), at(2, 3)), "past the book's end");
        assert!(!ok(at(1, 1), at(1, 1)), "empty");
        assert!(ok(next(0), next(2)), "the second book on its own");
        assert!(!ok(at(1, 0), next(0)), "an end in the next book, even its first word");
        assert!(!ok(at(1, 0), next(1)), "a range across books");
        assert!(!ok(at(1, 0), next(2)), "or up to the next book's end");
        assert!(t.check("other", at(1, 0), at(1, 1)).is_err());
        assert!(super::parse(&[]).is_none());
    }

    #[test]
    fn a_page_of_a_limited_text_fits_its_limits() {
        use crate::pages::{Limits, PageError, PagePart};
        let mut t = texts();
        assert_eq!(t.page("t", 1, 2), Err(PageError::NotLimited), "not limited: served whole");
        assert_eq!(t.page("x", 1, 2), Err(PageError::UnknownText));
        // Book 2 has 2 verses (slots 1, 2; slot 0 is an empty title), book 3 one (slot 4).
        t.set_limits("t", Limits { verses: 2, book_percent: 100 });
        let book2 = PagePart {
            book: 0,
            from_slot: 0,
            to_slot: 2,
            first_word: 0,
            words: 5,
            start_ord: 2 << 24 | 1 << 16 | 1 << 8,
            end_ord: 2 << 24 | 1 << 16 | 2 << 8 | 2,
        };
        assert_eq!(t.page("t", 0, 2), Ok(vec![book2.clone()]), "the empty title doesn't count");
        assert_eq!(t.page("t", 2, 4).map(|p| p.len()), Ok(2), "across books: a part of each");
        assert_eq!(t.page("t", 0, 4), Err(PageError::OverLimits), "3 verses in all");
        assert_eq!(t.page("t", 2, 1), Err(PageError::BadRange));
        assert_eq!(t.page("t", 0, 5), Err(PageError::BadRange));
        t.set_limits("t", Limits { verses: 500, book_percent: 50 });
        assert_eq!(t.page("t", 1, 1).map(|p| p[0].words), Ok(3), "1 of book 2's 2 verses");
        assert_eq!(t.page("t", 1, 2), Err(PageError::OverLimits), "both of them");
        assert_eq!(t.page("t", 4, 4), Err(PageError::OverLimits), "half of 1 verse is none");
        let p = t.page("t", 2, 2).unwrap().remove(0);
        assert_eq!((p.first_word, p.words, p.end_ord), (3, 2, 2 << 24 | 1 << 16 | 2 << 8 | 2), "1:2: the book's end");
    }

    #[test]
    fn a_book_file_sliced_to_a_page_keeps_what_is_anchored_inside() {
        use crate::pages::{slice_book, PagePart};
        let at = |v: u32, w: u32| u64::from(2u32 << 24 | 1 << 16 | v << 8 | w);
        let book = serde_json::json!({
            "book": "GEN", "id": 2, "first_ord": at(1, 0), "first_slot": 0,
            "words": ["a", "b", "c", "d", "e"],
            "italic": [[at(1, 1), at(2, 1)]], "divine_name": [], "words_of_jesus": [[at(1, 0), at(1, 1)]],
            "joined": [at(1, 2)], "pilcrow": [1, 2], "breaks": [[at(1, 0), "p"], [at(2, 0), "q1"]],
            "headings": [{"at": at(2, 0), "pos": "before", "kind": "s1", "text": "H"}],
            "notes": [{"after": at(1, 2), "ref": "1.1", "text": "n"}, {"after": at(2, 1), "ref": "1.2", "text": "m"}],
        });
        #[allow(clippy::cast_possible_truncation)]
        let part = PagePart {
            book: 0,
            from_slot: 2,
            to_slot: 2,
            first_word: 3,
            words: 2,
            start_ord: at(2, 0) as u32,
            end_ord: at(2, 2) as u32,
        };
        let page = slice_book(&book, &part);
        assert_eq!(page["words"], serde_json::json!(["d", "e"]));
        assert_eq!(page["first_ord"], at(2, 0));
        assert_eq!(page["first_slot"], 2);
        assert_eq!(page["italic"], serde_json::json!([[at(2, 0), at(2, 1)]]), "a run clipped to the page");
        assert_eq!(page["words_of_jesus"], serde_json::json!([]), "a run outside it dropped");
        assert_eq!(page["joined"], serde_json::json!([]));
        assert_eq!(page["pilcrow"], serde_json::json!([2]));
        assert_eq!(page["breaks"], serde_json::json!([[at(2, 0), "q1"]]));
        assert_eq!(page["headings"][0]["text"], "H");
        assert_eq!(page["notes"], serde_json::json!([{"after": at(2, 1), "ref": "1.2", "text": "m"}]));
    }
}
