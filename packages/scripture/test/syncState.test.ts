import { test } from "node:test";
import assert from "node:assert/strict";
import { effect, MemRows, OpError, type Change, type OpIntent, type Row } from "../src/ops.ts";
import { SyncState, type ClientMsg, type ServerMsg } from "../src/syncState.ts";

const K = "kjv-1769-v1";

/** Authoritative rows + per-connection queues; mirrors crates/server/src/sync.rs semantics. */
class FakeServer {
  rows = new MemRows();
  log: Change[] = [];
  conns = new Set<Conn>();

  connect(client: SyncState): Conn {
    const conn = new Conn(this, client);
    this.conns.add(conn);
    client.connected((m) => conn.up.push(m));
    return conn;
  }

  handle(conn: Conn, m: ClientMsg) {
    if (m.type === "hello") {
      for (const c of this.log) if (c.seq > m.after) conn.down.push({ type: "change", change: c });
      conn.down.push({ type: "ready", seq: this.log.length });
      conn.live = true;
      return;
    }
    const op = m.op;
    const dup = this.log.find((c) => c.op_id === op.op_id);
    if (dup) {
      conn.down.push({ type: "ack", op_id: op.op_id, seq: dup.seq });
      return;
    }
    let change: Change;
    try {
      change = { ...effect(this.rows, op), seq: this.log.length + 1 };
    } catch (e) {
      if (!(e instanceof OpError)) throw e;
      conn.down.push({ type: "reject", op_id: op.op_id, code: e.code, message: e.code });
      return;
    }
    this.rows.applyChange(change);
    this.log.push(change);
    conn.down.push({ type: "ack", op_id: op.op_id, seq: change.seq });
    for (const c of this.conns) if (c.live) c.down.push({ type: "change", change });
  }
}

class Conn {
  up: ClientMsg[] = [];
  down: ServerMsg[] = [];
  live = false;
  readonly server: FakeServer;
  readonly client: SyncState;
  constructor(server: FakeServer, client: SyncState) {
    this.server = server;
    this.client = client;
  }
  /** Deliver one queued message in each direction (if any). Returns false once closed. */
  stepUp() {
    const m = this.up.shift();
    if (m) this.server.handle(this, m);
  }
  stepDown(): boolean {
    const m = this.down.shift();
    if (m && !this.client.receive(m)) {
      this.close();
      return false;
    }
    return true;
  }
  close() {
    this.server.conns.delete(this);
    this.client.disconnected();
    this.up = [];
    this.down = [];
  }
  flush() {
    while (this.up.length || this.down.length) {
      this.stepUp();
      if (!this.stepDown()) return;
    }
  }
}

let idn = 0;
const newId = () => `id${++idn}`;
const now = () => 1000;
const client = () => new SyncState("owner", newId, now);

/** All rows, sorted, for comparing a client's view with the server. */
const dump = (m: MemRows): Row[] =>
  m.allRows().sort((a, b) => {
    const ka = a.table + (a.table === "palette" ? "" : a.id);
    const kb = b.table + (b.table === "palette" ? "" : b.id);
    return ka < kb ? -1 : ka > kb ? 1 : 0;
  });

const createTag = (id: string, name: string): OpIntent => ({ kind: "tag_create", id, name, color: "#336699", now: 1 });
const paint = (tag: string, start: number, end: number, id: string): OpIntent => ({ kind: "paint", tag, text_id: K, start, end, new_id: id, now: 2 });

function converged(server: FakeServer, ...clients: SyncState[]) {
  for (const c of clients) {
    assert.deepEqual(dump(c.view), dump(server.rows));
    assert.deepEqual(c.pending, []);
    assert.equal(c.seq, server.log.length);
  }
}

test("sync: online edits are confirmed and match the server", () => {
  const server = new FakeServer();
  const a = client();
  const conn = server.connect(a);
  conn.flush();
  assert.equal(a.status, "synced");
  assert.equal(a.submit([createTag("T", "Grace")]), null);
  assert.equal(a.submit([paint("T", 10, 20, "g1")]), null);
  assert.equal(a.pending.length, 2, "optimistic until confirmed");
  assert.ok(a.view.taggings.get("g1"), "visible immediately");
  conn.flush();
  converged(server, a);
});

test("sync: offline edits queue and are sent after reconnecting", () => {
  const server = new FakeServer();
  const a = client();
  assert.equal(a.submit([createTag("T", "Grace"), paint("T", 1, 5, "g1")]), null);
  assert.equal(a.status, "offline");
  server.connect(a).flush();
  converged(server, a);
  assert.deepEqual(server.log.map((c) => c.kind), ["tag_create", "paint"]);
});

test("sync: concurrent edits from two devices rebase and converge", () => {
  const server = new FakeServer();
  const [a, b] = [client(), client()];
  const [ca, cb] = [server.connect(a), server.connect(b)];
  ca.flush();
  cb.flush();
  a.submit([createTag("T", "Grace")]);
  ca.flush();
  cb.flush();
  // Both paint overlapping ranges before seeing each other's edit.
  a.submit([paint("T", 10, 20, "ga")]);
  b.submit([paint("T", 15, 30, "gb")]);
  ca.stepUp(); // server applies a's paint first
  cb.flush(); // b receives a's change (rebasing its pending paint), then its own op is merged on the server
  ca.flush();
  converged(server, a, b);
  const live = [...a.view.taggings.values()].filter((t) => t.deleted_at === null);
  assert.deepEqual(live.map((t) => [t.start, t.end]), [[10, 30]], "merged into one tagging");
});

test("sync: an edit invalidated by another device is dropped with a notice", () => {
  const server = new FakeServer();
  const [a, b] = [client(), client()];
  const [ca, cb] = [server.connect(a), server.connect(b)];
  ca.flush();
  cb.flush();
  a.submit([createTag("T", "Grace")]);
  ca.flush();
  cb.flush();
  const notices: string[] = [];
  b.onNotice((m) => notices.push(m));
  b.submit([paint("T", 1, 2, "gb")]); // queued...
  a.submit([{ kind: "tag_delete", id: "T", now: 5 }]); // ...while a deletes the tag
  ca.flush();
  cb.flush();
  ca.flush();
  converged(server, a, b);
  assert.equal(notices.length, 1);
  assert.equal(b.canUndo(), false, "the dropped op left the undo stack");
});

test("sync: undo and redo go through the server; a conflicting undo is refused", () => {
  const server = new FakeServer();
  const [a, b] = [client(), client()];
  const [ca, cb] = [server.connect(a), server.connect(b)];
  ca.flush();
  cb.flush();
  a.submit([createTag("T", "Grace")]);
  a.submit([paint("T", 10, 20, "g1")]);
  ca.flush();
  assert.equal(a.undoStep(), null);
  ca.flush();
  assert.equal(a.view.taggings.get("g1")?.deleted_at, 1000, "undo tombstoned the tagging");
  assert.equal(a.redoStep(), null);
  ca.flush();
  assert.equal(a.view.taggings.get("g1")?.deleted_at, null, "redo restored it");
  cb.flush();
  // b extends the tagging; a's undo of its paint would clobber that, so it is refused and kept.
  b.submit([paint("T", 20, 25, "g2")]);
  cb.flush();
  ca.flush();
  const notices: string[] = [];
  a.onNotice((m) => notices.push(m));
  assert.equal(a.undoStep()?.code, "conflict");
  assert.equal(notices.length, 1);
  assert.equal(a.canUndo(), true);
  converged(server, a, b);
});

test("sync: a snapshot restores state, pending ops and undo across a reload", () => {
  const server = new FakeServer();
  const a = client();
  server.connect(a).flush();
  a.submit([createTag("T", "Grace")]);
  a.disconnected();
  a.submit([paint("T", 3, 9, "g1")]); // offline
  const snap = JSON.parse(JSON.stringify(a.snapshot()));
  const reloaded = new SyncState("owner", newId, now, snap);
  assert.deepEqual(dump(reloaded.view), dump(a.view));
  server.connect(reloaded).flush();
  converged(server, reloaded);
  assert.equal(reloaded.undoStep(), null, "undo history survived the reload");
  assert.ok(new SyncState("someone-else", newId, now, snap).view.tags.size === 0, "another owner starts empty");
});

test("sync: a gap in seq makes the client reconnect", () => {
  const a = client();
  a.connected(() => {});
  const change: Change = { seq: 5, op_id: "x", kind: "tag_create", rows: [] };
  assert.equal(a.receive({ type: "change", change }), false);
});

function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

test("sync: random multi-device interleavings always converge", () => {
  for (let seed = 1; seed <= 120; seed++) {
    const r = rng(seed);
    const int = (n: number) => Math.floor(r() * n);
    const server = new FakeServer();
    const clients = [client(), client(), client()];
    const conns: (Conn | null)[] = clients.map(() => null);
    for (let step = 0; step < 300; step++) {
      const i = int(clients.length);
      const c = clients[i]!;
      const conn = conns[i];
      const roll = int(10);
      if (roll < 3) {
        const tags = [...c.view.tags.values()].filter((t) => t.deleted_at === null).map((t) => t.id);
        const tag = tags[int(tags.length)];
        const s = int(50);
        // Comments: on a tagging this client sees (another client may have merged or erased it meanwhile).
        const taggings = [...c.view.taggings.values()].filter((t) => t.deleted_at === null).map((t) => t.id);
        const comments = [...c.view.comments.values()].filter((x) => x.deleted_at === null).map((x) => x.id);
        const commentOp = (): OpIntent | null => {
          if (comments.length && int(2)) {
            const id = comments[int(comments.length)]!;
            return int(2) ? { kind: "comment_update", id, body: `edit ${step}`, now: step } : { kind: "comment_delete", id, now: step };
          }
          if (!taggings.length) return null;
          return { kind: "comment_create", id: `c${seed}_${step}`, tagging: taggings[int(taggings.length)]!, body: `note ${step}`, now: step };
        };
        const comment = int(4) === 0 ? commentOp() : null;
        const intent: OpIntent = comment ??
          (!tag || int(8) === 0
            ? createTag(`t${seed}_${step}`, `n${int(4)}`)
            : int(10) === 0
              ? { kind: "tag_delete", id: tag, now: step }
              : int(3) === 0
                ? { kind: "erase", tag, text_id: K, start: s, end: s + 1 + int(8), split_id: `s${seed}_${step}`, now: step }
                : paint(tag, s, s + 1 + int(8), `p${seed}_${step}`));
        c.submit([intent]);
      } else if (roll < 4) {
        if (int(2)) c.undoStep();
        else c.redoStep();
      } else if (roll < 5) {
        if (conn) conn.close();
        conns[i] = conn ? null : server.connect(c);
      } else if (conn) {
        if (int(2)) conn.stepUp();
        else if (!conn.stepDown()) conns[i] = null;
      }
    }
    // Everyone comes online and drains.
    for (let round = 0; round < 3; round++) {
      clients.forEach((c, i) => {
        conns[i] ??= server.connect(c);
        conns[i]!.flush();
        if (!server.conns.has(conns[i]!)) conns[i] = null;
      });
    }
    clients.forEach((c, i) => {
      conns[i] ??= server.connect(c);
      conns[i]!.flush();
    });
    for (const c of clients) {
      assert.deepEqual(dump(c.view), dump(server.rows), `seed ${seed}: views converge on the server state`);
      assert.deepEqual(c.pending, [], `seed ${seed}: nothing left pending`);
    }
  }
});

test("sync: a multi-op step undoes in reverse order (palette before the tag it references)", () => {
  const server = new FakeServer();
  const a = client();
  const conn = server.connect(a);
  conn.flush();
  assert.equal(a.submit([createTag("T", "Grace"), { kind: "palette_set", tags: ["T"], now: 3 }]), null);
  conn.flush();
  assert.equal(a.undoStep(), null, "reverting the palette first, then the tag, is valid");
  conn.flush();
  assert.equal(a.view.tags.get("T")?.deleted_at, 1000);
  assert.deepEqual(a.view.pal?.tags, []);
  assert.equal(a.redoStep(), null);
  conn.flush();
  assert.deepEqual(a.view.pal?.tags, ["T"]);
  converged(server, a);
});

test("sync: two tabs sharing an outbox never lose or double-apply ops", () => {
  const server = new FakeServer();
  const a = client();
  server.connect(a).flush();
  a.submit([createTag("T", "Grace")]);
  a.disconnected();
  a.submit([paint("T", 1, 4, "g1")]); // offline in tab A
  // Tab B starts from the shared snapshot and adopts A's outbox (A may close before it reconnects).
  const snap = JSON.parse(JSON.stringify(a.snapshot()));
  const b = new SyncState("owner", newId, now, { ...snap, pending: [], undo: [], redo: [] });
  b.adopt(snap.pending);
  assert.ok(b.view.taggings.get("g1"), "B shows A's queued edit");
  assert.equal(b.canUndo(), false, "undo history is per tab");
  const cb = server.connect(b);
  cb.flush();
  converged(server, b);
  // A comes back later and re-sends the same ops: acked as duplicates, applied once, no spurious notices.
  const notices: string[] = [];
  a.onNotice((m) => notices.push(m));
  server.connect(a).flush();
  converged(server, a, b);
  assert.equal(server.log.filter((c) => c.op_id === snap.pending[snap.pending.length - 1].op_id).length, 1);
  assert.deepEqual(notices, []);
  // Adopting an op that is already confirmed is a no-op.
  b.adopt(snap.pending);
  assert.deepEqual(b.pending, []);
});

test("sync: reset replaces the confirmed state and rebases pending ops onto it", () => {
  const a = client();
  a.connected(() => {});
  a.receive({ type: "ready", seq: 0 });
  a.submit([createTag("T", "Grace")]); // pending, never confirmed by this "server"
  const other: Row[] = [{ table: "tag", id: "U", name: "Hope", color: "#123456", created_at: 1, updated_at: 1, deleted_at: null }];
  const ch: Change = { seq: 7, op_id: "elsewhere", kind: "tag_create", rows: [{ before: null, after: other[0]! }] };
  assert.equal(a.receive({ type: "reset", seq: 7, rows: other, changes: [ch] }), true);
  assert.equal(a.seq, 7);
  assert.ok(a.view.tags.get("U"), "the reset state is shown");
  assert.ok(a.view.tags.get("T"), "the pending op is replayed on top");
  assert.equal(a.pending.length, 1);
  a.adopt([{ op_id: "elsewhere", kind: "tag_create", id: "U", name: "Hope", color: "#123456", now: 1 }]);
  assert.equal(a.pending.length, 1, "ops in the reset's changes count as confirmed");
});

test("sync: confirmed comments survive a snapshot (the persisted base includes every table)", () => {
  const server = new FakeServer();
  const a = client();
  server.connect(a).flush();
  a.submit([createTag("T", "Grace")]);
  a.submit([paint("T", 3, 9, "g1")]);
  a.submit([{ kind: "comment_create", id: "c1", tagging: "g1", body: "Note", now: 1 }]);
  server.connect(a).flush();
  assert.equal(a.pending.length, 0, "all confirmed");
  const snap = JSON.parse(JSON.stringify(a.snapshot()));
  const reloaded = new SyncState("owner", newId, now, snap);
  assert.equal(reloaded.view.comments.get("c1")?.body, "Note");
});
