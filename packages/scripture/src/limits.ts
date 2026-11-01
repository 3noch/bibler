// Reading a licensed text a page at a time. Its license limits how much may be shown (and kept on a device) at once:
// at most `verses` verses and at most `book_percent` percent of any one book (the ESV API's terms: 500 verses or
// half a book, whichever is less). A page is a chunk of one chapter: the whole chapter when it fits, else equal
// parts (Jude 1–12 | 13–25), plus the verse just before and just after it, so tags across a boundary can be seen
// and painted. Verses are counted as slots with words (an empty title slot or an omitted verse shows nothing).
// The server checks the same rule (`fits`) on every page it hands out (crates/api `limits`).
import type { TextIndex } from "./textIndex.ts";

export interface Limits {
  verses: number;
  book_percent: number;
}

/** A page's own verses: slots `from..to` (inclusive) of one chapter. */
export interface Chunk {
  from: number;
  to: number;
}

const bookOfSlot = (ix: TextIndex, slot: number) => ix.bookOfChapter[ix.chapterOfVerse[slot]!]!;
const bookSlots = (ix: TextIndex, book: number): [number, number] => [
  ix.chapterFirstVerse[ix.bookFirstChapter[book]!]!,
  ix.chapterFirstVerse[ix.bookFirstChapter[book + 1]!]!,
];
const verses = (ix: TextIndex, from: number, to: number) => {
  let n = 0;
  for (let s = from; s <= to; s++) if (ix.slotWords[s]! > 0) n++;
  return n;
};

/** The most verses of `book` that may show at once. */
export function capOf(ix: TextIndex, book: number, limits: Limits): number {
  const [lo, hi] = bookSlots(ix, book);
  return Math.min(limits.verses, Math.floor((verses(ix, lo, hi - 1) * limits.book_percent) / 100));
}

/** Whether slots `from..to` may show at once: within the cap of each book they touch, and of the whole. */
export function fits(ix: TextIndex, limits: Limits, from: number, to: number): boolean {
  if (from > to || from < 0 || to >= ix.slots) return false;
  let total = 0;
  for (let s = from; s <= to; ) {
    const book = bookOfSlot(ix, s);
    const end = Math.min(to, bookSlots(ix, book)[1] - 1);
    const n = verses(ix, s, end);
    if (n > capOf(ix, book, limits)) return false;
    total += n;
    s = end + 1;
  }
  return total <= limits.verses;
}

/** Chapter `ch`'s pages: one chunk if it fits with a context verse on each side, else equal parts that do. */
export function chunksOf(ix: TextIndex, ch: number, limits: Limits | null): Chunk[] {
  const [a, b] = [ix.chapterFirstVerse[ch]!, ix.chapterFirstVerse[ch + 1]! - 1];
  if (!limits) return [{ from: a, to: b }];
  const budget = Math.max(1, capOf(ix, ix.bookOfChapter[ch]!, limits) - 2);
  const n = verses(ix, a, b);
  const parts = Math.max(1, Math.ceil(n / budget));
  if (parts === 1) return [{ from: a, to: b }];
  // Equal parts by verse count; empty slots go with the verse after them (a title with the first part).
  const out: Chunk[] = [];
  let from = a;
  let seen = 0;
  for (let p = 1; p < parts; p++) {
    const want = Math.round((n * p) / parts);
    let s = from;
    while (seen < want) if (ix.slotWords[s++]! > 0) seen++;
    out.push({ from, to: s - 1 });
    from = s;
  }
  out.push({ from, to: b });
  return out;
}

/** The chunk of chapter `ch` holding `slot` (the first if `slot` is outside the chapter), and its position. */
export function chunkAt(ix: TextIndex, ch: number, slot: number, limits: Limits | null): { chunk: Chunk; part: number; parts: number } {
  const list = chunksOf(ix, ch, limits);
  const part = Math.max(0, list.findIndex((c) => slot >= c.from && slot <= c.to));
  return { chunk: list[part]!, part, parts: list.length };
}

/** A page's slots: the chunk plus the nearest verse with words on each side (in a neighbouring chapter or book). */
export function pageSlots(ix: TextIndex, c: Chunk): [number, number] {
  let from = c.from - 1;
  while (from >= 0 && ix.slotWords[from] === 0) from--;
  let to = c.to + 1;
  while (to < ix.slots && ix.slotWords[to] === 0) to++;
  return [from >= 0 ? from : c.from, to < ix.slots ? to : c.to];
}
