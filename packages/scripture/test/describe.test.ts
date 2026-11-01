import { test } from "node:test";
import assert from "node:assert/strict";
import { describeChanges, type DescribeContext } from "../src/describe.ts";
import type { OpIntent } from "../src/ops.ts";
import { SyncState } from "../src/syncState.ts";

let n = 0;
const newId = () => `id${String(++n).padStart(5, "0")}`;
const K = "kjv";

/** A client and a way to say what its next undo or redo does. Places are "<text> <start>-<end>". */
function setup() {
  const s = new SyncState("owner", newId, () => 1);
  const ctx: DescribeContext = {
    tagName: (id) => s.view.tag(id)?.name,
    tagging: (id) => s.view.tagging(id),
    place: (textId, [a, b]) => (textId === K ? `${a}-${b}` : undefined),
  };
  const did = (...intents: OpIntent[]) => assert.equal(s.submit(intents), null);
  const next = (which: "undo" | "redo") => describeChanges(s.stepChanges(which), ctx, which === "redo");
  return { s, did, next };
}

const paint = (tag: string, start: number, end: number): OpIntent => ({ kind: "paint", tag, text_id: K, start, end, new_id: newId(), now: 2 });
const erase = (tag: string, start: number, end: number): OpIntent => ({ kind: "erase", tag, text_id: K, start, end, split_id: newId(), now: 3 });

test("describe: each kind of step, as undo and then as redo", () => {
  const { s, did, next } = setup();
  const both = (want: string) => {
    assert.equal(next("undo"), want, "undo");
    assert.equal(s.undoStep(), null);
    assert.equal(next("redo"), want, "redo says the same action");
    assert.equal(s.redoStep(), null);
    assert.equal(next("undo"), want, "and undo again after the redo");
  };

  did({ kind: "tag_create", id: "L", name: "Love", color: "#ff0000", now: 1 }, { kind: "palette_set", tags: ["L"], now: 1 });
  both("creating the tag “Love”");

  did(paint("L", 10, 20));
  both("tagging 10-20 with “Love”");

  // Extending a tagging (a merge) names only the new words.
  did(paint("L", 15, 30));
  both("tagging 20-30 with “Love”");

  did(erase("L", 12, 14));
  both("erasing “Love” from 12-14");

  did({ kind: "tag_update", id: "L", name: "Charity", now: 4 });
  both("renaming “Love” to “Charity”");

  did({ kind: "tag_update", id: "L", color: "#00ff00", now: 5 });
  both("changing the color of “Charity”");

  const tagging = [...s.view.taggings.values()].find((t) => t.deleted_at === null)!;
  did({ kind: "comment_create", id: "C", tagging: tagging.id, body: "hi", now: 6 });
  both(`adding a comment on ${tagging.start}-${tagging.end}`);
  did({ kind: "comment_update", id: "C", body: "hello", now: 7 });
  both(`editing a comment on ${tagging.start}-${tagging.end}`);
  did({ kind: "comment_delete", id: "C", now: 8 });
  both(`deleting a comment on ${tagging.start}-${tagging.end}`);

  did({ kind: "tag_create", id: "H", name: "Hope", color: "#0000ff", now: 9 });
  did({ kind: "palette_set", tags: ["H", "L"], now: 9 });
  both("changing the palette");

  // Erasing every tag over words that carry two.
  did(paint("H", 10, 30));
  did(erase("L", 0, 100), erase("H", 0, 100));
  assert.equal(next("undo"), "erasing 2 tags from 10-30");

  did({ kind: "tag_delete", id: "H", now: 10 });
  both("deleting the tag “Hope”");
});

test("describe: a place in another text is left out", () => {
  const { did, next } = setup();
  did({ kind: "tag_create", id: "L", name: "Love", color: "#ff0000", now: 1 });
  did({ kind: "paint", tag: "L", text_id: "bsb", start: 1, end: 5, new_id: newId(), now: 2 });
  assert.equal(next("undo"), "tagging words with “Love”");
  did({ kind: "erase", tag: "L", text_id: "bsb", start: 1, end: 5, now: 3 });
  assert.equal(next("undo"), "erasing “Love”");
});

test("describe: nothing to undo is the empty step", () => {
  const { s } = setup();
  assert.deepEqual(s.stepChanges("undo"), []);
  assert.deepEqual(s.stepChanges("redo"), []);
});
