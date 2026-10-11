// Offline-only builds (VITE_SYNC=local, e.g. GitHub Pages): a stand-in for the sync server that lives in the
// browser. It speaks the same protocol to SyncState (hello/op -> reset/ready/ack/reject/change) and applies ops with
// the same canonical `effect`, so the rest of the client (optimistic view, rebase, undo/redo, outbox) is unchanged.
//
// Storage: the confirmed state (rows + recent changes, which are the undo targets) in localStorage, updated under
// a Web Lock so tabs never interleave writes. Tabs hear each other's changes over a BroadcastChannel; a tab that
// falls behind (a gap) says hello again and gets a `reset`.
import { effect, MemRows, OpError, type Change, type ClientMsg, type Row, type ServerMsg, type SyncState } from "@bible-tags/scripture";

const DB_KEY = "bible-tags/local-db/v1";
const KEEP_CHANGES = 1000;

export interface LocalDb {
  seq: number;
  rows: Row[];
  changes: Change[];
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

function withLock(f: () => void) {
  if (navigator.locks) void navigator.locks.request("bible-tags-local-db", async () => f());
  else f();
}

/** Connect `sync` to the browser-local store. Returns a function reporting storage failures to the UI. */
export function connectLocal(sync: SyncState, onError: (msg: string) => void) {
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
    withLock(() => {
      const db = loadLocalDb();
      const done = db.changes.find((c) => c.op_id === op.op_id);
      if (done) {
        reply({ type: "ack", op_id: op.op_id, seq: done.seq });
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
      const next: LocalDb = {
        seq: change.seq,
        rows: rows.allRows(),
        changes: [...db.changes, change].slice(-KEEP_CHANGES),
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
