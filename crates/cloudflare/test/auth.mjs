// Sign in with Google (fake provider) + merge on the Cloudflare Worker under wrangler dev.
import assert from "node:assert/strict";
import http from "node:http";
import crypto from "node:crypto";
const B = process.argv[2], G = 2 ** 25;
const FAKE_PORT = Number(process.argv[3]);
const b64 = (s) => Buffer.from(s).toString("base64url");

let pending = {}; // code -> {sub, name, email}
let lastForm;
const fake = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    const f = Object.fromEntries(new URLSearchParams(body));
    lastForm = f;
    const who = pending[f.code];
    if (req.url !== "/token" || !who || f.client_id !== "cid" || f.client_secret !== "sec" || f.grant_type !== "authorization_code"
        || f.redirect_uri !== B + "/api/auth/google/callback"
        || crypto.createHash("sha256").update(f.code_verifier).digest("base64url") !== who.challenge) {
      res.writeHead(400).end("bad");
      return;
    }
    const claims = { iss: "https://accounts.google.com", aud: "cid", sub: who.sub, exp: Math.floor(Date.now() / 1000) + 600, email: who.email, email_verified: true, name: who.name };
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ id_token: `h.${b64(JSON.stringify(claims))}.s` }));
  });
}).listen(FAKE_PORT, "127.0.0.1");

async function session(cookie) {
  const r = await fetch(B + "/api/session", { method: "POST", headers: { Origin: B, ...(cookie ? { Cookie: cookie } : {}) } });
  const sc = r.headers.get("set-cookie");
  return { ...(await r.json()), cookie: sc ? sc.split(";")[0] : cookie };
}
function sync(cookie) {
  const ws = new WebSocket(B.replace("http", "ws") + "/api/sync", { headers: { Cookie: cookie, Origin: B } });
  const msgs = [];
  ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
  const until = async (pred) => { for (let i = 0; i < 100; i++) { const m = msgs.find(pred); if (m) return m; await new Promise((r) => setTimeout(r, 50)); } throw new Error(JSON.stringify(msgs)); };
  return new Promise((r) => (ws.onopen = () => { ws.send(JSON.stringify({ type: "hello", after: 0 })); r({ ws, msgs, until }); }));
}
async function ops(cookie, list) {
  const s = await sync(cookie);
  await s.until((m) => m.type === "ready");
  for (const o of list) s.ws.send(JSON.stringify({ type: "op", op: o }));
  for (const o of list) assert.equal((await s.until((m) => m.op_id === o.op_id && m.type !== "change")).type, "ack", o.op_id);
  s.ws.close();
}
async function signIn(cookie, who, ret = "#john/3") {
  const r = await fetch(`${B}/api/auth/google?return=${encodeURIComponent(ret)}`, { redirect: "manual", headers: cookie ? { Cookie: cookie } : {} });
  assert.equal(r.status, 302);
  const loc = new URL(r.headers.get("location"));
  assert.equal(loc.origin + loc.pathname, `http://127.0.0.1:${FAKE_PORT}/auth`);
  assert.equal(loc.searchParams.get("redirect_uri"), B + "/api/auth/google/callback");
  const flow = r.headers.get("set-cookie").split(";")[0];
  const code = "code" + Math.random();
  pending[code] = { ...who, challenge: loc.searchParams.get("code_challenge") };
  const cb = await fetch(`${B}/api/auth/google/callback?code=${code}&state=${loc.searchParams.get("state")}`, {
    redirect: "manual", headers: { Cookie: [cookie, flow].filter(Boolean).join("; ") } });
  assert.equal(cb.status, 302);
  const cookies = cb.headers.getSetCookie();
  const sess = cookies.find((c) => c.startsWith("bt_session="));
  return { location: cb.headers.get("location"), cookie: sess && sess.split(";")[0], cookies };
}

// 1. An anonymous owner signs in with a new identity: it becomes the account.
const a = await session();
await ops(a.cookie, [
  { op_id: "a1", kind: "tag_create", id: "TA", name: "Grace", color: "#ff0000", now: 1 },
  { op_id: "a2", kind: "paint", tag: "TA", text_id: "kjv-1769-v1", start: G + 10, end: G + 20, new_id: "pa", now: 2 },
]);
const s1 = await signIn(a.cookie, { sub: "s1", name: "Ann", email: "ann@example.com" });
assert.equal(s1.location, "/#john/3");
assert.notEqual(s1.cookie, a.cookie, "the session rotates");
const acct = await session(s1.cookie);
assert.equal(acct.owner, a.owner);
assert.deepEqual(acct.account, { provider: "google", email: "ann@example.com", name: "Ann" });
assert.deepEqual(acct.providers, ["google"]);
assert.notEqual((await session(a.cookie)).owner, a.owner, "the old session is gone");

// 2. Another device's anonymous data merges into the existing account.
const b = await session();
await ops(b.cookie, [
  { op_id: "b1", kind: "tag_create", id: "TB", name: "grace", color: "#00ff00", now: 3 },
  { op_id: "b2", kind: "paint", tag: "TB", text_id: "kjv-1769-v1", start: G + 15, end: G + 30, new_id: "pb", now: 4 },
  { op_id: "b3", kind: "tag_create", id: "TL", name: "Love", color: "#0000ff", now: 5 },
  { op_id: "b4", kind: "paint", tag: "TL", text_id: "kjv-1769-v1", start: G + 50, end: G + 60, new_id: "pl", now: 6 },
]);
const live = await sync(s1.cookie); // a's socket sees the merge live
await live.until((m) => m.type === "ready");
const s2 = await signIn(b.cookie, { sub: "s1", name: "Ann", email: "ann@example.com" });
const merged = await session(s2.cookie);
assert.equal(merged.owner, a.owner);
await live.until((m) => m.type === "change" && m.change.op_id.startsWith("merge-"));
live.ws.close();
const all = await sync(s2.cookie);
await all.until((m) => m.type === "ready");
// Replay the changes into a row map.
const rows = new Map();
for (const m of all.msgs.filter((m) => m.type === "change")) for (const rc of m.change.rows) { const r = rc.after; rows.set(r.table + ":" + (r.id ?? ""), r); }
const spans = [...rows.values()].filter((r) => r.table === "tagging" && r.deleted_at == null).map((r) => [r.tag, r.start - G, r.end - G]).sort((x, y) => x[1] - y[1]);
const tags = new Map([...rows.values()].filter((r) => r.table === "tag").map((r) => [r.id, r.name]));
assert.deepEqual(spans.map(([t, s, e]) => [tags.get(t), s, e]), [["Grace", 10, 30], ["Love", 50, 60]]);
all.ws.close();

// 3. Forged state and a cancelled consent fail safely.
const r = await fetch(`${B}/api/auth/google?return=%23x`, { redirect: "manual" });
const flow = r.headers.get("set-cookie").split(";")[0];
const forged = await fetch(`${B}/api/auth/google/callback?code=c&state=WRONG`, { redirect: "manual", headers: { Cookie: flow } });
assert.equal(forged.headers.get("location"), "/?auth_error=expired#x");
const cancelled = await fetch(`${B}/api/auth/google/callback?error=access_denied`, { redirect: "manual", headers: { Cookie: flow } });
assert.equal(cancelled.headers.get("location"), "/?auth_error=cancelled#x");
const evil = await signIn(undefined, { sub: "s2", name: "Bo" }, "https://evil.example");
assert.equal(evil.location, "/", "only fragments are returned to");

fake.close();
console.log("cloudflare auth e2e: ok");
