/**
 * Tag queries (see "Tag queries" in CLAUDE.md). Instances of a tag set S = maximal runs of words covered by every
 * tag in S (superset semantics: other tags may also be present). Inputs are each tag's taggings sorted by start;
 * same-tag taggings are disjoint and non-adjacent, which makes every intersection run maximal by construction.
 */
import type { Interval } from "./textIndex.ts";

/** Runs covered by every list. Each list: sorted, disjoint, non-adjacent intervals. Linear merge, k pointers. */
export function intersectRuns(lists: readonly (readonly Interval[])[]): Interval[] {
  if (lists.length === 0) return [];
  if (lists.length === 1) return [...lists[0]!];
  const idx = lists.map(() => 0);
  const out: Interval[] = [];
  for (;;) {
    let start = -Infinity;
    let end = Infinity;
    for (let k = 0; k < lists.length; k++) {
      const iv = lists[k]![idx[k]!];
      if (!iv) return out;
      start = Math.max(start, iv[0]);
      end = Math.min(end, iv[1]);
    }
    if (start < end) out.push([start, end]);
    // Advance every list whose current interval ends first (it can't intersect anything later).
    for (let k = 0; k < lists.length; k++) if (lists[k]![idx[k]!]![1] === end) idx[k]!++;
  }
}

export interface Nearby {
  /** The window of instances, in text order. */
  items: Interval[];
  /** Index (in `items`) of the instance containing `pos`, or -1 if none does. */
  current: number;
  /** Index of items[0] in the full list (for "3 of 12"). */
  offset: number;
  total: number;
}

/** Up to `before` instances before `pos` and `after` after it (plus the one containing it, if any). */
export function nearby(instances: readonly Interval[], pos: number, before: number, after: number): Nearby {
  // First instance that ends after pos (instances are disjoint, so end order == start order).
  let lo = 0;
  let hi = instances.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (instances[mid]![1] <= pos) lo = mid + 1;
    else hi = mid;
  }
  const containing = lo < instances.length && instances[lo]![0] <= pos;
  const from = Math.max(0, lo - before);
  const to = Math.min(instances.length, lo + (containing ? 1 : 0) + after);
  return {
    items: instances.slice(from, to),
    current: containing ? lo - from : -1,
    offset: from,
    total: instances.length,
  };
}
