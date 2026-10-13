// Groups on the Cloudflare Worker + Durable Objects: shared rows, roles, comment authors, kicks, leaving.
import assert from "node:assert/strict";
const B = process.argv[2], G = 18 * 2 ** 24 + 8 * 2 ** 16 + 9 * 2 ** 8; // Esther 8:9, word 0 (90 words)

async function user() {
  const r = await fetch(B + "/api/session", { method: "POST", headers: { Origin: B } });
  const cookie = r.headers.get("set-cookie").split(";")[0];
  const owner = (await r.json()).owner;
  const call = async (method, path, body) => {
    const res = await fetch(B + "/api" + path, { method, headers: { Origin: B, Cookie: cookie, "Content-Type": "application/json" }, body: body && JSON.stringify(body) });
    return { status: res.status, ...(await res.json().catch(() => ({}))) };
  };
  return { cookie, owner, call };
}
function sync(u, space) {
  const ws = new WebSocket(`${B.replace("http", "ws")}/api/sync?space=${space}`, { headers: { Cookie: u.cookie, Origin: B } });
  const msgs = [];
  ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
  const until = async (pred) => { for (let i = 0; i < 100; i++) { const m = msgs.find(pred); if (m) return m; await new Promise((r) => setTimeout(r, 50)); } throw new Error(JSON.stringify(msgs)); };
  const op = (o) => ws.send(JSON.stringify({ type: "op", op: o }));
  return new Promise((ok, fail) => {
    ws.onerror = () => fail(new Error("refused"));
    ws.onopen = async () => { ws.send(JSON.stringify({ type: "hello", after: 0 })); await until((m) => m.type === "ready"); ok({ ws, msgs, until, op }); };
  });
}

const [ann, bob, cat, dan] = [await user(), await user(), await user(), await user()];
const g = await ann.call("POST", "/groups", { name: "Study" });
assert.equal(g.role, "admin");
const read = (await ann.call("POST", `/groups/${g.id}/invites`, { role: "read" })).invites.read;
const edit = (await ann.call("POST", `/groups/${g.id}/invites`, { role: "edit" })).invites.edit;
assert.equal((await bob.call("POST", "/join", { token: read })).role, "read");
assert.equal((await cat.call("POST", "/join", { token: edit })).role, "edit");
assert.equal((await cat.call("GET", "/groups")).groups[0].name, "Study");
assert.equal((await bob.call("PATCH", `/groups/${g.id}`, { name: "x" })).status, 403);
await assert.rejects(sync(dan, g.id), /refused/, "not a member");

const [a, b, c] = [await sync(ann, g.id), await sync(bob, g.id), await sync(cat, g.id)];
a.op({ op_id: "t", kind: "tag_create", id: "T", name: "Grace", color: "#336699", now: 1 });
await a.until((m) => m.type === "ack");
c.op({ op_id: "p", kind: "paint", tag: "T", text_id: "kjv-1769-v2", start: G + 1, end: G + 5, new_id: "g1", now: 2 });
c.op({ op_id: "c", kind: "comment_create", id: "c1", tagging: "g1", body: "Hi", now: 3 });
const seen = await b.until((m) => m.type === "change" && m.change.seq === 3);
assert.equal(seen.change.rows[0].after.author, cat.owner, "comments carry their author; readers get live changes");
b.op({ op_id: "pb", kind: "paint", tag: "T", text_id: "kjv-1769-v2", start: G + 20, end: G + 22, new_id: "g2", now: 4 });
assert.equal((await b.until((m) => m.op_id === "pb")).code, "read_only");
a.op({ op_id: "e", kind: "comment_update", id: "c1", body: "Mine", now: 5 });
assert.equal((await a.until((m) => m.op_id === "e" && m.type === "reject")).code, "not_author");

// Demoting Cat disconnects her; she comes back as a reader.
const r = await ann.call("PUT", `/groups/${g.id}/members/${cat.owner}`, { role: "read" });
assert.equal(r.members.find((m) => m.owner === cat.owner).role, "read");
await c.until((m) => m.type === "error");
const c2 = await sync(cat, g.id);
c2.op({ op_id: "pc", kind: "paint", tag: "T", text_id: "kjv-1769-v2", start: G + 30, end: G + 32, new_id: "g3", now: 6 });
assert.equal((await c2.until((m) => m.op_id === "pc")).code, "read_only");
// Leaving revokes access.
await bob.call("DELETE", `/groups/${g.id}/members/${bob.owner}`);
await b.until((m) => m.type === "error");
await assert.rejects(sync(bob, g.id), /refused/);
// The group's rows are not Ann's personal rows.
const mine = await sync(ann, ann.owner);
assert.equal(mine.msgs.filter((m) => m.type === "change").length, 0);

for (const x of [a, b, c, c2, mine]) x.ws.close();
console.log("cloudflare groups e2e: ok");
