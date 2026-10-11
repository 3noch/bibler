import { test } from "node:test";
import assert from "node:assert/strict";
import { applyChangeset, normalize, type Changeset, type Intent, type Tagging } from "../src/normalize.ts";

// Seeded PRNG (mulberry32) so failures are reproducible.
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

const N = 60; // ordinal space
const TAGS = ["a", "b"];

function runSeed(seed: number, steps: number) {
  const rand = rng(seed);
  const int = (n: number) => Math.floor(rand() * n);

  let state: Tagging[] = [];
  const model: Record<string, boolean[]> = Object.fromEntries(TAGS.map((t) => [t, Array<boolean>(N).fill(false)]));

  // Comment bookkeeping: every created tagging gets one comment; comment id = "c:" + tagging id.
  const commentOwner = new Map<string, string>(); // comment -> tagging id it currently hangs on
  const tombstones = new Map<string, string | null>(); // tagging id -> merged_into (null = plain delete)
  const ageOf = new Map<string, number>(); // tagging id -> created_at

  const track = (cs: Changeset) => {
    for (const t of cs.create) {
      commentOwner.set(`c:${t.id}`, t.id);
      ageOf.set(t.id, t.created_at);
    }
    for (const d of cs.delete) tombstones.set(d.id, d.merged_into);
    for (const m of cs.move_comments) {
      for (const [c, owner] of commentOwner) if (owner === m.from) commentOwner.set(c, m.to);
    }
  };

  for (let step = 0; step < steps; step++) {
    const tag = TAGS[int(TAGS.length)]!;
    const s = int(N - 1);
    const e = s + 1 + int(Math.min(N - s, 12));
    const now = Math.floor(step / 3); // deliberate created_at ties
    const intent: Intent =
      rand() < 0.6
        ? { kind: "paint", tag, start: s, end: e, new_id: `n${step}`, now }
        : { kind: "erase", tag, start: s, end: e, split_id: `s${step}` };

    // Locality: passing only overlapping/abutting same-tag taggings must give the same result.
    const cs = normalize(state, intent);
    const local = state.filter((t) => t.tag === tag && t.start <= e && t.end >= s);
    assert.deepEqual(normalize(local, intent), cs, `locality (seed ${seed}, step ${step})`);

    // Input order is irrelevant: output is canonical.
    assert.deepEqual(normalize([...local].reverse(), intent), cs, `order independence (seed ${seed}, step ${step})`);

    // Merge survivor is the oldest involved tagging by (created_at, id).
    for (const d of cs.delete) {
      if (d.merged_into === null) continue;
      const loser = ageOf.get(d.id)!;
      const winner = ageOf.get(d.merged_into)!;
      const older = winner < loser || (winner === loser && d.merged_into < d.id);
      assert.ok(older, `survivor must be oldest (seed ${seed}, step ${step})`);
    }

    // Erase-split: the right piece inherits the original's created_at.
    if (intent.kind === "erase") {
      for (const c of cs.create) {
        const src = local.find((t) => t.start < s && t.end > e);
        assert.ok(src, `split has a source (seed ${seed}, step ${step})`);
        assert.equal(c.created_at, src.created_at, `split keeps age (seed ${seed}, step ${step})`);
      }
    }

    track(cs);
    state = applyChangeset(state, cs);

    // Brute-force model.
    const cov = model[tag]!;
    for (let i = s; i < e; i++) cov[i] = intent.kind === "paint";

    // Idempotence: repeating the same intent yields an empty changeset.
    const again = normalize(state, { ...intent, kind: intent.kind, ...(intent.kind === "paint" ? { new_id: "dup" } : { split_id: "dup" }) } as Intent);
    assert.deepEqual(
      again,
      { create: [], update: [], delete: [], move_comments: [] },
      `idempotence (seed ${seed}, step ${step})`,
    );

    // Invariants per tag.
    for (const t of TAGS) {
      const mine = state.filter((x) => x.tag === t).sort((x, y) => x.start - y.start);
      for (let i = 0; i < mine.length; i++) {
        const cur = mine[i]!;
        assert.ok(cur.start < cur.end, "non-empty");
        const next = mine[i + 1];
        if (next) assert.ok(cur.end < next.start, `disjoint & non-adjacent (seed ${seed}, step ${step})`);
      }
      const got = Array<boolean>(N).fill(false);
      for (const x of mine) for (let i = x.start; i < x.end; i++) got[i] = true;
      assert.deepEqual(got, model[t], `coverage matches model (seed ${seed}, step ${step}, tag ${t})`);
    }

    // Comment conservation: none lost; none left hanging on a merged-away tagging.
    const live = new Set(state.map((x) => x.id));
    for (const [c, owner] of commentOwner) {
      const ok = live.has(owner) || (tombstones.has(owner) && tombstones.get(owner) === null);
      assert.ok(ok, `comment ${c} orphaned on ${owner} (seed ${seed}, step ${step})`);
    }
  }
}

test("randomized paint/erase sequences preserve all invariants", () => {
  for (let seed = 1; seed <= 200; seed++) runSeed(seed, 150);
});
