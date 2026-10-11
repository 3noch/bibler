import { test } from "node:test";
import assert from "node:assert/strict";
import { BOOK_GROUPS, groupInterval, inScope, matchGroups, mergeIntervals } from "../src/scopes.ts";
import { BOOKS } from "../src/books.ts";
import type { Interval } from "../src/textIndex.ts";
import { kjv } from "./kjv.ts";

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

test("inScope keeps exactly the instances overlapping some scope interval (brute force)", () => {
  for (let seed = 1; seed <= 400; seed++) {
    const r = rng(seed);
    const instances: Interval[] = [];
    for (let x = Math.floor(r() * 5); x < 200; x += 2 + Math.floor(r() * 12)) {
      const e = x + 1 + Math.floor(r() * 6);
      instances.push([x, e]);
      x = e;
    }
    const scope: Interval[] = Array.from({ length: Math.floor(r() * 4) }, () => {
      const s = Math.floor(r() * 200);
      return [s, s + 1 + Math.floor(r() * 30)] as Interval;
    });
    const want = scope.length === 0 ? instances : instances.filter(([s, e]) => scope.some(([a, b]) => a < e && s < b));
    assert.deepEqual(inScope(instances, scope), want, `seed ${seed}`);
  }
});

test("mergeIntervals sorts and merges overlapping and abutting ranges", () => {
  assert.deepEqual(mergeIntervals([[10, 12], [0, 3], [3, 5], [11, 20], [30, 31]]), [[0, 5], [10, 20], [30, 31]]);
});

test("book groups resolve to the right books and match by name or alias prefix", () => {
  const { index } = kjv;
  const name = (code: string) => BOOKS.find((b) => b.code === code)!.name;
  for (const g of BOOK_GROUPS) {
    const [s, e] = groupInterval(index, g);
    assert.equal(BOOKS[index.locate(s).book]!.code, g.from, g.name);
    assert.equal(BOOKS[index.locate(e - 1).book]!.code, g.to, g.name);
  }
  assert.deepEqual(matchGroups("gos").map((g) => g.name), ["Gospels"]);
  assert.deepEqual(matchGroups("paul").map((g) => g.name), ["Pauline epistles"]);
  assert.deepEqual(matchGroups("nt").map((g) => g.name), ["New Testament"]);
  assert.deepEqual(matchGroups("Minor").map((g) => g.name), ["Minor Prophets"]);
  assert.deepEqual(matchGroups("p").map((g) => g.name), [], "one letter is too short");
  assert.ok(name("JHN") === "John");
});
