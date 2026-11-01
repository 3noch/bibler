import { test } from "node:test";
import assert from "node:assert/strict";
import { findRefs } from "../src/findRefs.ts";
import { formatInterval } from "../src/refs.ts";
import { kjv } from "./kjv.ts";

const { index } = kjv;

/** Each link as "its text -> where it goes". */
const links = (text: string) => findRefs(index, text).map((r) => `${text.slice(r.start, r.end)} -> ${formatInterval(index, r.interval)}`);

test("findRefs: references in prose become links", () => {
  assert.deepEqual(links("See John 3:16 and Rom 8:28."), ["John 3:16 -> John 3:16", "Rom 8:28 -> Romans 8:28"]);
  assert.deepEqual(links("Read Psalm 23 tonight"), ["Psalm 23 -> Psalm 23"]);
  assert.deepEqual(links("cf. 1 Cor 13:4–7; 1 John 4:8"), ["1 Cor 13:4–7 -> 1 Corinthians 13:4–7", "1 John 4:8 -> 1 John 4:8"]);
  assert.deepEqual(links("(Gen. 1:1)"), ["Gen. 1:1 -> Genesis 1:1"]);
  assert.deepEqual(links("Song of Solomon 2:4"), ["Song of Solomon 2:4 -> Song of Solomon 2:4"]);
  assert.deepEqual(links("the Gospel of John 3:16"), ["John 3:16 -> John 3:16"]);
  assert.deepEqual(links("John 3:16–4:2"), ["John 3:16–4:2 -> John 3:16–4:2"]);
  assert.deepEqual(links("Jude 5"), ["Jude 5 -> Jude 5"]);
  assert.deepEqual(links("II Kings 2:11"), ["II Kings 2:11 -> 2 Kings 2:11"]);
});

test("findRefs: each reference in a list is its own link", () => {
  assert.deepEqual(links("Rom 8:28, 31"), ["Rom 8:28 -> Romans 8:28", "31 -> Romans 8:31"]);
});

test("findRefs: short abbreviations and lowercase need a verse; words aren't books", () => {
  assert.deepEqual(links("Is 53:5"), ["Is 53:5 -> Isaiah 53:5"]);
  assert.deepEqual(links("So 2 of them came; Is 3 enough? Ex 4 times"), []);
  assert.deepEqual(links("I am 3 years in"), []);
  assert.deepEqual(links("as mark 5 says"), [], "lowercase without a verse");
  assert.deepEqual(links("as john 3:16 says"), ["john 3:16 -> John 3:16"], "lowercase with a verse");
  assert.deepEqual(links("as Mark 5 says"), ["Mark 5 -> Mark 5"]);
  assert.deepEqual(links("Job 3 is bleak"), ["Job 3 -> Job 3"]);
});

test("findRefs: no typos, no ambiguity, nothing out of range, nothing bare", () => {
  assert.deepEqual(links("Jhon 3:16"), [], "a typo the location field would forgive");
  assert.deepEqual(links("Jon 1:1"), [], "Jonah or John");
  assert.deepEqual(links("John 30:1"), [], "no such chapter");
  assert.deepEqual(links("John 3:16, 99"), [], "a list with a verse that doesn't exist: none of it, rather than a guess");
  assert.deepEqual(links("verse 3:16 here"), [], "no book");
  assert.deepEqual(links("Romans"), [], "a book alone");
  assert.deepEqual(links("abcJohn 3:16"), [], "inside a word");
  assert.deepEqual(links("John 3:16a"), [], "glued to a word");
});
