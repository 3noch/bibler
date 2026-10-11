/**
 * ops: what a client op does to a user's rows. TS twin of `crates/ops` (the server's canonical version); both are
 * verified against fixtures/ops.json. The client runs this optimistically; the server's result is authoritative.
 * See the Rust crate docs for the model (row {before, after} changes, soft deletes, revert = undo/redo).
 */
import { normalize, NormalizeError, type Intent as NormalizeIntent, type Tagging } from "./normalize.ts";

export const MAX_PALETTE = 9;
export const MAX_TAG_NAME = 100;
export const PALETTE_ID = "palette";

export interface TagRow {
  table: "tag";
  id: string;
  name: string;
  color: string;
  created_at: number;
  updated_at: number;
  deleted_at: number | null;
}

export interface TaggingRow {
  table: "tagging";
  id: string;
  tag: string;
  text_id: string;
  start: number;
  end: number;
  created_at: number;
  deleted_at: number | null;
  merged_into: string | null;
}

export interface PaletteRow {
  table: "palette";
  tags: string[];
  updated_at: number;
}

export type Row = TagRow | TaggingRow | PaletteRow;

export interface RowChange {
  /** null = the row did not exist. */
  before: Row | null;
  after: Row;
}

export interface Change {
  /** Assigned by the server; 0 while optimistic. */
  seq: number;
  op_id: string;
  kind: OpIntent["kind"];
  /** Sorted by (table: tag < tagging < palette, id). */
  rows: RowChange[];
}

export type OpIntent =
  | { kind: "paint"; tag: string; text_id: string; start: number; end: number; new_id: string; now: number }
  | { kind: "erase"; tag: string; text_id: string; start: number; end: number; split_id?: string | null; now: number }
  | { kind: "tag_create"; id: string; name: string; color: string; now: number }
  | { kind: "tag_update"; id: string; name?: string | null; color?: string | null; now: number }
  | { kind: "tag_delete"; id: string; now: number }
  | { kind: "palette_set"; tags: string[]; now: number }
  | { kind: "revert"; target: string; now: number };

export type Op = { op_id: string } & OpIntent;

export type OpErrorCode =
  | "empty_interval"
  | "missing_split_id"
  | "unknown_tag"
  | "tag_deleted"
  | "id_exists"
  | "bad_name"
  | "name_taken"
  | "bad_color"
  | "bad_palette"
  | "unknown_op"
  | "conflict";

export class OpError extends Error {
  readonly code: OpErrorCode;
  constructor(code: OpErrorCode) {
    super(code);
    this.name = "OpError";
    this.code = code;
  }
}

/** One owner's rows. Id lookups include deleted rows; list queries return live rows only. */
export interface RowSource {
  tag(id: string): TagRow | undefined;
  tagging(id: string): TaggingRow | undefined;
  palette(): PaletteRow | undefined;
  /** Live tag whose name equals `name` case-insensitively. */
  liveTagNamed(name: string): TagRow | undefined;
  /** Live taggings of `tag` in `textId` overlapping or abutting [start, end]. */
  taggingsNear(tag: string, textId: string, start: number, end: number): TaggingRow[];
  liveTaggingsOfTag(tag: string): TaggingRow[];
  change(opId: string): Change | undefined;
}

const TABLE_ORDER = { tag: 0, tagging: 1, palette: 2 } as const;
export const rowKey = (r: Row): string => (r.table === "palette" ? "palette:" : `${r.table}:${r.id}`);
const cmpKey = (a: Row, b: Row) => {
  const t = TABLE_ORDER[a.table] - TABLE_ORDER[b.table];
  if (t) return t;
  const [x, y] = [a.table === "palette" ? PALETTE_ID : a.id, b.table === "palette" ? PALETTE_ID : b.id];
  return x < y ? -1 : x > y ? 1 : 0;
};

/** Deep equality for rows (plain JSON data). */
export function rowEquals(a: Row | null | undefined, b: Row | null | undefined): boolean {
  return JSON.stringify(a ?? null, Object.keys(a ?? {}).sort()) === JSON.stringify(b ?? null, Object.keys(b ?? {}).sort());
}

export function tombstone(r: Row, now: number): Row {
  switch (r.table) {
    case "tag":
      return { ...r, deleted_at: now, updated_at: now };
    case "tagging":
      return { ...r, deleted_at: now };
    case "palette":
      return { table: "palette", tags: [], updated_at: now };
  }
}

function current(src: RowSource, r: Row): Row | undefined {
  switch (r.table) {
    case "tag":
      return src.tag(r.id);
    case "tagging":
      return src.tagging(r.id);
    case "palette":
      return src.palette();
  }
}

function liveTag(src: RowSource, id: string): TagRow {
  const tag = src.tag(id);
  if (!tag) throw new OpError("unknown_tag");
  if (tag.deleted_at !== null) throw new OpError("tag_deleted");
  return tag;
}

/** Same characters as Rust's `str::trim` (Unicode White_Space). */
const trim = (s: string) => s.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");

function checkName(name: string): string {
  const t = trim(name);
  // Count code points, like Rust's chars().count() (not UTF-16 units).
  if (t.length === 0 || [...t].length > MAX_TAG_NAME) throw new OpError("bad_name");
  return t;
}

function checkColor(color: string) {
  if (!/^#[0-9a-fA-F]{6}$/.test(color)) throw new OpError("bad_color");
}

const toTagging = (t: TaggingRow): Tagging => ({ id: t.id, tag: t.tag, start: t.start, end: t.end, created_at: t.created_at });

/** Compute the change an op makes (seq 0). Throws OpError with a stable code when the op is invalid. */
export function effect(src: RowSource, op: Op): Change {
  let rows: RowChange[];
  switch (op.kind) {
    case "paint": {
      liveTag(src, op.tag);
      if (src.tagging(op.new_id)) throw new OpError("id_exists");
      rows = taggingRows(src, op.tag, op.text_id, op.start, op.end, { ...pick(op), kind: "paint", new_id: op.new_id, now: op.now }, op.now);
      break;
    }
    case "erase": {
      liveTag(src, op.tag);
      if (op.split_id && src.tagging(op.split_id)) throw new OpError("id_exists");
      const intent: NormalizeIntent = { ...pick(op), kind: "erase", ...(op.split_id ? { split_id: op.split_id } : {}) };
      rows = taggingRows(src, op.tag, op.text_id, op.start, op.end, intent, op.now);
      break;
    }
    case "tag_create": {
      const name = checkName(op.name);
      checkColor(op.color);
      if (src.tag(op.id)) throw new OpError("id_exists");
      if (src.liveTagNamed(name)) throw new OpError("name_taken");
      rows = [
        {
          before: null,
          after: { table: "tag", id: op.id, name, color: op.color, created_at: op.now, updated_at: op.now, deleted_at: null },
        },
      ];
      break;
    }
    case "tag_update": {
      const before = liveTag(src, op.id);
      const after: TagRow = { ...before };
      if (op.name != null) {
        const name = checkName(op.name);
        const other = src.liveTagNamed(name);
        if (other && other.id !== op.id) throw new OpError("name_taken");
        after.name = name;
      }
      if (op.color != null) {
        checkColor(op.color);
        after.color = op.color;
      }
      if (rowEquals(after, before)) rows = [];
      else rows = [{ before, after: { ...after, updated_at: op.now } }];
      break;
    }
    case "tag_delete":
      rows = tagDeleteRows(src, op.id, op.now);
      break;
    case "palette_set":
      rows = paletteRows(src, op.tags, op.now);
      break;
    case "revert":
      rows = revertRows(src, op.target, op.now);
      break;
  }
  return { seq: 0, op_id: op.op_id, kind: op.kind, rows: rows.sort((a, b) => cmpKey(a.after, b.after)) };
}

const pick = (op: { tag: string; start: number; end: number }) => ({ tag: op.tag, start: op.start, end: op.end });

function taggingRows(
  src: RowSource,
  tag: string,
  textId: string,
  start: number,
  end: number,
  intent: NormalizeIntent,
  now: number,
): RowChange[] {
  const near = src.taggingsNear(tag, textId, start, end);
  let cs;
  try {
    cs = normalize(near.map(toTagging), intent);
  } catch (e) {
    if (e instanceof NormalizeError) throw new OpError(e.code);
    throw e;
  }
  const byId = new Map(near.map((t) => [t.id, t]));
  const rows: RowChange[] = [];
  for (const c of cs.create) {
    rows.push({
      before: null,
      after: { table: "tagging", id: c.id, tag: c.tag, text_id: textId, start: c.start, end: c.end, created_at: c.created_at, deleted_at: null, merged_into: null },
    });
  }
  for (const u of cs.update) {
    const before = byId.get(u.id)!;
    rows.push({ before, after: { ...before, start: u.start, end: u.end } });
  }
  for (const d of cs.delete) {
    const before = byId.get(d.id)!;
    rows.push({ before, after: { ...before, deleted_at: now, merged_into: d.merged_into } });
  }
  return rows;
}

function tagDeleteRows(src: RowSource, id: string, now: number): RowChange[] {
  const tag = liveTag(src, id);
  const rows: RowChange[] = [{ before: tag, after: { ...tag, deleted_at: now, updated_at: now } }];
  for (const t of src.liveTaggingsOfTag(id)) rows.push({ before: t, after: { ...t, deleted_at: now } });
  const p = src.palette();
  if (p && p.tags.includes(id)) rows.push({ before: p, after: { table: "palette", tags: p.tags.filter((t) => t !== id), updated_at: now } });
  return rows;
}

function paletteRows(src: RowSource, tags: string[], now: number): RowChange[] {
  if (tags.length > MAX_PALETTE || new Set(tags).size !== tags.length) throw new OpError("bad_palette");
  for (const t of tags) liveTag(src, t);
  const before = src.palette();
  if (before && before.tags.length === tags.length && before.tags.every((t, i) => t === tags[i])) return [];
  return [{ before: before ?? null, after: { table: "palette", tags: [...tags], updated_at: now } }];
}

function revertRows(src: RowSource, target: string, now: number): RowChange[] {
  const change = src.change(target);
  if (!change) throw new OpError("unknown_op");
  const rows: RowChange[] = [];
  for (const rc of change.rows) {
    const cur = current(src, rc.after);
    if (!cur || !rowEquals(cur, rc.after)) throw new OpError("conflict"); // changed since: would clobber
    rows.push({ before: cur, after: rc.before ?? tombstone(rc.after, now) });
  }
  checkRevertInvariants(src, change, rows);
  return rows;
}

/** Same rules as the Rust `check_revert_invariants`. */
function checkRevertInvariants(src: RowSource, target: Change, rows: RowChange[]) {
  const touched = new Set(target.rows.map((rc) => rowKey(rc.after)));
  const resultTag = (id: string): TagRow | undefined => {
    for (const rc of rows) if (rc.after.table === "tag" && rc.after.id === id) return rc.after;
    return src.tag(id);
  };
  const liveInResult = (t: TaggingRow): boolean => {
    for (const rc of rows) if (rc.after.table === "tagging" && rc.after.id === t.id) return rc.after.deleted_at === null;
    return t.deleted_at === null;
  };
  for (const rc of rows) {
    const t = rc.after;
    if (t.table !== "tagging" || t.deleted_at !== null) continue;
    const tag = resultTag(t.tag);
    if (!tag || tag.deleted_at !== null) throw new OpError("conflict");
    // Restored taggings cannot clash with each other (valid together before, unchanged since); neighbors can.
    const clash = src.taggingsNear(t.tag, t.text_id, t.start, t.end).some((n) => !touched.has(rowKey(n)) && liveInResult(n));
    if (clash) throw new OpError("conflict");
  }
  const resultPalette = (rows.find((rc) => rc.after.table === "palette")?.after as PaletteRow | undefined) ?? src.palette();
  for (const rc of rows) {
    const r = rc.after;
    if (r.table === "tag" && r.deleted_at !== null) {
      const orphaned = src.liveTaggingsOfTag(r.id).some(liveInResult);
      if (orphaned || resultPalette?.tags.includes(r.id)) throw new OpError("conflict");
    } else if (r.table === "tag") {
      const other = src.liveTagNamed(r.name);
      if (other && other.id !== r.id && resultTag(other.id)?.deleted_at === null) throw new OpError("name_taken");
    } else if (r.table === "palette") {
      if (r.tags.some((id) => resultTag(id)?.deleted_at !== null)) throw new OpError("conflict");
    }
  }
}

// ---------------------------------------------------------------- in-memory rows (client store, tests)

/** Rows of one owner in memory: a RowSource plus `applyChange`. */
export class MemRows implements RowSource {
  tags = new Map<string, TagRow>();
  taggings = new Map<string, TaggingRow>();
  pal: PaletteRow | undefined;
  changes = new Map<string, Change>();

  constructor(rows: Row[] = [], changes: Change[] = []) {
    for (const r of rows) this.put(r);
    for (const c of changes) this.changes.set(c.op_id, c);
  }

  clone(): MemRows {
    const m = new MemRows();
    m.tags = new Map(this.tags);
    m.taggings = new Map(this.taggings);
    m.pal = this.pal;
    m.changes = new Map(this.changes);
    return m;
  }

  put(r: Row) {
    if (r.table === "tag") this.tags.set(r.id, r);
    else if (r.table === "tagging") this.taggings.set(r.id, r);
    else this.pal = r;
  }

  /** Upsert a change's `after` rows and remember it (as a revert target). */
  applyChange(c: Change) {
    for (const rc of c.rows) this.put(rc.after);
    this.changes.set(c.op_id, c);
  }

  tag(id: string) {
    return this.tags.get(id);
  }
  tagging(id: string) {
    return this.taggings.get(id);
  }
  palette() {
    return this.pal;
  }
  liveTagNamed(name: string) {
    const n = name.toLowerCase();
    for (const t of this.tags.values()) if (t.deleted_at === null && t.name.toLowerCase() === n) return t;
    return undefined;
  }
  taggingsNear(tag: string, textId: string, start: number, end: number) {
    return this.liveTaggingsOfTag(tag).filter((t) => t.text_id === textId && t.start <= end && t.end >= start);
  }
  liveTaggingsOfTag(tag: string) {
    return [...this.taggings.values()].filter((t) => t.tag === tag && t.deleted_at === null);
  }
  change(opId: string) {
    return this.changes.get(opId);
  }
}
