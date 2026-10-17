import { test } from "node:test";
import assert from "node:assert/strict";
import { BOOKS } from "../src/books.ts";
import { bookIdOf, makeOrd } from "../src/textIndex.ts";
import { kjv, bookWords } from "./kjv.ts";

const { index, manifest } = kjv;

test("book table matches the built text", () => {
  assert.equal(index.books, BOOKS.length);
  assert.deepEqual(manifest.books.map((b) => b.code), BOOKS.map((b) => b.code));
  assert.deepEqual([...index.bookId], BOOKS.map((b) => b.id));
  manifest.books.forEach((b, i) => assert.equal(b.first_ord, index.bookInterval(i)[0], b.code));
  manifest.books.forEach((b, i) => assert.equal(index.chapterCount(i), b.chapters, b.code));
  assert.equal(index.tokens, 791484);
  assert.equal(index.slots, 32291);
});

test("every word round-trips pos -> ord -> ref -> interval; gaps belong to the word before them", () => {
  let p = 0;
  for (let b = 0; b < index.books; b++) {
    const [bs, be] = index.bookInterval(b);
    assert.equal(index.ordAt(p), bs);
    assert.equal(bs, makeOrd(BOOKS[b]!.id, 1, 1, 0), "every book starts at 1:1 (Psalm 1 has no title)");
    for (let ord = bs; ord < be; ord = index.after(ord), p++) {
      if (index.ordAt(p) !== ord || index.pos(ord) !== p || !index.isWord(ord)) assert.fail(`ord ${ord} at ${p}`);
      const { book, chapter, verse } = index.locate(ord);
      const [s, e] = index.verseInterval(book, chapter, verse);
      if (book !== b || !(s <= ord && ord < e)) assert.fail(`ord ${ord} -> ${book} ${chapter}:${verse} -> [${s}, ${e})`);
      if (ord !== makeOrd(BOOKS[b]!.id, chapter, verse, ord - s)) assert.fail(`ord ${ord} is not its address`);
      // The ordinal after a word is either the next word or a gap owned by this word.
      const next = ord + 1;
      if (next < be && !index.isWord(next)) {
        if (index.pos(next) !== p + 1 || index.locate(next).verse !== verse || index.slotOf(next) !== index.slotOf(ord)) {
          assert.fail(`gap ${next} after ${ord}`);
        }
      }
    }
    assert.equal(index.pos(be), p, "a book's end is the position after its last word");
    assert.equal(index.bookOf(be), -1);
    assert.equal(index.bookOf(bs - 1), -1);
    assert.throws(() => index.locate(be));
  }
  assert.equal(p, index.tokens);
  assert.equal(index.bookOf(0), -1);
  assert.equal(index.bookOf(2 ** 32), -1);
});

test("ordinal helpers", () => {
  const ord = makeOrd(96, 22, 21, 5);
  assert.equal(ord, 96 * 2 ** 24 + 22 * 2 ** 16 + 21 * 2 ** 8 + 5);
  // Ids >= 128 exceed int32, where `ord >> 24` would go negative; the helpers must not.
  const big = makeOrd(200, 1, 1, 7);
  assert.ok(big > 2 ** 31);
  assert.equal(bookIdOf(big), 200);
  const [gs] = index.bookInterval(0);
  const [, re] = index.bookInterval(65);
  assert.equal(index.wordCount([gs, re]), index.tokens, "word count skips the gaps");
  // John 11:35 "Jesus wept." ends at 11:36's first word; the gap after "wept." is not a word.
  const jw = makeOrd(73, 11, 35, 0);
  assert.deepEqual(index.verseInterval(42, 11, 35), [jw, makeOrd(73, 11, 36, 0)]);
  assert.equal(index.wordCount(index.verseInterval(42, 11, 35)), 2);
  assert.equal(index.after(jw + 1), makeOrd(73, 11, 36, 0));
  assert.equal(index.isWord(jw + 2), false);
  // The last word of a chapter ends at the next chapter's first word (Psalm 3's title), the last of a book at the book's end.
  const ps2End = index.verseInterval(18, 2, 12)[1];
  assert.equal(ps2End, makeOrd(20, 3, 0, 0));
  assert.equal(index.after(index.ordAt(index.pos(index.bookEnd[0]!) - 1)), index.bookEnd[0]);
});

test("every verse slot round-trips ref -> slot -> ref", () => {
  for (let book = 0; book < index.books; book++) {
    const [bs, be] = index.bookInterval(book);
    let covered = 0;
    for (let ch = 1; ch <= index.chapterCount(book); ch++) {
      const [cs, ce] = index.chapterInterval(book, ch);
      covered += ce - cs;
      for (let v = 0; v <= index.verseCount(book, ch); v++) {
        const slot = index.slotIndex(book, ch, v);
        assert.deepEqual(index.locateSlot(slot), { book, chapter: ch, verse: v });
      }
      assert.equal(index.slotIndex(book, ch, index.verseCount(book, ch) + 1), -1);
    }
    assert.equal(covered, be - bs, BOOKS[book]!.code);
    assert.equal(index.chapterIndex(book, index.chapterCount(book) + 1), -1);
    assert.equal(index.chapterIndex(book, 0), -1);
  }
});

test("landmarks", () => {
  const words = (code: string, ch: number, v: number) => {
    const b = BOOKS.findIndex((x) => x.code === code);
    const [s, e] = index.verseInterval(b, ch, v);
    const w = bookWords(code);
    const first = index.pos(w.first_ord);
    return w.words.slice(index.pos(s) - first, index.pos(e) - first).join(" ");
  };
  assert.equal(words("JHN", 3, 16).slice(0, 26), "For God so loved the world");
  assert.equal(words("PSA", 51, 0).slice(0, 26), "To the chief Musician, A P");
  assert.equal(words("PSA", 23, 0), "A Psalm of David.");
  assert.equal(words("GEN", 1, 0), "");
  assert.equal(index.hasTitle(18, 3), true);
  assert.equal(index.hasTitle(18, 1), false);
  assert.deepEqual(index.locate(index.bookEnd[65]! - 1), { book: 65, chapter: 22, verse: 21 });
  assert.deepEqual(index.locate(makeOrd(70, 1, 1, 0)), { book: 39, chapter: 1, verse: 1 });
  assert.throws(() => index.locate(makeOrd(41, 1, 1, 0)), "41 is reserved (Tobit) and absent");
  assert.throws(() => index.locate(makeOrd(2, 1, 0, 0)), "Genesis has no title: before its first word");
  assert.throws(() => index.locate(-1));
});

test("byBook cuts an interval at book boundaries (one piece per book with words)", () => {
  const gen = BOOKS.findIndex((b) => b.code === "GEN");
  const exo = gen + 1;
  const [gs, ge] = index.bookInterval(gen);
  const [es] = index.bookInterval(exo);
  const lastGen = index.ordAt(index.pos(ge) - 1);
  const iv = [lastGen, index.after(es)] as const; // Gen 50:26's last word + Exodus' first word
  assert.deepEqual(index.byBook(iv), [[lastGen, ge], [es, index.after(es)]]);
  assert.deepEqual(index.byBook([gs, index.after(gs)]), [[gs, index.after(gs)]], "within a book: unchanged");
  assert.deepEqual(index.byBook([ge, es]), [], "between books there are no words");
  assert.equal(index.byBook(index.bookInterval(gen)).length, 1);
  assert.equal(index.byBook([gs, index.bookEnd[index.books - 1]!]).length, index.books, "the whole Bible: every book");
});
