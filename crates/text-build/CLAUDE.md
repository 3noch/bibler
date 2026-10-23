# text-build: texts (KJV, BSB), tokenizer, index

(Moved from the root CLAUDE.md; spec: `docs/text-format.md`.)

## Text: dataset, tokenizer, index (DONE for KJV; spec in `docs/text-format.md`)
Source: eBible.org `eng-kjv2006` USFM (1769 text, public domain), vendored verbatim in `data/source/eng-kjv2006/`
(see `SOURCE.md`; never edit or refresh it). Protocanon only: **Apocrypha is not in `kjv-1769-v2`** (if wanted
later, a separate `text_id`). `text-build` (`cargo run -p text-build --release`) writes content-hashed artifacts to
`build/text/kjv-1769-v2/` (gitignored): `index.<hash>.bin` (v3, ~230 KB), one JSON per book, `manifest.json`. It
verifies `fixtures/text/kjv-1769-v2.golden.json` (tokens 791,484; slots 32,291; per-book counts; token-stream,
displayed-text, verseStart and words-per-slot sha256; the TS test loader checks the last two against the built
index); `--write-golden` regenerates it, which is legitimate only before any user has
taggings. The v1 token stream was cross-checked byte-identical against an independent Python tokenizer, and v2's
displayed text equals v1's token stream book by book.
- **Ordinals are addresses** (2026-10-12): `book << 24 | chapter << 16 | verse << 8 | word` (`make_ord`); intervals
  tile, so an end is the next word's ordinal or the book's end (`Text::end_after_word`). 8-bit limits are checked
  by the build. See `docs/text-format.md`.
- **Hyphens split words** (2026-10-12): `Beth-el` = `Beth-` (flag `JOINED`, book JSON `joined`) + `el`.

- **Verse slot 0 exists in every chapter**; the source has Psalm titles as `\d` (116, 1,034 tokens), so no patch
  file was needed. `Ps 3:0` and `Ps 3:title` should parse; formatter renders slot 0 as `Ps 3:title`.
- Tokens: whitespace-delimited, punctuation attached, split after hyphens, text verbatim (`’` kept).
  Per-token flags as ordinal intervals: `italic` (`\add`), `divine_name` (`\nd`), `words_of_jesus` (`\wj`). Per-slot
  `pilcrow` (KJV `¶`, 2,970); `breaks` before a word (KJV `p`/`q1`/`b`, 7,305, CrossWire layout, not 1769).
- Non-token: book titles/names, Ps 119 letter headings (anchored before a word), 14 epistle subscriptions (after
  the book's last word), 6,959 KJV marginal notes (anchored after an ordinal).
- The parser is strict: any unknown USFM marker is a build error (that's how footnote-internal `\+nd` was found).

## Several texts (2026-10-23: the BSB added; the user: tags show only in the text they were made in, for now)
- `SOURCES` in `lib.rs` lists each text (id, `data/source/<dir>`, abbr, name, origin, rights line); `text-build`
  builds all of them (or `--id`) into `build/text/<id>/` and writes `build/text/texts.json`; `--sources`,
  `--fixtures`, `--out` replace the repo paths (default.nix). Each has its own golden file.
- BSB `bsb-2026-v1`: eBible.org `engbsb` (public domain), vendored verbatim in `data/source/engbsb/` (never edit or
  refresh it either); 726,957 tokens, the KJV's 32,291 slots (16 omitted verses are empty slots: `BSB_OMITS` in the
  tests), ~1.8 MB gzipped like the KJV. What it needed (none of it changes a KJV ordinal; the KJV golden file is
  unchanged): skipped verse numbers -> empty slots; numerals are words; punctuation after a spaced footnote joins
  the word before, punctuation opening a verse joins the next; text after paragraph/poetry markers continues the
  verse, so breaks and headings are anchored at words (book JSON `breaks: [[ord, kind]]`, `headings: [{at, pos,
  kind, text}]`, which gave every KJV book file a new hash); more markers (`\m \pmo \pc \q2 \qr \li1 \li2`, headings
  `\s2 \r \ms1 \mr \qa`, `\it`); word attributes stripped from heading text; an empty `\d` ignored. Its source has
  typos we keep verbatim (Psalm 3's title "A Psalms of David"). Mutation-checked (8/8: empty slots, break and
  heading anchors, footnote glue, verse-opening punctuation, numerals, `\it`, heading attributes).
- Adding another eBible.org text: vendor it with a `SOURCE.md`, add a `Source`, run `text-build --id <id>
  --write-golden`, and expect new markers or punctuation cases to fail the build first (that is the point).
