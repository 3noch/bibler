import { test } from "node:test";
import assert from "node:assert/strict";
import { intersectRuns, nearby } from "../src/queries.ts";
import type { Interval } from "../src/textIndex.ts";

function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A random tag's taggings: sorted, disjoint, non-adjacent (the core invariant). */
function taggings(r: () => number, n: number): Interval[] {
  const out: Interval[] = [];
  let x = Math.floor(r() * 4);
  while (x < n) {
    const len = 1 + Math.floor(r() * 8);
    const end = Math.min(n, x + len);
    out.push([x, end]);
    x = end + 1 + Math.floor(r() * 10); // gap >= 1: non-adjacent
  }
  return out;
}

/** Brute force: maximal runs of positions covered by every list. */
function bruteRuns(lists: Interval[][], n: number): Interval[] {
  const covered = (p: number) => lists.every((l) => l.some(([s, e]) => s <= p && p < e));
  const out: Interval[] = [];
  for (let p = 0; p < n; p++) {
    if (!covered(p)) continue;
    if (out.length && out[out.length - 1]![1] === p) out[out.length - 1]![1] = p + 1;
    else out.push([p, p + 1]);
  }
  return out;
}

test("intersectRuns matches brute force on random tag sets", () => {
  for (let seed = 1; seed <= 500; seed++) {
    const r = rng(seed);
    const n = 120;
    const k = 1 + Math.floor(r() * 4);
    const lists = Array.from({ length: k }, () => taggings(r, n));
    assert.deepEqual(intersectRuns(lists), bruteRuns(lists, n), `seed ${seed}`);
  }
  assert.deepEqual(intersectRuns([]), []);
  assert.deepEqual(intersectRuns([[[0, 5]], []]), []);
});

test("nearby: window around a position, marking the containing instance", () => {
  const ivs: Interval[] = [[0, 2], [5, 9], [12, 13], [20, 30], [40, 41]];
  assert.deepEqual(nearby(ivs, 6, 1, 1), { items: [[0, 2], [5, 9], [12, 13]], current: 1, offset: 0, total: 5 });
  assert.deepEqual(nearby(ivs, 10, 1, 1), { items: [[5, 9], [12, 13]], current: -1, offset: 1, total: 5 }, "between instances");
  assert.deepEqual(nearby(ivs, 0, 3, 0), { items: [[0, 2]], current: 0, offset: 0, total: 5 });
  assert.deepEqual(nearby(ivs, 41, 1, 3), { items: [[40, 41]], current: -1, offset: 4, total: 5 }, "past the end");
  assert.deepEqual(nearby([], 3, 2, 2), { items: [], current: -1, offset: 0, total: 0 });
  // Brute-force check of the window bounds on random lists.
  for (let seed = 1; seed <= 300; seed++) {
    const r = rng(seed);
    const list = taggings(r, 100);
    const pos = Math.floor(r() * 110);
    const [b, a] = [Math.floor(r() * 4), Math.floor(r() * 4)];
    const got = nearby(list, pos, b, a);
    const ci = list.findIndex(([s, e]) => s <= pos && pos < e);
    const firstAfter = list.findIndex(([, e]) => e > pos);
    const anchor = firstAfter < 0 ? list.length : firstAfter;
    const want = list.slice(Math.max(0, anchor - b), Math.min(list.length, anchor + (ci >= 0 ? 1 : 0) + a));
    assert.deepEqual(got.items, want, `seed ${seed}`);
    assert.equal(got.current >= 0 ? got.items[got.current] : null, ci >= 0 ? list[ci] : null, `seed ${seed}`);
  }
});
