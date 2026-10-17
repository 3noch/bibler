// Offline-only builds (VITE_SYNC=local, e.g. GitHub Pages): a stand-in for the sync server that lives in the
// browser. It speaks the same protocol to SyncState (hello/op -> reset/ready/ack/reject/change) and applies ops with
// the same canonical `effect`, so the rest of the client (optimistic view, rebase, undo/redo, outbox) is unchanged.
//
// Storage: the confirmed state (rows + recent changes, which are the undo targets, + the op ids of older changes,
// so a re-sent op is a duplicate forever, as on the server) in localStorage, updated under a Web Lock so tabs never
// interleave writes. Ops are checked as the server checks them (`checkOp`: ids, and tagging ends against the text
// index). Tabs hear each other's changes over a BroadcastChannel; a tab that falls behind (a gap) says hello again
// and gets a `reset`.
import { checkOp, effect, MemRows, OpError, type Change, type ClientMsg, type Row, type ServerMsg, type SyncState, type TextIndex } from "@bible-tags/scripture";
import { TEXT_ID } from "./textId.ts";

const DB_KEY = "bible-tags/local-db/v1";
const KEEP_CHANGES = 1000;

export interface LocalDb {
  seq: number;
  rows: Row[];
  changes: Change[];
  /** Op ids of changes no longer in `changes` (older; absent in databases from before it was kept). */
  older?: string[];
}

export function loadLocalDb(): LocalDb {
  try {
    const raw = localStorage.getItem(DB_KEY);
    if (raw) return JSON.parse(raw) as LocalDb;
  } catch {
    // unreadable: start empty
  }
  return { seq: 0, rows: [], changes: [] };
}

function saveLocalDb(db: LocalDb) {
  localStorage.setItem(DB_KEY, JSON.stringify(db));
}

/** Run `f` under the cross-tab lock, after every earlier call (in order, even while one awaits). */
let queue = Promise.resolve();
function withLock(f: () => void | Promise<void>) {
  queue = queue.then(() => (navigator.locks ? navigator.locks.request("bible-tags-local-db", async () => f()) : f())).catch((e: unknown) => console.error(e));
}

/** Connect `sync` to the browser-local store; `index` (the text's) checks ops' ordinals. */
export function connectLocal(sync: SyncState, index: Promise<TextIndex>, onError: (msg: string) => void) {
  const channel = "BroadcastChannel" in window ? new BroadcastChannel("bible-tags-local") : null;
  const reply = (m: ServerMsg) => {
    if (!sync.receive(m)) hello(); // a gap: start over with a reset
  };

  const handle = (m: ClientMsg) => {
    if (m.type === "hello") {
      const db = loadLocalDb();
      if (db.seq !== m.after) reply({ type: "reset", seq: db.seq, rows: db.rows, changes: db.changes });
      reply({ type: "ready", seq: db.seq });
      return;
    }
    const op = m.op;
    withLock(async () => {
      const bad = checkOp(await index, TEXT_ID, op);
      const db = loadLocalDb();
      const done = db.changes.find((c) => c.op_id === op.op_id);
      if (done || db.older?.includes(op.op_id)) {
        // (An older one's seq is gone with its change; any seq at or below ours confirms it: see SyncState "ack".)
        reply({ type: "ack", op_id: op.op_id, seq: done?.seq ?? 0 });
        return;
      }
      if (bad) {
        reply({ type: "reject", op_id: op.op_id, ...bad });
        return;
      }
      const rows = new MemRows(db.rows, db.changes);
      let change: Change;
      try {
        change = { ...effect(rows, op), seq: db.seq + 1 };
      } catch (e) {
        if (!(e instanceof OpError)) throw e;
        reply({ type: "reject", op_id: op.op_id, code: e.code, message: e.code });
        return;
      }
      rows.applyChange(change);
      const all = [...db.changes, change];
      const trimmed = all.slice(0, -KEEP_CHANGES).map((c) => c.op_id);
      const next: LocalDb = {
        seq: change.seq,
        rows: rows.allRows(),
        changes: all.slice(-KEEP_CHANGES),
        older: [...(db.older ?? []), ...trimmed],
      };
      try {
        saveLocalDb(next);
      } catch {
        onError("Couldn't save in this browser (storage full or blocked). Your last edit was not kept.");
        reply({ type: "reject", op_id: op.op_id, code: "storage", message: "storage full" });
        return;
      }
      reply({ type: "ack", op_id: op.op_id, seq: change.seq });
      reply({ type: "change", change });
      channel?.postMessage({ type: "change", change } satisfies ServerMsg);
    });
  };

  const hello = () => sync.connected(handle);
  if (channel) channel.onmessage = (e: MessageEvent<ServerMsg>) => reply(e.data);
  hello();
}
