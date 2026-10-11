import { createEffect, createMemo, createResource, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { BOOKS, parseRef, type Interval } from "@bible-tags/scripture";
import { loadText, type Text } from "./text.ts";
import { openTagStore, type Tag, type TagStore } from "./store.ts";
import { Bible, type Nav } from "./Bible.tsx";
import { NearbyPanel } from "./Nearby.tsx";
import { Omnibox, ResultList, SearchPanel, type Chip } from "./Search.tsx";
import { TagsSheet } from "./Tags.tsx";
import { BrushIcon, CloseIcon, EraserIcon, HelpIcon, PencilIcon, PlusIcon, RedoIcon, TagsIcon, UndoIcon } from "./icons.tsx";

export function App() {
  const [text] = createResource(loadText);
  const [store] = createResource(openTagStore);
  const failure = () =>
    text.error ? `Couldn't load the text (${text.error})` : store.error ? `Couldn't reach the server (${store.error})` : null;
  return (
    <Show when={text() && store()} fallback={<p class="splash">{failure() ?? ""}</p>}>
      <Main text={text()!} store={store()!} />
    </Show>
  );
}

function Main(props: { text: Text; store: TagStore }) {
  const { index } = props.text;
  const store = props.store;
  const [nav, setNav] = createSignal<Nav>({ ch: 0, target: null, n: 0 });
  const [here, setHere] = createSignal(0); // global chapter at the top of the viewport
  const [activeTag, setActiveTag] = createSignal<string | null>(null);
  const [eraser, setEraser] = createSignal(false);
  const [near, setNear] = createSignal<{ tags: string[]; pos: number } | null>(null);
  const [sheetTag, setSheetTag] = createSignal<string | null>(null);
  const [help, setHelp] = createSignal(false);
  const [chips, setChips] = createSignal<Chip[]>([]);
  const [tagsOpen, setTagsOpen] = createSignal(false);

  const chapterLabel = (ch: number, style: "full" | "short" = "full") => {
    const book = index.bookOfChapter[ch]!;
    const b = BOOKS[book]!;
    if (index.chapterCount(book) === 1) return style === "short" ? b.short : b.name;
    return `${style === "short" ? b.short : (b.refName ?? b.name)} ${ch - index.bookFirstChapter[book]! + 1}`;
  };

  // --- navigation: the URL hash is the location (`#/John 3:16`) ------------------------------------------
  const goTo = (ch: number, target: Interval | null = null) => setNav((n) => ({ ch, target, n: n.n + 1 }));
  const goRef = (raw: string): string | null => {
    const r = parseRef(index, raw, { book: index.bookOfChapter[here()]!, chapter: here() - index.bookFirstChapter[index.bookOfChapter[here()]!]! + 1 });
    const ref = r.refs[0];
    if (!ref) return r.warnings[0]?.message ?? "Not a reference";
    goTo(index.chapterIndex(ref.start.book, ref.start.chapter), ref.start.verse === null ? null : ref.interval);
    return null;
  };
  const goInterval = (iv: Interval) => {
    const { book, chapter } = index.locate(iv[0]);
    goTo(index.chapterIndex(book, chapter), iv);
  };
  const readHash = () => goRef(decodeURIComponent(location.hash.replace(/^#\/?/, "")) || "Genesis 1");
  onMount(() => {
    readHash();
    window.addEventListener("hashchange", readHash);
    onCleanup(() => window.removeEventListener("hashchange", readHash));
  });
  const onLocation = (ch: number) => {
    setHere(ch);
    history.replaceState(null, "", "#/" + encodeURIComponent(chapterLabel(ch)));
  };

  // --- tags -----------------------------------------------------------------------------------------------
  /** Ordinal range of the current native text selection inside the reader, if any. */
  const selectionRange = (): [number, number] | null => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
    const range = sel.getRangeAt(0);
    let lo = Infinity;
    let hi = -Infinity;
    for (const el of document.querySelectorAll<HTMLElement>(".bible [data-ord]")) {
      if (!range.intersectsNode(el)) continue;
      const o = Number(el.dataset.ord);
      lo = Math.min(lo, o);
      hi = Math.max(hi, o);
    }
    return lo <= hi ? [lo, hi + 1] : null;
  };
  const stopPainting = () => {
    setActiveTag(null);
    setEraser(false);
  };
  // The eraser and a selected tag are exclusive: the eraser erases every tag (Alt-drag with a tag erases just it).
  const toggleEraser = () => {
    const on = !eraser();
    setEraser(on);
    if (on) setActiveTag(null);
  };
  /** Remove one tag from one place (a whole tagging), leaving overlapping tags alone. Undoable like any edit. */
  const unhighlight = (tagId: string, iv: Interval) => store.erase(tagId, iv[0], iv[1]);
  const selectTag = (id: string) => {
    setActiveTag(id);
    setEraser(false);
  };
  // A selected tag that no longer exists (deleted here or on another device, or its creation undone) is deselected.
  createEffect(() => {
    const id = activeTag();
    if (id && !store.tagById().has(id)) setActiveTag(null);
  });
  const chooseTag = (tag: Tag, erase = false) => {
    const range = selectionRange();
    if (range) {
      if (erase) store.erase(tag.id, ...range);
      else store.paint(tag.id, ...range);
      window.getSelection()?.removeAllRanges();
      return;
    }
    setNear(null);
    if (activeTag() === tag.id) setActiveTag(null);
    else selectTag(tag.id);
  };

  const onKey = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.toLowerCase();
    if (mod && key === "z") {
      e.preventDefault();
      if (e.shiftKey) store.redo();
      else store.undo();
    } else if (mod && key === "y") {
      e.preventDefault();
      store.redo();
    } else if (mod) {
      return;
    } else if (/^[1-9]$/.test(key)) {
      const tag = store.palette()[Number(key) - 1];
      if (tag) chooseTag(tag);
    } else if (key === "escape") {
      if (near() || help() || sheetTag()) {
        setNear(null);
        setHelp(false);
        setSheetTag(null);
      } else if (tagsOpen()) setTagsOpen(false); else if (chips().length) setChips([]);
      else stopPainting();
    } else if (key === "e") {
      toggleEraser();
    } else if (key === "[" && here() > 0) {
      goTo(here() - 1);
    } else if (key === "]" && here() < index.chapters - 1) {
      goTo(here() + 1);
    } else if (key === "/" || key === "g") {
      e.preventDefault();
      document.querySelector<HTMLInputElement>(".omnibox input")?.focus();
    }
  };
  onMount(() => {
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <>
      <header class="topbar">
        <Omnibox
          text={props.text}
          store={store}
          label={chapterLabel(here())}
          ctx={{ book: index.bookOfChapter[here()]!, chapter: here() - index.bookFirstChapter[index.bookOfChapter[here()]!]! + 1 }}
          chips={chips()}
          onChips={(c) => {
            setChips(c);
            setNear(null);
            setSheetTag(null);
          }}
          onGo={goRef}
        />
        <div class="actions">
          <button class="icon" disabled={!store.canUndo()} onClick={() => store.undo()} title="Undo (⌘Z)" aria-label="Undo">
            <UndoIcon />
          </button>
          <button class="icon" disabled={!store.canRedo()} onClick={() => store.redo()} title="Redo (⇧⌘Z)" aria-label="Redo">
            <RedoIcon />
          </button>
          <Show when={store.mode === "local"} fallback={<SyncDot store={store} />}>
            <button class="local-badge" onClick={() => setHelp((h) => !h)} title="Your tags are only stored in this browser. Click for details.">
              <span aria-hidden="true">⚠</span> Local only
            </button>
          </Show>
          <button class="icon" classList={{ on: help() }} onClick={() => setHelp((h) => !h)} title="Help" aria-label="Help">
            <HelpIcon />
          </button>
        </div>
      </header>
      <Show when={help()}>
        <Help local={store.mode === "local"} onClose={() => setHelp(false)} />
      </Show>

      <main class="page">
        <Bible
          text={props.text}
          store={store}
          activeTag={activeTag()}
          eraser={eraser()}
          nav={nav()}
          onLocation={onLocation}
          onInspect={(ord) => {
            // Tags at this word, in palette order (then by name).
            const ids = [...new Set(store.taggings().filter((t) => t.start <= ord && ord < t.end).map((t) => t.tag))];
            const pal = store.palette().map((t) => t.id);
            const rank = (id: string) => (pal.includes(id) ? pal.indexOf(id) : 100);
            ids.sort((a, b) => rank(a) - rank(b) || (store.tagById().get(a)?.name ?? "").localeCompare(store.tagById().get(b)?.name ?? ""));
            setSheetTag(null);
            setNear(ids.length ? { tags: ids, pos: ord } : null);
          }}
        />
      </main>

      <Show when={near()}>
        {(n) => (
          <NearbyPanel
            text={props.text}
            store={store}
            tags={n().tags}
            pos={n().pos}
            onGo={(iv) => {
              setNear({ tags: n().tags, pos: iv[0] });
              goInterval(iv);
            }}
            onEdit={(id) => {
              setNear(null);
              setSheetTag(id);
            }}
            onUnhighlight={unhighlight}
            onSearch={(ids) => {
              setNear(null);
              setChips(ids.map((id): Chip => ({ kind: "tag", id })));
            }}
            onClose={() => setNear(null)}
          />
        )}
      </Show>

      <Show when={tagsOpen() && !near() && !sheetTag()}>
        <TagsSheet
          store={store}
          active={activeTag()}
          onChoose={(t) => {
            selectTag(t.id);
            setTagsOpen(false);
          }}
          onCreated={(t) => {
            selectTag(t.id);
            setTagsOpen(false);
          }}
          onEdit={setSheetTag}
          onClose={() => setTagsOpen(false)}
        />
      </Show>

      <Show when={chips().length > 0 && !near() && !sheetTag() && !tagsOpen()}>
        <SearchPanel text={props.text} store={store} chips={chips()} onChips={setChips} onGo={goInterval} onUnhighlight={unhighlight} onClose={() => setChips([])} />
      </Show>

      <Dock
        store={store}
        active={activeTag()}
        eraser={eraser()}
        onChoose={chooseTag}
        onEraser={toggleEraser}
        onAllTags={() => {
          setNear(null);
          setSheetTag(null);
          setTagsOpen((o) => !o);
        }}
        onEdit={(id) => {
          setNear(null);
          setSheetTag(id);
        }}
        onCreated={(t) => selectTag(t.id)}
      />

      <Show when={store.tagById().get(sheetTag() ?? "")}>
        {(tag) => (
          <TagSheet
            tag={tag()}
            store={store}
            text={props.text}
            onGo={goInterval}
            onUnhighlight={unhighlight}
            onClose={() => setSheetTag(null)}
            onDeleted={(id) => {
              if (activeTag() === id) setActiveTag(null);
              setSheetTag(null);
            }}
          />
        )}
      </Show>

      <div class="toasts" role="status">
        <For each={store.notices()}>{(n) => <p class="toast">{n.text}</p>}</For>
      </div>
    </>
  );
}

function SyncDot(props: { store: TagStore }) {
  const title = () => {
    if (props.store.mode === "local") return "Saved in this browser only (no account or sync in this version)";
    const n = props.store.pendingCount();
    switch (props.store.status()) {
      case "synced":
        return n ? `Saving ${n} change${n === 1 ? "" : "s"}…` : "All changes saved";
      case "syncing":
        return "Connecting…";
      case "offline":
        return `Offline — ${n ? `${n} change${n === 1 ? "" : "s"} saved on this device` : "changes are saved on this device"}`;
    }
  };
  return (
    <span class="sync" data-status={props.store.status()} title={title()} aria-label={title()}>
      <Show when={props.store.status() === "offline"}>
        <span class="sync-label">Offline{props.store.pendingCount() ? ` · ${props.store.pendingCount()}` : ""}</span>
      </Show>
    </span>
  );
}

function Help(props: { local: boolean; onClose: () => void }) {
  return (
    <div class="help" role="dialog" aria-label="Help">
      <button class="icon close" onClick={props.onClose} aria-label="Close">
        <CloseIcon />
      </button>
      <Show when={props.local}>
        <div class="local-note">
          <strong>Local only.</strong> This version has no account and no server. Your tags are kept only in this
          browser on this device: they are not saved anywhere else, not backed up, and not synced to your other
          devices. Clearing this site's data (or using a private window) erases them.
        </div>
      </Show>
      <dl>
        <dt>Tag</dt>
        <dd>Pick a tag below, then drag over words. On a phone: tap a word, or long-press and drag.</dd>
        <dt>Or</dt>
        <dd>Select text, then tap a tag.</dd>
        <dt>Stop</dt>
        <dd>Tap the active tag again (or Esc). Its pencil edits it.</dd>
        <dt>Search</dt>
        <dd>Type in the top box: pick tags (all must overlap) and ranges like “Romans” or “Gospels”. <kbd>n</kbd> / <kbd>p</kbd> step through results.</dd>
        <dt>Look up</dt>
        <dd>Tap tagged words to see nearby places with the same tags.</dd>
        <dt>Keys</dt>
        <dd>
          <kbd>1</kbd>–<kbd>9</kbd> tags · <kbd>E</kbd> eraser · <kbd>/</kbd> go to or search · <kbd>[</kbd> <kbd>]</kbd> chapters ·{" "}
          <kbd>⌘Z</kbd> undo
        </dd>
      </dl>
    </div>
  );
}

/** The palette: tag wells, eraser, add. The active well shows a brush (tap again to stop) and an edit button. */
function Dock(props: {
  store: TagStore;
  active: string | null;
  eraser: boolean;
  onChoose: (t: Tag, erase?: boolean) => void;
  onEraser: () => void;
  onEdit: (id: string) => void;
  onCreated: (t: Tag) => void;
  onAllTags: () => void;
}) {
  const [adding, setAdding] = createSignal(false);
  /** The active tag when it is not in the palette (shown as an extra well so you can see what you paint with). */
  const offPalette = createMemo(() => {
    const id = props.active;
    return id && !props.store.paletteIds().includes(id) ? props.store.tagById().get(id) : undefined;
  });
  const hidden = () => props.store.tags().length - props.store.palette().length;
  // Keep a text selection alive when pressing a well (it may be "apply to selection").
  const keepSelection: JSX.EventHandler<HTMLElement, MouseEvent> = (e) => e.preventDefault();

  // ---- drag to reorder (mouse: press + move; touch: long-press, then move — a plain swipe scrolls the dock,
  // and a long-press released without moving opens the editor). Committed as one palette_set on release.
  const [order, setOrder] = createSignal<string[] | null>(null); // live preview while dragging
  const [dragId, setDragId] = createSignal<string | null>(null);
  const ids = () => order() ?? props.store.paletteIds();
  const wellsShown = createMemo(() => ids().map((id) => props.store.tagById().get(id)).filter((t): t is Tag => !!t));
  let wells!: HTMLDivElement;
  let press: { id: string; pointerId: number; x: number; y: number; touch: boolean; armed: boolean; timer: number } | null = null;
  let suppressClick = false;
  const suppressNextClick = () => {
    suppressClick = true;
    setTimeout(() => (suppressClick = false));
  };
  const endPress = () => {
    if (press) clearTimeout(press.timer);
    press = null;
    setDragId(null);
    setOrder(null);
  };
  const wellIdAt = (t: EventTarget | null) => (t as HTMLElement | null)?.closest<HTMLElement>("[data-well]")?.dataset.well;

  const onPointerDown = (e: PointerEvent) => {
    const id = wellIdAt(e.target);
    if (!id || e.button !== 0 || !props.store.paletteIds().includes(id)) return;
    const touch = e.pointerType === "touch";
    press = { id, pointerId: e.pointerId, x: e.clientX, y: e.clientY, touch, armed: false, timer: 0 };
    if (touch) {
      press.timer = window.setTimeout(() => {
        if (!press) return;
        press.armed = true;
        navigator.vibrate?.(10);
      }, 350);
    }
  };
  const onPointerMove = (e: PointerEvent) => {
    if (!press || e.pointerId !== press.pointerId) return;
    const dist = Math.hypot(e.clientX - press.x, e.clientY - press.y);
    if (!dragId()) {
      if (press.touch && !press.armed) {
        if (dist > 8) endPress(); // a swipe: let the dock scroll
        return;
      }
      if (dist < (press.touch ? 4 : 6)) return;
      setOrder(props.store.paletteIds());
      setDragId(press.id);
      wells.setPointerCapture(e.pointerId);
    }
    // Insertion index = number of other wells whose middle is left of the pointer.
    const current = order()!;
    const others = [...wells.querySelectorAll<HTMLElement>("[data-well]")].filter(
      (el) => el.dataset.well !== press!.id && current.includes(el.dataset.well!),
    );
    const to = others.filter((el) => {
      const r = el.getBoundingClientRect();
      return e.clientX > r.left + r.width / 2;
    }).length;
    if (to !== current.indexOf(press.id)) {
      const next = current.filter((x) => x !== press!.id);
      next.splice(to, 0, press.id);
      setOrder(next);
    }
    const r = wells.getBoundingClientRect();
    if (e.clientX < r.left + 28) wells.scrollLeft -= 10;
    else if (e.clientX > r.right - 28) wells.scrollLeft += 10;
  };
  const onPointerUp = (e: PointerEvent) => {
    if (!press || e.pointerId !== press.pointerId) return;
    if (dragId()) {
      const next = order()!;
      if (next.join() !== props.store.paletteIds().join()) props.store.setPalette(next);
      suppressNextClick();
    } else if (press.touch && press.armed) {
      props.onEdit(press.id); // long-press without moving
      suppressNextClick();
    }
    endPress();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    // Alt+←/→ on a focused well moves it (keyboard reordering).
    if (!e.altKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    const id = wellIdAt(e.target);
    const pal = props.store.paletteIds();
    const i = id ? pal.indexOf(id) : -1;
    const j = i + (e.key === "ArrowLeft" ? -1 : 1);
    if (i < 0 || j < 0 || j >= pal.length) return;
    e.preventDefault();
    const next = [...pal];
    [next[i], next[j]] = [next[j]!, next[i]!];
    props.store.setPalette(next);
    queueMicrotask(() => wells.querySelector<HTMLElement>(`[data-well="${id}"]`)?.focus());
  };
  onMount(() => {
    // While a touch drag is armed or active, the dock must not scroll.
    const block = (e: TouchEvent) => {
      if (press?.armed || dragId()) e.preventDefault();
    };
    wells.addEventListener("touchmove", block, { passive: false });
    onCleanup(() => wells.removeEventListener("touchmove", block));
  });

  const well = (tag: Tag, n: number) => (
    <Well
      tag={tag}
      n={n}
      active={props.active === tag.id}
      dragging={dragId() === tag.id}
      onTap={(e) => {
        if (!suppressClick) props.onChoose(tag, e.altKey);
      }}
      onEdit={() => props.onEdit(tag.id)}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!press?.touch) props.onEdit(tag.id); // touch long-press is handled above
      }}
      onMouseDown={keepSelection}
    />
  );

  return (
    <nav class="dock" aria-label="Tags">
      <div
        class="wells"
        classList={{ reordering: !!dragId() }}
        ref={wells}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={endPress}
        onKeyDown={onKeyDown}
      >
        <For each={wellsShown()}>{(tag, i) => well(tag, i() + 1)}</For>
        <Show when={offPalette()}>{(tag) => well(tag(), 0)}</Show>
        <Show
          when={adding()}
          fallback={
            <button class="well add" onClick={() => setAdding(true)} title="New tag" aria-label="New tag">
              <PlusIcon />
              <Show when={props.store.palette().length === 0}>
                <span class="well-name">New tag</span>
              </Show>
            </button>
          }
        >
          <form
            class="well adding"
            onSubmit={(e) => {
              e.preventDefault();
              const input = e.currentTarget.elements.namedItem("name") as HTMLInputElement;
              const tag = input.value.trim() ? props.store.createTag(input.value) : undefined;
              if (tag) props.onCreated(tag);
              setAdding(false);
            }}
          >
            <input name="name" placeholder="Tag name" ref={(el) => queueMicrotask(() => el.focus())} onBlur={() => setAdding(false)} />
          </form>
        </Show>
      </div>
      <div class="tools">
        <button class="well tool all-tags" onClick={props.onAllTags} title={hidden() > 0 ? `All tags (${hidden()} not in the palette)` : "All tags"} aria-label="All tags">
          <TagsIcon />
          <Show when={hidden() > 0}>
            <span class="badge-count">{hidden()}</span>
          </Show>
        </button>
        <button class="well tool" classList={{ active: props.eraser }} onMouseDown={keepSelection} onClick={props.onEraser} title={props.eraser ? "Eraser — tap to stop" : "Eraser (E)"} aria-label="Eraser" aria-pressed={props.eraser}>
          <EraserIcon />
        </button>
      </div>
    </nav>
  );
}

/** A tag well. Tap: choose (tap the active one again to stop). The active well carries a visible edit button;
 *  right-click / touch long-press also edit; drag reorders (handled by the dock). */
function Well(props: {
  tag: Tag;
  n: number;
  active: boolean;
  dragging: boolean;
  onTap: (e: MouseEvent) => void;
  onEdit: () => void;
  onContextMenu: (e: MouseEvent) => void;
  onMouseDown: JSX.EventHandler<HTMLElement, MouseEvent>;
}) {
  let group!: HTMLSpanElement;
  // The dock scrolls sideways when there are many wells: keep the active one in view.
  createEffect(() => {
    if (props.active) queueMicrotask(() => group.scrollIntoView({ block: "nearest", inline: "nearest" }));
  });
  const hint = () => (props.n ? " · drag to reorder" : "");
  return (
    <span class="well-group" ref={group} classList={{ active: props.active, dragging: props.dragging }} style={{ "--c": props.tag.color }}>
      <button
        class="well"
        data-well={props.tag.id}
        title={props.active ? `Painting with ${props.tag.name} — tap to stop${hint()}` : props.n ? `${props.tag.name} (${props.n})${hint()}` : props.tag.name}
        aria-pressed={props.active}
        onMouseDown={props.onMouseDown}
        onContextMenu={props.onContextMenu}
        onClick={props.onTap}
      >
        <span class="swatch">
          <Show when={props.active}>
            <BrushIcon />
          </Show>
        </span>
        <span class="well-name">{props.tag.name}</span>
      </button>
      <Show when={props.active}>
        <button class="well-edit" onClick={props.onEdit} title={`Edit ${props.tag.name}`} aria-label={`Edit ${props.tag.name}`}>
          <PencilIcon />
        </button>
      </Show>
    </span>
  );
}

function TagSheet(props: {
  tag: Tag;
  store: TagStore;
  text: Text;
  onGo: (iv: Interval) => void;
  onUnhighlight: (tagId: string, iv: Interval) => void;
  onClose: () => void;
  onDeleted: (id: string) => void;
}) {
  const [confirming, setConfirming] = createSignal(false);
  const [renaming, setRenaming] = createSignal(false);
  const instances = createMemo(() => (props.store.byTag().get(props.tag.id) ?? []).map((t) => [t.start, t.end] as Interval));
  let cancelled = false;
  const saveName = (value: string) => {
    if (!cancelled && value.trim() && value.trim() !== props.tag.name) props.store.updateTag(props.tag.id, { name: value });
    cancelled = false;
    setRenaming(false);
  };
  return (
    <aside class="sheet" role="dialog" aria-label={`Tag ${props.tag.name}`}>
      <div class="sheet-head">
        <label class="color-edit" style={{ "--c": props.tag.color }} title="Change color">
          <span class="swatch big" />
          <span class="badge" aria-hidden="true">
            <PencilIcon />
          </span>
          <input
            type="color"
            value={props.tag.color}
            onChange={(e) => props.store.updateTag(props.tag.id, { color: e.currentTarget.value })}
            aria-label="Change color"
          />
        </label>
        <Show
          when={renaming()}
          fallback={
            <button class="name-edit" onClick={() => setRenaming(true)} title="Rename">
              <span class="sheet-title">{props.tag.name}</span>
              <PencilIcon />
            </button>
          }
        >
          <input
            class="sheet-name"
            value={props.tag.name}
            aria-label="Tag name"
            ref={(el) =>
              queueMicrotask(() => {
                el.focus();
                el.select();
              })
            }
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") {
                e.stopPropagation();
                cancelled = true;
                e.currentTarget.blur();
              }
            }}
            onBlur={(e) => saveName(e.currentTarget.value)}
          />
        </Show>
        <button class="icon" onClick={props.onClose} aria-label="Close">
          <CloseIcon />
        </button>
      </div>
      <div class="nearby-body">
        <ResultList
          text={props.text}
          tags={[props.tag]}
          results={instances()}
          onGo={props.onGo}
          onUnhighlight={(iv) => props.onUnhighlight(props.tag.id, iv)}
          empty="Not used anywhere yet."
        />
      </div>
      <div class="sheet-foot">
        <Show
          when={confirming()}
          fallback={
            <button class="danger" onClick={() => setConfirming(true)}>
              Delete tag
            </button>
          }
        >
          <span>Delete “{props.tag.name}”?</span>
          <button
            class="danger"
            onClick={() => {
              // Read the id first: deleting unmounts this sheet, after which props.tag is stale.
              const id = props.tag.id;
              props.store.deleteTag(id);
              props.onDeleted(id);
            }}
          >
            Delete
          </button>
          <button onClick={() => setConfirming(false)}>Cancel</button>
        </Show>
      </div>
    </aside>
  );
}
