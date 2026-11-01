// What an undo step did, in words ("tagging John 3:16 with “Love”"), for the bubble shown after an undo or redo.
// Read from the step's changes (row before/after pairs), so it needs no record of the intents: an undo step's
// changes are the action itself; a redo step's are reverts of it, read backwards (`inverse`).
import type { Change, CommentRow, Row, TagRow, TaggingRow } from "./ops.ts";

export interface DescribeContext {
  /** A tag's name (any state: it may be deleted now). */
  tagName: (id: string) => string | undefined;
  /** A tagging (any state), for where a comment is. */
  tagging: (id: string) => TaggingRow | undefined;
  /** Where an interval of a text is ("John 3:16"), or undefined if not shown (another text than the one read). */
  place: (textId: string, iv: [number, number]) => string | undefined;
}

type Pair<R> = { before: R | null; after: R | null };

const live = (r: { deleted_at: number | null } | null): boolean => !!r && r.deleted_at === null;
const quoted = (s: string | undefined) => (s === undefined ? "a tag" : `“${s}”`);

/** [start, end) intervals, sorted and merged. */
function union(ivs: [number, number][]): [number, number][] {
  const out: [number, number][] = [];
  for (const [s, e] of [...ivs].sort((a, b) => a[0] - b[0])) {
    const last = out[out.length - 1];
    if (last && s <= last[1]) last[1] = Math.max(last[1], e);
    else out.push([s, e]);
  }
  return out;
}

/** The parts of `a` not covered by `b` (both from `union`). */
function minus(a: [number, number][], b: [number, number][]): [number, number][] {
  const out: [number, number][] = [];
  for (const [s0, e] of a) {
    let s = s0;
    for (const [bs, be] of b) {
      if (be <= s || bs >= e) continue;
      if (bs > s) out.push([s, bs]);
      s = Math.max(s, be);
      if (s >= e) break;
    }
    if (s < e) out.push([s, e]);
  }
  return out;
}

/** The span from the first interval's start to the last one's end, or undefined if there are none. */
function span(ivs: [number, number][]): [number, number] | undefined {
  return ivs.length ? [ivs[0]![0], ivs[ivs.length - 1]![1]] : undefined;
}

/** What `changes` did (as the action, or undone if `inverse`), as a phrase: "creating the tag “Love”". */
export function describeChanges(changes: Change[], ctx: DescribeContext, inverse = false): string {
  const pairs = changes.flatMap((c) => c.rows).map((r): Pair<Row> => (inverse ? { before: r.after, after: r.before } : r));
  const of = <T extends Row["table"]>(t: T) => pairs.filter((p) => (p.after ?? p.before)!.table === t) as Pair<Extract<Row, { table: T }>>[];

  const comments = of("comment");
  if (comments.length) {
    const p = comments[0]!;
    const verb = !live(p.before) && live(p.after) ? "adding" : live(p.before) && !live(p.after) ? "deleting" : "editing";
    const row = (p.after ?? p.before) as CommentRow;
    const t = ctx.tagging(row.tagging);
    const where = t && ctx.place(t.text_id, [t.start, t.end]);
    return `${verb} a comment${where ? ` on ${where}` : ""}`;
  }

  const tags = of("tag");
  if (tags.length) {
    const p = tags[0]!;
    const [b, a] = [p.before, p.after] as [TagRow | null, TagRow | null];
    // (After an undo and a redo, a creation's change starts from the tombstone the undo left.)
    if (!live(b) && live(a)) return `creating the tag ${quoted(a!.name)}`;
    if (live(b) && !live(a)) return `deleting the tag ${quoted(b!.name)}`;
    if (b && a && b.name !== a.name) return `renaming ${quoted(b.name)} to ${quoted(a.name)}`;
    if (b && a && b.color !== a.color) return `changing the color of ${quoted(a.name)}`;
  }

  const taggings = of("tagging");
  if (taggings.length) {
    const rows = taggings.map((p) => (p.after ?? p.before) as TaggingRow);
    const cover = (side: "before" | "after") =>
      union(taggings.filter((p) => live(p[side])).map((p) => [p[side]!.start, p[side]!.end] as [number, number]));
    const [b, a] = [cover("before"), cover("after")];
    const added = span(minus(a, b));
    const removed = span(minus(b, a));
    const textId = rows[0]!.text_id;
    const names = [...new Set(rows.map((r) => r.tag))];
    if (added && !removed) {
      const where = ctx.place(textId, added);
      return `tagging ${where ?? "words"} with ${quoted(ctx.tagName(names[0]!))}`;
    }
    if (removed) {
      const where = ctx.place(textId, removed);
      const what = names.length === 1 ? quoted(ctx.tagName(names[0]!)) : `${names.length} tags`;
      return `erasing ${what}${where ? ` from ${where}` : ""}`;
    }
  }

  if (of("palette").length) return "changing the palette";
  return "the last change";
}
