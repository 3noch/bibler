/**
 * Reference parsing and formatting. Cases: fixtures/refs/parse.json (behavioral spec).
 *
 * Book names: normalize (books.ts) -> exact alias -> unique prefix (FORCED_AMBIGUOUS entries count as prefixes of
 * all their books, so `jon` is ambiguous) -> ambiguous prefix
 * (ranked candidates, canonical order) -> fuzzy (optimal-string-alignment Damerau-Levenshtein; inputs of >= 3
 * chars; 1 edit up to 4 chars, 2 edits beyond; a tie between books is ambiguous).
 *
 * Grammar (whitespace-insensitive except `3 16`):
 *   input    := group (';' group)*
 *   group    := book | [book] item (',' item)*        no book = previous group's book, else context
 *   item     := endpoint [dash endpoint]              dash = - – —
 *   endpoint := [book] (C | C sep V | C sep 'title' | 'v' V)      sep = ':' '.' 'v' or a space between numbers
 * Conventions: after an item that ended in a verse, a bare number is a verse of that chapter (`Jn 3:16,18`);
 * otherwise a chapter (`Gen 1,3`). In single-chapter books (Obadiah, Philemon, 2–3 John, Jude) a bare number is a
 * verse (`Jude 5`). Verse 0 / `title` is the chapter's title slot. A book alone as a range endpoint means its start
 * (or end). Out-of-range references are reported and dropped, never clamped.
 */
import { ALIASES, BOOKS, FORCED_AMBIGUOUS, bookByCode, normalizeBookName } from "./books.ts";
import type { Interval, TextIndex } from "./textIndex.ts";

// ---------------------------------------------------------------- book names

export type BookMatch =
  | { kind: "exact" | "prefix" | "fuzzy"; book: number }
  | { kind: "ambiguous"; candidates: number[] }
  | { kind: "none" };

/**
 * Books having an alias that starts with `prefix` (canonical order). Drives autocomplete. If nothing matches, a
 * numbered book's alias also matches without its number (`cor` -> 1 & 2 Corinthians; never Colossians by fuzz).
 */
export function completeBook(prefix: string): number[] {
  const s = normalizeBookName(prefix);
  const set = new Set<number>();
  if (!s) return [];
  for (const [alias, book] of ALIASES) if (alias.startsWith(s)) set.add(book);
  for (const [key, codes] of Object.entries(FORCED_AMBIGUOUS)) {
    if (key.startsWith(s)) for (const c of codes) set.add(bookByCode(c));
  }
  if (set.size === 0 && !/^\d/.test(s)) {
    for (const [alias, book] of ALIASES) if (/^\d/.test(alias) && alias.slice(1).startsWith(s)) set.add(book);
  }
  return [...set].sort((a, b) => a - b);
}

export function resolveBook(raw: string): BookMatch {
  const s = normalizeBookName(raw);
  if (!s) return { kind: "none" };
  const exact = ALIASES.get(s);
  if (exact !== undefined) return { kind: "exact", book: exact };
  const prefixed = completeBook(s);
  if (prefixed.length === 1) return { kind: "prefix", book: prefixed[0]! };
  if (prefixed.length > 1) return { kind: "ambiguous", candidates: prefixed };
  if (s.length < 3) return { kind: "none" };

  const max = s.length <= 4 ? 1 : 2;
  const best = new Map<number, number>();
  for (const [alias, book] of ALIASES) {
    if (Math.abs(alias.length - s.length) > max) continue;
    const d = osaDistance(s, alias);
    if (d <= max && d < (best.get(book) ?? Infinity)) best.set(book, d);
  }
  if (best.size === 0) return { kind: "none" };
  const min = Math.min(...best.values());
  const books = [...best].filter(([, d]) => d === min).map(([b]) => b).sort((a, b) => a - b);
  return books.length === 1 ? { kind: "fuzzy", book: books[0]! } : { kind: "ambiguous", candidates: books };
}

/** Optimal string alignment distance (Damerau-Levenshtein with adjacent transpositions, no substring re-edits). */
export function osaDistance(a: string, b: string): number {
  const d: number[][] = Array.from({ length: a.length + 1 }, (_, i) =>
    Array.from({ length: b.length + 1 }, (_, j) => (i === 0 ? j : j === 0 ? i : 0)),
  );
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let v = Math.min(d[i - 1]![j]! + 1, d[i]![j - 1]! + 1, d[i - 1]![j - 1]! + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) v = Math.min(v, d[i - 2]![j - 2]! + 1);
      d[i]![j] = v;
    }
  }
  return d[a.length]![b.length]!;
}

// ---------------------------------------------------------------- parsing

export interface Point {
  book: number;
  chapter: number;
  /** null = the whole chapter. */
  verse: number | null;
}

export interface ParsedRef {
  start: Point;
  end: Point;
  interval: Interval;
  /** [from, to) character offsets of the item in the input. */
  span: [number, number];
}

export type WarningCode =
  | "unknown_book"
  | "ambiguous_book"
  | "fuzzy_book"
  | "out_of_range"
  | "empty_title"
  | "reversed_range"
  | "no_context"
  | "syntax";

export interface ParseWarning {
  code: WarningCode;
  message: string;
  span: [number, number];
  /** ambiguous_book: candidate books. fuzzy_book: the book chosen. */
  candidates?: number[];
}

export interface ParseContext {
  book?: number;
  chapter?: number;
}

export interface ParseResult {
  refs: ParsedRef[];
  intervals: Interval[];
  warnings: ParseWarning[];
}

type Tok =
  | { t: "num"; v: number; s: number; e: number; sp: boolean }
  | { t: "word"; v: string; s: number; e: number; sp: boolean }
  | { t: "colon" | "dot" | "comma" | "semi" | "dash"; s: number; e: number; sp: boolean };

const VERSE_WORDS = new Set(["v", "vv", "vs", "ver", "verse", "verses"]);
const TITLE_WORDS = new Set(["title", "superscription"]);

function lex(input: string, warn: (w: ParseWarning) => void): Tok[] {
  const out: Tok[] = [];
  const re = /(\s+)|(\d+)|([A-Za-z][A-Za-z.]*)|(:)|(\.)|(,)|(;)|([-–—])|(.)/gsy;
  let sp = false;
  for (let m: RegExpExecArray | null; (m = re.exec(input)); ) {
    const s = m.index;
    const e = s + m[0].length;
    if (m[1]) {
      sp = true;
      continue;
    }
    if (m[2]) out.push({ t: "num", v: Number(m[2]), s, e, sp });
    else if (m[3]) {
      // A word's trailing dot is part of an abbreviation ("Gen."), but "v." before a number is a verse marker.
      out.push({ t: "word", v: m[3], s, e, sp });
    } else if (m[4]) out.push({ t: "colon", s, e, sp });
    else if (m[5]) out.push({ t: "dot", s, e, sp });
    else if (m[6]) out.push({ t: "comma", s, e, sp });
    else if (m[7]) out.push({ t: "semi", s, e, sp });
    else if (m[8]) out.push({ t: "dash", s, e, sp });
    else warn({ code: "syntax", message: `unexpected character ${JSON.stringify(m[0])}`, span: [s, e] });
    sp = false;
  }
  return out;
}

const wordIs = (tok: Tok | undefined, set: Set<string>) =>
  tok?.t === "word" && set.has(tok.v.toLowerCase().replace(/\.$/, ""));

interface Endpoint {
  book?: number;
  /** A book with no numbers after it. */
  bookOnly?: boolean;
  a?: number;
  b?: number;
  verseOnly?: boolean;
  s: number;
  e: number;
}

class Bail extends Error {}

export function parseRef(index: TextIndex, input: string, ctx: ParseContext = {}): ParseResult {
  const warnings: ParseWarning[] = [];
  const refs: ParsedRef[] = [];
  const warn = (w: ParseWarning) => warnings.push(w);
  const toks = lex(input, warn);
  let i = 0;
  const peek = (k = 0) => toks[i + k];
  const at = (t: Tok["t"]) => peek()?.t === t;

  let book: number | undefined = ctx.book;
  let chapter: number | undefined = ctx.chapter;
  let verseMode = false;

  /** Try to read a book name at the cursor. Returns undefined if none starts here; throws Bail if unresolvable. */
  const bookName = (): number | undefined => {
    const t0 = peek();
    const t1 = peek(1);
    let j = i;
    if (t0?.t === "num" && t0.v >= 1 && t0.v <= 3 && t1?.t === "word" && !wordIs(t1, VERSE_WORDS)) j = i + 1;
    else if (t0?.t !== "word" || wordIs(t0, VERSE_WORDS)) return undefined;
    while (toks[j + 1]?.t === "word" && !wordIs(toks[j + 1], VERSE_WORDS)) j++;
    const s = toks[i]!.s;
    const e = toks[j]!.e;
    i = j + 1;
    const raw = input.slice(s, e);
    const m = resolveBook(raw);
    if (m.kind === "none") {
      warn({ code: "unknown_book", message: `unknown book ${JSON.stringify(raw)}`, span: [s, e] });
      throw new Bail();
    }
    if (m.kind === "ambiguous") {
      const names = m.candidates.map((c) => BOOKS[c]!.name).join(", ");
      warn({ code: "ambiguous_book", message: `${JSON.stringify(raw)} could be ${names}`, span: [s, e], candidates: m.candidates });
      throw new Bail();
    }
    if (m.kind === "fuzzy") {
      warn({ code: "fuzzy_book", message: `read ${JSON.stringify(raw)} as ${BOOKS[m.book]!.name}`, span: [s, e], candidates: [m.book] });
    }
    return m.book;
  };

  const syntax = (msg: string): never => {
    const t = peek() ?? toks[toks.length - 1];
    warn({ code: "syntax", message: msg, span: t ? [t.s, t.e] : [0, input.length] });
    throw new Bail();
  };

  const num = (): number => {
    const t = peek();
    if (t?.t !== "num") return syntax("expected a number");
    i++;
    return t.v;
  };

  const endpoint = (allowBook: boolean): Endpoint => {
    const s = peek()?.s ?? input.length;
    const bk = allowBook ? bookName() : undefined;
    const done = (ep: Omit<Endpoint, "s" | "e">): Endpoint => ({ ...ep, book: bk, s, e: toks[i - 1]?.e ?? s });
    const t = peek();
    if (bk !== undefined && (!t || t.t === "comma" || t.t === "semi" || t.t === "dash")) return done({ bookOnly: true });
    if (wordIs(t, VERSE_WORDS)) {
      i++;
      if (at("dot")) i++;
      return done({ verseOnly: true, b: num() });
    }
    const a = num();
    const n = peek();
    if (n?.t === "colon" || n?.t === "dot" || wordIs(n, VERSE_WORDS)) {
      i++;
      if (wordIs(peek(), TITLE_WORDS)) {
        i++;
        return done({ a, b: 0 });
      }
      return done({ a, b: num() });
    }
    if (n?.t === "num" && n.sp) {
      i++;
      return done({ a, b: n.v });
    }
    return done({ a });
  };

  const singleChapter = (b: number) => index.chapterCount(b) === 1;

  /** Interpret an endpoint as a Point. `prev` = the range's start point when this is the end endpoint. */
  const toPoint = (ep: Endpoint, bk: number, prev: Point | null, isEnd: boolean): Point => {
    if (ep.bookOnly) {
      return isEnd ? { book: bk, chapter: index.chapterCount(bk), verse: null } : { book: bk, chapter: 1, verse: null };
    }
    if (ep.verseOnly) {
      const ch = prev?.chapter ?? chapter;
      if (ch === undefined) {
        warn({ code: "no_context", message: "verse without a chapter", span: [ep.s, ep.e] });
        throw new Bail();
      }
      return { book: bk, chapter: ch, verse: ep.b! };
    }
    if (ep.b !== undefined) return { book: bk, chapter: ep.a!, verse: ep.b };
    if (singleChapter(bk)) return { book: bk, chapter: 1, verse: ep.a! };
    if (ep.book === undefined && (prev ? prev.verse !== null : verseMode)) {
      const ch = prev?.chapter ?? chapter!;
      return { book: bk, chapter: ch, verse: ep.a! };
    }
    return { book: bk, chapter: ep.a!, verse: null };
  };

  const range = (p: Point, ep: Endpoint): boolean => {
    const ch = index.chapterIndex(p.book, p.chapter);
    const name = BOOKS[p.book]!.name;
    if (ch < 0) {
      const n = index.chapterCount(p.book);
      warn({ code: "out_of_range", message: `${name} has ${n} chapter${n === 1 ? "" : "s"}`, span: [ep.s, ep.e] });
      return false;
    }
    if (p.verse !== null && p.verse > index.verseCount(p.book, p.chapter)) {
      const n = index.verseCount(p.book, p.chapter);
      warn({ code: "out_of_range", message: `${name} ${p.chapter} has ${n} verses`, span: [ep.s, ep.e] });
      return false;
    }
    return true;
  };

  const startOrd = (p: Point) =>
    p.verse === null ? index.chapterInterval(p.book, p.chapter)[0] : index.verseInterval(p.book, p.chapter, p.verse)[0];
  const endOrd = (p: Point) =>
    p.verse === null ? index.chapterInterval(p.book, p.chapter)[1] : index.verseInterval(p.book, p.chapter, p.verse)[1];

  const item = () => {
    const e1 = endpoint(true);
    if (e1.book !== undefined) {
      book = e1.book;
      chapter = undefined;
      verseMode = false;
    }
    if (book === undefined) {
      warn({ code: "no_context", message: "no book given", span: [e1.s, e1.e] });
      throw new Bail();
    }
    const p1 = toPoint(e1, book, null, false);
    let p2 = e1.bookOnly ? toPoint(e1, book, p1, true) : p1;
    let e2 = e1;
    if (at("dash")) {
      i++;
      e2 = endpoint(true);
      p2 = toPoint(e2, e2.book ?? p1.book, p1, true);
    }
    if (!range(p1, e1) || !range(p2, e2)) return;
    const span: [number, number] = [e1.s, e2.e];
    const iv: Interval = [startOrd(p1), endOrd(p2)];
    if (iv[1] <= iv[0]) {
      if (p1 === p2 && p1.verse === 0) {
        warn({ code: "empty_title", message: `${BOOKS[p1.book]!.name} ${p1.chapter} has no title`, span });
      } else {
        warn({ code: "reversed_range", message: "range ends before it starts", span });
      }
      return;
    }
    refs.push({ start: p1, end: p2, interval: iv, span });
    book = p2.book;
    chapter = p2.chapter;
    verseMode = p2.verse !== null;
  };

  const skipTo = (stop: ReadonlySet<Tok["t"]>) => {
    while (i < toks.length && !stop.has(peek()!.t)) i++;
  };
  const SEPARATORS: ReadonlySet<Tok["t"]> = new Set(["comma", "semi"]);

  while (i < toks.length) {
    try {
      item();
    } catch (e) {
      if (!(e instanceof Bail)) throw e;
      skipTo(SEPARATORS);
    }
    if (i < toks.length && !SEPARATORS.has(peek()!.t)) {
      const t = peek()!;
      warn({ code: "syntax", message: "expected ',' or ';'", span: [t.s, t.e] });
      skipTo(SEPARATORS);
    }
    if (at("comma")) i++;
    else if (at("semi")) {
      // A new group: a bare number is a chapter again (`Jn 3:16; 4` = John 4).
      i++;
      verseMode = false;
    }
  }
  return { refs, intervals: refs.map((r) => r.interval), warnings };
}

// ---------------------------------------------------------------- formatting

export type FormatStyle = "full" | "short";

const bookName = (b: number, style: FormatStyle) => (style === "short" ? BOOKS[b]!.short : BOOKS[b]!.name);
const refName = (b: number, style: FormatStyle) =>
  style === "short" ? BOOKS[b]!.short : (BOOKS[b]!.refName ?? BOOKS[b]!.name);
const verseLabel = (v: number) => (v === 0 ? "title" : String(v));

/**
 * Format an interval as a reference. Word-level intervals are widened to whole verses (the result names the verses
 * containing the first and last word). Whole chapters/books are recognized: `John 3`, `John 3–4`, `John`.
 */
export function formatInterval(index: TextIndex, iv: Interval, style: FormatStyle = "full"): string {
  const [s, e] = iv;
  if (!(e > s)) throw new RangeError("empty interval");
  const a = index.locate(s);
  const b = index.locate(e - 1);
  const single = (bk: number) => index.chapterCount(bk) === 1;

  if (s === index.bookInterval(a.book)[0] && e === index.bookInterval(b.book)[1]) {
    return a.book === b.book ? bookName(a.book, style) : `${bookName(a.book, style)}–${bookName(b.book, style)}`;
  }

  const wholeChapters = s === index.chapterInterval(a.book, a.chapter)[0] && e === index.chapterInterval(b.book, b.chapter)[1];
  if (wholeChapters) {
    if (a.book === b.book) {
      const r = refName(a.book, style);
      return a.chapter === b.chapter ? `${r} ${a.chapter}` : `${r} ${a.chapter}–${b.chapter}`;
    }
    const left = single(a.book) ? bookName(a.book, style) : `${refName(a.book, style)} ${a.chapter}`;
    const right = single(b.book) ? bookName(b.book, style) : `${refName(b.book, style)} ${b.chapter}`;
    return `${left}–${right}`;
  }

  // Verse form. A chapter-start ordinal of a titled chapter located at slot 0 renders as "title".
  const pt = (l: typeof a) => (single(l.book) ? verseLabel(l.verse) : `${l.chapter}:${verseLabel(l.verse)}`);
  const r = refName(a.book, style);
  if (a.book !== b.book) return `${r} ${pt(a)}–${refName(b.book, style)} ${pt(b)}`;
  if (a.chapter !== b.chapter) return `${r} ${a.chapter}:${verseLabel(a.verse)}–${b.chapter}:${verseLabel(b.verse)}`;
  if (a.verse === b.verse) return `${r} ${pt(a)}`;
  return `${r} ${pt(a)}–${verseLabel(b.verse)}`;
}
