// The tags at a clicked word. "Here" comes first: each tag's tagging at that word, with its comments and a button to
// remove it, so acting on one place never means finding it in a list. Then nearby instances, for moving around: the
// combination (all of them overlapping), then each tag on its own (a list whose only instance is the one here is
// left out: it would repeat Here); opening one makes it "here". Each instance shows
// its text: short ones with some context around them, long ones fading out.
import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import { formatInterval, intersectRuns, nearby, type Interval } from "@bible-tags/scripture";
import { JOINED, joinWords, loadBook, type Text } from "./text.ts";
import { intervalOf, type Tag, type TagStore } from "./store.ts";
import { segStyle } from "./Verse.tsx";
import { CloseIcon, CommentIcon, EraserIcon, PencilIcon } from "./icons.tsx";
import { CommentThread } from "./Comments.tsx";

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
  /** Remove one tag's highlight (a whole tagging) without touching overlapping tags. */
  onUnhighlight: (tagId: string, iv: Interval) => void;
  onClose: () => void;
}

export function NearbyPanel(props: Props) {
  const tags = createMemo(() => props.tags.map((id) => props.store.tagById().get(id)).filter((t): t is Tag => !!t));
  const lists = createMemo(() => {
    const byTag = props.store.byTag();
    return new Map(tags().map((t) => [t.id, (byTag.get(t.id) ?? []).map(intervalOf)]));
  });
  const combo = createMemo(() => (tags().length > 1 ? intersectRuns(tags().map((t) => lists().get(t.id)!)) : null));
  /** Each tag's tagging covering the position (a tag missing there, after moving to another instance, is skipped). */
  const here = createMemo(() =>
    tags().flatMap((t) => {
      const iv = lists().get(t.id)?.find((x) => x[0] <= props.pos && props.pos < x[1]);
      return iv ? [{ tag: t, iv }] : [];
    }),
  );
  /** A list whose only instance is the one at the position adds nothing to Here (the user found it shown twice). */
  const more = (ivs: Interval[]) => !(ivs.length === 1 && ivs[0]![0] <= props.pos && props.pos < ivs[0]![1]);
  return (
    <aside class="sheet nearby" role="dialog" aria-label="Nearby">
      <div class="sheet-head">
        <h2 class="nearby-title">Nearby</h2>
        <button class="icon" onClick={props.onClose} aria-label="Close">
          <CloseIcon />
        </button>
      </div>
      <div class="nearby-body">
        <Show when={here().length}>
          <section class="here">
            <h3 class="here-title">Here</h3>
            <For each={here()}>
              {(h) => (
                <div class="here-item" style={{ "--c": h.tag.color }}>
                  <div class="here-head">
                    <span class="swatch" />
                    <span class="nearby-name">{h.tag.name}</span>
                    <span class="here-actions">
                      <Show when={!props.store.readOnly()}>
                        <button class="chip-btn" onClick={() => props.onUnhighlight(h.tag.id, h.iv)} title={`Remove ${h.tag.name} from ${formatInterval(props.text.index, h.iv, "short")} (undo brings it back)`}>
                          <EraserIcon /> Remove
                        </button>
                        <button class="icon small" onClick={() => props.onEdit(h.tag.id)} title={`Edit ${h.tag.name}`} aria-label={`Edit ${h.tag.name}`}>
                          <PencilIcon />
                        </button>
                      </Show>
                    </span>
                  </div>
                  <Snippet text={props.text} iv={h.iv} tags={[h.tag]} current={false} onClick={() => props.onGo(h.iv)} />
                  <Show when={props.store.taggingAt(h.tag.id, h.iv[0])}>{(t) => <CommentThread store={props.store} tagging={t().id} />}</Show>
                </div>
              )}
            </For>
          </section>
        </Show>
        <Show when={combo() && more(combo()!) && combo()}>
          {(runs) => <Section title={tags().map((t) => t.name).join(" + ")} members={tags()} instances={runs()} store={props.store} text={props.text} pos={props.pos} onGo={props.onGo} onEdit={props.onEdit} onSearch={props.onSearch} />}
        </Show>
        <For each={tags().filter((t) => more(lists().get(t.id) ?? []))}>
          {(t) => <Section title={t.name} members={[t]} instances={lists().get(t.id) ?? []} editable={t.id} store={props.store} text={props.text} pos={props.pos} onGo={props.onGo} onEdit={props.onEdit} onSearch={props.onSearch} />}
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
  store: TagStore;
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
        <Show when={props.editable && !props.store.readOnly()}>
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
              <Snippet
                store={props.store}
                text={props.text}
                iv={iv}
                tags={props.members}
                current={i() === view().current}
                thread={false}
                onClick={() => props.onGo(iv)}
              />
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

export function Snippet(props: {
  /** Given: a single tag's snippet shows its tagging's comment count, and the current one its thread. */
  store?: TagStore;
  text: Text;
  iv: Interval;
  tags: Tag[];
  current: boolean;
  /** Show the current snippet's comment thread under it (default true). */
  thread?: boolean;
  onClick: () => void;
  /** Remove this highlight (only for a single tag's tagging, never for a combination). */
  onUnhighlight?: () => void;
}) {
  const { index } = props.text;
  const [bt] = createResource(
    () => props.iv,
    (iv) => loadBook(props.text, index.bookOf(iv[0])),
  );
  const parts = createMemo(() => {
    const b = bt();
    if (!b) return null;
    // Word arithmetic on dense positions (ordinals are sparse addresses).
    const [s, e] = props.iv;
    const bookEnd = index.bookEnd[index.bookOf(s)]!;
    const ps = index.pos(s);
    const pe = index.pos(Math.min(e, bookEnd)); // a cross-book instance shows its first book's part
    const words = (from: number, to: number) => joinWords(b, from - b.firstPos, to - b.firstPos);
    /** The space before position p, unless the word before it is joined to it (`Beth-` + `el`). */
    const space = (p: number) => (b.flags[p - 1 - b.firstPos]! & JOINED ? "" : " ");
    if (pe - ps > LIMIT) return { before: "", inside: words(ps, ps + LIMIT), after: "", sepBefore: "", sepAfter: "", cutBefore: false, cutAfter: false, long: true };
    // Context stays inside the verse(s) the instance is in (no spilling into another verse, title or chapter).
    const ctx = Math.min(CONTEXT, Math.floor((LIMIT - (pe - ps)) / 2));
    const verseStart = index.pos(index.slotInterval(index.slotOf(s))[0]);
    const verseEnd = index.pos(index.slotInterval(index.slotOf(Math.min(e, bookEnd) - 1))[1]);
    const from = Math.max(verseStart, ps - ctx);
    const to = Math.min(verseEnd, pe + ctx);
    return {
      before: words(from, ps),
      inside: words(ps, pe),
      after: words(pe, to),
      sepBefore: space(ps),
      sepAfter: space(pe),
      cutBefore: from > verseStart,
      cutAfter: to < verseEnd,
      long: false,
    };
  });
  const ref = () => formatInterval(index, props.iv, "short");
  const tagging = createMemo(() => (props.store && props.tags.length === 1 ? props.store.taggingAt(props.tags[0]!.id, props.iv[0]) : undefined));
  const count = () => {
    const t = tagging();
    return t ? (props.store!.commentsByTagging().get(t.id)?.length ?? 0) : 0;
  };
  return (
    <div class="snippet-row" classList={{ current: props.current, removable: !!props.onUnhighlight }}>
    <button class="snippet" classList={{ current: props.current }} onClick={props.onClick} aria-current={props.current ? "location" : undefined}>
      <span class="snip-ref">
        {ref()}
        <Show when={count()}>
          <span class="snip-count" title={`${count()} comment${count() === 1 ? "" : "s"}`}>
            <CommentIcon />
            {count()}
          </span>
        </Show>
      </span>
      <Show when={parts()} fallback={<span class="snip-text muted">…</span>}>
        {(p) => (
          <span class="snip-text" classList={{ long: p().long }}>
            <Show when={p().before}>
              <span class="ctx">
                {p().cutBefore ? "…" : ""}
                {p().before}
                {p().sepBefore}
              </span>
            </Show>
            <mark style={segStyle(props.tags)}>{p().inside}</mark>
            <Show when={p().after}>
              <span class="ctx">
                {p().sepAfter}
                {p().after}
                {p().cutAfter ? "…" : ""}
              </span>
            </Show>
          </span>
        )}
      </Show>
    </button>
      <Show when={props.onUnhighlight}>
        <button
          class="snip-remove"
          onClick={() => props.onUnhighlight!()}
          title={`Remove ${props.tags[0]?.name ?? "tag"} from ${ref()}`}
          aria-label={`Remove ${props.tags[0]?.name ?? "tag"} from ${ref()}`}
        >
          <EraserIcon />
        </button>
      </Show>
      <Show when={props.current && props.thread !== false && tagging()}>{(t) => <CommentThread store={props.store!} tagging={t().id} />}</Show>
    </div>
  );
}
