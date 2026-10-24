/**
 * Client sync state (pure: no DOM, no Solid; the app wraps it). Protocol: crates/server/src/sync.rs.
 *
 * - `base`: rows confirmed by the server, up to `seq`. `pending`: ops sent or queued but not yet seen as changes.
 *   `view` = base + pending ops replayed with `effect` (the optimistic state the UI shows).
 * - A server change is applied to `base`; the pending op with its op_id (if any) is confirmed; the rest are
 *   replayed on the new base (rebase). A pending op that no longer applies is dropped with a notice.
 * - Undo/redo: stacks of steps (op ids in execution order). Undoing a step submits `revert` ops for it in reverse
 *   order; the reverts form the redo step, whose undo is again a revert (so redo = revert of the revert).
 * - Offline: ops queue in `pending` and are (re)sent after the next `ready`. Ops are idempotent by op_id.
 * - Several tabs/devices may hold the same op (a shared outbox): `adopt` takes ops queued elsewhere, and
 *   `confirmed` (recent op ids seen as changes) lets an already-applied op be dropped silently.
 */
import type { HistoryEntry } from "./history.ts";
import { effect, MemRows, OpError, type Change, type Op, type OpIntent, type Row } from "./ops.ts";

/** `hello.last_op`: the op of the client's change at `after`, so a server whose space went back (restored from an
 *  older backup) notices. `hello.pending`: the unconfirmed ops this client holds, so a `reset` can say which are
 *  already applied. */
export type ClientMsg = { type: "hello"; after: number; last_op?: string | null; pending?: string[]; history?: boolean } | { type: "op"; op: Op } | { type: "ping" } | { type: "history"; entries: HistoryEntry[] };
/** The commit a server was built from and its time (ISO 8601); empty when it didn't know. Sent in `ready`. */
export type Build = { commit: string; committed_at: string };
export type ServerMsg =
  | { type: "change"; change: Change }
  | { type: "ready"; seq: number; build?: Build; client?: string }
  /** The answer to a ping (handled by the socket's owner, not SyncState): the client version served now. */
  | { type: "pong"; client?: string }
  /** This member's reading history (all of it after `ready`, then other devices' changes); handled by the
   *  socket's owner, not SyncState. */
  | { type: "history"; entries: HistoryEntry[] }
  | { type: "ack"; op_id: string; seq: number }
  | { type: "reject"; op_id: string; code: string; message: string }
  | { type: "error"; message: string }
  /** Replace the confirmed state wholesale (the log can't be replayed: the server pruned it, or its space is behind
   *  this client or has a different change at `after`, or the browser-local store in offline-only builds). `applied` = which of the hello's `pending` ops are
   *  already in `rows` (dropped silently); the rest are rebased onto it. Undo steps whose changes aren't in
   *  `changes` (or pending) are dropped, since they can no longer be reverted. */
  | { type: "reset"; seq: number; last_op?: string | null; rows: Row[]; changes: Change[]; applied?: string[] };

export interface Snapshot {
  owner: string;
  seq: number;
  /** The op of the change at `seq`, if known. */
  lastOp?: string | null;
  rows: Row[];
  /** Recent changes (revert targets for undo/redo). */
  changes: Change[];
  /** Recently confirmed op ids. */
  confirmed?: string[];
  pending: Op[];
  undo: string[][];
  redo: string[][];
}

export type Status = "offline" | "syncing" | "synced";

const MAX_UNDO = 200;
const KEEP_CHANGES = 1000;
const KEEP_CONFIRMED = 2000;

export class SyncState {
  owner: string;
  seq = 0;
  /** The op of the change at `seq`, if known (see `hello.last_op`). */
  lastOp: string | null = null;
  /** A reset took this client to a lower seq (its space went back): its base may replace a cache at a higher seq. */
  wentBack = false;
  base: MemRows;
  view: MemRows;
  pending: Op[] = [];
  undo: string[][] = [];
  redo: string[][] = [];
  status: Status = "offline";
  /** Recently confirmed op ids (oldest first) and the same as a set. */
  private confirmed: string[] = [];
  /** Op ids this instance dropped (rejected or no longer applicable): never re-send them. */
  readonly dropped = new Set<string>();
  private confirmedSet = new Set<string>();
  /** The op ids the last hello listed (a reset says which of those are applied). */
  private helloPending = new Set<string>();
  /** Pending ops a reset couldn't vouch for (not in the hello): if one no longer applies, it was probably applied
   *  already, so it waits for the server's ack (or reject) instead of being dropped. */
  private unsure = new Set<string>();
  private send: ((m: ClientMsg) => void) | null = null;
  private listeners = new Set<() => void>();
  private noticeListeners = new Set<(msg: string) => void>();

  private readonly newId: () => string;
  private readonly now: () => number;

  constructor(owner: string, newId: () => string, now: () => number, snap?: Snapshot) {
    this.owner = owner;
    this.newId = newId;
    this.now = now;
    this.base = new MemRows();
    if (snap && snap.owner === owner) {
      this.seq = snap.seq;
      this.lastOp = snap.lastOp ?? null;
      this.base = new MemRows(snap.rows, snap.changes);
      this.pending = snap.pending;
      this.undo = snap.undo;
      this.redo = snap.redo;
      this.confirmed = snap.confirmed ?? [];
      this.confirmedSet = new Set(this.confirmed);
    }
    this.view = this.base;
    this.rebuild();
  }

  onChange(f: () => void) {
    this.listeners.add(f);
    return () => this.listeners.delete(f);
  }
  onNotice(f: (msg: string) => void) {
    this.noticeListeners.add(f);
    return () => this.noticeListeners.delete(f);
  }
  private changed() {
    for (const f of this.listeners) f();
  }
  private notice(msg: string) {
    for (const f of this.noticeListeners) f(msg);
  }

  // ------------------------------------------------------------------ local edits

  /**
   * Submit intents as one undo step (or as the result of undo/redo). All-or-nothing: if any is invalid against
   * the current view, nothing is submitted and the error is returned.
   */
  submit(intents: OpIntent[], as: "do" | "undo" | "redo" = "do"): OpError | null {
    const view = this.view.clone();
    const ops: Op[] = [];
    try {
      for (const intent of intents) {
        const op = { op_id: this.newId(), ...intent } as Op;
        view.applyChange(effect(view, op));
        ops.push(op);
      }
    } catch (e) {
      if (e instanceof OpError) return e;
      throw e;
    }
    if (!ops.length) return null;
    this.view = view;
    this.pending.push(...ops);
    const step = ops.map((o) => o.op_id);
    if (as === "do") {
      this.undo = [...this.undo, step].slice(-MAX_UNDO);
      this.redo = [];
    } else if (as === "undo") {
      this.redo = [...this.redo, step];
    } else {
      this.undo = [...this.undo, step];
    }
    if (this.send && this.status === "synced") for (const op of ops) this.send({ type: "op", op });
    this.changed();
    return null;
  }

  canUndo() {
    return this.undo.length > 0;
  }
  canRedo() {
    return this.redo.length > 0;
  }

  undoStep(): OpError | null {
    return this.revertStep("undo");
  }
  redoStep(): OpError | null {
    return this.revertStep("redo");
  }

  private revertStep(which: "undo" | "redo"): OpError | null {
    const stack = which === "undo" ? this.undo : this.redo;
    const step = stack[stack.length - 1];
    if (!step) return null;
    const now = this.now();
    const err = this.submit([...step].reverse().map((target) => ({ kind: "revert" as const, target, now })), which);
    if (which === "undo") this.undo = err ? this.undo : this.undo.slice(0, -1);
    else this.redo = err ? this.redo : this.redo.slice(0, -1);
    if (err) this.notice(err.code === "conflict" ? `Can't ${which}: changed since (elsewhere or by a later edit)` : `Can't ${which}: ${err.code}`);
    else this.changed();
    return err;
  }

  // ------------------------------------------------------------------ connection

  /** A socket opened: say hello. Pending ops are sent once the replay is done (`ready`). */
  connected(send: (m: ClientMsg) => void) {
    this.send = send;
    this.status = "syncing";
    this.helloPending = new Set(this.pending.map((o) => o.op_id));
    send({ type: "hello", after: this.seq, last_op: this.lastOp, pending: [...this.helloPending], history: true });
    this.changed();
  }

  disconnected() {
    this.send = null;
    this.status = "offline";
    this.changed();
  }

  /** Handle a server message. Returns false if the connection should be dropped (and reopened). */
  receive(msg: ServerMsg): boolean {
    switch (msg.type) {
      case "change": {
        const c = msg.change;
        if (c.seq <= this.seq) return true; // already have it
        if (c.seq !== this.seq + 1) return false; // gap: reconnect and replay from our seq
        this.base.applyChange(c);
        this.seq = c.seq;
        this.lastOp = c.op_id;
        this.confirm(c.op_id);
        this.pending = this.pending.filter((op) => op.op_id !== c.op_id);
        this.rebuild();
        this.changed();
        return true;
      }
      case "ready":
        this.status = "synced";
        if (this.send) for (const op of this.pending) this.send({ type: "op", op });
        this.changed();
        return true;
      case "ack":
        // Normally the change itself confirms the op. An ack at or below our seq is an op applied long ago whose
        // change we never saw (it was pruned and we got a reset): it is already in the base.
        if (msg.seq <= this.seq && this.pending.some((o) => o.op_id === msg.op_id)) {
          this.confirm(msg.op_id);
          this.pending = this.pending.filter((o) => o.op_id !== msg.op_id);
          this.rebuild();
          this.changed();
        }
        return true;
      case "reject":
        this.drop(msg.op_id, `Not saved: ${msg.message || msg.code}`);
        return true;
      case "error":
        return false;
      case "pong":
      case "history":
        return true; // the socket's owner reads them (the served client version; reading history)
      case "reset": {
        this.base = new MemRows(msg.rows, msg.changes);
        if (msg.seq < this.seq) this.wentBack = true;
        this.seq = msg.seq;
        this.lastOp = msg.last_op ?? null;
        for (const c of msg.changes) this.confirm(c.op_id);
        for (const id of msg.applied ?? []) this.confirm(id);
        for (const o of this.pending) if (!this.helloPending.has(o.op_id) && !this.confirmedSet.has(o.op_id)) this.unsure.add(o.op_id);
        // Undo can only revert changes we hold (or ops still pending, whose changes we'll see).
        const revertible = new Set([...msg.changes.map((c) => c.op_id), ...this.pending.filter((o) => !this.confirmedSet.has(o.op_id)).map((o) => o.op_id)]);
        const keep = (steps: string[][]) => steps.filter((s) => s.every((id) => revertible.has(id)));
        this.undo = keep(this.undo);
        this.redo = keep(this.redo);
        this.rebuild();
        this.changed();
        return true;
      }
      default:
        // A newer server's message: ignore it. (Dropping the socket would reconnect into the same message forever,
        // as clients from before reading history did on `history`.)
        return true;
    }
  }

  /** Take ops queued elsewhere (another tab's outbox); ones already pending or confirmed are ignored. */
  adopt(ops: Op[]) {
    const known = new Set(this.pending.map((o) => o.op_id));
    const fresh = ops.filter((o) => !known.has(o.op_id) && !this.confirmedSet.has(o.op_id));
    if (!fresh.length) return;
    this.pending.push(...fresh);
    this.rebuild();
    if (this.send && this.status === "synced") for (const op of fresh) if (this.pending.includes(op)) this.send({ type: "op", op });
    this.changed();
  }

  isConfirmed(opId: string) {
    return this.confirmedSet.has(opId);
  }

  // ------------------------------------------------------------------ internals

  private confirm(opId: string) {
    this.unsure.delete(opId);
    if (this.confirmedSet.has(opId)) return;
    this.confirmed.push(opId);
    this.confirmedSet.add(opId);
    if (this.confirmed.length > KEEP_CONFIRMED) for (const id of this.confirmed.splice(0, this.confirmed.length - KEEP_CONFIRMED)) this.confirmedSet.delete(id);
  }

  /** view = base + pending, dropping pending ops that no longer apply. */
  private rebuild() {
    const view = this.base.clone();
    const kept: Op[] = [];
    for (const op of this.pending) {
      if (this.confirmedSet.has(op.op_id)) continue; // already in base (confirmed via another tab or a replay)
      try {
        view.applyChange(effect(view, op));
        kept.push(op);
      } catch (e) {
        if (!(e instanceof OpError)) throw e;
        if (this.unsure.has(op.op_id)) {
          kept.push(op); // sent after `ready`; out of the view meanwhile
          continue;
        }
        this.dropped.add(op.op_id);
        this.forget(op.op_id);
        this.notice(`An edit could not be applied after syncing (${e.code}) and was dropped`);
      }
    }
    this.pending = kept;
    this.view = view;
  }

  private drop(opId: string, msg: string) {
    if (!this.pending.some((op) => op.op_id === opId)) return;
    this.unsure.delete(opId);
    this.pending = this.pending.filter((op) => op.op_id !== opId);
    this.dropped.add(opId);
    this.forget(opId);
    this.rebuild();
    this.notice(msg);
    this.changed();
  }

  /** Remove an op that never happened from the undo/redo stacks. */
  private forget(opId: string) {
    const strip = (steps: string[][]) => steps.map((s) => s.filter((id) => id !== opId)).filter((s) => s.length > 0);
    this.undo = strip(this.undo);
    this.redo = strip(this.redo);
  }

  snapshot(): Snapshot {
    const needed = new Set([...this.undo.flat(), ...this.redo.flat()]);
    for (const op of this.pending) if (op.kind === "revert") needed.add(op.target);
    const recent = [...this.base.changes.values()].sort((a, b) => b.seq - a.seq);
    return {
      owner: this.owner,
      seq: this.seq,
      lastOp: this.lastOp,
      rows: this.base.allRows(),
      changes: recent.filter((c, i) => i < KEEP_CHANGES || needed.has(c.op_id)),
      pending: this.pending,
      undo: this.undo,
      redo: this.redo,
      confirmed: this.confirmed,
    };
  }
}
