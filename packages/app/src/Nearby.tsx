// Nearby instances of the tags at a clicked word: the combination first (all of them overlapping), then each tag on
// its own. Each instance shows its text: short ones with some context around them, long ones fading out.
import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import { formatInterval, intersectRuns, nearby, type Interval } from "@bible-tags/scripture";
import { loadBook, type Text } from "./text.ts";
import type { Tag, TagStore } from "./store.ts";
import { segStyle } from "./Verse.tsx";
import { CloseIcon, PencilIcon } from "./icons.tsx";

/** Show at most this many words of an instance; shorter ones get up to CONTEXT words on each side. */
const LIMIT = 40;
const CONTEXT = 10;
const STEP = 5;

interface Props {
  text: Text;
  store: TagStore;
  /** Tag ids at the clicked word. */
  tags: string[];
  /** The position the list is centered on (the clicked word, then whatever instance was opened). */
  pos: number;
  onGo: (iv: Interval) => void;
  onEdit: (tagId: string) => void;
  /** Open full search results for this tag set. */
  onSearch: (tagIds: string[]) => void;
  onClose: () => void;
}

export function NearbyPanel(props: Props) {
  const tags = createMemo(() => props.tags.map((id) => props.store.tagById().get(id)).filter((t): t is Tag => !!t));
  const lists = createMemo(() => {
    const byTag = props.store.byTag();
    return new Map(tags().map((t) => [t.id, (byTag.get(t.id) ?? []).map((x) => [x.start, x.end] as Interval)]));
  });
  const combo = createMemo(() => (tags().length > 1 ? intersectRuns(tags().map((t) => lists().get(t.id)!)) : null));
  return (
    <aside class="sheet nearby" role="dialog" aria-label="Nearby">
      <div class="sheet-head">
        <h2 class="nearby-title">Nearby</h2>
        <button class="icon" onClick={props.onClose} aria-label="Close">
          <CloseIcon />
        </button>
      </div>
      <div class="nearby-body">
        <Show when={combo()}>
          {(runs) => <Section title={tags().map((t) => t.name).join(" + ")} members={tags()} instances={runs()} text={props.text} pos={props.pos} onGo={props.onGo} onEdit={props.onEdit} onSearch={props.onSearch} />}
        </Show>
        <For each={tags()}>
          {(t) => <Section title={t.name} members={[t]} instances={lists().get(t.id) ?? []} editable={t.id} text={props.text} pos={props.pos} onGo={props.onGo} onEdit={props.onEdit} onSearch={props.onSearch} />}
        </For>
      </div>
    </aside>
  );
}

function Section(props: {
  title: string;
  /** The tag set this section lists instances of (one tag, or the combination). */
  members: Tag[];
  instances: Interval[];
  editable?: string;
  text: Text;
  pos: number;
  onGo: (iv: Interval) => void;
  onEdit: (tagId: string) => void;
  onSearch: (tagIds: string[]) => void;
}) {
  const [before, setBefore] = createSignal(STEP);
  const [after, setAfter] = createSignal(STEP);
  const view = createMemo(() => nearby(props.instances, props.pos, before(), after()));
  return (
    <section class="nearby-section">
      <h3>
        <span class="swatches">
          <For each={props.members}>{(t) => <span class="swatch" style={{ "--c": t.color }} />}</For>
        </span>
        <span class="nearby-name">{props.title}</span>
        <span class="muted">{props.instances.length}</span>
        <button class="more search-all" onClick={() => props.onSearch(props.members.map((t) => t.id))} title="Show every place">
          Search all
        </button>
        <Show when={props.editable}>
          <button class="icon small" onClick={() => props.onEdit(props.editable!)} aria-label={`Edit ${props.title}`}>
            <PencilIcon />
          </button>
        </Show>
      </h3>
      <Show when={view().offset > 0}>
        <button class="more" onClick={() => setBefore((n) => n + STEP * 2)}>
          {view().offset} earlier
        </button>
      </Show>
      <ol class="snippets">
        <For each={view().items}>
          {(iv, i) => (
            <li>
              <Snippet text={props.text} iv={iv} tags={props.members} current={i() === view().current} onClick={() => props.onGo(iv)} />
            </li>
          )}
        </For>
      </ol>
      <Show when={view().offset + view().items.length < view().total}>
        <button class="more" onClick={() => setAfter((n) => n + STEP * 2)}>
          {view().total - view().offset - view().items.length} later
        </button>
      </Show>
    </section>
  );
}

export function Snippet(props: { text: Text; iv: Interval; tags: Tag[]; current: boolean; onClick: () => void }) {
  const { index } = props.text;
  const [bt] = createResource(
    () => props.iv,
    (iv) => loadBook(props.text, index.bookOf(iv[0])),
  );
  const parts = createMemo(() => {
    const b = bt();
    if (!b) return null;
    const [s, e] = props.iv;
    const bookEnd = b.firstOrd + b.words.length;
    const end = Math.min(e, bookEnd); // a cross-book instance shows its first book's part
    const words = (from: number, to: number) => b.words.slice(from - b.firstOrd, to - b.firstOrd).join(" ");
    if (end - s > LIMIT) return { before: "", inside: words(s, s + LIMIT), after: "", cutBefore: false, cutAfter: false, long: true };
    // Context stays inside the verse(s) the instance is in (no spilling into another verse, title or chapter).
    const ctx = Math.min(CONTEXT, Math.floor((LIMIT - (end - s)) / 2));
    const verseStart = index.slotInterval(index.slotOf(s))[0];
    const verseEnd = index.slotInterval(index.slotOf(end - 1))[1];
    const from = Math.max(verseStart, s - ctx);
    const to = Math.min(verseEnd, end + ctx);
    return { before: words(from, s), inside: words(s, end), after: words(end, to), cutBefore: from > verseStart, cutAfter: to < verseEnd, long: false };
  });
  return (
    <button class="snippet" classList={{ current: props.current }} onClick={props.onClick} aria-current={props.current ? "location" : undefined}>
      <span class="snip-ref">{formatInterval(index, props.iv, "short")}</span>
      <Show when={parts()} fallback={<span class="snip-text muted">…</span>}>
        {(p) => (
          <span class="snip-text" classList={{ long: p().long }}>
            <Show when={p().before}>
              <span class="ctx">
                {p().cutBefore ? "…" : ""}
                {p().before}{" "}
              </span>
            </Show>
            <mark style={segStyle(props.tags)}>{p().inside}</mark>
            <Show when={p().after}>
              <span class="ctx">
                {" "}
                {p().after}
                {p().cutAfter ? "…" : ""}
              </span>
            </Show>
          </span>
        )}
      </Show>
    </button>
  );
}
