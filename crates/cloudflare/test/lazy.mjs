// A space's Durable Object stores nothing until its first write: run on fresh state, it reads wrangler's
// Durable Object files (STATE, from run.sh) to count the spaces that have the schema.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
const [B, STATE] = process.argv.slice(2);
const G = 18 * 2 ** 24 + 8 * 2 ** 16 + 9 * 2 ** 8; // Esther 8:9, word 0

const files = (dir) => fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? files(path.join(dir, e.name)) : [path.join(dir, e.name)]));
/** How many Durable Objects have a `changes` table. */
const created = () =>
  files(STATE).filter((f) => f.includes(`${path.sep}do${path.sep}`) && f.endsWith(".sqlite")).filter((f) => {
    const db = new DatabaseSync(f, { readOnly: true });
    try {
      return db.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'changes'").all().length > 0;
    } finally {
      db.close();
    }
  }).length;

async function session() {
  const r = await fetch(B + "/api/session", { method: "POST", headers: { Origin: B } });
  return r.headers.get("set-cookie").split(";")[0];
}
/** Say hello after `after`, send `ops`, and return every message up to the last op's answer (or `ready`). */
async function visit(cookie, after, ops = []) {
  const ws = new WebSocket(B.replace("http", "ws") + "/api/sync", { headers: { Cookie: cookie, Origin: B } });
  const msgs = [];
  const done = (m) => (ops.length ? m.op_id === ops.at(-1).op_id && m.type !== "change" : m.type === "ready");
  const all = new Promise((r) => (ws.onmessage = (e) => (msgs.push(JSON.parse(e.data)), done(msgs.at(-1)) && r(msgs))));
  await new Promise((r) => (ws.onopen = r));
  ws.send(JSON.stringify({ type: "hello", after }));
  for (const op of ops) ws.send(JSON.stringify({ type: "op", op }));
  await all;
  ws.close();
  return msgs;
}

assert.equal(created(), 0);
const reader = await session();
const ready = (msgs) => msgs.map(({ build, client, ...m }) => m); // (sync.mjs checks those)
assert.deepEqual(ready(await visit(reader, 0)), [{ type: "ready", seq: 0 }]);
// A client that thinks it saw more (e.g. a wiped space) is told it's empty.
const behind = ready(await visit(reader, 7));
assert.deepEqual([behind[0].type, behind[0].seq, behind[0].rows, behind.at(-1)], ["reset", 0, [], { type: "ready", seq: 0 }]);
assert.equal(created(), 0, "reading a space stores nothing");

const writer = await session();
const msgs = await visit(writer, 0, [{ op_id: "w1", kind: "tag_create", id: "TW", name: "Faith", color: "#00ff00", now: 1 },
  { op_id: "w2", kind: "paint", tag: "TW", text_id: "kjv-1769-v2", start: G, end: G + 2, new_id: "pw", now: 2 }]);
assert.deepEqual(msgs.filter((m) => m.type === "ack").map((m) => m.seq), [1, 2]);
assert.equal(created(), 1, "the first write creates the space");
assert.deepEqual((await visit(writer, 0)).map((m) => m.type), ["change", "change", "ready"], "and it is read back");
console.log("cloudflare lazy spaces e2e: ok");
