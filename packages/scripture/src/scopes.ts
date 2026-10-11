/**
 * Search scopes: named book groups ("Gospels", "Pauline epistles", ...) and the scope filter for tag search.
 * A scope is a list of ordinal intervals; instances overlapping any of them are in scope.
 */
import { bookByCode, normalizeBookName } from "./books.ts";
import type { Interval, TextIndex } from "./textIndex.ts";

export interface BookGroup {
  name: string;
  /** First and last book code (inclusive, canonical order). */
  from: string;
  to: string;
  aliases: string[];
}

export const BOOK_GROUPS: readonly BookGroup[] = [
  { name: "Old Testament", from: "GEN", to: "MAL", aliases: ["ot", "oldtestament"] },
  { name: "New Testament", from: "MAT", to: "REV", aliases: ["nt", "newtestament"] },
  { name: "Law", from: "GEN", to: "DEU", aliases: ["torah", "pentateuch", "books of moses"] },
  { name: "History", from: "JOS", to: "EST", aliases: ["historical books"] },
  { name: "Poetry", from: "JOB", to: "SNG", aliases: ["wisdom", "wisdom literature", "poetic books"] },
  { name: "Prophets", from: "ISA", to: "MAL", aliases: [] },
  { name: "Major Prophets", from: "ISA", to: "DAN", aliases: [] },
  { name: "Minor Prophets", from: "HOS", to: "MAL", aliases: ["the twelve"] },
  { name: "Gospels", from: "MAT", to: "JHN", aliases: ["evangelists"] },
  { name: "Pauline epistles", from: "ROM", to: "PHM", aliases: ["paul", "pauline", "letters of paul"] },
  { name: "General epistles", from: "HEB", to: "JUD", aliases: ["catholic epistles", "general letters"] },
];

const key = (s: string) => normalizeBookName(s);

/** Book groups whose name or alias starts with `query` (normalized like book names). */
export function matchGroups(query: string): BookGroup[] {
  const q = key(query);
  if (q.length < 2) return [];
  return BOOK_GROUPS.filter((g) => [g.name, ...g.aliases].some((a) => key(a).startsWith(q)));
}

export function groupInterval(index: TextIndex, g: BookGroup): Interval {
  return [index.bookInterval(bookByCode(g.from))[0], index.bookInterval(bookByCode(g.to))[1]];
}

/** Sort and merge overlapping/abutting intervals. */
export function mergeIntervals(ivs: readonly Interval[]): Interval[] {
  const sorted = [...ivs].sort((a, b) => a[0] - b[0]);
  const out: [number, number][] = [];
  for (const [s, e] of sorted) {
    const last = out[out.length - 1];
    if (last && s <= last[1]) last[1] = Math.max(last[1], e);
    else out.push([s, e]);
  }
  return out;
}

/** Instances (sorted by start, disjoint) that overlap any scope interval. `scope` empty = everything. */
export function inScope(instances: readonly Interval[], scope: readonly Interval[]): Interval[] {
  if (scope.length === 0) return [...instances];
  const sc = mergeIntervals(scope);
  const out: Interval[] = [];
  let j = 0;
  for (const iv of instances) {
    while (j < sc.length && sc[j]![1] <= iv[0]) j++; // scope ranges ending before this instance
    if (j < sc.length && sc[j]![0] < iv[1]) out.push(iv);
  }
  return out;
}
