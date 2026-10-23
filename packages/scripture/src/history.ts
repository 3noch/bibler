// Reading history: an automatic bookmark per reading track. An entry is where a jump landed (`anchor`) and where
// scrolling from there got to (`last`), each with its time; an entry's time is the later of the two, and orders the
// list. Kept per member and space, outside the change log (crates/space/src/history.rs stores it on the server).
//
// Merging is conflict-free: each part keeps the later position (by time, then ordinal) and `gone` stays set, so
// devices agree whatever order entries arrive in (spec shared with Rust: fixtures/history.json). What to drop
// (`prune`: near-duplicates, then the oldest beyond HISTORY_SIZE, never-read ones first) needs the text index, so
// only clients decide it; a dropped entry is marked `gone`, which then reaches every device.
import type { TextIndex } from "./textIndex.ts";

export type Pos = { ord: number; at: number };
export type HistoryEntry = { id: string; anchor: Pos; last: Pos; gone: boolean };

/** Live entries kept. */
export const HISTORY_SIZE = 40;
/** Dropped entries remembered (so a device that still has one learns it went), newest first. */
const GONE_KEPT = 100;
/** Two places this many verses apart or closer are "the same place". */
export const NEAR_VERSES = 5;

const later = (a: Pos, b: Pos): Pos => (a.at > b.at || (a.at === b.at && a.ord >= b.ord) ? a : b);

/** The two versions of one entry (same id) combined. */
export function mergeEntry(a: HistoryEntry, b: HistoryEntry): HistoryEntry {
  return { id: a.id, anchor: later(a.anchor, b.anchor), last: later(a.last, b.last), gone: a.gone || b.gone };
}

export const sameEntry = (a: HistoryEntry, b: HistoryEntry) =>
  a.id === b.id && a.gone === b.gone && a.anchor.ord === b.anchor.ord && a.anchor.at === b.anchor.at && a.last.ord === b.last.ord && a.last.at === b.last.at;

export const entryTime = (e: HistoryEntry) => Math.max(e.anchor.at, e.last.at);

const newestFirst = (a: HistoryEntry, b: HistoryEntry) => entryTime(b) - entryTime(a) || (a.id < b.id ? 1 : a.id > b.id ? -1 : 0);

/** Both lists' entries, the same id merged. */
export function mergeHistory(mine: readonly HistoryEntry[], theirs: readonly HistoryEntry[]): HistoryEntry[] {
  const byId = new Map(mine.map((e) => [e.id, e]));
  for (const e of theirs) {
    const m = byId.get(e.id);
    byId.set(e.id, m ? mergeEntry(m, e) : e);
  }
  return [...byId.values()];
}

/** The entries to show, newest first. */
export const liveHistory = (entries: readonly HistoryEntry[]) => entries.filter((e) => !e.gone).sort(newestFirst);

/** Within NEAR_VERSES of each other (ordinals of any word; unknown ordinals are never near). */
export function near(index: TextIndex, a: number, b: number): boolean {
  if (index.bookOf(a) < 0 || index.bookOf(b) < 0) return false;
  return Math.abs(index.slotOf(a) - index.slotOf(b)) <= NEAR_VERSES;
}

/** Never scrolled away from where it landed. */
const unread = (index: TextIndex, e: HistoryEntry) => near(index, e.anchor.ord, e.last.ord);

/** Drop near-duplicates (an older entry whose anchor and last are both near a newer one's) and, beyond
 *  HISTORY_SIZE, the oldest, never-read ones first. Dropped entries are marked gone (kept up to GONE_KEPT). */
export function prune(index: TextIndex, entries: readonly HistoryEntry[]): HistoryEntry[] {
  const live = liveHistory(entries);
  const drop = new Set<string>();
  live.forEach((e, i) => {
    if (drop.has(e.id)) return;
    for (const older of live.slice(i + 1)) {
      if (!drop.has(older.id) && near(index, e.anchor.ord, older.anchor.ord) && near(index, e.last.ord, older.last.ord)) drop.add(older.id);
    }
  });
  let kept = live.filter((e) => !drop.has(e.id));
  if (kept.length > HISTORY_SIZE) {
    const oldestFirst = [...kept].reverse();
    const order = [...oldestFirst.filter((e) => unread(index, e)), ...oldestFirst.filter((e) => !unread(index, e))];
    for (const e of order.slice(0, kept.length - HISTORY_SIZE)) drop.add(e.id);
    kept = kept.filter((e) => !drop.has(e.id));
  }
  const gone = entries
    .map((e) => (drop.has(e.id) && !e.gone ? { ...e, gone: true } : e))
    .filter((e) => e.gone)
    .sort(newestFirst)
    .slice(0, GONE_KEPT);
  return [...kept, ...gone];
}

/** A jump to `ord`: resumes the newest entry whose `last` is near it (its anchor moves there: where this sitting
 *  began), else starts a new one with `newId`. */
export function visit(index: TextIndex, entries: readonly HistoryEntry[], ord: number, now: number, newId: string): { entries: HistoryEntry[]; id: string } {
  const here: Pos = { ord, at: now };
  const resume = liveHistory(entries).find((e) => near(index, e.last.ord, ord));
  const entry: HistoryEntry = resume ? { ...resume, anchor: here, last: here } : { id: newId, anchor: here, last: here, gone: false };
  const rest = entries.filter((e) => e.id !== entry.id);
  return { entries: prune(index, [entry, ...rest]), id: entry.id };
}

/** Scrolling in entry `id` reached `ord`. */
export function scrolled(index: TextIndex, entries: readonly HistoryEntry[], id: string, ord: number, now: number): HistoryEntry[] {
  const e = entries.find((x) => x.id === id);
  if (!e || e.gone || e.last.ord === ord) return [...entries];
  return prune(index, entries.map((x) => (x.id === id ? { ...x, last: { ord, at: now } } : x)));
}

/** Back to entry `id` where it left off (from the list, or Back): it becomes the newest, anchored there. */
export function resumeEntry(index: TextIndex, entries: readonly HistoryEntry[], id: string, now: number): HistoryEntry[] {
  return prune(
    index,
    entries.map((e) => (e.id === id && !e.gone ? { ...e, anchor: { ord: e.last.ord, at: now }, last: { ord: e.last.ord, at: now } } : e)),
  );
}
