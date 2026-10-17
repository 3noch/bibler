//! text-build: USFM -> tokenized, indexed scripture artifacts. The spec is `docs/text-format.md`; anything that
//! changes ordinals must fail the golden check (`fixtures/text/<text_id>.golden.json`).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub const TEXT_ID: &str = "kjv-1769-v2";
pub const INDEX_MAGIC: &[u8; 4] = b"BTIX";
pub const INDEX_VERSION: u32 = 3;
/// Ordinals are addresses: `book_id << 24 | chapter << 16 | verse << 8 | word` (chapter and verse numbers as
/// printed, verse 0 = the title slot, word = position in the verse). Books have stable ids (the source's
/// file-number prefix: GEN = 2, MAL = 40, MAT = 70, REV = 96, leaving 41..=69 for deuterocanon), so adding a book
/// shifts no existing ordinal, a correction inside a verse moves no other verse's words, and any text with the
/// same versification gives the same verse the same ordinals.
pub const BOOK_SHIFT: u32 = 24;
pub const CHAPTER_SHIFT: u32 = 16;
pub const VERSE_SHIFT: u32 = 8;
/// Chapters, verses and words per verse must each fit in 8 bits.
pub const MAX_FIELD: u32 = 255;

/// The ordinal of `word` in `chapter:verse` of book `id`.
#[must_use]
pub const fn make_ord(id: u8, chapter: u32, verse: u32, word: u32) -> u32 {
    (id as u32) << BOOK_SHIFT | chapter << CHAPTER_SHIFT | verse << VERSE_SHIFT | word
}

// Index integer widths: words and slots are u32, chapter indexes u16, book indexes u8. The builder refuses input
// that would outgrow them (`Builder::check_capacity`, the word check in `tokenize`) before anything is pushed, so
// these narrowing conversions cannot truncate.
#[allow(clippy::cast_possible_truncation)]
const fn narrow32(n: usize) -> u32 {
    n as u32
}
#[allow(clippy::cast_possible_truncation)]
const fn narrow16(n: usize) -> u16 {
    n as u16
}
#[allow(clippy::cast_possible_truncation)]
const fn narrow8(n: usize) -> u8 {
    n as u8
}
const MAX_WORDS: usize = u32::MAX as usize;

const ITALIC: u8 = 1;
const DIVINE_NAME: u8 = 2;
const WORDS_OF_JESUS: u8 = 4;
/// Not followed by a space: the word ends with a hyphen (`Beth-` of `Beth-el`).
const JOINED: u8 = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heading {
    pub slot: u32,
    pub pos: HeadingPos,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HeadingPos {
    Before,
    After,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub after: u32,
    #[serde(rename = "ref")]
    pub reference: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Book {
    pub code: String,
    /// Source file stem without extension, e.g. `02-GEN`; fixes canonical order.
    pub file_key: String,
    pub name_long: String,
    pub name_short: String,
    pub abbr: String,
    pub titles: Vec<String>,
    /// Stable book id (file-number prefix).
    pub id: u8,
    pub first_chapter: u32,
    pub chapters: u32,
    pub first_slot: u32,
    pub slots: u32,
    /// Index of the book's first word in `Text::words` (dense; not an ordinal).
    pub first_word: u32,
    /// Ordinal of the book's first word.
    pub first_ord: u32,
    pub tokens: u32,
}

/// The whole tokenized text. `words`/`flags` are dense (all books concatenated); ordinals are sparse addresses
/// (`make_ord`). Slots are global and dense.
///
/// Intervals are half-open and **tile** the ordinal line: a word owns the ordinals up to the next word of its
/// book, and a book's last word owns only itself. So an interval's end is the ordinal of the word after its last
/// word, or the book's end (last word + 1), and painting two neighbouring verses separately still gives
/// abutting intervals.
#[derive(Debug, Default)]
pub struct Text {
    pub books: Vec<Book>,
    pub words: Vec<String>,
    /// Per-token flag bits (`ITALIC | DIVINE_NAME | WORDS_OF_JESUS | JOINED`), dense like `words`.
    pub flags: Vec<u8>,
    /// First ordinal of each slot, plus a sentinel (= the last book's end ordinal). An empty slot starts where the
    /// next slot with words starts (or at the book's end). While building, these are dense word indices; `build`
    /// converts them to ordinals at the end.
    pub verse_start: Vec<u32>,
    /// Index in `words` of each slot's first word, plus a sentinel (= number of words).
    pub slot_first_word: Vec<u32>,
    /// Words in each slot.
    pub slot_words: Vec<u8>,
    /// End ordinal (exclusive) of each book. A book's last slot ends here, not at the next book's start.
    pub book_end: Vec<u32>,
    pub chapter_of_verse: Vec<u16>,
    /// First slot of each chapter, plus a sentinel (= number of slots) once finished.
    pub chapter_first_verse: Vec<u32>,
    pub book_of_chapter: Vec<u8>,
    /// First chapter of each book, plus a sentinel (= number of chapters) once finished.
    pub book_first_chapter: Vec<u16>,
    pub pilcrow: Vec<u32>,
    pub breaks: Vec<(u32, String)>,
    pub headings: Vec<Heading>,
    pub notes: Vec<Note>,
}

impl Text {
    #[must_use]
    pub fn slots(&self) -> u32 {
        narrow32(self.chapter_of_verse.len())
    }
    #[must_use]
    pub fn chapters(&self) -> u32 {
        narrow32(self.book_of_chapter.len())
    }
    #[must_use]
    pub fn tokens(&self) -> u32 {
        narrow32(self.words.len())
    }

    /// Book index of an ordinal inside a book's range `[first word, end)` (a word, or a gap a word owns).
    #[must_use]
    pub fn book_of(&self, ord: u32) -> Option<usize> {
        let id = ord >> BOOK_SHIFT;
        let b = self.books.iter().position(|b| u32::from(b.id) == id)?;
        (self.books[b].first_ord <= ord && ord < self.book_end[b]).then_some(b)
    }

    /// The slot owning an ordinal of a book's range: upperBound over that book's slots, minus one.
    fn slot_of(&self, ord: u32) -> Option<usize> {
        let b = &self.books[self.book_of(ord)?];
        let lo = b.first_slot as usize;
        Some(lo + self.verse_start[lo..lo + b.slots as usize].partition_point(|&s| s <= ord) - 1)
    }

    /// The word at an ordinal, if it is a word.
    #[must_use]
    pub fn word(&self, ord: u32) -> Option<&str> {
        let slot = self.slot_of(ord)?;
        let i = ord - self.verse_start[slot];
        (i < u32::from(self.slot_words[slot])).then(|| self.words[(self.slot_first_word[slot] + i) as usize].as_str())
    }

    /// The book's words as displayed: hyphen-joined parts put back together.
    #[must_use]
    pub fn display_words(&self, b: &Book) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let mut glue = false;
        for w in b.first_word..b.first_word + b.tokens {
            let word = &self.words[w as usize];
            match out.last_mut() {
                Some(last) if glue => last.push_str(word),
                _ => out.push(word.clone()),
            }
            glue = self.flags[w as usize] & JOINED != 0;
        }
        out
    }

    /// The ordinal of the word at index `w` of `words`.
    #[must_use]
    pub fn ord_of_word(&self, w: u32) -> u32 {
        let slot = self.slot_first_word[..self.slot_words.len()].partition_point(|&s| s <= w) - 1;
        self.verse_start[slot] + w - self.slot_first_word[slot]
    }

    /// The interval end after the word at index `w` of `words`: the next word's ordinal, or the book's end.
    ///
    /// # Panics
    /// If `w` is not an index of `words`.
    #[must_use]
    pub fn end_after_word(&self, w: u32) -> u32 {
        let book = self.books.iter().rposition(|b| b.first_word <= w).expect("a word");
        if w + 1 < self.books[book].first_word + self.books[book].tokens {
            self.ord_of_word(w + 1)
        } else {
            self.book_end[book]
        }
    }

    /// ord -> (book index, chapter number, verse number) of the slot owning it. Same algorithm the TS index uses:
    /// the book comes from the high bits, then upperBound over that book's slots.
    #[must_use]
    pub fn locate(&self, ord: u32) -> Option<(usize, u32, u32)> {
        let slot = self.slot_of(ord)?;
        let book = self.book_of(ord)?;
        let ch = self.chapter_of_verse[slot];
        let verse = narrow32(slot) - self.chapter_first_verse[usize::from(ch)];
        let chapter = u32::from(ch) - u32::from(self.book_first_chapter[book]) + 1;
        Some((book, chapter, verse))
    }

    /// End ordinal (exclusive) of a slot.
    #[must_use]
    pub fn slot_end(&self, slot: usize) -> u32 {
        let book = usize::from(self.book_of_chapter[usize::from(self.chapter_of_verse[slot])]);
        let b = &self.books[book];
        if slot + 1 == (b.first_slot + b.slots) as usize {
            self.book_end[book]
        } else {
            self.verse_start[slot + 1]
        }
    }

    /// (book index, chapter number, verse number) -> half-open ordinal interval.
    #[must_use]
    pub fn interval(&self, book: usize, chapter: u32, verse: u32) -> (u32, u32) {
        let ch = u32::from(self.book_first_chapter[book]) + chapter - 1;
        let slot = (self.chapter_first_verse[ch as usize] + verse) as usize;
        (self.verse_start[slot], self.slot_end(slot))
    }
}

/// Inline style state while scanning a line.
#[derive(Default)]
struct Styles {
    add: u32,
    nd: u32,
    wj: u32,
    tl: u32,
}

impl Styles {
    fn bits(&self) -> u8 {
        (if self.add > 0 { ITALIC } else { 0 })
            | (if self.nd > 0 { DIVINE_NAME } else { 0 })
            | (if self.wj > 0 { WORDS_OF_JESUS } else { 0 })
    }
}

struct Builder {
    text: Text,
    book_code: String,
    chapter: u32,
    verse: Option<u32>,
    pending_breaks: Vec<String>,
    pending_headings: Vec<String>,
}

impl Builder {
    fn here(&self) -> String {
        match self.verse {
            Some(v) => format!("{} {}:{}", self.book_code, self.chapter, v),
            None => format!("{} {}", self.book_code, self.chapter),
        }
    }

    fn slot(&self) -> u32 {
        narrow32(self.text.chapter_of_verse.len()) - 1
    }

    fn slot_has_tokens(&self) -> bool {
        self.text.verse_start.last().is_some_and(|&s| s < narrow32(self.text.words.len()))
    }

    // Capacity checks: refuse input that would outgrow the index's integer widths (see `narrow32`). Each runs
    // before the push it guards, at the exact boundary.

    /// Book indexes are u8: at most 256 books.
    fn check_book_capacity(&self) -> Result<(), String> {
        if self.text.books.len() > usize::from(u8::MAX) {
            return Err(format!("more than {} books", usize::from(u8::MAX) + 1));
        }
        Ok(())
    }

    /// Chapter indexes are u16 and so is the chapter-count sentinel: at most 65,535 chapters.
    fn check_chapter_capacity(&self) -> Result<(), String> {
        if self.text.book_of_chapter.len() >= usize::from(u16::MAX) {
            return Err(format!("{}: too many chapters for the index", self.here()));
        }
        Ok(())
    }

    /// Slot indexes are u32 (with a sentinel).
    fn check_slot_capacity(&self) -> Result<(), String> {
        if self.text.chapter_of_verse.len() >= MAX_WORDS {
            return Err(format!("{}: too many verse slots for the index", self.here()));
        }
        Ok(())
    }

    fn start_chapter(&mut self, n: u32) -> Result<(), String> {
        if n != self.chapter + 1 {
            return Err(format!("{}: chapter {n} out of sequence", self.here()));
        }
        self.check_chapter_capacity()?;
        self.check_slot_capacity()?;
        self.chapter = n;
        let ch = self.text.book_of_chapter.len();
        self.text.book_of_chapter.push(narrow8(self.text.books.len() - 1));
        self.text.chapter_first_verse.push(narrow32(self.text.chapter_of_verse.len()));
        self.verse = Some(0);
        self.push_slot(ch);
        Ok(())
    }

    fn push_slot(&mut self, ch: usize) {
        self.text.verse_start.push(narrow32(self.text.words.len()));
        self.text.slot_first_word.push(narrow32(self.text.words.len()));
        self.text.chapter_of_verse.push(narrow16(ch));
    }

    fn start_verse(&mut self, n: u32) -> Result<(), String> {
        let Some(prev) = self.verse else {
            return Err(format!("{}: verse {n} before any chapter", self.here()));
        };
        if n != prev + 1 {
            return Err(format!("{}: verse {n} out of sequence", self.here()));
        }
        self.check_slot_capacity()?;
        self.verse = Some(n);
        self.push_slot(self.text.chapter_first_verse.len() - 1);
        Ok(())
    }

    /// Record a footnote (`\f + \fr c.v \ft text\f*` body) after the current or last token.
    fn footnote(&mut self, here: &str, body: &str, before_current_token: bool) -> Result<(), String> {
        let after = if before_current_token {
            narrow32(self.text.words.len())
        } else if self.slot_has_tokens() {
            narrow32(self.text.words.len()) - 1
        } else {
            return Err(format!("{here}: footnote before the first token"));
        };
        let reference = body
            .split_once("\\fr ")
            .and_then(|(_, r)| r.split_once("\\ft ").map(|(r, _)| r.trim().to_string()))
            .ok_or_else(|| format!("{here}: footnote without \\fr/\\ft: {body:?}"))?;
        let ft = body.split_once("\\ft ").map(|(_, t)| t).unwrap_or_default();
        if ft.contains("\\f") {
            return Err(format!("{here}: unexpected footnote marker in footnote: {body:?}"));
        }
        self.text.notes.push(Note { after, reference, text: plain(ft) });
        Ok(())
    }

    /// Attach pending breaks/headings to the current slot (which is about to receive tokens).
    fn attach_pending(&mut self) {
        let slot = self.slot();
        for kind in self.pending_breaks.drain(..) {
            self.text.breaks.push((slot, kind));
        }
        for text in self.pending_headings.drain(..) {
            self.text.headings.push(Heading { slot, pos: HeadingPos::Before, text });
        }
    }

    /// Tokenize the inline content of a `\v` or `\d` line into the current slot.
    #[allow(clippy::too_many_lines)] // one scan over the line; splitting it would scatter the marker rules
    fn tokenize(&mut self, s: &str) -> Result<(), String> {
        let here = self.here();
        let mut st = Styles::default();
        let mut cur = String::new();
        let mut cur_flags = 0u8;
        let mut cur_letter = false;
        // Just split at a hyphen: the next word must follow without a space.
        let mut joining = false;
        let mut chars = s.char_indices().peekable();

        let flush = |t: &mut Text, cur: &mut String, flags: &mut u8, letter: &mut bool| -> Result<(), String> {
            if cur.is_empty() {
                return Ok(());
            }
            if !*letter {
                return Err(format!("{here}: punctuation-only token {cur:?}"));
            }
            if t.words.len() >= MAX_WORDS {
                return Err(format!("{here}: too many words for the index"));
            }
            t.words.push(std::mem::take(cur));
            t.flags.push(*flags);
            *flags = 0;
            *letter = false;
            Ok(())
        };

        while let Some((i, c)) = chars.next() {
            if c == '\\' {
                let rest = &s[i + 1..];
                let name_len = rest
                    .char_indices()
                    .find(|&(j, ch)| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || (j == 0 && ch == '+')))
                    .map_or(rest.len(), |(j, _)| j);
                let name = &rest[..name_len];
                let closing = rest[name_len..].starts_with('*');
                for _ in 0..name_len + usize::from(closing) {
                    chars.next();
                }
                let base = name.trim_start_matches('+');

                if base == "f" && !closing {
                    // Footnote: `\f + \fr c.v \ft text\f*`, removed from the token stream.
                    let body_start = i + 1 + name_len;
                    let end = s[body_start..].find("\\f*").ok_or_else(|| format!("{here}: unterminated footnote"))?;
                    self.footnote(&here, &s[body_start..body_start + end], !cur.is_empty())?;
                    let skip_to = body_start + end + 3;
                    while chars.peek().is_some_and(|&(j, _)| j < skip_to) {
                        chars.next();
                    }
                    continue;
                }

                let counter = match base {
                    "add" => Some(&mut st.add),
                    "nd" => Some(&mut st.nd),
                    "wj" => Some(&mut st.wj),
                    "tl" => Some(&mut st.tl),
                    "w" => None,
                    _ => return Err(format!("{here}: unexpected marker \\{name}{}", if closing { "*" } else { "" })),
                };
                if closing {
                    if let Some(n) = counter {
                        *n = n.checked_sub(1).ok_or_else(|| format!("{here}: unbalanced \\{name}*"))?;
                    }
                } else {
                    if let Some(n) = counter {
                        *n += 1;
                    }
                    // An opening marker consumes exactly one following space.
                    if chars.peek().is_some_and(|&(_, ch)| ch == ' ') {
                        chars.next();
                    }
                }
                continue;
            }
            if c == '|' {
                // `\w word|strong="…"\w*`: skip attributes up to the closing marker.
                while chars.peek().is_some_and(|&(_, ch)| ch != '\\') {
                    chars.next();
                }
                continue;
            }
            if c == '¶' {
                if !cur.is_empty() || self.slot_has_tokens() {
                    return Err(format!("{here}: pilcrow after the first token"));
                }
                let slot = self.slot();
                if self.text.pilcrow.last() != Some(&slot) {
                    self.text.pilcrow.push(slot);
                }
                continue;
            }
            if c.is_whitespace() {
                if joining {
                    return Err(format!("{here}: hyphen before a space"));
                }
                flush(&mut self.text, &mut cur, &mut cur_flags, &mut cur_letter)?;
                continue;
            }
            joining = false;
            cur.push(c);
            if c.is_alphabetic() {
                cur_letter = true;
                cur_flags |= st.bits();
            }
            if c == '-' {
                // A hyphen ends a word (`Beth-el` = `Beth-` + `el`); the first part is shown without a space after.
                cur_flags |= JOINED;
                flush(&mut self.text, &mut cur, &mut cur_flags, &mut cur_letter)?;
                joining = true;
            }
        }
        if joining {
            return Err(format!("{here}: hyphen at the end of a verse"));
        }
        flush(&mut self.text, &mut cur, &mut cur_flags, &mut cur_letter)?;
        if st.add + st.nd + st.wj + st.tl != 0 {
            return Err(format!("{here}: unclosed character style"));
        }
        Ok(())
    }

    fn finish_book(&mut self) -> Result<(), String> {
        if !self.pending_breaks.is_empty() {
            return Err(format!("{}: trailing breaks {:?}", self.book_code, self.pending_breaks));
        }
        let last = self.slot();
        for text in self.pending_headings.drain(..) {
            self.text.headings.push(Heading { slot: last, pos: HeadingPos::After, text });
        }
        let t = &mut self.text;
        let (chapters, slots, tokens) = (t.chapters(), t.slots(), t.tokens());
        let last_book = t.books.len() - 1;
        let b = &mut t.books[last_book];
        b.chapters = chapters - b.first_chapter;
        b.slots = slots - b.first_slot;
        b.tokens = tokens - b.first_word;
        Ok(())
    }
}

/// Strip simple character markers (`\tl`, `\add`, …) from heading/title text.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let n = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '*')).unwrap_or(after.len());
        rest = &after[n..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Book source files in canonical order.
///
/// # Errors
/// If `src` cannot be read.
pub fn source_files(src: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "usfm"))
        .collect();
    files.sort();
    Ok(files)
}

/// Tokenize and index every book in `src` (spec: docs/text-format.md).
///
/// # Errors
/// On unreadable files and on anything in the source the spec does not cover (unknown markers, verse or chapter
/// numbering gaps, punctuation-only tokens, …): a source surprise must stop the build, not shift ordinals.
pub fn build(src: &Path) -> Result<Text, String> {
    let mut b = Builder {
        text: Text::default(),
        book_code: String::new(),
        chapter: 0,
        verse: None,
        pending_breaks: vec![],
        pending_headings: vec![],
    };

    for path in source_files(src)? {
        let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stem = path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
        b.book_code.clear();
        b.chapter = 0;
        b.verse = None;
        b.check_book_capacity()?;
        b.text.book_first_chapter.push(narrow16(b.text.book_of_chapter.len()));
        b.text.books.push(Book {
            code: String::new(),
            file_key: stem.get(..6).unwrap_or(&stem).to_string(),
            id: stem
                .get(..2)
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("{}: file name must start with a 2-digit book id", path.display()))?,
            name_long: String::new(),
            name_short: String::new(),
            abbr: String::new(),
            titles: vec![],
            first_chapter: narrow32(b.text.book_of_chapter.len()),
            chapters: 0,
            first_slot: narrow32(b.text.chapter_of_verse.len()),
            slots: 0,
            first_word: narrow32(b.text.words.len()),
            first_ord: 0,
            tokens: 0,
        });

        for line in raw.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let (marker, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            let last_book = b.text.books.len() - 1;
            let book = &mut b.text.books[last_book];
            match marker {
                "\\id" => {
                    let code = rest.split_whitespace().next().unwrap_or_default().to_string();
                    book.code.clone_from(&code);
                    b.book_code = code;
                }
                "\\h" => {}
                "\\toc1" => book.name_long = plain(rest),
                "\\toc2" => book.name_short = plain(rest),
                "\\toc3" => book.abbr = plain(rest),
                "\\mt1" | "\\mt2" => book.titles.push(plain(rest)),
                "\\c" => {
                    let n = rest.parse().map_err(|_| format!("{}: bad chapter {rest:?}", b.here()))?;
                    b.start_chapter(n)?;
                }
                "\\d" => {
                    if b.verse != Some(0) || b.slot_has_tokens() {
                        return Err(format!("{}: \\d must precede verse 1", b.here()));
                    }
                    b.attach_pending();
                    b.tokenize(rest)?;
                }
                "\\v" => {
                    let (num, body) = rest.split_once(' ').unwrap_or((rest, ""));
                    let n = num.parse().map_err(|_| format!("{}: bad verse {num:?}", b.here()))?;
                    b.start_verse(n)?;
                    b.attach_pending();
                    b.tokenize(body)?;
                    if !b.slot_has_tokens() {
                        return Err(format!("{}: empty verse", b.here()));
                    }
                }
                "\\p" | "\\q1" | "\\b" => {
                    if !rest.is_empty() {
                        return Err(format!("{}: text after {marker}", b.here()));
                    }
                    b.pending_breaks.push(marker[1..].to_string());
                }
                "\\s1" => b.pending_headings.push(plain(rest)),
                _ => return Err(format!("{}: unexpected line marker {marker}", b.here())),
            }
        }
        if b.book_code.is_empty() {
            return Err(format!("{}: no \\id", path.display()));
        }
        b.finish_book()?;
    }

    let mut t = b.text;
    if t.books.is_empty() {
        return Err(format!("{}: no .usfm files", src.display()));
    }
    t.chapter_first_verse.push(narrow32(t.chapter_of_verse.len()));
    t.book_first_chapter.push(narrow16(t.book_of_chapter.len()));
    t.slot_first_word.push(t.tokens());
    sparsify(&mut t)?;
    Ok(t)
}

/// Convert the builder's dense word indices (verse starts, note anchors) to address ordinals (`make_ord`).
fn sparsify(t: &mut Text) -> Result<(), String> {
    let mut prev_id = None;
    let dense = t.slot_first_word.clone();
    t.verse_start = vec![0; t.slots() as usize];
    t.slot_words = vec![0; t.slots() as usize];
    for (bi, b) in t.books.iter_mut().enumerate() {
        if prev_id.is_some_and(|p| p >= b.id) {
            return Err(format!("{}: book ids must increase in canonical order", b.code));
        }
        prev_id = Some(b.id);
        if b.chapters > MAX_FIELD {
            return Err(format!("{}: more than {MAX_FIELD} chapters", b.code));
        }
        // Backwards, so an empty slot can start where the next slot with words starts.
        let mut next = None;
        for slot in (b.first_slot as usize..(b.first_slot + b.slots) as usize).rev() {
            let ch = usize::from(t.chapter_of_verse[slot]);
            let chapter = narrow32(ch) - u32::from(t.book_first_chapter[bi]) + 1;
            let verse = narrow32(slot) - t.chapter_first_verse[ch];
            let words = dense[slot + 1] - dense[slot];
            if verse > MAX_FIELD || words > MAX_FIELD {
                return Err(format!("{} {chapter}:{verse}: verse number or word count over {MAX_FIELD}", b.code));
            }
            t.slot_words[slot] = narrow8(words as usize);
            if words > 0 {
                let start = make_ord(b.id, chapter, verse, 0);
                if next.is_none() {
                    t.book_end.push(start + words);
                }
                next = Some(start);
            }
            t.verse_start[slot] = next.ok_or_else(|| format!("{}: no words", b.code))?;
        }
        b.first_ord = t.verse_start[b.first_slot as usize];
    }
    t.verse_start.push(*t.book_end.last().unwrap());
    let notes = std::mem::take(&mut t.notes);
    t.notes = notes.into_iter().map(|n| Note { after: t.ord_of_word(n.after), ..n }).collect();
    Ok(())
}

/// Serialize the binary index (layout in docs/text-format.md).
#[must_use]
pub fn index_bytes(t: &Text) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(INDEX_MAGIC);
    for v in [INDEX_VERSION, narrow32(t.books.len()), t.chapters(), t.slots(), t.tokens()] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    t.verse_start.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    t.book_end.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    t.chapter_first_verse.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    t.chapter_of_verse.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    t.book_first_chapter.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    out.extend_from_slice(&t.book_of_chapter);
    out.extend(t.books.iter().map(|b| b.id));
    out.extend_from_slice(&t.slot_words);
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Per-book golden entry. Per-book on purpose: adding a book must not disturb existing books' entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoldenBook {
    pub code: String,
    pub id: u8,
    pub chapters: u32,
    pub slots: u32,
    pub tokens: u32,
    /// sha256 of the book's tokens joined by `\n`.
    pub tokens_sha256: String,
    /// sha256 of the book's words as displayed (hyphen-joined parts put back together) joined by `\n`: pins the
    /// text itself apart from how it is split into words.
    pub display_sha256: String,
    /// sha256 of the book's verse starts without the book id bits (u32 LE). With address ordinals this only pins
    /// which slots are empty, so:
    pub verse_starts_sha256: String,
    /// sha256 of the book's words per slot (one byte each): pins where verses split the word stream, i.e. every
    /// word's ordinal (moving a word into the next verse changes no other hash).
    pub slot_words_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Golden {
    pub text_id: String,
    pub book_shift: u32,
    pub books: Vec<GoldenBook>,
}

#[must_use]
pub fn golden(t: &Text) -> Golden {
    let books = t
        .books
        .iter()
        .map(|b| {
            let words = &t.words[b.first_word as usize..(b.first_word + b.tokens) as usize];
            let slots = &t.verse_start[b.first_slot as usize..(b.first_slot + b.slots) as usize];
            let vs: Vec<u8> = slots.iter().flat_map(|v| (v & ((1 << BOOK_SHIFT) - 1)).to_le_bytes()).collect();
            let sw = &t.slot_words[b.first_slot as usize..(b.first_slot + b.slots) as usize];
            GoldenBook {
                code: b.code.clone(),
                id: b.id,
                chapters: b.chapters,
                slots: b.slots,
                tokens: b.tokens,
                tokens_sha256: sha256_hex(words.join("\n").as_bytes()),
                display_sha256: sha256_hex(t.display_words(b).join("\n").as_bytes()),
                verse_starts_sha256: sha256_hex(&vs),
                slot_words_sha256: sha256_hex(sw),
            }
        })
        .collect();
    Golden { text_id: TEXT_ID.to_string(), book_shift: BOOK_SHIFT, books }
}

/// Half-open ordinal runs `[s, e)` of the book's words whose flags contain `bit` (ends as for any interval: the
/// next word's ordinal, or the book's end).
fn runs(t: &Text, b: &Book, bit: u8) -> Vec<[u32; 2]> {
    let mut out: Vec<[u32; 2]> = vec![];
    for w in b.first_word..b.first_word + b.tokens {
        if t.flags[w as usize] & bit != 0 {
            let (ord, end) = (t.ord_of_word(w), t.end_after_word(w));
            match out.last_mut() {
                Some(r) if r[1] == ord => r[1] = end,
                _ => out.push([ord, end]),
            }
        }
    }
    out
}

#[derive(Serialize)]
struct BookFile<'a> {
    book: &'a str,
    id: u8,
    first_ord: u32,
    first_slot: u32,
    words: &'a [String],
    italic: Vec<[u32; 2]>,
    divine_name: Vec<[u32; 2]>,
    words_of_jesus: Vec<[u32; 2]>,
    joined: Vec<u32>,
    pilcrow: Vec<u32>,
    breaks: Vec<&'a (u32, String)>,
    headings: Vec<&'a Heading>,
    notes: Vec<&'a Note>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestBook {
    pub code: String,
    pub id: u8,
    pub first_ord: u32,
    pub name_long: String,
    pub name_short: String,
    pub abbr: String,
    pub titles: Vec<String>,
    pub chapters: u32,
    pub tokens: u32,
    pub file: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub text_id: String,
    pub source: String,
    pub index: String,
    pub book_shift: u32,
    pub tokens: u32,
    pub books: Vec<ManifestBook>,
}

fn hashed_name(stem: &str, ext: &str, bytes: &[u8]) -> String {
    format!("{stem}.{}.{ext}", &sha256_hex(bytes)[..16])
}

/// Write content-hashed artifacts into `out` (created; stale files are not removed).
///
/// # Errors
/// If `out` cannot be created or written.
pub fn write_artifacts(t: &Text, out: &Path) -> Result<Manifest, String> {
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let write = |name: &str, bytes: &[u8]| fs::write(out.join(name), bytes).map_err(|e| format!("{name}: {e}"));

    let index = index_bytes(t);
    let index_name = hashed_name("index", "bin", &index);
    write(&index_name, &index)?;

    let mut books = vec![];
    for (i, b) in t.books.iter().enumerate() {
        let ords = b.first_ord..t.book_end[i];
        let slots = b.first_slot..b.first_slot + b.slots;
        let file = BookFile {
            book: &b.code,
            id: b.id,
            first_ord: b.first_ord,
            first_slot: b.first_slot,
            words: &t.words[b.first_word as usize..(b.first_word + b.tokens) as usize],
            italic: runs(t, b, ITALIC),
            divine_name: runs(t, b, DIVINE_NAME),
            words_of_jesus: runs(t, b, WORDS_OF_JESUS),
            joined: (b.first_word..b.first_word + b.tokens)
                .filter(|&w| t.flags[w as usize] & JOINED != 0)
                .map(|w| t.ord_of_word(w))
                .collect(),
            pilcrow: t.pilcrow.iter().copied().filter(|s| slots.contains(s)).collect(),
            breaks: t.breaks.iter().filter(|(s, _)| slots.contains(s)).collect(),
            headings: t.headings.iter().filter(|h| slots.contains(&h.slot)).collect(),
            notes: t.notes.iter().filter(|n| ords.contains(&n.after)).collect(),
        };
        let bytes = serde_json::to_vec(&file).map_err(|e| e.to_string())?;
        let name = hashed_name(&b.file_key, "json", &bytes);
        write(&name, &bytes)?;
        books.push(ManifestBook {
            code: b.code.clone(),
            id: b.id,
            first_ord: b.first_ord,
            name_long: b.name_long.clone(),
            name_short: b.name_short.clone(),
            abbr: b.abbr.clone(),
            titles: b.titles.clone(),
            chapters: b.chapters,
            tokens: b.tokens,
            file: name,
        });
    }

    let manifest = Manifest {
        text_id: TEXT_ID.to_string(),
        source: "eBible.org eng-kjv2006 USFM (2026-09-26), see data/source/eng-kjv2006/SOURCE.md".to_string(),
        index: index_name,
        book_shift: BOOK_SHIFT,
        tokens: t.tokens(),
        books,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    write("manifest.json", &bytes)?;
    Ok(manifest)
}

/// Repo root, for default paths.
#[must_use]
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[must_use]
pub fn default_source() -> PathBuf {
    repo_root().join("data/source/eng-kjv2006")
}

#[must_use]
pub fn golden_path() -> PathBuf {
    repo_root().join(format!("fixtures/text/{TEXT_ID}.golden.json"))
}
