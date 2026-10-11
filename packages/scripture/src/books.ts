/**
 * Canonical book table (protestant 66, KJV order). Index in BOOKS = book index used by TextIndex and must match
 * the order and ids of text-build's manifest (checked in tests).
 *
 * - `name`: full name for whole-book references ("Psalms").
 * - `refName`: full name before chapter/verse ("Psalm 23"); defaults to `name`.
 * - `short`: SBL-style abbreviation used by the short formatter.
 * - `aliases`: extra exact aliases, already in normalized form (see `normalizeBookName`). `name`, `refName` and
 *   `short` are aliases automatically. Aliases must be unique across books (tested).
 *
 * Deliberately NOT aliases (must stay ambiguous): `jon` (Jonah/John), `jud` (Judges/Jude), `ez` (Ezra/Ezekiel),
 * `ph` (Philippians/Philemon), `jo`, `ju`.
 */
export interface BookInfo {
  code: string;
  /** Stable book id = high bits of every ordinal in the book (see textIndex.ts). OT 2..40, NT 70..96;
   *  41..69 are reserved for deuterocanonical books (the source's file numbering). */
  id: number;
  name: string;
  refName?: string;
  short: string;
  aliases: string[];
}

export const BOOKS: readonly BookInfo[] = [
  { code: "GEN", id: 2, name: "Genesis", short: "Gen", aliases: ["ge", "gn"] },
  { code: "EXO", id: 3, name: "Exodus", short: "Exod", aliases: ["exo", "ex", "exd"] },
  { code: "LEV", id: 4, name: "Leviticus", short: "Lev", aliases: ["le", "lv"] },
  { code: "NUM", id: 5, name: "Numbers", short: "Num", aliases: ["nu", "nm", "nb"] },
  { code: "DEU", id: 6, name: "Deuteronomy", short: "Deut", aliases: ["de", "dt", "deu"] },
  { code: "JOS", id: 7, name: "Joshua", short: "Josh", aliases: ["jos", "jsh"] },
  { code: "JDG", id: 8, name: "Judges", short: "Judg", aliases: ["jdg", "jg", "jdgs"] },
  { code: "RUT", id: 9, name: "Ruth", short: "Ruth", aliases: ["rth", "ru"] },
  { code: "1SA", id: 10, name: "1 Samuel", short: "1 Sam", aliases: ["1sa", "1sm", "1s"] },
  { code: "2SA", id: 11, name: "2 Samuel", short: "2 Sam", aliases: ["2sa", "2sm", "2s"] },
  { code: "1KI", id: 12, name: "1 Kings", short: "1 Kgs", aliases: ["1ki", "1kin", "1k", "1kg"] },
  { code: "2KI", id: 13, name: "2 Kings", short: "2 Kgs", aliases: ["2ki", "2kin", "2k", "2kg"] },
  { code: "1CH", id: 14, name: "1 Chronicles", short: "1 Chr", aliases: ["1ch", "1chron"] },
  { code: "2CH", id: 15, name: "2 Chronicles", short: "2 Chr", aliases: ["2ch", "2chron"] },
  { code: "EZR", id: 16, name: "Ezra", short: "Ezra", aliases: ["ezr"] },
  { code: "NEH", id: 17, name: "Nehemiah", short: "Neh", aliases: ["ne"] },
  { code: "EST", id: 18, name: "Esther", short: "Esth", aliases: ["est", "es"] },
  { code: "JOB", id: 19, name: "Job", short: "Job", aliases: ["jb"] },
  { code: "PSA", id: 20, name: "Psalms", refName: "Psalm", short: "Ps", aliases: ["psa", "pss", "psm", "pslm"] },
  { code: "PRO", id: 21, name: "Proverbs", short: "Prov", aliases: ["pro", "prv", "pr"] },
  { code: "ECC", id: 22, name: "Ecclesiastes", short: "Eccl", aliases: ["ecc", "ec", "eccles", "qoh", "qoheleth"] },
  {
    code: "SNG", id: 23,
    name: "Song of Solomon",
    short: "Song",
    aliases: ["sng", "sos", "so", "songofsongs", "canticles", "cant"],
  },
  { code: "ISA", id: 24, name: "Isaiah", short: "Isa", aliases: ["is"] },
  { code: "JER", id: 25, name: "Jeremiah", short: "Jer", aliases: ["je", "jr"] },
  { code: "LAM", id: 26, name: "Lamentations", short: "Lam", aliases: ["la"] },
  { code: "EZK", id: 27, name: "Ezekiel", short: "Ezek", aliases: ["eze", "ezk"] },
  { code: "DAN", id: 28, name: "Daniel", short: "Dan", aliases: ["da", "dn"] },
  { code: "HOS", id: 29, name: "Hosea", short: "Hos", aliases: ["ho"] },
  { code: "JOL", id: 30, name: "Joel", short: "Joel", aliases: ["jl"] },
  { code: "AMO", id: 31, name: "Amos", short: "Amos", aliases: ["am"] },
  { code: "OBA", id: 32, name: "Obadiah", short: "Obad", aliases: ["ob", "oba"] },
  { code: "JON", id: 33, name: "Jonah", short: "Jonah", aliases: ["jnh"] },
  { code: "MIC", id: 34, name: "Micah", short: "Mic", aliases: ["mc"] },
  { code: "NAM", id: 35, name: "Nahum", short: "Nah", aliases: ["na"] },
  { code: "HAB", id: 36, name: "Habakkuk", short: "Hab", aliases: ["hb"] },
  { code: "ZEP", id: 37, name: "Zephaniah", short: "Zeph", aliases: ["zep", "zp"] },
  { code: "HAG", id: 38, name: "Haggai", short: "Hag", aliases: ["hg"] },
  { code: "ZEC", id: 39, name: "Zechariah", short: "Zech", aliases: ["zec", "zc"] },
  { code: "MAL", id: 40, name: "Malachi", short: "Mal", aliases: ["ml"] },
  { code: "MAT", id: 70, name: "Matthew", short: "Matt", aliases: ["mat", "mt"] },
  { code: "MRK", id: 71, name: "Mark", short: "Mark", aliases: ["mrk", "mk", "mr"] },
  { code: "LUK", id: 72, name: "Luke", short: "Luke", aliases: ["luk", "lk"] },
  { code: "JHN", id: 73, name: "John", short: "John", aliases: ["jhn", "jn"] },
  { code: "ACT", id: 74, name: "Acts", short: "Acts", aliases: ["act", "ac"] },
  { code: "ROM", id: 75, name: "Romans", short: "Rom", aliases: ["ro", "rm"] },
  { code: "1CO", id: 76, name: "1 Corinthians", short: "1 Cor", aliases: ["1co"] },
  { code: "2CO", id: 77, name: "2 Corinthians", short: "2 Cor", aliases: ["2co"] },
  { code: "GAL", id: 78, name: "Galatians", short: "Gal", aliases: ["ga"] },
  { code: "EPH", id: 79, name: "Ephesians", short: "Eph", aliases: ["ephes"] },
  { code: "PHP", id: 80, name: "Philippians", short: "Phil", aliases: ["php", "pp"] },
  { code: "COL", id: 81, name: "Colossians", short: "Col", aliases: [] },
  { code: "1TH", id: 82, name: "1 Thessalonians", short: "1 Thess", aliases: ["1th", "1thes"] },
  { code: "2TH", id: 83, name: "2 Thessalonians", short: "2 Thess", aliases: ["2th", "2thes"] },
  { code: "1TI", id: 84, name: "1 Timothy", short: "1 Tim", aliases: ["1ti", "1tm"] },
  { code: "2TI", id: 85, name: "2 Timothy", short: "2 Tim", aliases: ["2ti", "2tm"] },
  { code: "TIT", id: 86, name: "Titus", short: "Titus", aliases: ["tit", "ti"] },
  { code: "PHM", id: 87, name: "Philemon", short: "Phlm", aliases: ["phm", "philem"] },
  { code: "HEB", id: 88, name: "Hebrews", short: "Heb", aliases: [] },
  { code: "JAS", id: 89, name: "James", short: "Jas", aliases: ["jm"] },
  { code: "1PE", id: 90, name: "1 Peter", short: "1 Pet", aliases: ["1pe", "1pt", "1p"] },
  { code: "2PE", id: 91, name: "2 Peter", short: "2 Pet", aliases: ["2pe", "2pt", "2p"] },
  { code: "1JN", id: 92, name: "1 John", short: "1 John", aliases: ["1jn", "1jhn", "1jo", "1j"] },
  { code: "2JN", id: 93, name: "2 John", short: "2 John", aliases: ["2jn", "2jhn", "2jo", "2j"] },
  { code: "3JN", id: 94, name: "3 John", short: "3 John", aliases: ["3jn", "3jhn", "3jo", "3j"] },
  { code: "JUD", id: 95, name: "Jude", short: "Jude", aliases: ["jd"] },
  { code: "REV", id: 96, name: "Revelation", short: "Rev", aliases: ["re", "rv", "revelations", "apocalypse", "apoc"] },
];

/** Inputs that must never silently resolve even though a rule (prefix) would pick one book. */
export const FORCED_AMBIGUOUS: Readonly<Record<string, readonly string[]>> = {
  jon: ["JON", "JHN"],
};

const ORDINALS: Record<string, string> = {
  i: "1", ii: "2", iii: "3", "1st": "1", "2nd": "2", "3rd": "3", first: "1", second: "2", third: "3",
};

/**
 * Normalize a book name for lookup: lowercase; a leading ordinal word (`I`, `II`, `III`, `1st`, `First`, …) that is
 * followed by a space or `.` becomes a digit; then all spaces and dots are removed. `Isaiah` keeps its `I`.
 */
export function normalizeBookName(raw: string): string {
  let s = raw.toLowerCase().trim();
  const m = /^(iii|ii|i|1st|2nd|3rd|first|second|third)[\s.]+/.exec(s);
  if (m) s = ORDINALS[m[1]!]! + s.slice(m[0].length);
  return s.replace(/[\s.]+/g, "");
}

/** Every exact alias -> book index. Built once; throws on a collision. */
export const ALIASES: ReadonlyMap<string, number> = (() => {
  const map = new Map<string, number>();
  BOOKS.forEach((b, i) => {
    const all = [b.name, b.refName ?? b.name, b.short, ...b.aliases].map(normalizeBookName);
    for (const a of new Set(all)) {
      const prev = map.get(a);
      if (prev !== undefined && prev !== i) throw new Error(`alias ${a} used by ${BOOKS[prev]!.code} and ${b.code}`);
      map.set(a, i);
    }
  });
  return map;
})();

export const bookByCode = (code: string): number => {
  const i = BOOKS.findIndex((b) => b.code === code);
  if (i < 0) throw new Error(`unknown book code ${code}`);
  return i;
};
