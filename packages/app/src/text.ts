// Loads text-build artifacts (manifest, index, per-book JSON) from /text/<text_id>/. Files other than the manifest
// are content-hashed, so the browser cache and the service worker (src/sw.js) keep them forever.
import { TextIndex, type Interval } from "@bible-tags/scripture";
import { TEXT_ID } from "./textId.ts";

export { TEXT_ID };
// Vite's base: "/" for server builds (absolute, so the SPA fallback at deep paths still finds the text) and "./"
// for the static GitHub Pages build (works under any subpath, e.g. /<repo>/).
const base = `${import.meta.env.BASE_URL}text/${TEXT_ID}/`;

export interface ManifestBook {
  code: string;
  id: number;
  first_ord: number;
  name_long: string;
  name_short: string;
  titles: string[];
  chapters: number;
  tokens: number;
  file: string;
}

interface Manifest {
  text_id: string;
  index: string;
  books: ManifestBook[];
}

interface BookJson {
  book: string;
  first_ord: number;
  first_slot: number;
  words: string[];
  italic: [number, number][];
  divine_name: [number, number][];
  words_of_jesus: [number, number][];
  /** Words followed by no space (the first part of a hyphenated word: `Beth-` of `Beth-el`). */
  joined: number[];
  pilcrow: number[];
  breaks: [number, string][];
  headings: { slot: number; pos: "before" | "after"; text: string }[];
  notes: { after: number; ref: string; text: string }[];
}

export const ITALIC = 1;
export const DIVINE_NAME = 2;
export const WORDS_OF_JESUS = 4;
export const JOINED = 8;

export interface BookText {
  code: string;
  /** Dense position (`TextIndex.pos`) of the book's first word. */
  firstPos: number;
  /** The book's words in reading order: word `ord` is `words[index.pos(ord) - firstPos]`. */
  words: string[];
  /** Per-word flag bits, indexed like `words`. */
  flags: Uint8Array;
  pilcrow: Set<number>;
  breaks: Map<number, string[]>;
  headingsBefore: Map<number, string[]>;
  headingsAfter: Map<number, string[]>;
  notes: Map<number, { ref: string; text: string }[]>;
}

export interface Text {
  manifest: Manifest;
  index: TextIndex;
}

async function fetchOk(url: string): Promise<Response> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res;
}

export async function loadText(): Promise<Text> {
  const manifest = (await (await fetchOk(base + "manifest.json")).json()) as Manifest;
  const index = new TextIndex(await (await fetchOk(base + manifest.index)).arrayBuffer());
  return { manifest, index };
}

const books = new Map<string, Promise<BookText>>();

export function loadBook(text: Text, book: number): Promise<BookText> {
  const m = text.manifest.books[book]!;
  let p = books.get(m.code);
  if (!p) {
    p = fetchOk(base + m.file)
      .then((r) => r.json() as Promise<BookJson>)
      .then((j) => toBookText(j, text.index));
    books.set(m.code, p);
  }
  return p;
}

function toBookText(j: BookJson, index: TextIndex): BookText {
  const firstPos = index.pos(j.first_ord);
  const flags = new Uint8Array(j.words.length);
  const mark = (runs: [number, number][], bit: number) => {
    for (const [s, e] of runs) for (let p = index.pos(s); p < index.pos(e); p++) flags[p - firstPos]! |= bit;
  };
  mark(j.italic, ITALIC);
  mark(j.divine_name, DIVINE_NAME);
  mark(j.words_of_jesus, WORDS_OF_JESUS);
  for (const o of j.joined) flags[index.pos(o) - firstPos]! |= JOINED;
  const group = <V>(entries: [number, V][]) => {
    const m = new Map<number, V[]>();
    for (const [k, v] of entries) m.set(k, [...(m.get(k) ?? []), v]);
    return m;
  };
  return {
    code: j.book,
    firstPos,
    words: j.words,
    flags,
    pilcrow: new Set(j.pilcrow),
    breaks: group(j.breaks),
    headingsBefore: group(j.headings.filter((h) => h.pos === "before").map((h) => [h.slot, h.text])),
    headingsAfter: group(j.headings.filter((h) => h.pos === "after").map((h) => [h.slot, h.text])),
    notes: group(j.notes.map((n) => [n.after, { ref: n.ref, text: n.text }])),
  };
}

export const overlaps = (a: Interval, b: Interval) => a[0] < b[1] && b[0] < a[1];

/** Words as displayed: separated by spaces, except after a hyphen that joins two parts of one word. */
export function joinWords(bt: BookText, from: number, to: number): string {
  let out = "";
  for (let i = from; i < to; i++) out += bt.words[i]! + (i + 1 < to && !(bt.flags[i]! & JOINED) ? " " : "");
  return out;
}
