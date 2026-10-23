import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { HISTORY_SIZE, liveHistory, mergeEntry, mergeHistory, prune, resumeEntry, scrolled, visit, type HistoryEntry } from "../src/history.ts";
import { kjv } from "./kjv.ts";

const { index } = kjv;
const path = new URL("../../../fixtures/history.json", import.meta.url);
const { cases } = JSON.parse(readFileSync(path, "utf8")) as { cases: { name: string; a: HistoryEntry; b: HistoryEntry; out: HistoryEntry }[] };

for (const c of cases) {
  test(`merge: ${c.name}`, () => {
    assert.deepEqual(mergeEntry(c.a, c.b), c.out);
    assert.deepEqual(mergeEntry(c.b, c.a), c.out);
  });
}

const GEN = 0;
const JOHN = 42;
/** Ordinal of a verse's first word (book index, chapter, verse). */
const v = (book: number, ch: number, verse: number) => index.verseInterval(book, ch, verse)[0];

test("a jump starts an entry; a jump near where an entry left off resumes it, re-anchored there", () => {
  let { entries, id } = visit(index, [], v(GEN, 1, 1), 1, "a");
  assert.equal(id, "a");
  entries = scrolled(index, entries, "a", v(GEN, 3, 1), 2);
  const other = visit(index, entries, v(JOHN, 3, 16), 3, "b");
  assert.equal(other.id, "b");
  const back = visit(index, other.entries, v(GEN, 3, 4), 4, "c");
  assert.equal(back.id, "a", "within a few verses of where it left off");
  const a = back.entries.find((e) => e.id === "a")!;
  assert.deepEqual([a.anchor, a.last], [{ ord: v(GEN, 3, 4), at: 4 }, { ord: v(GEN, 3, 4), at: 4 }]);
  assert.deepEqual(liveHistory(back.entries).map((e) => e.id), ["a", "b"], "newest first");
  const far = visit(index, back.entries, v(GEN, 1, 1), 5, "d");
  assert.equal(far.id, "d", "near its old anchor isn't near where it is now");
});

test("near-duplicates: the older goes when both its anchor and its last are near the newer's", () => {
  const e = (id: string, a: number, l: number, at: number): HistoryEntry => ({ id, anchor: { ord: a, at }, last: { ord: l, at }, gone: false });
  const out = prune(index, [
    e("old", v(GEN, 5, 1), v(GEN, 9, 1), 1),
    e("new", v(GEN, 5, 3), v(GEN, 9, 2), 2),
    e("passing", v(GEN, 1, 1), v(GEN, 9, 1), 3), // passed over the same place from elsewhere: kept
  ]);
  assert.deepEqual(liveHistory(out).map((x) => x.id), ["passing", "new"]);
  assert.equal(out.find((x) => x.id === "old")?.gone, true, "kept as gone, so other devices drop it too");
});

test("beyond HISTORY_SIZE the oldest never-read entries go first", () => {
  const entries: HistoryEntry[] = [];
  // Tracks read some way (anchor and last far apart), oldest first, in distinct books' chapters.
  entries.push({ id: "read-old", anchor: { ord: v(GEN, 1, 1), at: 0 }, last: { ord: v(GEN, 30, 1), at: 0 }, gone: false });
  for (let i = 0; i < HISTORY_SIZE; i++) {
    const book = 1 + i; // Exodus.. (40 books): a glance at each one's first chapter
    const at = v(book, 1, 1);
    entries.push({ id: `glance${String(i).padStart(2, "0")}`, anchor: { ord: at, at: 10 + i }, last: { ord: at, at: 10 + i }, gone: false });
  }
  const out = liveHistory(prune(index, entries));
  assert.equal(out.length, HISTORY_SIZE);
  assert.ok(out.some((e) => e.id === "read-old"), "a read track outlives newer glances");
  assert.ok(!out.some((e) => e.id === "glance00"), "the oldest glance went");
});

test("merging lists: gone reaches the other device, positions keep the later", () => {
  const mine: HistoryEntry[] = [{ id: "a", anchor: { ord: 1, at: 1 }, last: { ord: 5, at: 5 }, gone: false }];
  const theirs: HistoryEntry[] = [
    { id: "a", anchor: { ord: 1, at: 1 }, last: { ord: 3, at: 3 }, gone: true },
    { id: "b", anchor: { ord: 9, at: 9 }, last: { ord: 9, at: 9 }, gone: false },
  ];
  const out = mergeHistory(mine, theirs);
  assert.deepEqual(out.find((e) => e.id === "a"), { id: "a", anchor: { ord: 1, at: 1 }, last: { ord: 5, at: 5 }, gone: true });
  assert.deepEqual(liveHistory(out).map((e) => e.id), ["b"]);
});

test("scrolling a gone or unknown entry changes nothing", () => {
  const entries: HistoryEntry[] = [{ id: "a", anchor: { ord: v(GEN, 1, 1), at: 1 }, last: { ord: v(GEN, 1, 1), at: 1 }, gone: true }];
  assert.deepEqual(scrolled(index, entries, "a", v(GEN, 2, 1), 2), entries);
  assert.deepEqual(scrolled(index, entries, "x", v(GEN, 2, 1), 2), entries);
});

test("resuming a chosen entry makes it the newest, anchored where it left off", () => {
  const entries: HistoryEntry[] = [
    { id: "a", anchor: { ord: v(GEN, 1, 1), at: 1 }, last: { ord: v(GEN, 4, 1), at: 2 }, gone: false },
    { id: "b", anchor: { ord: v(JOHN, 1, 1), at: 3 }, last: { ord: v(JOHN, 1, 1), at: 3 }, gone: false },
  ];
  const out = resumeEntry(index, entries, "a", 9);
  assert.deepEqual(liveHistory(out).map((e) => e.id), ["a", "b"]);
  assert.deepEqual(out.find((e) => e.id === "a")!.anchor, { ord: v(GEN, 4, 1), at: 9 });
});
