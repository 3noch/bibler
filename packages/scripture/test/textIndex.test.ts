import { test } from "node:test";
import assert from "node:assert/strict";
import { BOOKS } from "../src/books.ts";
import { BOOK_SPAN, bookIdOf, localOf, makeOrd } from "../src/textIndex.ts";
import { kjv, bookWords } from "./kjv.ts";

const { index, manifest } = kjv;

test("book table matches the built text", () => {
  assert.equal(index.books, BOOKS.length);
  assert.deepEqual(manifest.books.map((b) => b.code), BOOKS.map((b) => b.code));
  assert.deepEqual([...index.bookId], BOOKS.map((b) => b.id));
  assert.deepEqual(manifest.books.map((b) => b.first_ord), BOOKS.map((b) => b.id * BOOK_SPAN));
  manifest.books.forEach((b, i) => assert.equal(index.chapterCount(i), b.chapters, b.code));
  assert.equal(index.tokens, 790676);
  assert.equal(index.slots, 32291);
});

test("every ordinal round-trips ord -> ref -> interval; gaps between books are not words", () => {
  let total = 0;
  for (let b = 0; b < index.books; b++) {
    const [bs, be] = index.bookInterval(b);
    assert.equal(bs, makeOrd(BOOKS[b]!.id, 0));
    for (let ord = bs; ord < be; ord++) {
      const { book, chapter, verse } = index.locate(ord);
      const [s, e] = index.verseInterval(book, chapter, verse);
      if (book !== b || !(s <= ord && ord < e)) assert.fail(`ord ${ord} -> ${book} ${chapter}:${verse} -> [${s}, ${e})`);
    }
    total += be - bs;
    assert.equal(index.bookOf(be), -1);
    assert.equal(index.bookOf(bs - 1), -1);
    assert.throws(() => index.locate(be));
  }
  assert.equal(total, index.tokens);
  assert.equal(index.bookOf(0), -1);
  assert.equal(index.bookOf(2 ** 32), -1);
});

test("ordinal helpers", () => {
  const ord = makeOrd(96, 12345);
  assert.equal(ord, 96 * 16777216 + 12345);
  assert.deepEqual([bookIdOf(ord), localOf(ord)], [96, 12345]);
  // Ids >= 128 exceed int32, where `ord >> 24` would go negative; the helpers must not.
  const big = makeOrd(200, 7);
  assert.ok(big > 2 ** 31);
  assert.deepEqual([bookIdOf(big), localOf(big)], [200, 7]);
  const [gs] = index.bookInterval(0);
  const [, re] = index.bookInterval(65);
  assert.equal(index.wordCount([gs, re]), index.tokens, "word count skips the gaps");
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
    return w.words.slice(s - w.first_ord, e - w.first_ord).join(" ");
  };
  assert.equal(words("JHN", 3, 16).slice(0, 26), "For God so loved the world");
  assert.equal(words("PSA", 51, 0).slice(0, 26), "To the chief Musician, A P");
  assert.equal(words("PSA", 23, 0), "A Psalm of David.");
  assert.equal(words("GEN", 1, 0), "");
  assert.equal(index.hasTitle(18, 3), true);
  assert.equal(index.hasTitle(18, 1), false);
  assert.deepEqual(index.locate(index.bookEnd[65]! - 1), { book: 65, chapter: 22, verse: 21 });
  assert.deepEqual(index.locate(makeOrd(70, 0)), { book: 39, chapter: 1, verse: 1 });
  assert.throws(() => index.locate(makeOrd(41, 0)), "41 is reserved (Tobit) and absent");
  assert.throws(() => index.locate(-1));
});
