// Retention under wrangler dev with RETENTION_MS=0 (each op prunes every older change): a client too far behind
// gets a reset, a re-sent pruned op is still a duplicate, and replay from the pruning point still works.
import assert from "node:assert/strict";
const B = process.argv[2];
const G = 18 * 2 ** 24 + 8 * 2 ** 16 + 9 * 2 ** 8; // Esther 8:9, word 0

const r = await fetch(B + "/api/session", { method: "POST", headers: { Origin: B } });
const cookie = r.headers.get("set-cookie").split(";")[0];

function connect(hello) {
  const ws = new WebSocket(B.replace("http", "ws") + "/api/sync", { headers: { Cookie: cookie, Origin: B } });
  const msgs = [];
  const waiters = [];
  ws.onmessage = (e) => {
    msgs.push(JSON.parse(e.data));
    waiters.splice(0).forEach((w) => w());
  };
  const until = async (pred, ms = 5000) => {
    const t0 = Date.now();
    for (;;) {
      const m = msgs.find(pred);
      if (m) return m;
      if (Date.now() - t0 > ms) throw new Error("timeout; got " + JSON.stringify(msgs));
      await new Promise((r) => { waiters.push(r); setTimeout(r, 100); });
    }
  };
  const ready = new Promise((r, j) => { ws.onopen = r; ws.onerror = j; })
    .then(() => ws.send(JSON.stringify({ type: "hello", ...hello })))
    .then(() => until((m) => m.type === "ready"));
  return { ws, msgs, until, ready, op: (op) => ws.send(JSON.stringify({ type: "op", op })) };
}

const a = connect({ after: 0 });
await a.ready;
a.op({ op_id: "o1", kind: "tag_create", id: "T", name: "Grace", color: "#ff0000", now: 1 });
await a.until((m) => m.type === "change" && m.change.seq === 1);
await new Promise((r) => setTimeout(r, 20)); // so o1 is strictly older
a.op({ op_id: "o2", kind: "paint", tag: "T", text_id: "kjv-1769-v2", start: G, end: G + 3, new_id: "g", now: 2 });
await a.until((m) => m.type === "change" && m.change.seq === 2);

const late = connect({ after: 0, pending: ["o1", "never-sent"] });
assert.deepEqual(await late.ready, { type: "ready", seq: 2 });
const [reset, ...rest] = late.msgs;
assert.equal(reset.type, "reset");
assert.equal(reset.seq, 2);
assert.deepEqual(reset.rows.map((r) => r.id), ["T", "g"]);
assert.deepEqual(reset.applied, ["o1"]);
assert.deepEqual(rest.map((m) => m.type), ["ready"], "no replayed changes");
late.op({ op_id: "o1", kind: "tag_create", id: "T", name: "Grace", color: "#ff0000", now: 1 });
assert.equal((await late.until((m) => m.type === "ack")).seq, 1, "a pruned op is still a duplicate");

const recent = connect({ after: 1 });
await recent.ready;
assert.deepEqual(recent.msgs.filter((m) => m.type === "change").map((m) => m.change.seq), [2]);

for (const c of [a, late, recent]) c.ws.close();
console.log("cloudflare retention: ok");
