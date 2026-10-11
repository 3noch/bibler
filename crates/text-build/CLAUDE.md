# text-build: KJV dataset, tokenizer, index

(Moved from the root CLAUDE.md; spec: `docs/text-format.md`.)

## Text: dataset, tokenizer, index (DONE for KJV; spec in `docs/text-format.md`)
Source: eBible.org `eng-kjv2006` USFM (1769 text, public domain), vendored verbatim in `data/source/eng-kjv2006/`
(see `SOURCE.md`; never edit or refresh it). Protocanon only: **Apocrypha is not in `kjv-1769-v1`** (if wanted
later, a separate `text_id`). `text-build` (`cargo run -p text-build --release`) writes content-hashed artifacts to
`build/text/kjv-1769-v1/` (gitignored): `index.<hash>.bin` (200 KB, 54 KB gz), one JSON per book (1.7 MB gz total),
`manifest.json`. It verifies `fixtures/text/kjv-1769-v1.golden.json` (tokens 790,676; slots 32,291; per-book counts;
token-stream + verseStart sha256); `--write-golden` regenerates it, which is legitimate only before any user has
taggings. Token stream cross-checked byte-identical against an independent Python tokenizer.

- **Verse slot 0 exists in every chapter**; the source has Psalm titles as `\d` (116, 1,034 tokens), so no patch
  file was needed. `Ps 3:0` and `Ps 3:title` should parse; formatter renders slot 0 as `Ps 3:title`.
- Tokens: whitespace-delimited, punctuation attached, hyphenated words whole, text verbatim (`’` kept).
  Per-token flags as ordinal runs: `italic` (`\add`), `divine_name` (`\nd`), `words_of_jesus` (`\wj`). Per-slot:
  `pilcrow` (KJV `¶`, 2,970) and `breaks` (`p`/`q1`/`b`, CrossWire layout, not 1769).
- Non-token: book titles/names, Ps 119 letter headings (anchored before a slot), 14 epistle subscriptions (after
  the book's last slot), 6,959 KJV marginal notes (anchored after an ordinal).
- The parser is strict: any unknown USFM marker is a build error (that's how footnote-internal `\+nd` was found).
