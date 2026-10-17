// Loads the KJV artifacts produced by `cargo run -p text-build --release` (build/text/kjv-1769-v2/).
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { TextIndex } from "../src/textIndex.ts";

const root = new URL("../../../", import.meta.url);
const dir = new URL("build/text/kjv-1769-v2/", root);

export interface Manifest {
  text_id: string;
  index: string;
  tokens: number;
  books: { code: string; id: number; first_ord: number; chapters: number; tokens: number; file: string }[];
}

function load() {
  let manifest: Manifest;
  try {
    manifest = JSON.parse(readFileSync(new URL("manifest.json", dir), "utf8")) as Manifest;
  } catch {
    throw new Error("KJV artifacts missing: run `cargo run -p text-build --release` from the repo root first");
  }
  const bytes = readFileSync(new URL(manifest.index, dir));
  const buf = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
  const index = new TextIndex(buf);
  const golden = JSON.parse(readFileSync(new URL("fixtures/text/kjv-1769-v2.golden.json", root), "utf8")) as {
    books: { code: string; id: number; verse_starts_sha256: string; slot_words_sha256: string }[];
  };
  golden.books.forEach((g, b) => {
    const first = index.bookFirstSlot(b);
    const base = g.id * 2 ** 24;
    const local = Uint32Array.from(index.verseStart.subarray(first, index.bookFirstSlot(b + 1)), (v) => v - base);
    const hash = createHash("sha256").update(Buffer.from(local.buffer)).digest("hex");
    const words = index.slotWords.subarray(first, index.bookFirstSlot(b + 1));
    const wordsHash = createHash("sha256").update(words).digest("hex");
    if (index.bookId[b] !== g.id || hash !== g.verse_starts_sha256 || wordsHash !== g.slot_words_sha256) {
      throw new Error(`built index does not match the golden file for ${g.code}; rebuild with text-build`);
    }
  });
  return { manifest, index };
}

export const kjv = load();

export function bookWords(code: string): { first_ord: number; words: string[]; joined: number[] } {
  const b = kjv.manifest.books.find((x) => x.code === code)!;
  return JSON.parse(readFileSync(new URL(b.file, dir), "utf8"));
}
