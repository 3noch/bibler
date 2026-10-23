// Loads text-build artifacts: the list of texts (/text/texts.json), and each text's manifest, index and per-book
// JSON from /text/<text_id>/. Files other than the list and the manifests are content-hashed, so the browser cache
// and the service worker (src/sw.js) keep them forever. Which text is read is chosen per device; the service worker
// keeps the files of the last few read (KEEP_TEXTS) for offline reading and drops the rest, since a device can't
// hold every translation we may offer.
import { createSignal } from "solid-js";
import { TextIndex, type Interval } from "@bible-tags/scripture";

/** The text read until another is chosen. */
export const DEFAULT_TEXT = "kjv-1769-v2";
/** Texts kept for offline reading on this device: the most recently read. */
export const KEEP_TEXTS = 3;
const CHOSEN_KEY = "bible-tags/text/v1";
const KEPT_KEY = "bible-tags/texts-kept/v1";

// Vite's base: "/" for server builds (absolute, so the SPA fallback at deep paths still finds the text) and "./"
// for the static GitHub Pages build (works under any subpath, e.g. /<repo>/).
const root = `${import.meta.env.BASE_URL}text/`;

/** An entry of texts.json. */
export interface TextInfo {
  id: string;
  abbr: string;
  name: string;
  /** The license or copyright line, shown with the text. */
  rights: string;
}

const storageGet = (k: string) => {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
};
const storageSet = (k: string, v: string) => {
  try {
    localStorage.setItem(k, v);
  } catch {
    // blocked: the choice lasts until the page closes
  }
};

const [textId, setTextId] = createSignal(storageGet(CHOSEN_KEY) ?? DEFAULT_TEXT);
/** The text being read (this device's choice). */
export { textId };
export function chooseText(id: string) {
  storageSet(CHOSEN_KEY, id);
  setTextId(id);
}

/** The texts on offer. Offline without a cached list: just the one being read. */
export async function loadCatalog(): Promise<TextInfo[]> {
  try {
    return (await (await fetchOk(root + "texts.json")).json()) as TextInfo[];
  } catch {
    const t = await loadText(textId()).catch(() => null);
    return t ? [t.info] : [];
  }
}

/** Ask the service worker (production only) to download text `id` for offline reading and to drop every text but
 *  the KEEP_TEXTS most recently read. */
export function keepOffline(id: string) {
  let kept: string[] = [];
  try {
    kept = JSON.parse(storageGet(KEPT_KEY) ?? "[]") as string[];
  } catch {
    // start over
  }
  kept = [id, ...kept.filter((k) => k !== id)].slice(0, KEEP_TEXTS);
  storageSet(KEPT_KEY, JSON.stringify(kept));
  if (import.meta.env.PROD && "serviceWorker" in navigator) {
    void navigator.serviceWorker.ready.then((r) => r.active?.postMessage({ type: "keep-texts", texts: kept }));
  }
}

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
  abbr: string;
  name: string;
  rights: string;
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
  /** Layout breaks before a word (ordinal): USFM paragraph/poetry markers (`p`, `b`, `q1`, `q2`, `m`, …). */
  breaks: [number, string][];
  /** Before a word (it may be inside a verse), or after the book's last word (epistle subscriptions). */
  headings: { at: number; pos: "before" | "after"; kind: string; text: string }[];
  notes: { after: number; ref: string; text: string }[];
}

/** A heading's USFM kind (`s1` section, `s2` subsection, `r` parallel passages, `ms1`/`mr` a book of Psalms and its
 *  range, `qa` an acrostic letter) and text. */
export interface Heading {
  kind: string;
  text: string;
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
  /** By the ordinal of the word they come before. */
  breaks: Map<number, string[]>;
  headingsBefore: Map<number, Heading[]>;
  /** By the ordinal of the word they come after. */
  headingsAfter: Map<number, Heading[]>;
  notes: Map<number, { ref: string; text: string }[]>;
}

export interface Text {
  id: string;
  info: TextInfo;
  manifest: Manifest;
  index: TextIndex;
}

async function fetchOk(url: string): Promise<Response> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res;
}

const texts = new Map<string, Promise<Text>>();

/** Text `id`'s manifest and index (once per page; a failure is retried on the next call). */
export function loadText(id: string): Promise<Text> {
  let p = texts.get(id);
  if (!p) {
    const base = `${root}${id}/`;
    p = (async () => {
      const manifest = (await (await fetchOk(base + "manifest.json")).json()) as Manifest;
      const index = new TextIndex(await (await fetchOk(base + manifest.index)).arrayBuffer());
      const info = { id, abbr: manifest.abbr, name: manifest.name, rights: manifest.rights };
      return { id, info, manifest, index };
    })();
    p.catch(() => texts.delete(id));
    texts.set(id, p);
  }
  return p;
}

/** Books of the text being read (another text's are let go when one of this one's is asked for). */
let booksOf = "";
const books = new Map<string, Promise<BookText>>();

export function loadBook(text: Text, book: number): Promise<BookText> {
  if (booksOf !== text.id) {
    books.clear();
    booksOf = text.id;
  }
  const m = text.manifest.books[book]!;
  let p = books.get(m.code);
  if (!p) {
    p = fetchOk(`${root}${text.id}/${m.file}`)
      .then((r) => r.json() as Promise<BookJson>)
      .then((j) => toBookText(j, text.index));
    p.catch(() => books.delete(m.code));
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
    headingsBefore: group(j.headings.filter((h) => h.pos === "before").map((h) => [h.at, { kind: h.kind, text: h.text }])),
    headingsAfter: group(j.headings.filter((h) => h.pos === "after").map((h) => [h.at, { kind: h.kind, text: h.text }])),
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
