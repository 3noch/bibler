// Tag search. Query = chips: tag chips are ANDed (places where all of them overlap, superset semantics), scope chips
// (references or book groups) are ORed and keep the instances overlapping them. With scope chips only, the panel
// lists what is tagged in that scope. The omnibox also stays the "go to" box: a plain reference + Enter navigates.
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { BOOKS, formatInterval, groupInterval, inScope, intersectRuns, matchGroups, parseRef, type Interval } from "@bible-tags/scripture";
import type { Text } from "./text.ts";
import type { Tag, TagStore } from "./store.ts";
import { Snippet } from "./Nearby.tsx";
import { CloseIcon, NextIcon, PrevIcon, SearchIcon } from "./icons.tsx";

export type Chip = { kind: "tag"; id: string } | { kind: "scope"; label: string; intervals: Interval[] };

type Suggestion =
  | { kind: "go"; label: string; ref: string }
  | { kind: "chip"; label: string; hint: string; chip: Chip; color?: string };

const PAGE = 50;

export function Omnibox(props: {
  text: Text;
  store: TagStore;
  /** Where the reader is ("Psalm 23"), shown when idle. */
  label: string;
  /** Book/chapter context for bare references like "3:16". */
  ctx: { book: number; chapter: number };
  chips: Chip[];
  onChips: (chips: Chip[]) => void;
  onGo: (ref: string) => string | null;
}) {
  const { index } = props.text;
  const [focused, setFocused] = createSignal(false);
  const [value, setValue] = createSignal("");
  const [sel, setSel] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  let input!: HTMLInputElement;

  const chipLabel = (c: Chip) => (c.kind === "tag" ? (props.store.tagById().get(c.id)?.name ?? "?") : `in ${c.label}`);

  const suggestions = createMemo<Suggestion[]>(() => {
    const q = value().trim();
    const chosen = new Set(props.chips.flatMap((c) => (c.kind === "tag" ? [c.id] : [])));
    const tagSugs = (list: Tag[]) =>
      list
        .filter((t) => !chosen.has(t.id))
        .slice(0, 6)
        .map((t): Suggestion => ({ kind: "chip", label: t.name, hint: "tag", chip: { kind: "tag", id: t.id }, color: t.color }));
    if (!q) return focused() ? tagSugs([...props.store.palette(), ...props.store.tags().filter((t) => !props.store.palette().includes(t))]) : [];
    const out: Suggestion[] = [];
    const ql = q.toLowerCase();
    const tags = props.store
      .tags()
      .filter((t) => t.name.toLowerCase().includes(ql))
      .sort((a, b) => Number(!a.name.toLowerCase().startsWith(ql)) - Number(!b.name.toLowerCase().startsWith(ql)) || a.name.localeCompare(b.name));
    const parsed = parseRef(index, q, props.ctx);
    const refs = parsed.warnings.some((w) => w.code === "fuzzy_book") ? [] : parsed.refs;
    if (refs.length) {
      const label = refs.map((r) => formatInterval(index, r.interval)).join("; ");
      if (!props.chips.length) out.push({ kind: "go", label: `Go to ${label}`, ref: q });
      out.push({ kind: "chip", label: label, hint: "search within", chip: { kind: "scope", label, intervals: refs.map((r) => r.interval) } });
    }
    out.push(...tagSugs(tags));
    for (const g of matchGroups(q)) {
      out.push({ kind: "chip", label: g.name, hint: "search within", chip: { kind: "scope", label: g.name, intervals: [groupInterval(index, g)] } });
    }
    return out;
  });
  createEffect(on(suggestions, () => setSel(0)));

  const apply = (s: Suggestion | undefined) => {
    if (!s) {
      const err = value().trim() ? props.onGo(value()) : null;
      setError(err);
      if (!err) input.blur();
      return;
    }
    if (s.kind === "go") {
      setError(props.onGo(s.ref));
      input.blur();
      return;
    }
    props.onChips([...props.chips, s.chip]);
    setValue("");
    setError(null);
  };

  const showList = () => focused() && suggestions().length > 0;
  return (
    <form
      class="omnibox"
      classList={{ focused: focused(), searching: props.chips.length > 0 }}
      onSubmit={(e) => {
        e.preventDefault();
        apply(suggestions()[sel()]);
      }}
    >
      <div class="omni-field" onClick={() => input.focus()} title="Go to a passage or search tags (/)">
        <span class="omni-icon" aria-hidden="true">
          <SearchIcon />
        </span>
        <For each={props.chips}>
          {(c, i) => (
            <span class="chip" classList={{ scope: c.kind === "scope" }} style={c.kind === "tag" ? { "--c": props.store.tagById().get(c.id)?.color } : undefined}>
              {chipLabel(c)}
              <button
                type="button"
                aria-label={`Remove ${chipLabel(c)}`}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onChips(props.chips.filter((_, j) => j !== i()));
                }}
              >
                ×
              </button>
            </span>
          )}
        </For>
        <input
          ref={input}
          value={focused() || props.chips.length ? value() : props.label}
          aria-label="Go to a passage or search tags"
          placeholder={props.chips.length ? "Add a tag or range…" : "Go to or search…"}
          autocomplete="off"
          spellcheck={false}
          onFocus={() => {
            setFocused(true);
            setValue("");
          }}
          onBlur={() => setTimeout(() => (setFocused(false), setError(null)), 120)}
          onInput={(e) => {
            setValue(e.currentTarget.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            const n = suggestions().length;
            if (e.key === "ArrowDown" && n) {
              e.preventDefault();
              setSel((sel() + 1) % n);
            } else if (e.key === "ArrowUp" && n) {
              e.preventDefault();
              setSel((sel() - 1 + n) % n);
            } else if (e.key === "Backspace" && !value() && props.chips.length) {
              props.onChips(props.chips.slice(0, -1));
            } else if (e.key === "Escape") {
              input.blur();
            }
          }}
        />
        <Show when={!focused() && !props.chips.length}>
          <kbd class="omni-kbd" aria-hidden="true">/</kbd>
        </Show>
      </div>
      <Show when={showList()}>
        <ul class="suggestions" role="listbox">
          <For each={suggestions()}>
            {(s, i) => (
              <li
                role="option"
                aria-selected={i() === sel()}
                classList={{ selected: i() === sel() }}
                onPointerDown={(e) => {
                  e.preventDefault(); // keep focus in the input
                  apply(s);
                }}
              >
                <Show when={s.kind === "chip" && s.color} fallback={<span class="sug-icon">{s.kind === "go" ? "→" : "⌕"}</span>}>
                  <span class="swatch" style={{ "--c": (s as { color: string }).color }} />
                </Show>
                <span class="sug-label">{s.label}</span>
                <span class="sug-hint">{s.kind === "go" ? "" : (s as { hint: string }).hint}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={error()}>
        <p class="location-error">{error()}</p>
      </Show>
    </form>
  );
}

export function SearchPanel(props: {
  text: Text;
  store: TagStore;
  chips: Chip[];
  onChips: (chips: Chip[]) => void;
  onGo: (iv: Interval) => void;
  onUnhighlight: (tagId: string, iv: Interval) => void;
  onClose: () => void;
}) {
  const tags = createMemo(() =>
    props.chips.flatMap((c) => (c.kind === "tag" ? [props.store.tagById().get(c.id)] : [])).filter((t): t is Tag => !!t),
  );
  const scope = createMemo(() => props.chips.flatMap((c) => (c.kind === "scope" ? c.intervals : [])));
  const scopeLabel = createMemo(() => props.chips.flatMap((c) => (c.kind === "scope" ? [c.label] : [])).join(", "));
  const listOf = (id: string) => (props.store.byTag().get(id) ?? []).map((t) => [t.start, t.end] as Interval);

  const results = createMemo(() => (tags().length ? inScope(intersectRuns(tags().map((t) => listOf(t.id))), scope()) : []));
  /** Scope-only query: every tag with its number of instances in scope. */
  const tagCounts = createMemo(() =>
    tags().length
      ? []
      : props.store
          .tags()
          .map((t) => ({ tag: t, n: inScope(listOf(t.id), scope()).length }))
          .filter((x) => x.n > 0)
          .sort((a, b) => b.n - a.n || a.tag.name.localeCompare(b.tag.name)),
  );

  const title = () => (tags().length ? tags().map((t) => t.name).join(" + ") : "Tags") + (scopeLabel() ? ` in ${scopeLabel()}` : "");
  return (
    <aside class="sheet search" role="dialog" aria-label="Search results">
      <div class="sheet-head">
        <div class="search-title">
          <h2>{title()}</h2>
          <Show when={!tags().length}>
            <p class="muted">{`${tagCounts().length} tag${tagCounts().length === 1 ? "" : "s"}`}</p>
          </Show>
        </div>
        <button class="icon" onClick={props.onClose} aria-label="Close search">
          <CloseIcon />
        </button>
      </div>
      <div class="nearby-body">
        <Show when={tags().length}>
          <ResultList
            text={props.text}
            tags={tags()}
            results={results()}
            onGo={props.onGo}
            onUnhighlight={tags().length === 1 ? (iv) => props.onUnhighlight(tags()[0]!.id, iv) : undefined}
            empty={`No places where ${tags().length > 1 ? "all of these overlap" : "this tag is used"}${scopeLabel() ? ` in ${scopeLabel()}` : ""}.`}
          />
        </Show>
        <Show when={!tags().length}>
          <Show when={tagCounts().length} fallback={<p class="empty">Nothing tagged{scopeLabel() ? ` in ${scopeLabel()}` : ""} yet.</p>}>
            <ul class="tag-counts">
              <For each={tagCounts()}>
                {(x) => (
                  <li>
                    <button onClick={() => props.onChips([...props.chips, { kind: "tag", id: x.tag.id }])}>
                      <span class="swatch" style={{ "--c": x.tag.color }} />
                      <span class="nearby-name">{x.tag.name}</span>
                      <span class="muted">{x.n}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </div>
    </aside>
  );
}

/**
 * Instances as text snippets grouped by book (in text order), 50 at a time, with a ‹ i/N › stepper (also `n`/`p`)
 * that jumps the reader to each one. Used by search results and the tag sheet.
 */
export function ResultList(props: {
  text: Text;
  tags: Tag[];
  results: Interval[];
  onGo: (iv: Interval) => void;
  empty: string;
  /** Offered only when the results are one tag's taggings. */
  onUnhighlight?: (iv: Interval) => void;
}) {
  const { index } = props.text;
  const [limit, setLimit] = createSignal(PAGE);
  const [cur, setCur] = createSignal(-1);
  createEffect(on(() => props.results, () => (setLimit(PAGE), setCur(-1))));
  const groups = createMemo(() => {
    const out: { book: number; items: { iv: Interval; i: number }[] }[] = [];
    props.results.slice(0, limit()).forEach((iv, i) => {
      const book = index.locate(iv[0]).book;
      if (out[out.length - 1]?.book !== book) out.push({ book, items: [] });
      out[out.length - 1]!.items.push({ iv, i });
    });
    return out;
  });

  let list!: HTMLDivElement;
  const open = (i: number) => {
    const iv = props.results[i];
    if (!iv) return;
    if (i >= limit()) setLimit(Math.ceil((i + 1) / PAGE) * PAGE);
    setCur(i);
    props.onGo(iv);
    queueMicrotask(() => list.querySelector(`[data-i="${i}"]`)?.scrollIntoView({ block: "nearest" }));
  };
  const step = (d: 1 | -1) => {
    const n = props.results.length;
    if (n) open(cur() < 0 ? (d > 0 ? 0 : n - 1) : (cur() + d + n) % n);
  };
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if ((t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "n") step(1);
      else if (e.key === "p" || e.key === "N") step(-1);
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <div class="result-list" ref={list}>
      <Show when={props.results.length} fallback={<p class="empty">{props.empty}</p>}>
        <div class="results-bar">
          <span class="muted">
            {props.results.length} place{props.results.length === 1 ? "" : "s"}
          </span>
          <div class="stepper">
            <button class="icon small" onClick={() => step(-1)} aria-label="Previous (p)">
              <PrevIcon />
            </button>
            <span>
              {cur() < 0 ? "–" : cur() + 1}/{props.results.length}
            </span>
            <button class="icon small" onClick={() => step(1)} aria-label="Next (n)">
              <NextIcon />
            </button>
          </div>
        </div>
        <For each={groups()}>
          {(g) => (
            <section class="result-group">
              <h3 class="result-book">{BOOKS[g.book]!.name}</h3>
              <ol class="snippets">
                <For each={g.items}>
                  {(it) => (
                    <li data-i={it.i}>
                      <Snippet
                        text={props.text}
                        iv={it.iv}
                        tags={props.tags}
                        current={it.i === cur()}
                        onClick={() => open(it.i)}
                        onUnhighlight={props.onUnhighlight && props.tags.length === 1 ? () => props.onUnhighlight!(it.iv) : undefined}
                      />
                    </li>
                  )}
                </For>
              </ol>
            </section>
          )}
        </For>
        <Show when={limit() < props.results.length}>
          <button class="more" onClick={() => setLimit((n) => n + PAGE)}>
            Show {Math.min(PAGE, props.results.length - limit())} more
          </button>
        </Show>
      </Show>
    </div>
  );
}
