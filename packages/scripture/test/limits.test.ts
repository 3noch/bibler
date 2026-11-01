import { test } from "node:test";
import assert from "node:assert/strict";
import { chunkAt, chunksOf, fits, pageSlots, type Limits } from "../src/limits.ts";
import { formatInterval } from "../src/refs.ts";
import { bsb, kjv } from "./kjv.ts";

const ESV: Limits = { verses: 500, book_percent: 50 };

for (const t of [kjv, bsb]) {
  const ix = t.index;
  test(`limits (${t.manifest.text_id}): every page fits, chunks tile each chapter in order`, () => {
    for (let ch = 0; ch < ix.chapters; ch++) {
      const list = chunksOf(ix, ch, ESV);
      assert.equal(list[0]!.from, ix.chapterFirstVerse[ch]);
      assert.equal(list.at(-1)!.to, ix.chapterFirstVerse[ch + 1]! - 1);
      list.forEach((c, i) => {
        assert.ok(c.from <= c.to, `chapter ${ch} part ${i} is empty`);
        if (i) assert.equal(c.from, list[i - 1]!.to + 1, `chapter ${ch}: parts abut`);
        const [a, b] = pageSlots(ix, c);
        assert.ok(fits(ix, ESV, a, b), `chapter ${ch} part ${i} with its context doesn't fit`);
      });
      assert.deepEqual(chunksOf(ix, ch, null), [{ from: list[0]!.from, to: list.at(-1)!.to }], "no limits: whole chapters");
    }
  });
}

test("limits: only chapters over their book's cap are split", () => {
  const ix = bsb.index;
  const split: string[] = [];
  for (let ch = 0; ch < ix.chapters; ch++) {
    const list = chunksOf(ix, ch, ESV);
    if (list.length > 1) split.push(`${formatInterval(ix, [ix.verseStart[list[0]!.from]!, ix.slotEnd(list.at(-1)!.to)])}: ${list.length}`);
  }
  assert.deepEqual(split, ["Obadiah: 3", "Haggai 2: 2", "Philemon: 3", "2 John: 4", "3 John: 3", "Jude: 3"]);
});

test("limits: a page shows the verse before and after its chunk, across chapters and books", () => {
  const ix = bsb.index;
  const ch = (book: number, n: number) => ix.chapterIndex(book, n);
  const john3 = chunksOf(ix, ch(42, 3), ESV)[0]!; // John is book index 42
  const [a, b] = pageSlots(ix, john3);
  assert.equal(formatInterval(ix, [ix.verseStart[a]!, ix.slotEnd(a)]), "John 2:25");
  assert.equal(formatInterval(ix, [ix.verseStart[b]!, ix.slotEnd(b)]), "John 4:1");
  const obadiah = chunksOf(ix, ch(30, 1), ESV)[0]!; // Obadiah: after Amos 9:15
  assert.equal(formatInterval(ix, [ix.verseStart[pageSlots(ix, obadiah)[0]]!, ix.slotEnd(pageSlots(ix, obadiah)[0])]), "Amos 9:15");
  assert.equal(chunkAt(ix, ch(64, 1), ix.slotOf(ix.verseStart[ix.chapterFirstVerse[ch(64, 1)]! + 20]!), ESV).part, 2, "Jude 20 is on page 3");
});

test("limits: fits counts verses per book and in all", () => {
  const ix = bsb.index;
  const jude = ix.chapterFirstVerse[ix.chapterIndex(64, 1)]!;
  assert.ok(fits(ix, ESV, jude + 1, jude + 12), "12 of Jude's 25");
  assert.ok(!fits(ix, ESV, jude + 1, jude + 13), "13 of Jude's 25");
  const gen = ix.chapterFirstVerse[0]!;
  assert.ok(!fits(ix, ESV, gen, ix.chapterFirstVerse[ix.chapterIndex(0, 22)]!), "over 500 verses");
  assert.ok(!fits(ix, ESV, 10, 5), "backwards");
});
