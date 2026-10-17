# text-build: KJV dataset, tokenizer, index

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
  Per-token flags as ordinal intervals: `italic` (`\add`), `divine_name` (`\nd`), `words_of_jesus` (`\wj`). Per-slot:
  `pilcrow` (KJV `¶`, 2,970) and `breaks` (`p`/`q1`/`b`, CrossWire layout, not 1769).
- Non-token: book titles/names, Ps 119 letter headings (anchored before a slot), 14 epistle subscriptions (after
  the book's last slot), 6,959 KJV marginal notes (anchored after an ordinal).
- The parser is strict: any unknown USFM marker is a build error (that's how footnote-internal `\+nd` was found).
