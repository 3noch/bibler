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
import { effect, MemRows, OpError, type Change, type Op, type OpIntent, type Row } from "./ops.ts";

export type ClientMsg = { type: "hello"; after: number } | { type: "op"; op: Op };
export type ServerMsg =
  | { type: "change"; change: Change }
  | { type: "ready"; seq: number }
  | { type: "ack"; op_id: string; seq: number }
  | { type: "reject"; op_id: string; code: string; message: string }
  | { type: "error"; message: string }
  /** Replace the confirmed state wholesale (the client is behind and the log can't be replayed, e.g. the
   *  browser-local store in offline-only builds). Pending ops are rebased onto it. */
  | { type: "reset"; seq: number; rows: Row[]; changes: Change[] };

export interface Snapshot {
  owner: string;
  seq: number;
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
    send({ type: "hello", after: this.seq });
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
        return true; // the change itself confirms the op
      case "reject":
        this.drop(msg.op_id, `Not saved: ${msg.message || msg.code}`);
        return true;
      case "error":
        return false;
      case "reset": {
        this.base = new MemRows(msg.rows, msg.changes);
        this.seq = msg.seq;
        for (const c of msg.changes) this.confirm(c.op_id);
        this.rebuild();
        this.changed();
        return true;
      }
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
      rows: [...this.base.tags.values(), ...this.base.taggings.values(), ...(this.base.pal ? [this.base.pal] : [])],
      changes: recent.filter((c, i) => i < KEEP_CHANGES || needed.has(c.op_id)),
      pending: this.pending,
      undo: this.undo,
      redo: this.redo,
      confirmed: this.confirmed,
    };
  }
}
