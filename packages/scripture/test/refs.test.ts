import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { ALIASES, BOOKS, bookByCode } from "../src/books.ts";
import { completeBook, formatInterval, parseRef, resolveBook, type FormatStyle } from "../src/refs.ts";
import type { Interval } from "../src/textIndex.ts";
import { kjv } from "./kjv.ts";

const { index } = kjv;

interface Case {
  input: string;
  ctx?: { book?: string; chapter?: number };
  refs: string[];
  short?: string[];
  warnings?: (string | { code: string; candidates: string[] })[];
}

const path = new URL("../../../fixtures/refs/parse.json", import.meta.url);
const { cases } = JSON.parse(readFileSync(path, "utf8")) as { cases: Case[] };

/** parse(format(iv)) must give back exactly iv, with no warnings. */
function assertRoundTrip(iv: Interval, style: FormatStyle, label: string) {
  const text = formatInterval(index, iv, style);
  const back = parseRef(index, text);
  assert.deepEqual(back.warnings, [], `${label}: ${JSON.stringify(text)} warned`);
  assert.deepEqual(back.intervals, [iv], `${label}: ${JSON.stringify(text)} did not round-trip`);
}

for (const c of cases) {
  test(`parse ${JSON.stringify(c.input)}`, () => {
    const ctx = { book: c.ctx?.book ? bookByCode(c.ctx.book) : undefined, chapter: c.ctx?.chapter };
    const r = parseRef(index, c.input, ctx);
    assert.deepEqual(r.intervals.map((iv) => formatInterval(index, iv)), c.refs, "full");
    if (c.short) assert.deepEqual(r.intervals.map((iv) => formatInterval(index, iv, "short")), c.short, "short");
    const want = (c.warnings ?? []).map((w) => (typeof w === "string" ? { code: w } : w));
    const got = r.warnings.map((w, k) =>
      typeof c.warnings?.[k] === "object" ? { code: w.code, candidates: w.candidates?.map((b) => BOOKS[b]!.code) } : { code: w.code },
    );
    assert.deepEqual(got, want, `warnings: ${JSON.stringify(r.warnings)}`);
    for (const ref of r.refs) {
      assert.ok(ref.span[0] >= 0 && ref.span[1] <= c.input.length && ref.span[0] < ref.span[1], "span");
      assertRoundTrip(ref.interval, "full", c.input);
      assertRoundTrip(ref.interval, "short", c.input);
    }
  });
}

test("every alias, name and short form resolves exactly to its book", () => {
  let n = 0;
  BOOKS.forEach((b, i) => {
    for (const name of [b.name, b.refName ?? b.name, b.short, ...b.aliases]) {
      assert.deepEqual(resolveBook(name), { kind: "exact", book: i }, name);
      const one = index.chapterCount(i) === 1;
      const r = parseRef(index, `${name} 1${one ? "" : ":1"}`);
      assert.deepEqual(r.intervals, [index.verseInterval(i, 1, 1)], name);
      n++;
    }
  });
  assert.ok(ALIASES.size >= 250, `only ${ALIASES.size} distinct aliases`);
});

test("last verse of every chapter parses; one past it is out of range", () => {
  for (let b = 0; b < index.books; b++) {
    const name = BOOKS[b]!.short;
    const chapters = index.chapterCount(b);
    for (let ch = 1; ch <= chapters; ch++) {
      const last = index.verseCount(b, ch);
      const ok = parseRef(index, `${name} ${ch}:${last}`);
      assert.deepEqual(ok.intervals, [index.verseInterval(b, ch, last)], `${name} ${ch}:${last}`);
      const bad = parseRef(index, `${name} ${ch}:${last + 1}`);
      assert.deepEqual(bad.warnings.map((w) => w.code), ["out_of_range"], `${name} ${ch}:${last + 1}`);
      assert.deepEqual(parseRef(index, `${BOOKS[b]!.name} ${ch}:${last}`).intervals, ok.intervals);
    }
    const past = parseRef(index, chapters === 1 ? `${name} 1:1-2:1` : `${name} ${chapters + 1}`);
    assert.deepEqual(past.warnings.map((w) => w.code), ["out_of_range"], `${name} past last chapter`);
  }
});

test("ambiguous short prefixes never silently resolve", () => {
  for (const s of ["jo", "ju", "jud", "jon", "ph", "ez", "j", "ma", "1", "cor", "sam", "kings", "pet", "tim", "thess"]) {
    const m = resolveBook(s);
    assert.equal(m.kind === "ambiguous" || m.kind === "none", true, `${s} -> ${JSON.stringify(m)}`);
  }
  assert.deepEqual(completeBook("jon").map((b) => BOOKS[b]!.code), ["JON", "JHN"]);
  assert.deepEqual(completeBook("1 j").map((b) => BOOKS[b]!.code), ["1JN"]);
  assert.deepEqual(completeBook("phil").map((b) => BOOKS[b]!.code), ["PHP", "PHM"]);
  assert.deepEqual(completeBook(""), []);
});

test("randomized verse-aligned intervals round-trip through format and parse", () => {
  let seed = 12345;
  const rand = () => {
    seed = (seed + 0x6d2b79f5) >>> 0;
    let t = seed;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  // Random word = random dense position mapped to its (sparse) ordinal.
  const ordAt = (n: number) => {
    for (let b = 0; b < index.books; b++) {
      const [s, e] = index.bookInterval(b);
      if (n < e - s) return s + n;
      n -= e - s;
    }
    return index.bookEnd[index.books - 1]! - 1;
  };
  for (let k = 0; k < 3000; k++) {
    const n = Math.floor(rand() * index.tokens);
    // Mostly short spans, sometimes long ones (cross-chapter and cross-book).
    const len = rand() < 0.8 ? Math.floor(rand() * 200) : Math.floor(rand() * 100000);
    const a = ordAt(n);
    const b = ordAt(Math.min(index.tokens - 1, n + len));
    const iv: Interval = [index.slotInterval(index.slotOf(a))[0], index.slotInterval(index.slotOf(b))[1]];
    assertRoundTrip(iv, "full", `case ${k}`);
    assertRoundTrip(iv, "short", `case ${k}`);
  }
  // Whole chapters and books, including across book boundaries.
  for (let k = 0; k < 500; k++) {
    const c1 = Math.floor(rand() * index.chapters);
    const c2 = Math.min(index.chapters - 1, c1 + Math.floor(rand() * 3) * (rand() < 0.5 ? 1 : 40));
    const iv: Interval = [index.verseStart[index.chapterFirstVerse[c1]!]!, index.slotEnd(index.chapterFirstVerse[c2 + 1]! - 1)];
    assertRoundTrip(iv, "full", `chapters ${c1}-${c2}`);
    assertRoundTrip(iv, "short", `chapters ${c1}-${c2}`);
  }
});

test("formatting a word-level interval widens to whole verses", () => {
  const [s, e] = index.verseInterval(bookByCode("JHN"), 3, 16);
  assert.equal(formatInterval(index, [s + 2, e - 3]), "John 3:16");
  const [, e2] = index.verseInterval(bookByCode("JHN"), 3, 17);
  assert.equal(formatInterval(index, [s + 5, e2 - 1]), "John 3:16–17");
  assert.throws(() => formatInterval(index, [s, s]));
});
