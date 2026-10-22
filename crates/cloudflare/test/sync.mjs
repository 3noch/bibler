// End-to-end check of the Cloudflare Worker + Durable Object under wrangler dev.
import assert from "node:assert/strict";
const B = process.argv[2];
const G = 18 * 2 ** 24 + 8 * 2 ** 16 + 9 * 2 ** 8; // Esther 8:9, word 0 (90 words: G..G+89 are words)

async function session(cookie) {
  const r = await fetch(B + "/api/session", { method: "POST", headers: { Origin: B, ...(cookie ? { Cookie: cookie } : {}) } });
  assert.equal(r.status, 200);
  const sc = r.headers.get("set-cookie");
  return { ...(await r.json()), cookie: sc ? sc.split(";")[0] : cookie };
}

function connect(cookie, after = 0) {
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
  const open = new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  return { ws, msgs, until, ready: open.then(() => ws.send(JSON.stringify({ type: "hello", after }))).then(() => until((m) => m.type === "ready")) };
}

const a = await session();
assert.equal(a.account, null);
const again = await session(a.cookie);
assert.equal(again.owner, a.owner, "the session cookie keeps the owner");

const c1 = connect(a.cookie), c2 = connect(a.cookie);
const first = await c1.ready;
assert.deepEqual([first.seq, Object.keys(first.build)], [0, ["commit", "committed_at"]], "ready says the server's build");
assert.match(first.build.commit, /^[0-9a-f]{40}$/);
const served = (await (await fetch(B + "/")).text()).match(/<meta name="build-client" content="([0-9a-f]+)">/)?.[1];
assert.ok(served && first.client === served, "ready says which client is deployed (so open apps can update)");
assert.equal((await (await fetch(B + "/client-version.txt")).text()).trim(), served, "the page and the file agree");
await c2.ready;
const op = (o) => c1.ws.send(JSON.stringify({ type: "op", op: o }));
op({ op_id: "o1", kind: "tag_create", id: "T1", name: "Grace", color: "#ff0000", now: 1 });
op({ op_id: "o2", kind: "paint", tag: "T1", text_id: "kjv-1769-v2", start: G + 10, end: G + 20, new_id: "p1", now: 2 });
assert.equal((await c1.until((m) => m.type === "ack" && m.op_id === "o2")).seq, 2);
assert.equal((await c2.until((m) => m.type === "change" && m.change.seq === 2)).change.op_id, "o2", "the other socket sees it live");
op({ op_id: "o3", kind: "paint", tag: "T1", text_id: "kjv-1769-v2", start: 5, end: 9, new_id: "p2", now: 3 });
assert.equal((await c1.until((m) => m.op_id === "o3")).code, "bad_text");
op({ op_id: "o4", kind: "paint", tag: "NOPE", text_id: "kjv-1769-v2", start: G, end: G + 1, new_id: "p3", now: 3 });
assert.equal((await c1.until((m) => m.op_id === "o4")).code, "unknown_tag");
op({ op_id: "bad id!", kind: "tag_delete", id: "T1", now: 3 });
assert.equal((await c1.until((m) => m.op_id === "bad id!")).code, "bad_id");
op({ op_id: "o1", kind: "tag_create", id: "T1", name: "Grace", color: "#ff0000", now: 1 });
await new Promise((r) => setTimeout(r, 300));
assert.equal(c1.msgs.filter((m) => m.type === "ack" && m.op_id === "o1").length, 2, "duplicate is acked again");
assert.equal(c2.msgs.filter((m) => m.type === "change").length, 2, "a duplicate is not broadcast");

// Replay after reconnect.
const c3 = connect(a.cookie, 1);
assert.equal((await c3.ready).seq, 2);
assert.deepEqual(c3.msgs.filter((m) => m.type === "change").map((m) => m.change.seq), [2]);

// Another owner is isolated: may reuse the id "T1".
const b = await session();
const d = connect(b.cookie);
assert.equal((await d.ready).seq, 0);
d.ws.send(JSON.stringify({ type: "op", op: { op_id: "o1", kind: "tag_create", id: "T1", name: "Grace", color: "#ff0000", now: 1 } }));
assert.equal((await d.until((m) => m.type === "ack")).seq, 1);

// Auth checks.
const r1 = await fetch(B + "/api/sync", { headers: { Origin: B } });
assert.equal(r1.status, 401);
const r2 = await fetch(B + "/api/sync", { headers: { Origin: "https://evil", Cookie: a.cookie } });
assert.equal(r2.status, 403);
// Protocol errors close the socket.
const e = connect(a.cookie);
await e.ready;
e.ws.send(JSON.stringify({ type: "hello", after: 0 }));
assert.equal((await e.until((m) => m.type === "error")).message, "expected an op");

// Logout ends the session.
const lo = await fetch(B + "/api/logout", { method: "POST", headers: { Origin: B, Cookie: b.cookie } });
assert.equal(lo.status, 204);
assert.notEqual((await session(b.cookie)).owner, b.owner);

for (const c of [c1, c2, c3, d, e]) c.ws.close();
console.log("cloudflare e2e: ok", { a: a.owner, b: b.owner });
