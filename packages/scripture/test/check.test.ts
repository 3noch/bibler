import { test } from "node:test";
import assert from "node:assert/strict";
import { checkOp, wordRange } from "../src/check.ts";
import { makeOrd } from "../src/textIndex.ts";
import type { Op } from "../src/ops.ts";
import { kjv } from "./kjv.ts";

const { index } = kjv;
const GEN = 2;
const EXOD = 3;
const at = (book: number, ch: number, v: number, w: number) => makeOrd(book, ch, v, w);

test("check: tagging ends follow the server's rule (crates/api/src/check.rs)", () => {
  const ok = (s: number, e: number) => wordRange(index, s, e);
  assert.ok(ok(at(GEN, 1, 1, 0), at(GEN, 1, 1, 1)));
  assert.ok(ok(at(GEN, 1, 1, 9), at(GEN, 1, 2, 0)), "the end of Gen 1:1 is the first word of 1:2");
  assert.ok(ok(at(GEN, 50, 26, 0), index.bookEnd[0]!), "the book's end");
  assert.ok(!ok(at(GEN, 1, 1, 0), at(GEN, 1, 1, 10)), "a gap is never an end (Gen 1:1 has 10 words)");
  assert.ok(!ok(at(GEN, 1, 1, 10), at(GEN, 1, 2, 1)), "or a start");
  assert.ok(!ok(at(GEN, 1, 1, 1), at(GEN, 1, 1, 1)), "empty");
  assert.ok(!ok(at(GEN, 1, 1, 2), at(GEN, 1, 1, 1)), "reversed");
  assert.ok(!ok(at(GEN, 50, 26, 0), at(EXOD, 1, 1, 0)), "an end in the next book, even its first word");
  assert.ok(!ok(at(GEN, 50, 26, 0), index.bookEnd[1]!), "or that book's end");
  assert.ok(ok(at(EXOD, 1, 1, 0), at(EXOD, 1, 1, 2)), "the second book on its own");
  assert.ok(!ok(at(1, 1, 1, 0), at(1, 1, 1, 1)), "no such book");
});

test("check: ops get bad_id / bad_text like the server", () => {
  const T = "kjv-1769-v2";
  const paint = (start: number, end: number, extra: Partial<Op> = {}) =>
    ({ op_id: "o1", kind: "paint", tag: "T", text_id: T, start, end, new_id: "g1", now: 1, ...extra }) as Op;
  const s = at(GEN, 1, 1, 0);
  const e = at(GEN, 1, 1, 3);
  assert.equal(checkOp(index, T, paint(s, e)), null);
  assert.equal(checkOp(index, T, paint(s, e + 7))?.code, "bad_text");
  assert.equal(checkOp(index, T, paint(s, e, { text_id: "other" } as Partial<Op>))?.code, "bad_text");
  assert.equal(checkOp(index, T, paint(s, e, { op_id: "" }))?.code, "bad_id");
  assert.equal(checkOp(index, T, paint(s, e, { new_id: "x".repeat(65) } as Partial<Op>))?.code, "bad_id");
  assert.equal(checkOp(index, T, { op_id: "o2", kind: "erase", tag: "T", text_id: T, start: s, end: e, split_id: "", now: 1 })?.code, "bad_id");
  assert.equal(checkOp(index, T, { op_id: "o2", kind: "erase", tag: "T", text_id: T, start: s, end: e, split_id: null, now: 1 }), null);
  assert.equal(checkOp(index, T, { op_id: "o3", kind: "palette_set", tags: ["a", "b c"], now: 1 })?.code, "bad_id");
  assert.equal(checkOp(index, T, { op_id: "o4", kind: "comment_create", id: "c", tagging: "g/1", body: "x", now: 1 })?.code, "bad_id");
  assert.equal(checkOp(index, T, { op_id: "o5", kind: "tag_create", id: "T", name: "Any name", color: "#123456", now: 1 }), null);
});
