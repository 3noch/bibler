import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { normalize, NormalizeError, type Changeset, type Intent, type Tagging } from "../src/normalize.ts";

interface Case {
  name: string;
  existing: Tagging[];
  intent: Intent;
  expected?: Partial<Changeset>;
  expect_error?: string;
}

const path = new URL("../../../fixtures/normalize.json", import.meta.url);
const { cases } = JSON.parse(readFileSync(path, "utf8")) as { cases: Case[] };

for (const c of cases) {
  test(c.name, () => {
    if (c.expect_error) {
      assert.throws(
        () => normalize(c.existing, c.intent),
        (e: unknown) => e instanceof NormalizeError && e.code === c.expect_error,
      );
      return;
    }
    const want: Changeset = { create: [], update: [], delete: [], move_comments: [], ...c.expected };
    assert.deepEqual(normalize(c.existing, c.intent), want);
  });
}
