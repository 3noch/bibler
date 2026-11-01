// Scripture references inside free text (a comment), for showing them as links. Strict where the location field is
// forgiving: a wrong guess in someone's comment would quietly send readers to the wrong place, so only exact book
// names and abbreviations count (no typos, no ambiguous prefixes), and a book needs a chapter after it. Short
// abbreviations are also ordinary words ("Is", "Am", "So", "Ex"), so an abbreviation must be capitalized, and one of
// two letters (or a lowercase book name) must name a verse too ("Is 53:5", "john 3:16"). Each reference in a list
// is its own link ("Rom 8:28, 31": two). Found when shown, never stored.
import { ALIASES, BOOKS, normalizeBookName } from "./books.ts";
import { parseRef } from "./refs.ts";
import type { Interval, TextIndex } from "./textIndex.ts";

export interface FoundRef {
  /** [start, end) character offsets in the text. */
  start: number;
  end: number;
  interval: Interval;
}

const ORDINAL = String.raw`(?:[1-3]|I{1,3}|1st|2nd|3rd|First|Second|Third)[ .]?\s?`;
const BOOK = String.raw`(?:${ORDINAL})?[A-Za-z]+(?:\s+of\s+[A-Za-z]+)?\.?`;
const NUM = String.raw`\d{1,3}`;
const VERSE = String.raw`\s*:\s*${NUM}`;
const DASH = String.raw`\s*[-–—]\s*`;
const TAIL = String.raw`${NUM}(?:${VERSE})?(?:${DASH}${NUM}(?:${VERSE})?)?(?:\s*,\s*${NUM}(?:${DASH}${NUM})?)*`;
/** A book phrase, then a chapter (and more): a candidate for `parseRef`. Not inside a word or number. */
const CANDIDATE = new RegExp(String.raw`(?<![\p{L}\p{N}])(${BOOK})\s*(${TAIL})(?![\p{L}\p{N}]|\s*:\s*\d)`, "gu");

/** Book names spelled out (not abbreviations): a name, its ref form ("Psalm"), or a long alias ("Song of Songs"). */
const NAMES = new Set(
  BOOKS.flatMap((b) => [b.name, b.refName ?? b.name].map(normalizeBookName)).concat(
    [...ALIASES.keys()].filter((a) => a.replace(/^\d/, "").length >= 5),
  ),
);

/** The references in `text` that `index` has, in order. */
export function findRefs(index: TextIndex, text: string): FoundRef[] {
  const out: FoundRef[] = [];
  CANDIDATE.lastIndex = 0;
  for (let m = CANDIDATE.exec(text); m; m = CANDIDATE.exec(text)) {
    const [whole, phrase, tail] = m as unknown as [string, string, string];
    if (!acceptable(phrase, tail)) {
      CANDIDATE.lastIndex = m.index + 1; // a book may start further in ("Gospel of John 3:16")
      continue;
    }
    const parsed = parseRef(index, whole);
    if (parsed.warnings.length || !parsed.refs.length) {
      CANDIDATE.lastIndex = m.index + 1;
      continue;
    }
    for (const r of parsed.refs) out.push({ start: m.index + r.span[0], end: m.index + r.span[1], interval: r.interval });
  }
  return out;
}

function acceptable(phrase: string, tail: string): boolean {
  const key = normalizeBookName(phrase);
  if (ALIASES.get(key) === undefined) return false;
  // The book's own word, after any number ("1 John", "1John", "II Kings"); a Roman numeral needs a space or dot
  // after it, else "Is" would be "I" + "s".
  const word = phrase.replace(/^(?:[1-3](?:st|nd|rd)?[ .]*|(?:I{1,3}|First|Second|Third)[ .]+)/, "");
  const verse = tail.includes(":");
  const name = NAMES.has(key);
  if (word[0] === word[0]!.toLowerCase()) return name && verse;
  return name || key.replace(/^\d/, "").length > 2 || verse;
}
