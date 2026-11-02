// Continuous reader: chapters stacked in one scroll, extended at either edge as the reader nears it and trimmed far
// from the viewport (at most MAX_CHAPTERS rendered). Painting works across chapter (and book) boundaries because
// the whole stack is one hit-testing container. Edge changes above the viewport keep the visible text still
// (manual scroll anchoring; Safari's native anchoring can't be relied on).
// Or pages (`paged`: a reading setting, and always for a text its license limits): one chapter at a time, or the
// part of one that fits the text's limits (packages/scripture limits.ts), with the verse before and after it shown
// dimmed so tags across the boundary can be seen and painted, and ‹ › to the neighbouring pages. A limited text's
// pages come from the server (`loadPage`), never whole books.
import { createEffect, createMemo, createSignal, For, type JSX, on, onCleanup, onMount, Show } from "solid-js";
import { BOOKS, chunkAt, chunksOf, formatInterval, pageSlots, type Chunk, type Interval, type Tagging } from "@bible-tags/scripture";
import { loadBook, loadPage, overlaps, type BookText, type Text } from "./text.ts";
import type { TagStore } from "./store.ts";
import { Verse, type Stroke } from "./Verse.tsx";

/** A navigation request: show global chapter `ch` (and flash/center `target`, if any, or put the verse of ordinal
 *  `top` at the top: back where reading left off). `n` makes each unique. */
export interface Nav {
  ch: number;
  target: Interval | null;
  top?: number | null;
  n: number;
}

interface Props {
  text: Text;
  /** A page at a time instead of one continuous scroll. */
  paged: boolean;
  store: TagStore;
  activeTag: string | null;
  eraser: boolean;
  nav: Nav;
  /** The chapter at the top of the viewport changed (by scrolling or navigation). */
  onLocation: (ch: number) => void;
  /** The user scrolled and stopped: the first word of the verse now at the top (reading history). */
  onPosition: (ord: number) => void;
  /** Gets a function giving the first word of the verse at the top now (or null before anything shows). */
  topRef?: (top: () => number | null) => void;
  /** A word was clicked while not painting. */
  onInspect: (ord: number, rect: DOMRect) => void;
}

const MAX_CHAPTERS = 9;
/** Extend when an edge is within this many px of the viewport. */
const EDGE_PX = 1500;
const EMPTY: readonly Tagging[] = [];
const LONG_PRESS_MS = 300;
/** Scrolling has stopped this long: report the position. */
const SETTLE_MS = 1000;
const TOUCH_SLOP_PX = 8;
const sameRefs = (a: readonly Tagging[], b: readonly Tagging[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** Where to scroll to show a verse at the top: its chapter when it is the first verse there (the number, a book's
 *  head, a Psalm's title), else the headings just before it, so none is hidden under the bar. */
function withHeadings(verse: Element): Element {
  let top = verse;
  for (let p = verse.previousElementSibling; p; p = p.previousElementSibling) {
    if (p.classList.contains("verse") && !p.classList.contains("title")) return top;
    if (p.classList.contains("heading")) top = p;
  }
  return verse.closest(".chapter") ?? top;
}

/** Height of the fixed top bar (content under it is not "visible"). */
const barHeight = () => document.querySelector<HTMLElement>(".topbar")?.offsetHeight ?? 0;

/** The page shown: chapter `ch`'s part `part` of `parts`, its own slots (`chunk`) and those shown (`slots`: with the
 *  context verse on either side). */
interface Page {
  ch: number;
  part: number;
  parts: number;
  chunk: Chunk;
  slots: [number, number];
}

export function Bible(props: Props) {
  const index = props.text.index;
  const lastChapter = index.chapters - 1;
  const limits = props.text.info.limits ?? null;
  const [range, setRange] = createSignal<[number, number] | null>(null);
  const [page, setPage] = createSignal<Page | null>(null);
  const books = new Map<number, BookText>();
  const [target, setTarget] = createSignal<Interval | null>(null);
  const [stroke, setStroke] = createSignal<Stroke | null>(null);
  let root!: HTMLDivElement;
  let busy = false;
  let navigating = false;
  let navSeq = 0;
  let flashTimer = 0;
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
    if (props.paged) {
      reportLocation();
      return measureRunouts();
    }
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

  /** The verse at the top of the reading area, once the user's own scrolling stops. */
  let settleTimer = 0;
  const reportPosition = () => {
    if (navigating || pinned) return;
    const ord = topVerse();
    if (ord !== null) props.onPosition(ord);
  };
  /** The first verse that starts at the top or below (a resume puts that verse's first line at the top, which may
   *  also hold the end of the one before), unless that is far down: then the long verse already under way. */
  const topVerse = (): number | null => {
    const top = barHeight() - 2;
    let under: HTMLElement | null = null;
    for (const el of root.querySelectorAll<HTMLElement>(".chapter:not(.context) .verse[data-slot]")) {
      const start = el.querySelector("[data-ord]")?.getBoundingClientRect().top ?? Infinity;
      if (start >= top) {
        const pick = start > top + innerHeight / 2 && under ? under : el;
        return index.verseStart[Number(pick.dataset.slot)]!;
      }
      under = el;
    }
    return under ? index.verseStart[Number(under.dataset.slot)]! : null;
  };
  props.topRef?.(topVerse);

  let raf = 0;
  const onScroll = () => {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(pump);
    clearTimeout(settleTimer);
    if (!navigating && !pinned) settleTimer = window.setTimeout(reportPosition, SETTLE_MS);
  };

  createEffect(
    on(
      () => props.nav,
      async (nav) => {
        // Navigations may overlap (`n` pressed twice across books that aren't loaded yet): only the latest may
        // set the range, scroll and flash, whichever book finishes loading last.
        const seq = ++navSeq;
        navigating = true;
        if (props.paged) return navPage(nav, seq);
        const r = range();
        if (!r || nav.ch < r[0] || nav.ch > r[1]) {
          const lo = Math.max(0, nav.ch - 1);
          const hi = Math.min(lastChapter, nav.ch + 1);
          try {
            await ensure(lo, hi);
          } catch (e) {
            if (seq === navSeq) navigating = false; // else scrolling would never load more chapters
            throw e;
          }
          if (seq !== navSeq) return;
          setRange([lo, hi]);
        }
        requestAnimationFrame(() => {
          if (seq !== navSeq) return;
          const word = nav.target && root.querySelector(`[data-ord="${nav.target[0]}"]`);
          const verse = nav.top != null && index.bookOf(nav.top) >= 0 && root.querySelector(`.verse[data-slot="${index.slotOf(nav.top)}"]`);
          if (word) word.scrollIntoView({ block: "center" });
          else if (verse) withHeadings(verse).scrollIntoView({ block: "start" });
          else root.querySelector(`[data-ch="${nav.ch}"]`)?.scrollIntoView({ block: "start" });
          setTarget(nav.target);
          clearTimeout(flashTimer); // an earlier navigation's timer would end this flash early
          if (nav.target) flashTimer = window.setTimeout(() => setTarget(null), 2500);
          navigating = false;
          pinned = true;
          reported = nav.ch;
          props.onLocation(nav.ch);
          pump();
        });
      },
    ),
  );

  // ------------------------------------------------------------------ pages

  const bookOfSlot = (s: number) => index.bookOfChapter[index.chapterOfVerse[s]!]!;
  /** The words of slots a..b: a page from the server for a limited text (replacing the last), else whole books. */
  const loadSlots = async ([a, b]: [number, number]) => {
    if (limits) {
      const got = await loadPage(props.text, a, b);
      books.clear();
      for (const [k, v] of got) books.set(k, v);
    } else {
      for (let bk = bookOfSlot(a); bk <= bookOfSlot(b); bk++) if (!books.has(bk)) books.set(bk, await loadBook(props.text, bk));
    }
  };
  /** Load and show chapter `ch`'s page `part`; false if a later navigation took over meanwhile. */
  const showPage = async (ch: number, part: number, seq: number) => {
    const list = chunksOf(index, ch, limits);
    const chunk = list[part]!;
    const slots = pageSlots(index, chunk);
    try {
      await loadSlots(slots);
    } catch (e) {
      if (seq === navSeq) navigating = false;
      throw e;
    }
    if (seq !== navSeq) return false;
    setPage({ ch, part, parts: list.length, chunk, slots });
    return true;
  };
  /** The page holding a navigation's target (or verse, or chapter start), then scroll to it as the scroll does. */
  const navPage = async (nav: Nav, seq: number) => {
    const slot = nav.target
      ? index.slotOf(nav.target[0])
      : nav.top != null && index.bookOf(nav.top) >= 0
        ? index.slotOf(nav.top)
        : index.chapterFirstVerse[nav.ch]!;
    const ch = index.chapterOfVerse[slot]!;
    const at = chunkAt(index, ch, slot, limits);
    if (!(await showPage(ch, at.part, seq))) return;
    requestAnimationFrame(() => {
      if (seq !== navSeq) return;
      const own = ".chapter:not(.context)";
      const word = nav.target && root.querySelector(`${own} [data-ord="${nav.target[0]}"]`);
      const verse = nav.top != null && slot > at.chunk.from && root.querySelector(`${own} .verse[data-slot="${slot}"]`);
      if (word) word.scrollIntoView({ block: "center" });
      else if (verse) withHeadings(verse).scrollIntoView({ block: "start" });
      else toPageStart();
      setTarget(nav.target);
      clearTimeout(flashTimer);
      if (nav.target) flashTimer = window.setTimeout(() => setTarget(null), 2500);
      navigating = false;
      pinned = true;
      reported = ch;
      props.onLocation(ch);
    });
  };
  /** The page before or after this one (the next part, or the neighbouring chapter's nearest part). */
  const neighbor = (dir: -1 | 1): { ch: number; part: number } | null => {
    const p = page();
    if (!p) return null;
    if (p.part + dir >= 0 && p.part + dir < p.parts) return { ch: p.ch, part: p.part + dir };
    const ch = p.ch + dir;
    if (ch < 0 || ch > lastChapter) return null;
    return { ch, part: dir < 0 ? chunksOf(index, ch, limits).length - 1 : 0 };
  };
  const pageLabel = (to: { ch: number; part: number }) => {
    const c = chunksOf(index, to.ch, limits)[to.part]!;
    return formatInterval(index, [index.verseStart[c.from]!, index.slotEnd(c.to)]);
  };
  /** The page's own top (its top run-out stays above, out of view). */
  const toPageStart = () => root.querySelector(".page-body")?.scrollIntoView({ block: "start" });
  const firstVerse = (c: Chunk) => {
    let s = c.from;
    while (s < c.to && index.slotWords[s] === 0) s++;
    return s;
  };
  const lastVerse = (c: Chunk) => {
    let s = c.to;
    while (s > c.from && index.slotWords[s] === 0) s--;
    return s;
  };
  /** ‹ › : reading on, like the chapter keys (not a jump): the page's top, its first verse as the reading place. */
  const goPage = async (to: { ch: number; part: number }) => {
    const seq = ++navSeq;
    navigating = true;
    if (!(await showPage(to.ch, to.part, seq))) return;
    requestAnimationFrame(() => {
      if (seq !== navSeq) return;
      toPageStart();
      navigating = false;
      pinned = true;
      reported = to.ch;
      props.onLocation(to.ch);
      props.onPosition(index.verseStart[firstVerse(page()!.chunk)]!);
    });
  };
  // Run-outs: a band past each end of the page ("Keep scrolling for John 4") that fills as you scroll into it;
  // scrolling to its end turns the page. The verse both pages show (this page's last verse is the next one's dimmed
  // first, and its first the previous one's dimmed last) stays where it was on screen, so the new text appears
  // where the band was and the eyes needn't find their place again. Ordinary scrolling, no overscroll (which the
  // browser's pull-to-refresh would take). Only the user's own scrolling turns pages (`pinned` is false), so a page
  // short enough to show its band at once doesn't turn by itself.
  const [runout, setRunout] = createSignal<{ dir: -1 | 1; p: number } | null>(null);
  let topBand: HTMLDivElement | undefined;
  let bottomBand: HTMLDivElement | undefined;
  const measureRunouts = () => {
    if (!props.paged || navigating) return;
    if (bottomBand?.isConnected) {
      const r = bottomBand.getBoundingClientRect();
      const p = (innerHeight - r.top) / r.height;
      if (p > 0) {
        if (!pinned && r.bottom <= innerHeight + 1) return void continueTo(1);
        return setRunout({ dir: 1, p: Math.min(1, p) });
      }
    }
    if (topBand?.isConnected) {
      const r = topBand.getBoundingClientRect();
      const p = (r.bottom - barHeight()) / r.height;
      if (p > 0) {
        if (!pinned && scrollY <= 0) return void continueTo(-1);
        return setRunout({ dir: -1, p: Math.min(1, p) });
      }
    }
    setRunout(null);
  };
  const continueTo = async (dir: -1 | 1) => {
    const to = neighbor(dir);
    const old = page();
    if (!to || !old || navigating) return;
    const anchor = dir > 0 ? lastVerse(old.chunk) : firstVerse(old.chunk);
    const before = root.querySelector(`.verse[data-slot="${anchor}"]`)?.getBoundingClientRect().top;
    const seq = ++navSeq;
    navigating = true;
    setRunout(null);
    if (!(await showPage(to.ch, to.part, seq))) return;
    requestAnimationFrame(() => {
      if (seq !== navSeq) return;
      const now = root.querySelector(`.verse[data-slot="${anchor}"]`)?.getBoundingClientRect().top;
      if (now !== undefined && before !== undefined) window.scrollBy(0, now - before);
      else toPageStart();
      navigating = false;
      pinned = true; // until the user scrolls on (a fling's momentum may be cut short by the swap)
      reported = to.ch;
      props.onLocation(to.ch);
      props.onPosition(index.verseStart[firstVerse(page()!.chunk)]!);
    });
  };

  /** The page's pieces: the context verse before (dimmed), its own slots, the context verse after. */
  const sections = (p: Page) => [
    ...(p.slots[0] < p.chunk.from ? [{ ch: index.chapterOfVerse[p.slots[0]]!, from: p.slots[0], to: p.slots[0], context: true }] : []),
    { ch: p.ch, from: p.chunk.from, to: p.chunk.to, context: false },
    ...(p.slots[1] > p.chunk.to ? [{ ch: index.chapterOfVerse[p.slots[1]]!, from: p.slots[1], to: p.slots[1], context: true }] : []),
  ];
  const PageNav = () => {
    const prev = () => neighbor(-1);
    const next = () => neighbor(1);
    return (
      <nav class="page-nav" aria-label="Pages">
        <button class="page-step" disabled={!prev()} onClick={() => prev() && void goPage(prev()!)}>
          ‹ <span>{prev() ? pageLabel(prev()!) : ""}</span>
        </button>
        <button class="page-step" disabled={!next()} onClick={() => next() && void goPage(next()!)}>
          <span>{next() ? pageLabel(next()!) : ""}</span> ›
        </button>
      </nav>
    );
  };

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
    if ((e.target as HTMLElement).closest(".note, .ref-link, .page-nav")) return; // a note marker or a link does its own thing, even while painting
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
    // A stroke across books becomes one piece per book (taggings never span books).
    const ivs = index.byBook([Math.min(a, b), index.after(Math.max(a, b))]);
    if (erase) {
      if (props.activeTag) props.store.erase(props.activeTag, ivs);
      else props.store.eraseAll(ivs);
    } else if (props.activeTag) {
      props.store.paint(props.activeTag, ivs);
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
      clearTimeout(settleTimer);
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
      <Show
        when={props.paged}
        fallback={
          <>
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
          </>
        }
      >
        <Show when={page()} keyed>
          {(p) => (
            <>
              <Show when={neighbor(-1)}>
                {(to) => (
                  <div class="runout up" ref={(el) => (topBand = el)}>
                    <span class="runout-bar" style={{ "--p": String(runout()?.dir === -1 ? runout()!.p : 0) }} />
                    <span>Keep scrolling up for {pageLabel(to())}</span>
                  </div>
                )}
              </Show>
              <div class="page-body">
              <PageNav />
              <For each={sections(p)}>
                {(s) => (
                  <Chapter
                    ch={s.ch}
                    from={s.from}
                    to={s.to}
                    context={s.context}
                    bt={books.get(index.bookOfChapter[s.ch]!)!}
                    text={props.text}
                    store={props.store}
                    stroke={stroke}
                    target={target}
                  />
                )}
              </For>
              <PageNav />
              <Show when={limits}>
                <p class="page-rights">{props.text.info.rights}</p>
              </Show>
              </div>
              <Show when={neighbor(1)}>
                {(to) => (
                  <div class="runout" ref={(el) => (bottomBand = el)}>
                    <span>Keep scrolling for {pageLabel(to())}</span>
                    <span class="runout-bar" style={{ "--p": String(runout()?.dir === 1 ? runout()!.p : 0) }} />
                  </div>
                )}
              </Show>
            </>
          )}
        </Show>
      </Show>
    </div>

  );
}

function Chapter(props: {
  ch: number;
  /** Only slots from..to (a page's part of the chapter); `context`: a neighbouring verse shown dimmed, no headings. */
  from?: number;
  to?: number;
  context?: boolean;
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
  const lo = props.from ?? index.chapterFirstVerse[props.ch]!;
  const hi = props.to ?? index.chapterFirstVerse[props.ch + 1]! - 1;
  for (let s = lo; s <= hi; s++) slots.push(s);
  /** Book and chapter headings: only where the chapter itself starts. */
  const heads = !props.context && lo === index.chapterFirstVerse[props.ch];

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
    <section class="chapter" classList={{ context: !!props.context }} data-ch={props.context ? undefined : props.ch}>
      <Show when={heads && number === 1}>
        <header class="book-head">
          <h1>{info.name}</h1>
          {/* The text's own title, unless it only shortens the name (the BSB's "Song") */}
          <Show when={title && !info.name.startsWith(title)}>
            <p>{title}</p>
          </Show>
        </header>
      </Show>
      <Show when={heads && !single}>
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
