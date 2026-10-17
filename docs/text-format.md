# Scripture text: tokenizer spec and build artifacts

Status: **frozen for `kjv-1769-v2`** once any user has taggings (golden file `fixtures/text/kjv-1769-v2.golden.json`).
Changing anything below that affects ordinals then requires a new `text_id` and a remap. (`kjv-1769-v1` numbered
words by their position in the book and kept hyphenated words whole; it was replaced on 2026-10-12, before anyone
had data, with no migration.)

## Input

`data/source/eng-kjv2006/*.usfm` (see `SOURCE.md`): 66 books, one file per book, canonical order = filename
order (`02-GEN` … `96-REV`). Files have no BOM and no trailing newline; each file is parsed on its own.

## Verse slots

The unit of the index is the **verse slot**. Every chapter has slots `0..=n`, where `n` is the last verse:

- Slot 0 holds the chapter title (Psalm superscription, USFM `\d`) and is empty otherwise.
  116 Psalms have titles (1,034 tokens). Habakkuk 3:1 and 3:19 are ordinary verses (in the source too).
- Verses must be numbered `1, 2, 3, …` with no gaps or repeats, else the build fails.

Slots are numbered globally in canonical order (Gen 1:0 = slot 0, Gen 1:1 = slot 1, …). For KJV:
1,189 chapters, 31,102 verses, 32,291 slots.

## Tokens (ordinals)

Ordinals are **addresses**, one byte per field: `ord = bookId * 2^24 + chapter * 2^16 + verse * 2^8 + word`, with
chapter and verse as printed (verse 0 = the title slot) and `word` the 0-based position in the verse (Gen 1:1 "In"
= `0x02010100`). Book ids are stable and come from the source's file-number prefix: OT `02-GEN` … `40-MAL`, NT
`70-MAT` … `96-REV`; `41`…`69` are the deuterocanonical books' numbers in the same scheme (`41-TOB` … `58-2ES`),
i.e. between the Testaments where the 1611 KJV printed them. Decided 2026-10-12 (replacing book-local word
counts) so that other translations can share verse addresses and be indexed without their full text in hand.
Consequences:
- Adding a book shifts no existing ordinal, and a correction inside one verse only moves that verse's words.
- The same verse has the same ordinals in every text with the same versification; only the word field differs.
- Ordinals are sparse: within a book, only `slotWords[slot]` of each verse's 256 values are words. Never count
  words by subtracting ordinals (use `TextIndex.pos` / `wordCount`) and never walk ordinals one by one.
- Limits (the build refuses more): 255 chapters per book, verse numbers up to 255, 255 words per verse (KJV
  maxima: 150, 176, 90 in Esther 8:9). Ids >= 128 exceed int32, so JS must use `bookIdOf`/`makeOrd`, never `>>`.

**Intervals tile the line.** All intervals are half-open, and a word owns the ordinals from itself up to the next
word of its book (a book's last word owns only itself). So an interval **ends at the ordinal of the word after its
last word**, or at the book's end (last word + 1), never at `last + 1` inside a book: `TextIndex.after(ord)` gives
the end. That keeps "abutting" meaningful across verses (painting the end of one verse and then the start of the
next merges, exactly as before), and the server rejects any end that is neither a word nor a book end
(`api::check`, `bad_text`). A verse's interval is `[its first word, the next verse's first word)`; an empty title
slot starts where the next slot with words starts. Taggings at the end of one book and the start of the next never
abut.

Tokenizing a `\v N` / `\d` line:
1. Remove footnotes `\f … \f*` entirely (kept separately as notes, below).
2. Remove `\w`/`\+w` attributes (`|strong="…"`).
3. Remove character markers. An **opening** marker (`\add `, `\+add `, `\nd `, `\+nd `, `\wj `, `\w `, `\+w `)
   consumes exactly one following space. A **closing** marker (`\add*`, …) consumes nothing. So
   `\w heaven|strong="H8064"\w*,` becomes `heaven,`.
4. Remove `¶` (recorded as the slot's pilcrow flag; it only ever appears before the verse's first token).
5. Split on whitespace and **after each hyphen** (decided 2026-10-12, so either half of `Beth-el` can be tagged):
   `Beth-el` = `Beth-` + `el`, and `Beth-` is flagged `joined` (displayed with no space after it). Every non-empty
   piece is one token, punctuation attached (`heaven,` `LORD’s` `sin—;` `(for`); the em dash does not split. No
   token is punctuation-only in KJV, and a hyphen must be followed by more of the word (both checked by the
   build). KJV has 785 hyphenated words, which become 1,593 tokens.
6. Token text is stored exactly as in the source (U+2019 `’`, U+00E6 `æ`, U+2014 `—` kept). Case-folding and
   apostrophe normalization are a search concern, not a tokenizer concern.

Any other marker inside a verse is a build error, so source surprises surface instead of silently shifting ordinals.

### Per-token flags (stored as half-open ordinal runs)

- `italic`: any letter inside `\add` / `\+add` (KJV supplied words).
- `divine_name`: inside `\nd` / `\+nd` (LORD/GOD in small caps).
- `words_of_jesus`: inside `\wj`.
- `joined` (a list of ordinals, not runs): the token ends with a hyphen and is followed by no space.

Verified on the source: no markup boundary falls between two letters of one token, so the flags are exact for
letters. (360 italic spans include trailing punctuation; the flag ignores punctuation.)

### Per-slot flags

- `pilcrow`: the KJV's own paragraph mark `¶` (2,970; the KJV stops using them after Acts 20).
- `break`: a layout break from the source's `\p` (paragraph), `\q1` (poetry line), or `\b` (blank line), attached
  to the next slot that has tokens. These are the CrossWire editors' layout, not the 1769 text.

## Non-token material (no ordinals)

- Book titles: `\mt1` (+ `\mt2` for 1–2 Kings), names from `\toc1` (long), `\toc2` (short), `\toc3` (abbr).
- Section headings `\s1`: Psalm 119 letter headings (22, `\tl` letter + name) are anchored **before** the next
  slot; epistle subscriptions (14) have no following verse in their book and are anchored **after** the book's
  last slot.
- Notes (KJV marginal notes, `\f + \fr c.v \ft text\f*`, 6,959): anchored **after** the last token emitted before
  the note (ordinal), with the `\ft` text.

## Artifacts (`text-build` output, `build/text/<text_id>/`)

All multi-byte integers little-endian. File names carry a 16-hex content hash (sha256 prefix).

`index.<hash>.bin`:
```
magic "BTIX"  u32 version=3
u32 nBooks  u32 nChapters  u32 nSlots  u32 nTokens      // nTokens = word count, not the ordinal range
u32 verseStart[nSlots + 1]          // first ordinal of each slot (empty: the next non-empty slot's); sentinel = last book's end
u32 bookEnd[nBooks]                 // end ordinal (exclusive) of each book
u32 chapterFirstVerse[nChapters + 1]// first slot (= slot 0) of each chapter; sentinel = nSlots
u16 chapterOfVerse[nSlots]
u16 bookFirstChapter[nBooks + 1]    // sentinel = nChapters
u8  bookOfChapter[nChapters]
u8  bookId[nBooks]                  // book index -> stable id
u8  slotWords[nSlots]               // words in each slot: its words are verseStart[slot] + 0 .. + slotWords - 1
```
The index holds no text, so it can be built for a text whose words cannot be redistributed.
- ord → ref: `book = bookIndexOfId[floor(ord / 2^24)]` (reject if absent, before the book's first word or
  `ord >= bookEnd[book]`), then
  `slot = upperBound(verseStart, ord) - 1` over that book's slots (an empty slot 0 shares its start with slot 1,
  and upperBound skips it; a gap ordinal lands in the slot of the word owning it), `ch = chapterOfVerse[slot]`,
  `verse = slot - chapterFirstVerse[ch]`,
  `chapterNumber = ch - bookFirstChapter[book] + 1`.
- ref → interval: slot = `chapterFirstVerse[ch] + verse`; `[verseStart[slot], slotEnd(slot))`, where `slotEnd`
  is `verseStart[slot + 1]` except for a book's last slot, which ends at `bookEnd[book]` (not the next book).

`<NN>-<CODE>.<hash>.json` (one per book):
```
{ "book": "GEN", "id": 2, "first_ord": 33620224, "first_slot": 0,      // first_ord = the book's first word
  "words": ["In", "the", …],                    // reading order; ordinals come from the index (slot starts + i)
  "italic": [[s, e], …], "divine_name": [[s, e], …], "words_of_jesus": [[s, e], …],   // ord intervals (tiling)
  "joined": [ord, …],                                                                  // `Beth-` of `Beth-el`
  "pilcrow": [slot, …], "breaks": [[slot, "p"|"q1"|"b"], …],                         // global slots
  "headings": [{ "slot": n, "pos": "before"|"after", "text": "…" }],
  "notes": [{ "after": ord, "ref": "1.4", "text": "…" }] }
```

`manifest.json` (not hashed; the entry point): `text_id`, source info, `index` file name, `book_shift` (24), and for
each book its code, id, first word's ordinal, names, chapter count, token count and file name.

## Golden checks

`fixtures/text/kjv-1769-v2.golden.json` is **per book** (so adding a book only appends entries): code, id, chapters,
slots, tokens, sha256 of the book's tokens joined by `\n`, sha256 of its words as displayed (hyphen-joined parts put
back together; this equals v1's token hash, so the text itself did not change), sha256 of its verse starts
without the book id bits (with address ordinals this only pins which slots are empty), and sha256 of its words per
slot (one byte each; this pins where verses split the word stream, hence every word's ordinal: added 2026-10-17
after a review showed a word moving into the next verse passed every other check). `cargo test -p text-build` rebuilds from
the vendored source and fails on any difference. `text-build --write-golden` regenerates it (only legitimate before
any user has taggings).
