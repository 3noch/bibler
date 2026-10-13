// Continuous reader: chapters stacked in one scroll, extended at either edge as the reader nears it and trimmed far
// from the viewport (at most MAX_CHAPTERS rendered). Painting works across chapter (and book) boundaries because
// the whole stack is one hit-testing container. Edge changes above the viewport keep the visible text still
// (manual scroll anchoring; Safari's native anchoring can't be relied on).
import { createEffect, createMemo, createSignal, For, type JSX, on, onCleanup, onMount, Show } from "solid-js";
import { BOOKS, type Interval, type Tagging } from "@bible-tags/scripture";
import { loadBook, overlaps, type BookText, type Text } from "./text.ts";
import type { TagStore } from "./store.ts";
import { Verse, type Stroke } from "./Verse.tsx";

/** A navigation request: show global chapter `ch` (and flash/center `target`, if any). `n` makes each unique. */
export interface Nav {
  ch: number;
  target: Interval | null;
  n: number;
}

interface Props {
  text: Text;
  store: TagStore;
  activeTag: string | null;
  eraser: boolean;
  nav: Nav;
  /** The chapter at the top of the viewport changed (by scrolling or navigation). */
  onLocation: (ch: number) => void;
  /** A word was clicked while not painting. */
  onInspect: (ord: number, rect: DOMRect) => void;
}

const MAX_CHAPTERS = 9;
/** Extend when an edge is within this many px of the viewport. */
const EDGE_PX = 1500;
const EMPTY: readonly Tagging[] = [];
const LONG_PRESS_MS = 300;
const TOUCH_SLOP_PX = 8;
const sameRefs = (a: readonly Tagging[], b: readonly Tagging[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** Height of the fixed top bar (content under it is not "visible"). */
const barHeight = () => document.querySelector<HTMLElement>(".topbar")?.offsetHeight ?? 0;

export function Bible(props: Props) {
  const index = props.text.index;
  const lastChapter = index.chapters - 1;
  const [range, setRange] = createSignal<[number, number] | null>(null);
  const books = new Map<number, BookText>();
  const [target, setTarget] = createSignal<Interval | null>(null);
  const [stroke, setStroke] = createSignal<Stroke | null>(null);
  let root!: HTMLDivElement;
  let busy = false;
  let navigating = false;
  /** After a navigation, keep reporting its chapter until the user scrolls by hand (a centered target or a very
   *  short chapter would otherwise make the label name a neighbouring chapter). */
  let pinned = false;
  let reported = -1;

  const chapters = createMemo(() => {
    const r = range();
    if (!r) return [];
    return Array.from({ length: r[1] - r[0] + 1 }, (_, i) => r[0] + i);
  });

  /** Load the books of chapters lo..hi (before they are rendered, so they never render empty). */
  const ensure = async (lo: number, hi: number) => {
    for (let ch = lo; ch <= hi; ch++) {
      const b = index.bookOfChapter[ch]!;
      if (!books.has(b)) books.set(b, await loadBook(props.text, b));
    }
  };

  /** Run a DOM-changing update without moving the text the reader is looking at. */
  const anchored = (update: () => void) => {
    const top = barHeight();
    let anchor: Element | null = null;
    let before = 0;
    for (const el of root.querySelectorAll("[data-ch]")) {
      const r = el.getBoundingClientRect();
      if (r.bottom > top) {
        anchor = el;
        before = r.top;
        break;
      }
    }
    update();
    if (anchor?.isConnected) {
      const delta = anchor.getBoundingClientRect().top - before;
      if (delta) window.scrollBy(0, delta);
    }
  };

  const extend = async (dir: -1 | 1) => {
    const r = range();
    if (!r || busy || navigating) return;
    const [lo, hi] = r;
    const next = dir < 0 ? lo - 1 : hi + 1;
    if (next < 0 || next > lastChapter) return;
    busy = true;
    try {
      await ensure(next, next);
      const now = range();
      if (!now || now[0] !== lo || now[1] !== hi || navigating) return; // a navigation replaced the range
      let [nlo, nhi] = dir < 0 ? [next, hi] : [lo, next];
      if (nhi - nlo + 1 > MAX_CHAPTERS) {
        if (dir > 0) nlo++;
        else nhi--;
      }
      anchored(() => setRange([nlo, nhi]));
    } finally {
      busy = false;
    }
    pump();
  };

  /** Extend toward any edge that is near the viewport; report the current chapter. */
  const pump = () => {
    if (!range() || navigating) return;
    const first = root.querySelector("[data-ch]");
    const chs = root.querySelectorAll("[data-ch]");
    const lastEl = chs[chs.length - 1];
    if (first && first.getBoundingClientRect().top > -EDGE_PX) void extend(-1);
    else if (lastEl && lastEl.getBoundingClientRect().bottom < innerHeight + EDGE_PX) void extend(1);
    reportLocation();
  };

  /** The chapter at the middle of the reading area is "where you are". */
  const reportLocation = () => {
    if (navigating || pinned) return;
    const probe = barHeight() + (innerHeight - barHeight()) / 2;
    let current = -1;
    for (const el of root.querySelectorAll<HTMLElement>("[data-ch]")) {
      if (el.getBoundingClientRect().top <= probe) current = Number(el.dataset.ch);
      else break;
    }
    if (current < 0) current = Number(root.querySelector<HTMLElement>("[data-ch]")?.dataset.ch ?? -1);
    if (current >= 0 && current !== reported) {
      reported = current;
      props.onLocation(current);
    }
  };

  let raf = 0;
  const onScroll = () => {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(pump);
  };

  createEffect(
    on(
      () => props.nav,
      async (nav) => {
        navigating = true;
        const r = range();
        if (!r || nav.ch < r[0] || nav.ch > r[1]) {
          const lo = Math.max(0, nav.ch - 1);
          const hi = Math.min(lastChapter, nav.ch + 1);
          await ensure(lo, hi);
          setRange([lo, hi]);
        }
        requestAnimationFrame(() => {
          const word = nav.target && root.querySelector(`[data-ord="${nav.target[0]}"]`);
          if (word) word.scrollIntoView({ block: "center" });
          else root.querySelector(`[data-ch="${nav.ch}"]`)?.scrollIntoView({ block: "start" });
          setTarget(nav.target);
          if (nav.target) setTimeout(() => setTarget(null), 2500);
          navigating = false;
          pinned = true;
          reported = nav.ch;
          props.onLocation(nav.ch);
          pump();
        });
      },
    ),
  );

  // ------------------------------------------------------------------ painting (see packages/app/CLAUDE.md)

  /** Word under the point; between words (spaces, line ends) snap to the nearest word on the same line. */
  const ordAt = (x: number, y: number): number | null => {
    const el = document.elementFromPoint(x, y);
    const word = el?.closest<HTMLElement>("[data-ord]");
    if (word) return Number(word.dataset.ord);
    if (!el || !root.contains(el)) return null;
    const scope = el.closest<HTMLElement>(".verse") ?? el.closest<HTMLElement>("[data-ch]") ?? root;
    let best: HTMLElement | null = null;
    let bestDist = Infinity;
    for (const w of scope.querySelectorAll<HTMLElement>("[data-ord]")) {
      const r = w.getBoundingClientRect();
      if (y < r.top || y > r.bottom) continue;
      const d = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
      if (d < bestDist) [best, bestDist] = [w, d];
    }
    return best ? Number(best.dataset.ord) : null;
  };

  const painting = () => props.activeTag !== null || props.eraser;
  /** A touch that has not yet become a tap, a long-press stroke, or a scroll. */
  let pending: { id: number; x: number; y: number; ord: number; timer: number } | null = null;
  const clearPending = () => {
    if (pending) clearTimeout(pending.timer);
    pending = null;
  };
  const startStroke = (ord: number, erase: boolean) => setStroke({ a: ord, b: ord, erase: props.eraser || erase });

  const onPointerDown: JSX.EventHandler<HTMLDivElement, PointerEvent> = (e) => {
    if (!painting() || e.button !== 0) return;
    const ord = ordAt(e.clientX, e.clientY);
    if (ord === null) return;
    if (e.pointerType === "touch") {
      // Touch: swipe scrolls, tap tags one word, long-press then drag paints a range.
      clearPending();
      const timer = window.setTimeout(() => {
        if (!pending) return;
        startStroke(pending.ord, false);
        navigator.vibrate?.(10);
      }, LONG_PRESS_MS);
      pending = { id: e.pointerId, x: e.clientX, y: e.clientY, ord, timer };
      return;
    }
    e.preventDefault();
    root.setPointerCapture(e.pointerId);
    startStroke(ord, e.altKey);
  };

  const onPointerMove: JSX.EventHandler<HTMLDivElement, PointerEvent> = (e) => {
    const s = stroke();
    if (!s) {
      if (pending && e.pointerId === pending.id && Math.hypot(e.clientX - pending.x, e.clientY - pending.y) > TOUCH_SLOP_PX) {
        clearPending();
      }
      return;
    }
    const ord = ordAt(e.clientX, e.clientY);
    if (ord !== null && ord !== s.b) setStroke({ ...s, b: ord });
  };

  const commit = (a: number, b: number, erase: boolean) => {
    const start = Math.min(a, b);
    const end = index.after(Math.max(a, b));
    if (erase) {
      if (props.activeTag) props.store.erase(props.activeTag, start, end);
      else props.store.eraseAll(start, end);
    } else if (props.activeTag) {
      props.store.paint(props.activeTag, start, end);
    }
  };

  const onPointerUp = (e: PointerEvent) => {
    const s = stroke();
    if (!s && pending && e.pointerId === pending.id) {
      const ord = pending.ord;
      clearPending();
      commit(ord, ord, props.eraser);
      return;
    }
    clearPending();
    if (!s) return;
    setStroke(null);
    commit(s.a, s.b, s.erase);
  };

  const onClick: JSX.EventHandler<HTMLDivElement, MouseEvent> = (e) => {
    if (painting() || !window.getSelection()?.isCollapsed) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-ord]");
    if (el) props.onInspect(Number(el.dataset.ord), el.getBoundingClientRect());
  };

  onMount(() => {
    const block = (e: Event) => {
      if (stroke()) e.preventDefault(); // an active touch stroke must not scroll the page
    };
    const noMenu = (e: Event) => {
      if (painting()) e.preventDefault();
    };
    root.addEventListener("touchmove", block, { passive: false });
    root.addEventListener("contextmenu", noMenu);
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    const unpin = () => (pinned = false);
    const scrollKeys = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "]);
    const unpinKey = (e: KeyboardEvent) => {
      if (scrollKeys.has(e.key)) unpin();
    };
    for (const ev of ["wheel", "touchstart", "pointerdown"]) window.addEventListener(ev, unpin, { passive: true });
    window.addEventListener("keydown", unpinKey);
    onCleanup(() => {
      for (const ev of ["wheel", "touchstart", "pointerdown"]) window.removeEventListener(ev, unpin);
      window.removeEventListener("keydown", unpinKey);
    });
    onCleanup(() => {
      root.removeEventListener("touchmove", block);
      root.removeEventListener("contextmenu", noMenu);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      cancelAnimationFrame(raf);
      clearPending();
    });
  });

  return (
    <div
      class="bible"
      classList={{ painting: painting() }}
      ref={root}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => (clearPending(), setStroke(null))}
      onClick={onClick}
    >
      <Show when={range() && range()![0] > 0}>
        <div class="edge" aria-hidden="true" />
      </Show>
      <For each={chapters()}>
        {(ch) => (
          <Chapter ch={ch} bt={books.get(index.bookOfChapter[ch]!)!} text={props.text} store={props.store} stroke={stroke} target={target} />
        )}
      </For>
      <Show when={range() && range()![1] < lastChapter}>
        <div class="edge" aria-hidden="true" />
      </Show>
    </div>
  );
}

function Chapter(props: {
  ch: number;
  bt: BookText;
  text: Text;
  store: TagStore;
  stroke: () => Stroke | null;
  target: () => Interval | null;
}) {
  const index = props.text.index;
  const book = index.bookOfChapter[props.ch]!;
  const number = props.ch - index.bookFirstChapter[book]! + 1;
  const single = index.chapterCount(book) === 1;
  const info = BOOKS[book]!;
  const title = props.text.manifest.books[book]?.titles[0];
  const iv = index.chapterInterval(book, number);
  const slots: number[] = [];
  for (let s = index.chapterFirstVerse[props.ch]!; s < index.chapterFirstVerse[props.ch + 1]!; s++) slots.push(s);

  /** Verse-bucket index for this chapter: slot -> taggings overlapping it. */
  const buckets = createMemo(() => {
    const m = new Map<number, Tagging[]>();
    for (const t of props.store.visibleTaggings()) {
      if (!overlaps(iv, [t.start, t.end])) continue;
      const first = index.slotOf(Math.max(t.start, iv[0]));
      const last = index.slotOf(Math.min(t.end, iv[1]) - 1);
      for (let s = first; s <= last; s++) m.set(s, [...(m.get(s) ?? []), t]);
    }
    return m;
  });

  return (
    <section class="chapter" data-ch={props.ch}>
      <Show when={number === 1}>
        <header class="book-head">
          <h1>{info.name}</h1>
          <Show when={title && title !== info.name}>
            <p>{title}</p>
          </Show>
        </header>
      </Show>
      <Show when={!single}>
        <h2 class="chapter-head" aria-label={`${info.refName ?? info.name} ${number}`}>
          {number}
        </h2>
      </Show>
      <div class="text">
        <For each={slots}>
          {(slot) => (
            <Verse
              slot={slot}
              bt={props.bt}
              text={props.text}
              store={props.store}
              taggings={createMemo(() => buckets().get(slot) ?? EMPTY, EMPTY, { equals: sameRefs })}
              stroke={props.stroke}
              target={props.target}
            />
          )}
        </For>
      </div>
    </section>
  );
}
