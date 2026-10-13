/**
 * TextIndex: ordinal <-> reference over a text-build `index.<hash>.bin` (layout: docs/text-format.md).
 *
 * Ordinals are addresses: `bookId * 2^24 + chapter * 2^16 + verse * 2^8 + word` (`makeOrd`), where `bookId` is
 * the book's stable id (BookInfo.id), chapter and verse are the printed numbers (verse 0 = title) and `word` is the
 * position in the verse. So ordinals are sparse: only `slotWords` of each verse's 256 are words. Use
 * `bookIdOf`/`makeOrd`, not bitwise operators (JS bitwise ops are signed 32-bit).
 *
 * Intervals are half-open and tile the line: a word owns the ordinals up to the next word of its book, so an
 * interval ends at the word after its last word (`after`), or at the book's end (last word + 1). Never compute an
 * end as `last + 1`, and never count words by subtracting ordinals: use `pos` (a word's dense position) instead.
 *
 * Books, chapters and verse slots are 0-based indexes internally; chapter numbers are 1-based and verse numbers are
 * the slot within the chapter (0 = title slot, empty unless the chapter has a title).
 */

export type Interval = readonly [start: number, end: number];

export interface Location {
  book: number;
  chapter: number;
  verse: number;
}

export const BOOK_SHIFT = 24;
export const BOOK_SPAN = 2 ** BOOK_SHIFT;
export const bookIdOf = (ord: number) => Math.floor(ord / BOOK_SPAN);
/** The ordinal of word `word` (0-based) of `chapter:verse` in book `bookId`. */
export const makeOrd = (bookId: number, chapter: number, verse: number, word: number) =>
  bookId * BOOK_SPAN + chapter * 2 ** 16 + verse * 2 ** 8 + word;

const MAGIC = 0x58495442; // "BTIX" little-endian
const VERSION = 3;

export class TextIndex {
  readonly books: number;
  readonly chapters: number;
  readonly slots: number;
  /** Number of words (not the ordinal range, which is sparse). */
  readonly tokens: number;
  readonly verseStart: Uint32Array;
  readonly bookEnd: Uint32Array;
  readonly chapterFirstVerse: Uint32Array;
  readonly chapterOfVerse: Uint16Array;
  readonly bookFirstChapter: Uint16Array;
  readonly bookOfChapter: Uint8Array;
  readonly bookId: Uint8Array;
  /** Words in each slot. */
  readonly slotWords: Uint8Array;
  /** Dense position (0..tokens) of each slot's first word, plus a sentinel. */
  readonly slotPos: Uint32Array;
  /** book id -> book index, -1 if absent. */
  private readonly bookById: Int16Array;

  constructor(buf: ArrayBuffer) {
    if (new Uint8Array(new Uint16Array([1]).buffer)[0] !== 1) throw new Error("TextIndex requires little-endian");
    const head = new Uint32Array(buf, 0, 6);
    if (head[0] !== MAGIC || head[1] !== VERSION) throw new Error(`not a BTIX v${VERSION} index`);
    this.books = head[2]!;
    this.chapters = head[3]!;
    this.slots = head[4]!;
    this.tokens = head[5]!;
    let o = 24;
    const take = <T>(ctor: { new (b: ArrayBuffer, o: number, n: number): T; BYTES_PER_ELEMENT: number }, n: number) => {
      const a = new ctor(buf, o, n);
      o += n * ctor.BYTES_PER_ELEMENT;
      return a;
    };
    this.verseStart = take(Uint32Array, this.slots + 1);
    this.bookEnd = take(Uint32Array, this.books);
    this.chapterFirstVerse = take(Uint32Array, this.chapters + 1);
    this.chapterOfVerse = take(Uint16Array, this.slots);
    this.bookFirstChapter = take(Uint16Array, this.books + 1);
    this.bookOfChapter = take(Uint8Array, this.chapters);
    this.bookId = take(Uint8Array, this.books);
    this.slotWords = take(Uint8Array, this.slots);
    if (o !== buf.byteLength) throw new Error(`index size mismatch: ${o} != ${buf.byteLength}`);
    this.bookById = new Int16Array(256).fill(-1);
    this.bookId.forEach((id, b) => (this.bookById[id] = b));
    this.slotPos = new Uint32Array(this.slots + 1);
    for (let s = 0; s < this.slots; s++) this.slotPos[s + 1] = this.slotPos[s]! + this.slotWords[s]!;
  }

  /** Book index of an ordinal in a book's range `[first word, end)` (a word or a gap a word owns), else -1. */
  bookOf(ord: number): number {
    if (!(ord >= 0 && ord < 2 ** 32)) return -1;
    const b = this.bookById[bookIdOf(ord)] ?? -1;
    return b >= 0 && ord >= this.verseStart[this.bookFirstSlot(b)]! && ord < this.bookEnd[b]! ? b : -1;
  }

  /** Whether an ordinal is a word. */
  isWord(ord: number): boolean {
    if (this.bookOf(ord) < 0) return false;
    const slot = this.slotOf(ord);
    return ord - this.verseStart[slot]! < this.slotWords[slot]!;
  }

  /**
   * Dense position of an ordinal: its word's index in reading order (0..tokens). A gap counts as the position
   * after the word owning it, and a book's end as the position after its last word, so `pos(end) - pos(start)` is
   * the number of words in an interval within a book.
   */
  pos(ord: number): number {
    const b = this.bookById[bookIdOf(ord)] ?? -1;
    if (b < 0) throw new RangeError(`ordinal ${ord} is not in a book`);
    if (ord >= this.bookEnd[b]!) return this.slotPos[this.bookFirstSlot(b + 1)]!;
    if (ord < this.verseStart[this.bookFirstSlot(b)]!) return this.slotPos[this.bookFirstSlot(b)]!;
    const slot = this.slotOf(ord);
    return this.slotPos[slot]! + Math.min(ord - this.verseStart[slot]!, this.slotWords[slot]!);
  }

  /** The ordinal of the word at dense position `p` (0 <= p < tokens). */
  ordAt(p: number): number {
    if (!(p >= 0 && p < this.tokens)) throw new RangeError(`no word at position ${p}`);
    let lo = 0;
    let hi = this.slots;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.slotPos[mid]! <= p) lo = mid + 1;
      else hi = mid;
    }
    const slot = lo - 1; // the last slot starting at or before p (empty slots share their start with the next)
    return this.verseStart[slot]! + p - this.slotPos[slot]!;
  }

  /** The end of an interval whose last word is `ord`: the next word of its book, or the book's end. */
  after(ord: number): number {
    const b = this.bookOf(ord);
    if (b < 0) throw new RangeError(`ordinal ${ord} is not a word`);
    const p = this.pos(ord) + 1;
    return p < this.slotPos[this.bookFirstSlot(b + 1)]! ? this.ordAt(p) : this.bookEnd[b]!;
  }

  /** The ordinals of a slot's words. */
  slotOrds(slot: number): number[] {
    const s = this.verseStart[slot]!;
    return Array.from({ length: this.slotWords[slot]! }, (_, i) => s + i);
  }

  bookFirstSlot(book: number): number {
    return this.chapterFirstVerse[this.bookFirstChapter[book]!]!;
  }

  /** Global slot containing `ord` (never an empty slot). */
  slotOf(ord: number): number {
    const book = this.bookOf(ord);
    if (book < 0) throw new RangeError(`ordinal ${ord} is not a word`);
    // upperBound(verseStart, ord) - 1, within the book's slots.
    let lo = this.bookFirstSlot(book);
    let hi = this.bookFirstSlot(book + 1);
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.verseStart[mid]! <= ord) lo = mid + 1;
      else hi = mid;
    }
    return lo - 1;
  }

  locate(ord: number): Location {
    return this.locateSlot(this.slotOf(ord));
  }

  locateSlot(slot: number): Location {
    const ch = this.chapterOfVerse[slot]!;
    const book = this.bookOfChapter[ch]!;
    return { book, chapter: ch - this.bookFirstChapter[book]! + 1, verse: slot - this.chapterFirstVerse[ch]! };
  }

  chapterCount(book: number): number {
    return this.bookFirstChapter[book + 1]! - this.bookFirstChapter[book]!;
  }

  /** Global chapter index, or -1 if out of range. */
  chapterIndex(book: number, chapter: number): number {
    if (!(book >= 0 && book < this.books) || !(chapter >= 1 && chapter <= this.chapterCount(book))) return -1;
    return this.bookFirstChapter[book]! + chapter - 1;
  }

  /** Last verse number of a chapter (slot 0 not counted). */
  verseCount(book: number, chapter: number): number {
    const ch = this.chapterIndex(book, chapter);
    if (ch < 0) throw new RangeError(`no chapter ${chapter} in book ${book}`);
    return this.chapterFirstVerse[ch + 1]! - this.chapterFirstVerse[ch]! - 1;
  }

  /** Global slot of a verse, or -1 if out of range (verse 0 = title slot is always in range). */
  slotIndex(book: number, chapter: number, verse: number): number {
    const ch = this.chapterIndex(book, chapter);
    if (ch < 0 || !(verse >= 0 && verse <= this.chapterFirstVerse[ch + 1]! - this.chapterFirstVerse[ch]! - 1)) {
      return -1;
    }
    return this.chapterFirstVerse[ch]! + verse;
  }

  /** End ordinal (exclusive) of a slot; a book's last slot ends at the book's end, not the next book's start. */
  slotEnd(slot: number): number {
    const book = this.bookOfChapter[this.chapterOfVerse[slot]!]!;
    return slot + 1 === this.bookFirstSlot(book + 1) ? this.bookEnd[book]! : this.verseStart[slot + 1]!;
  }

  slotInterval(slot: number): Interval {
    return [this.verseStart[slot]!, this.slotEnd(slot)];
  }

  verseInterval(book: number, chapter: number, verse: number): Interval {
    const s = this.slotIndex(book, chapter, verse);
    if (s < 0) throw new RangeError(`no verse ${book} ${chapter}:${verse}`);
    return this.slotInterval(s);
  }

  chapterInterval(book: number, chapter: number): Interval {
    const ch = this.chapterIndex(book, chapter);
    if (ch < 0) throw new RangeError(`no chapter ${chapter} in book ${book}`);
    return [this.verseStart[this.chapterFirstVerse[ch]!]!, this.slotEnd(this.chapterFirstVerse[ch + 1]! - 1)];
  }

  bookInterval(book: number): Interval {
    return [this.verseStart[this.bookFirstSlot(book)]!, this.bookEnd[book]!];
  }

  /** Words in an interval. */
  wordCount(iv: Interval): number {
    const [s, e] = iv;
    if (!(e > s)) return 0;
    let n = 0;
    for (let b = 0; b < this.books; b++) {
      const [bs, be] = this.bookInterval(b);
      if (s < be && bs < e) n += this.pos(Math.min(e, be)) - this.pos(Math.max(s, bs));
    }
    return n;
  }

  hasTitle(book: number, chapter: number): boolean {
    const [s, e] = this.verseInterval(book, chapter, 0);
    return e > s;
  }
}
