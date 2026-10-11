/**
 * normalize: the pure core of tag painting/erasing.
 *
 * Contract (mirrored by crates/normalize, verified by fixtures/normalize.json):
 *  - Intervals are half-open [start, end) over word ordinals.
 *  - Per tag, taggings are disjoint and NON-adjacent (a.end < b.start).
 *  - ids are ASCII (ULIDs), so string comparison is identical in TS and Rust.
 *  - Callers may pass all of a tag's taggings or just the ones that overlap/abut the
 *    interval; extra, irrelevant taggings are ignored.
 *  - Deterministic: output depends only on the arguments (no clocks, no RNG).
 *    Any id a result might need (`new_id`, `split_id`) is supplied by the client.
 */

export type Id = string;

export interface Tagging {
  id: Id;
  tag: Id;
  start: number;
  end: number;
  created_at: number; // ms since epoch
}

export type Intent =
  | { kind: "paint"; tag: Id; start: number; end: number; new_id: Id; now: number }
  | { kind: "erase"; tag: Id; start: number; end: number; split_id?: Id };

export interface Changeset {
  create: Tagging[];
  update: { id: Id; start: number; end: number }[];
  /** merged_into = survivor id for merges; null for plain deletion (erase). */
  delete: { id: Id; merged_into: Id | null }[];
  /** Re-point all comments from one tagging to another (merge only). */
  move_comments: { from: Id; to: Id }[];
}

export type NormalizeErrorCode = "empty_interval" | "missing_split_id";

export class NormalizeError extends Error {
  readonly code: NormalizeErrorCode;
  constructor(code: NormalizeErrorCode) {
    super(code);
    this.name = "NormalizeError";
    this.code = code;
  }
}

const emptyChangeset = (): Changeset => ({ create: [], update: [], delete: [], move_comments: [] });

const cmpStr = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

function finish(cs: Changeset): Changeset {
  cs.create.sort((a, b) => cmpStr(a.id, b.id));
  cs.update.sort((a, b) => cmpStr(a.id, b.id));
  cs.delete.sort((a, b) => cmpStr(a.id, b.id));
  cs.move_comments.sort((a, b) => cmpStr(a.from, b.from));
  return cs;
}

function checkInterval(start: number, end: number): void {
  if (!(Number.isInteger(start) && Number.isInteger(end)) || start < 0 || start >= end) {
    throw new NormalizeError("empty_interval");
  }
}

export function normalize(existing: readonly Tagging[], intent: Intent): Changeset {
  checkInterval(intent.start, intent.end);
  return intent.kind === "paint" ? paint(existing, intent) : erase(existing, intent);
}

function paint(existing: readonly Tagging[], op: Extract<Intent, { kind: "paint" }>): Changeset {
  const cs = emptyChangeset();
  // Overlapping OR abutting (half-open: t.end === op.start / t.start === op.end).
  const rel = existing.filter((t) => t.tag === op.tag && t.start <= op.end && t.end >= op.start);

  rel.sort((a, b) => a.created_at - b.created_at || cmpStr(a.id, b.id));
  const [survivor, ...losers] = rel;

  if (survivor === undefined) {
    cs.create.push({ id: op.new_id, tag: op.tag, start: op.start, end: op.end, created_at: op.now });
    return finish(cs);
  }

  let start = op.start;
  let end = op.end;
  for (const t of rel) {
    start = Math.min(start, t.start);
    end = Math.max(end, t.end);
  }

  if (start !== survivor.start || end !== survivor.end) {
    cs.update.push({ id: survivor.id, start, end });
  }
  for (const t of losers) {
    cs.delete.push({ id: t.id, merged_into: survivor.id });
    cs.move_comments.push({ from: t.id, to: survivor.id });
  }
  return finish(cs);
}

function erase(existing: readonly Tagging[], op: Extract<Intent, { kind: "erase" }>): Changeset {
  const cs = emptyChangeset();
  // Strict overlap only: merely abutting taggings are untouched.
  const rel = existing.filter((t) => t.tag === op.tag && t.start < op.end && t.end > op.start);

  for (const t of rel) {
    const hasLeft = t.start < op.start;
    const hasRight = t.end > op.end;

    if (hasLeft && hasRight) {
      // Left piece keeps id (and comments). Right piece is new, with the original age.
      if (op.split_id === undefined) throw new NormalizeError("missing_split_id");
      cs.update.push({ id: t.id, start: t.start, end: op.start });
      cs.create.push({ id: op.split_id, tag: t.tag, start: op.end, end: t.end, created_at: t.created_at });
    } else if (hasLeft) {
      cs.update.push({ id: t.id, start: t.start, end: op.start });
    } else if (hasRight) {
      cs.update.push({ id: t.id, start: op.end, end: t.end });
    } else {
      cs.delete.push({ id: t.id, merged_into: null });
    }
  }
  return finish(cs);
}

/** Apply a changeset to a list of taggings (client store / tests). Returns a new array. */
export function applyChangeset(state: readonly Tagging[], cs: Changeset): Tagging[] {
  const deleted = new Set(cs.delete.map((d) => d.id));
  const updates = new Map(cs.update.map((u) => [u.id, u]));
  const out: Tagging[] = [];
  for (const t of state) {
    if (deleted.has(t.id)) continue;
    const u = updates.get(t.id);
    out.push(u ? { ...t, start: u.start, end: u.end } : t);
  }
  out.push(...cs.create);
  return out;
}
