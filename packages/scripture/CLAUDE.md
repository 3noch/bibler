# packages/scripture: text index and reference parsing

(Moved from the root CLAUDE.md.)

## TS text index + reference parsing (DONE; `packages/scripture`, spec = `fixtures/refs/parse.json`)
The server speaks ordinals only; parsing/formatting is client-side TS (Rust has no parser).
- `TextIndex` (`src/textIndex.ts`) wraps `index.<hash>.bin`: `locate(ord)`, `slotOf`, `verseInterval`,
  `chapterInterval`, `bookInterval`, `verseCount`, `chapterCount`, `hasTitle`. Tests round-trip all 790,676 ordinals
  and all 32,291 slots against the real build (tests load `build/text/`, so run `text-build` first; the loader
  checks the built index against the golden verseStart hash).
- Books (`src/books.ts`): 66 books, `name` / `refName` ("Psalm" for chapter refs) / `short` (SBL) + aliases;
  258 distinct normalized aliases (not ~500; brute-force fuzzy is trivially fast). Order must match the manifest
  (tested).
- Book resolution: normalize (lowercase; leading `I/II/III/1st/First…` + space or dot -> digit, so `Isaiah` is
  safe; strip spaces/dots) -> exact alias -> unique prefix -> ambiguous prefix (candidates, canonical order) ->
  if no prefix matched, numbered books also match without their digit (`cor`, `sam`, `kings`, `pet` are
  ambiguous; this stops `Cor` fuzzing to Colossians) -> fuzzy (OSA Damerau-Levenshtein, input >= 3 chars,
  1 edit <= 4 chars else 2; ties ambiguous; resolves with a `fuzzy_book` warning). `FORCED_AMBIGUOUS` makes `jon`
  ambiguous (Jonah/John). `jo`, `ju`, `jud`, `ph`, `ez`, `ma` are naturally ambiguous. A lone `s` resolves to Song
  (unique prefix; accepted). `completeBook(prefix)` drives autocomplete.
- `parseRef(index, input, ctx?)` -> `{ refs: {start, end, interval, span}[], intervals, warnings }` (each ref has
  its own book, since lists and ranges can span books). Warning codes: `unknown_book`, `ambiguous_book` (with
  candidates), `fuzzy_book`, `out_of_range`, `empty_title`, `reversed_range`, `no_context`, `syntax`. Bad items
  are reported and dropped; never clamped. Conventions: after a verse, a bare number after `,` is a verse
  (`Jn 3:16,18`), otherwise a chapter (`Gen 1,3`); `;` resets to chapter mode (`Jn 3:16; 4` = John 4); every
  comma item may name a new book; a book alone as a range endpoint = its start/end (`Genesis–Exodus`,
  `Titus 3–Philemon`); cross-book ranges are supported; `v5`/`vv 5-7` and bare `3:16`/`5` use ctx.
- `formatInterval(index, iv, "full"|"short")`: whole book (`Psalms`/`Ps`), whole chapters (`Psalm 23`,
  `John 3–4`; a chapter = slot 0 through its last verse, so `Ps 3:1-8` stays a verse range because the title is
  excluded), verses (`John 3:16–4:2`; single-chapter books omit the chapter: `Jude 5`; slot 0 renders `title`).
  Word-level intervals widen to whole verses. Invariant (property-tested, 3,500 random intervals, both styles):
  `parseRef(formatInterval(iv))` gives exactly `iv` with no warnings. Per-user abbreviation choice = the style arg.
