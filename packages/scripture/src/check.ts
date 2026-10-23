/**
 * The server's checks on an op before it is applied (TS twin of `crates/api/src/check.rs`), for the offline-only
 * build's in-browser server: ids have a bounded shape, and a tagging's start is a word and its end a word of the
 * same book or that book's end (taggings never span books; see the root CLAUDE.md).
 */
import type { Op } from "./ops.ts";
import type { TextIndex } from "./textIndex.ts";

/** Ids are client-generated (ULIDs): 1-64 characters of [0-9A-Za-z_-]. */
export const validId = (id: string): boolean => /^[0-9A-Za-z_-]{1,64}$/.test(id);

/** Whether `[start, end)` is a range of words within one book of `index`. */
export function wordRange(index: TextIndex, start: number, end: number): boolean {
  const book = index.bookOf(start);
  if (!(start < end) || !index.isWord(start)) return false;
  return (index.isWord(end) && index.bookOf(end) === book) || end === index.bookEnd[book];
}

/** `{code, message}` for a `reject` (`bad_id` or `bad_text`), or null if the op may be applied. `texts` gives the
 *  index of each known text (like `api::check::Texts`). */
export function checkOp(texts: (textId: string) => TextIndex | undefined, op: Op): { code: "bad_id" | "bad_text"; message: string } | null {
  const ids: string[] = (() => {
    switch (op.kind) {
      case "paint":
        return [op.tag, op.new_id];
      case "erase":
        return op.split_id != null ? [op.tag, op.split_id] : [op.tag];
      case "comment_create":
        return [op.id, op.tagging];
      case "palette_set":
        return op.tags;
      case "revert":
        return [op.target];
      default:
        return [op.id];
    }
  })();
  if (!validId(op.op_id) || !ids.every(validId)) return { code: "bad_id", message: "ids must be 1-64 characters of [0-9A-Za-z_-]" };
  if (op.kind === "paint" || op.kind === "erase") {
    const index = texts(op.text_id);
    if (!index) return { code: "bad_text", message: `unknown text ${op.text_id}` };
    if (!wordRange(index, op.start, op.end)) return { code: "bad_text", message: `[${op.start}, ${op.end}) is not a range of words in ${op.text_id}` };
  }
  return null;
}
