import { createMemo, For, Index, type JSX, Show } from "solid-js";
import type { Interval, Tagging } from "@bible-tags/scripture";
import { DIVINE_NAME, ITALIC, JOINED, WORDS_OF_JESUS, type BookText, type Text } from "./text.ts";
import type { Tag, TagStore } from "./store.ts";

export interface Stroke {
  a: number;
  b: number;
  erase: boolean;
}

export interface VerseProps {
  slot: number;
  bt: BookText;
  text: Text;
  store: TagStore;
  taggings: () => readonly Tagging[];
  stroke: () => Stroke | null;
  target: () => Interval | null;
}

interface Segment {
  tags: Tag[];
  ords: number[];
}

export function Verse(props: VerseProps) {
  const index = props.text.index;
  const ords = index.slotOrds(props.slot);
  const { verse } = index.locateSlot(props.slot);
  const bt = props.bt;

  /** Runs of consecutive words with an identical tag set. */
  const segments = createMemo<Segment[]>(() => {
    const out: Segment[] = [];
    let key = "";
    const tagById = props.store.tagById();
    for (const ord of ords) {
      const tags = props.taggings()
        .filter((t) => t.start <= ord && ord < t.end)
        .map((t) => tagById.get(t.tag))
        .filter((t): t is Tag => !!t)
        .sort((a, b) => (a.id < b.id ? -1 : 1));
      const k = tags.map((t) => t.id).join(",");
      if (out.length && k === key) out[out.length - 1]!.ords.push(ord);
      else out.push({ tags, ords: [ord] });
      key = k;
    }
    return out;
  });

  const breaks = bt.breaks.get(props.slot) ?? [];
  const local = (ord: number) => index.pos(ord) - bt.firstPos;
  /** A space follows the word unless it is the first part of a hyphenated word. */
  const spaced = (ord: number) => !(bt.flags[local(ord)]! & JOINED);
  const word = (ord: number) => {
    const f = bt.flags[local(ord)]!;
    const notes = bt.notes.get(ord);
    return (
      <>
        <span
          data-ord={ord}
          class="w"
          classList={{
            it: (f & ITALIC) !== 0,
            dn: (f & DIVINE_NAME) !== 0,
            wj: (f & WORDS_OF_JESUS) !== 0,
            preview: inStroke(ord, false),
            "preview-erase": inStroke(ord, true),
            target: inTarget(ord),
          }}
        >
          {bt.words[local(ord)]}
        </span>
        <Show when={notes}>
          <sup class="note" title={notes!.map((n) => n.text).join("\n")}>
            †
          </sup>
        </Show>
      </>
    );
  };
  const inStroke = (ord: number, erase: boolean) => {
    const st = props.stroke();
    return !!st && st.erase === erase && ord >= Math.min(st.a, st.b) && ord <= Math.max(st.a, st.b);
  };
  const inTarget = (ord: number) => {
    const t = props.target();
    return !!t && ord >= t[0] && ord < t[1];
  };

  return (
    <>
      <For each={bt.headingsBefore.get(props.slot) ?? []}>{(h) => <span class="heading">{h}</span>}</For>
      <Show when={breaks.includes("p") || breaks.includes("b")}>
        <span class="para-break" />
      </Show>
      <Show when={breaks.includes("q1")}>
        <span class="line-break" />
      </Show>
      <Show when={ords.length > 0}>
        <span class="verse" classList={{ title: verse === 0 }} data-slot={props.slot}>
          <Show when={verse > 0}>
            <sup class="vn">{verse}</sup>
          </Show>
          <Show when={bt.pilcrow.has(props.slot)}>
            <span class="pilcrow">¶ </span>
          </Show>
          <Index each={segments()}>
            {(seg, i) => (
              <>
                <span class="seg" data-tags={seg().tags.length} style={segStyle(seg().tags)}>
                  <Index each={seg().ords}>
                    {(ord, j) => (
                      <>
                        {j > 0 && spaced(seg().ords[j - 1]!) ? " " : ""}
                        {word(ord())}
                      </>
                    )}
                  </Index>
                </span>
                <Show when={i < segments().length - 1 && spaced(seg().ords[seg().ords.length - 1]!)}>
                  <span class="seg" style={segStyle(shared(seg().tags, segments()[i + 1]!.tags))}>
                    {" "}
                  </span>
                </Show>
              </>
            )}
          </Index>{" "}
        </span>
      </Show>
      <For each={bt.headingsAfter.get(props.slot) ?? []}>{(h) => <span class="heading subscription">{h}</span>}</For>
    </>
  );
}

/** Tags present on both sides of a segment boundary (the space between them is tagged with these). */
function shared(a: Tag[], b: Tag[]): Tag[] {
  const ids = new Set(b.map((t) => t.id));
  return a.filter((t) => ids.has(t.id));
}

/** Color is never the only signal: one tag = tint + underline; overlaps = stripes + double underline. */
export function segStyle(tags: Tag[]): JSX.CSSProperties | undefined {
  if (tags.length === 0) return undefined;
  if (tags.length === 1) {
    const c = tags[0]!.color;
    return { background: `${c}40`, "box-shadow": `inset 0 -2px 0 ${c}` };
  }
  const w = 6;
  const stops = tags.map((t, i) => `${t.color}55 ${i * w}px ${(i + 1) * w}px`).join(", ");
  return {
    background: `repeating-linear-gradient(135deg, ${stops})`,
    "box-shadow": `inset 0 -2px 0 ${tags[0]!.color}, inset 0 -4px 0 ${tags[1]!.color}`,
  };
}
