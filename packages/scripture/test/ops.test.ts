import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { effect, MemRows, OpError, resolveTagging, rowKey, type Change, type Op, type Row, type RowChange, type TaggingRow } from "../src/ops.ts";

interface Case {
  name: string;
  state: { rows: Row[]; changes: Change[] };
  op: Op;
  rows?: RowChange[];
  error?: string;
}

const path = new URL("../../../fixtures/ops.json", import.meta.url);
const { cases } = JSON.parse(readFileSync(path, "utf8")) as { cases: Case[] };

for (const c of cases) {
  test(`ops: ${c.name}`, () => {
    const src = new MemRows(c.state.rows, c.state.changes);
    if (c.error) {
      assert.throws(() => effect(src, c.op), (e: unknown) => e instanceof OpError && e.code === c.error);
      return;
    }
    const ch = effect(src, c.op);
    assert.deepEqual(ch.rows, c.rows);
    assert.deepEqual([ch.seq, ch.op_id, ch.kind], [0, "op", c.op.kind]);
  });
}

// ---- property test: random ops keep the invariants; undoing everything restores the start

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function checkInvariants(m: MemRows, ctx: string) {
  const groups = new Map<string, TaggingRow[]>();
  for (const t of m.taggings.values()) {
    if (t.deleted_at !== null) continue;
    assert.equal(m.tags.get(t.tag)?.deleted_at, null, `${ctx}: live tagging of a deleted tag`);
    const k = `${t.tag}|${t.text_id}`;
    groups.set(k, [...(groups.get(k) ?? []), t]);
  }
  for (const list of groups.values()) {
    list.sort((a, b) => a.start - b.start);
    for (let i = 1; i < list.length; i++) assert.ok(list[i - 1]!.end < list[i]!.start, `${ctx}: overlap or abut`);
  }
  const names = new Set<string>();
  for (const t of m.tags.values()) {
    if (t.deleted_at !== null) continue;
    assert.ok(!names.has(t.name.toLowerCase()), `${ctx}: duplicate name`);
    names.add(t.name.toLowerCase());
  }
  for (const id of m.pal?.tags ?? []) assert.equal(m.tags.get(id)?.deleted_at, null, `${ctx}: palette has a dead tag`);
  // Comments never dangle: the merged_into chain ends in a live tagging or a plain deletion, never a missing row.
  for (const c of m.comments.values()) {
    assert.ok(c.body.length > 0, `${ctx}: empty comment`);
    try {
      resolveTagging(m, c.tagging);
    } catch (e) {
      assert.ok(e instanceof OpError && e.code === "tagging_deleted", `${ctx}: comment ${c.id} dangles`);
    }
  }
}

const liveRows = (m: MemRows) =>
  [...m.tags.values(), ...m.taggings.values(), ...m.comments.values()].filter((r) => r.deleted_at === null).sort((a, b) => (rowKey(a) < rowKey(b) ? -1 : 1));

test("ops: random sequences keep invariants and undo-all restores the start", () => {
  const kinds = new Map<string, number>();
  for (let seed = 1; seed <= 150; seed++) {
    const r = rng(seed);
    const int = (n: number) => Math.floor(r() * n);
    const m = new MemRows();
    const applied: string[] = [];
    for (let step = 0; step < 100; step++) {
      const now = step * 10;
      const tagIds = [...m.tags.keys()];
      const pickTag = () => tagIds[int(tagIds.length)]!;
      const roll = int(26);
      const taggingIds = [...m.taggings.keys()];
      const commentIds = [...m.comments.keys()];
      let op: Op;
      // Any tagging, including merged and deleted ones (exercises resolve + tagging_deleted).
      if (roll >= 20 && roll <= 21 && taggingIds.length)
        op = { op_id: `op${step}`, kind: "comment_create", id: `c${step}`, tagging: taggingIds[int(taggingIds.length)]!, body: `note ${int(3)}`, now };
      else if (roll >= 22 && roll <= 23 && commentIds.length)
        op = { op_id: `op${step}`, kind: "comment_update", id: commentIds[int(commentIds.length)]!, body: `note ${int(3)}`, now };
      else if (roll >= 24 && commentIds.length) op = { op_id: `op${step}`, kind: "comment_delete", id: commentIds[int(commentIds.length)]!, now };
      else if (roll <= 1 || tagIds.length === 0) op = { op_id: `op${step}`, kind: "tag_create", id: `t${step}`, name: `name${int(6)}`, color: "#336699", now };
      else if (roll === 2) op = { op_id: `op${step}`, kind: "tag_delete", id: pickTag(), now };
      else if (roll === 3) op = { op_id: `op${step}`, kind: "tag_update", id: pickTag(), name: `name${int(6)}`, now };
      else if (roll === 4) op = { op_id: `op${step}`, kind: "palette_set", tags: Array.from({ length: int(3) }, pickTag), now };
      else if (roll <= 7 && applied.length) op = { op_id: `op${step}`, kind: "revert", target: applied[int(applied.length)]!, now };
      else {
        const start = int(59);
        const end = start + 1 + int(Math.min(60 - start, 10));
        const text_id = r() < 0.5 ? "kjv-1769-v1" : "other-v1";
        op =
          r() < 0.6
            ? { op_id: `op${step}`, kind: "paint", tag: pickTag(), text_id, start, end, new_id: `p${step}`, now }
            : { op_id: `op${step}`, kind: "erase", tag: pickTag(), text_id, start, end, split_id: `s${step}`, now };
      }
      let ch: Change;
      try {
        ch = effect(m, op);
      } catch (e) {
        if (e instanceof OpError) continue;
        throw e;
      }
      for (const rc of ch.rows) {
        const a = rc.after;
        const prior = a.table === "tag" ? m.tags.get(a.id) : a.table === "tagging" ? m.taggings.get(a.id) : a.table === "comment" ? m.comments.get(a.id) : m.pal;
        assert.deepEqual(rc.before, prior ?? null, `seed ${seed} step ${step}: before must be the prior row`);
      }
      kinds.set(ch.kind, (kinds.get(ch.kind) ?? 0) + 1);
      m.applyChange(ch);
      applied.push(op.op_id);
      checkInvariants(m, `seed ${seed} step ${step}`);
    }
    applied.reverse().forEach((target, k) => {
      m.applyChange(effect(m, { op_id: `undo${k}`, kind: "revert", target, now: 1e6 }));
      checkInvariants(m, `seed ${seed} undo ${target}`);
    });
    assert.deepEqual(liveRows(m), [], `seed ${seed}: undo-all must restore the (empty) start`);
    assert.deepEqual(m.pal?.tags ?? [], []);
  }
  for (const k of ["paint", "erase", "tag_create", "tag_update", "tag_delete", "palette_set", "revert", "comment_create", "comment_update", "comment_delete"])
    assert.ok((kinds.get(k) ?? 0) >= 50, `${k} applied only ${kinds.get(k)} times`);
});
