//! Reading a licensed text a page at a time (docs/text-format.md). A license may limit how much of a text is shown
//! at once (the ESV API's terms: 500 verses or half a book, whichever is less); such a text's book files are never
//! served, only pages: `Texts::page` checks a page's slots against its limits, and `slice_book` cuts the part of a
//! book file it covers, in the book file's own format (so the client reads it like a book). The client splits a
//! chapter into pages that fit (packages/scripture limits.ts); the server only checks.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A license's limits (text-build's `Limits`): at most `verses` verses at once, and at most `book_percent` percent
/// of one book's verses. Verses are slots with words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub verses: u32,
    pub book_percent: u32,
}

/// Why a page can't be had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
    UnknownText,
    /// Not limited: its book files are served whole.
    NotLimited,
    BadRange,
    OverLimits,
}

impl PageError {
    /// The HTTP status and message for it.
    #[must_use]
    pub fn status(self) -> (u16, &'static str) {
        match self {
            PageError::UnknownText | PageError::NotLimited => (404, "no such limited text"),
            PageError::BadRange => (400, "bad range"),
            PageError::OverLimits => (403, "more than this text's license allows at once"),
        }
    }
}

/// The part of one book a page covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePart {
    /// The book's index (its place in the manifest's `books`).
    pub book: usize,
    pub from_slot: usize,
    pub to_slot: usize,
    /// Where its words are in the book file's `words`.
    pub first_word: usize,
    pub words: usize,
    /// Its ordinals: `[start_ord, end_ord)`.
    pub start_ord: u32,
    pub end_ord: u32,
}

/// The part `p` of a book file: its words, and whatever of the rest is anchored inside it (runs clipped to it).
#[must_use]
pub fn slice_book(book: &Value, p: &PagePart) -> Value {
    let ord_in = |o: &Value| o.as_u64().is_some_and(|o| (u64::from(p.start_ord)..u64::from(p.end_ord)).contains(&o));
    let list = |k: &str| book[k].as_array().cloned().unwrap_or_default();
    let runs = |k: &str| -> Vec<Value> {
        list(k)
            .iter()
            .filter_map(|r| {
                let (s, e) = (r[0].as_u64()?.max(u64::from(p.start_ord)), r[1].as_u64()?.min(u64::from(p.end_ord)));
                (s < e).then(|| json!([s, e]))
            })
            .collect()
    };
    let slot_in = |s: &Value| s.as_u64().is_some_and(|s| (p.from_slot as u64..=p.to_slot as u64).contains(&s));
    let words: Vec<Value> = list("words").into_iter().skip(p.first_word).take(p.words).collect();
    json!({
        "book": book["book"],
        "id": book["id"],
        "first_ord": p.start_ord,
        "first_slot": p.from_slot,
        "words": words,
        "italic": runs("italic"),
        "divine_name": runs("divine_name"),
        "words_of_jesus": runs("words_of_jesus"),
        "joined": list("joined").into_iter().filter(ord_in).collect::<Vec<_>>(),
        "pilcrow": list("pilcrow").into_iter().filter(slot_in).collect::<Vec<_>>(),
        "breaks": list("breaks").into_iter().filter(|b| ord_in(&b[0])).collect::<Vec<_>>(),
        "headings": list("headings").into_iter().filter(|h| ord_in(&h["at"])).collect::<Vec<_>>(),
        "notes": list("notes").into_iter().filter(|n| ord_in(&n["after"])).collect::<Vec<_>>(),
    })
}
