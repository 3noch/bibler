import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { BOOKS, liveHistory, near as samePlace, parseRef, resumeEntry, scrolled, visit, type HistoryEntry, type Interval } from "@bible-tags/scripture";
import { chooseText, DEFAULT_TEXT, findText, hashOf, keepOffline, loadCatalog, loadText, splitHash, textId, type Text, type TextInfo } from "./text.ts";
import { intervalOf, openTagStore, type Tag, type TagStore } from "./store.ts";
import { Bible, type Nav } from "./Bible.tsx";
import { NearbyPanel } from "./Nearby.tsx";
import { Omnibox, ResultList, SearchPanel, type Chip } from "./Search.tsx";
import { TagsSheet } from "./Tags.tsx";
import { Menu } from "./Menu.tsx";
import { SpaceButton, SpacesCard } from "./Groups.tsx";
import { UpdateNotice } from "./Update.tsx";
import { BrushIcon, CloseIcon, EraserIcon, MenuIcon, PencilIcon, PlusIcon, TagsIcon } from "./icons.tsx";
import { ulid } from "./ulid.ts";

/** This tab's reading-history entry (App's `reading`). */
const READING_KEY = "bible-tags/reading/v1";
const sessionGet = (k: string) => {
  try {
    return sessionStorage.getItem(k);
  } catch {
    return null;
  }
};
const sessionSet = (k: string, v: string | null) => {
  try {
    if (v === null) sessionStorage.removeItem(k);
    else sessionStorage.setItem(k, v);
  } catch {
    // blocked: a reload just starts from the newest entry
  }
};

export function App() {
  const [catalog] = createResource(loadCatalog);
  // A link may name a translation (`#/BSB/John%203`): it becomes the one read. Without one, this device's choice.
  const named = splitHash(location.hash).text;
  const [ready, setReady] = createSignal(!named);
  if (named) {
    void loadCatalog().then((list) => {
      const t = findText(list, named);
      if (t) chooseText(t.id);
      setReady(true);
    });
  }
  const [text] = createResource(() => ready() && textId(), loadText);
  const [store] = createResource(() => openTagStore((id) => loadText(id).then((t) => t.index, () => null)));
  const failure = () =>
    text.error ? `Couldn't load the text (${text.error})` : store.error ? `Couldn't reach the server (${store.error})` : null;
  // A text no longer offered (a stale choice): the default instead.
  createEffect(() => {
    const list = catalog();
    if (list?.length && !list.some((t) => t.id === textId())) chooseText(DEFAULT_TEXT);
  });
  createEffect(() => {
    const t = text();
    if (t) keepOffline(t.id);
  });
  /** Where the reader starts after switching texts: the verse that was at the top, or the link that switched. */
  let start: number | "link" | null = null;
  const switchText = async (id: string, top: number | "link" | null) => {
    if (id === textId()) return;
    try {
      await loadText(id); // first, so the reader switches at once (and stays put if the text can't be had)
    } catch (e) {
      store()?.notify(`Couldn't load that translation (${String(e)})`);
      return;
    }
    start = top;
    chooseText(id);
  };
  // Keyed: each text gets its own reader (its index is fixed for the reader's life).
  return (
    <Show when={store() && text()} keyed fallback={<p class="splash">{failure() ?? ""}</p>}>
      {(t) => <Main text={t} store={store()!} texts={catalog() ?? [t.info]} start={start} onText={(id, top) => void switchText(id, top)} />}
    </Show>
  );
}

function Main(props: {
  text: Text;
  store: TagStore;
  /** The texts on offer. */
  texts: TextInfo[];
  /** Open at this verse (switching texts) or at the link in the URL (which switched them), not where history says. */
  start: number | "link" | null;
  onText: (id: string, top: number | "link" | null) => void;
}) {
  const { index } = props.text;
  const store = props.store;
  const [nav, setNav] = createSignal<Nav>({ ch: 0, target: null, n: 0 });
  const [here, setHere] = createSignal(0); // global chapter at the top of the viewport
  const [activeTag, setActiveTag] = createSignal<string | null>(null);
  const [eraser, setEraser] = createSignal(false);
  const [near, setNear] = createSignal<{ tags: string[]; pos: number } | null>(null);
  const [sheetTag, setSheetTag] = createSignal<string | null>(null);
  const [help, setHelp] = createSignal(false);
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [spacesOpen, setSpacesOpen] = createSignal(false);
  const [chips, setChips] = createSignal<Chip[]>([]);
  const [tagsOpen, setTagsOpen] = createSignal(false);

  const chapterLabel = (ch: number, style: "full" | "short" = "full") => {
    const book = index.bookOfChapter[ch]!;
    const b = BOOKS[book]!;
    if (index.chapterCount(book) === 1) return style === "short" ? b.short : b.name;
    return `${style === "short" ? b.short : (b.refName ?? b.name)} ${ch - index.bookFirstChapter[book]! + 1}`;
  };

  // --- navigation: the URL hash is the location (`#/John 3:16`) ------------------------------------------
  const goTo = (ch: number, target: Interval | null = null, top: number | null = null) => setNav((n) => ({ ch, target, top, n: n.n + 1 }));
  const chapterOf = (ord: number) => {
    const { book, chapter } = index.locate(ord);
    return index.chapterIndex(book, chapter);
  };

  // --- reading history (packages/scripture/src/history.ts): a jump (the omnibox, a clicked place, the contents,
  // a link) starts or resumes an entry, scrolling moves that entry's `last`, and each jump is a browser history
  // step whose Back returns to where the previous entry left off. Stepping chapters ([ ]) is reading on.
  /** This tab's entry (sessionStorage: a reload resumes it). */
  const [reading, setReading] = createSignal<string | null>(sessionGet(READING_KEY));
  const setEntry = (id: string, push: boolean) => {
    setReading(id);
    sessionSet(READING_KEY, id);
    (push ? history.pushState : history.replaceState).call(history, { entry: id }, "", location.hash);
  };
  /** Opened without a place to go (the app, a group): follow the newest entry, also when the server's list
   *  arrives, until the user goes somewhere themselves. */
  let following = false;
  const record = (ord: number, push: boolean) => {
    following = false;
    const r = visit(index, store.history(), ord, Date.now(), ulid());
    store.setHistory(r.entries, true);
    setEntry(r.id, push);
  };
  const resume = (e: HistoryEntry, push: boolean) => {
    store.setHistory(resumeEntry(index, store.history(), e.id, Date.now()), true);
    setEntry(e.id, push);
    goTo(chapterOf(e.last.ord), null, e.last.ord);
  };
  /** A jump: recorded in history, then shown. */
  const jump = (ch: number, target: Interval | null, push = true) => {
    record(target ? index.verseStart[index.slotOf(target[0])]! : index.verseStart[index.chapterFirstVerse[ch]!]!, push);
    goTo(ch, target);
  };
  const goRef = (raw: string, push = true): string | null => {
    const r = parseRef(index, raw, { book: index.bookOfChapter[here()]!, chapter: here() - index.bookFirstChapter[index.bookOfChapter[here()]!]! + 1 });
    const ref = r.refs[0];
    if (!ref) return r.warnings[0]?.message ?? "Not a reference";
    jump(index.chapterIndex(ref.start.book, ref.start.chapter), ref.start.verse === null ? null : ref.interval, push);
    return null;
  };
  const goInterval = (iv: Interval) => jump(chapterOf(iv[0]), iv);
  const hashRef = () => splitHash(location.hash).ref;
  const hashFor = (e: HistoryEntry) => hashOf(props.text.info, chapterLabel(chapterOf(e.last.ord)));
  /** Back (or Forward) to a history step: that entry, where it left off now. The hash change that follows is it. */
  let popped: string | null = null;
  const onPop = (ev: PopStateEvent) => {
    const id = (ev.state as { entry?: string } | null)?.entry;
    const e = id && liveHistory(store.history()).find((x) => x.id === id);
    if (!e) return;
    popped = location.hash;
    following = false;
    resume(e, false);
    history.replaceState({ entry: e.id }, "", location.hash);
  };
  // A typed or followed link (#/Ref) in an open tab; its browser history step exists already.
  const onHash = () => {
    if (location.hash === popped) return void (popped = null);
    const named = splitHash(location.hash).text;
    const other = named && findText(props.texts, named);
    if (other && other.id !== props.text.id) return props.onText(other.id, "link"); // the new reader goes there
    goRef(hashRef() || "Genesis 1", false);
  };
  /** A fresh start: Back walks the recent entries (oldest at the bottom of this app's part of the stack; Back
   *  from there leaves the app as before). Not again for a browser history step the app already used. */
  const seedBack = (live: HistoryEntry[]) => {
    if (history.state !== null) return;
    const older = live.slice(1, 10).reverse();
    older.forEach((e, i) => (i === 0 ? history.replaceState : history.pushState).call(history, { entry: e.id }, "", hashFor(e)));
    if (older.length) history.pushState({ entry: live[0]!.id }, "", hashFor(live[0]!));
  };
  const resumeNewest = () => {
    const newest = liveHistory(store.history())[0];
    if (!newest) return false;
    resume(newest, false);
    following = true;
    return true;
  };
  /** The verse at the top of the reader, from `Bible`. */
  let topVerse: () => number | null = () => null;
  onMount(() => {
    const mine = liveHistory(store.history()).find((e) => e.id === reading());
    if (typeof props.start === "number") goTo(chapterOf(props.start), null, props.start); // another text, same place
    else if (props.start === "link") goRef(hashRef() || "Genesis 1", false); // a link that named another text
    else if (mine) resume(mine, false); // a reload: where this tab was
    else if (hashRef()) goRef(hashRef(), false); // a link
    else {
      seedBack(liveHistory(store.history()));
      // Nothing here yet: the start of the Bible, not recorded (the server's list may still bring a place).
      if (!resumeNewest()) goTo(0);
      following = true;
    }
    window.addEventListener("hashchange", onHash);
    window.addEventListener("popstate", onPop);
    onCleanup(() => {
      window.removeEventListener("hashchange", onHash);
      window.removeEventListener("popstate", onPop);
    });
  });
  // The server's list arrived (a new device, or another one read on): follow it while nothing else was chosen.
  createEffect(
    on(
      () => liveHistory(store.history())[0],
      (newest) => {
        if (!following || !newest) return;
        const shown = liveHistory(store.history()).find((e) => e.id === reading());
        if (newest.id !== shown?.id || (shown && !samePlace(index, newest.last.ord, shown.last.ord))) resumeNewest();
      },
      { defer: true },
    ),
  );
  // Another space has its own history: open where you last read there (else stay).
  createEffect(
    on(
      () => store.space,
      () => {
        setReading(null);
        sessionSet(READING_KEY, null);
        resumeNewest();
        following = true;
      },
      { defer: true },
    ),
  );
  const onPosition = (ord: number) => {
    following = false;
    const id = reading();
    if (id) store.setHistory(scrolled(index, store.history(), id, ord, Date.now()), false);
    else record(ord, false); // scrolled from a place no jump recorded (e.g. after switching spaces)
  };
  /** The next or previous chapter: reading on in the same entry. */
  const step = (ch: number) => {
    goTo(ch);
    onPosition(index.verseStart[index.chapterFirstVerse[ch]!]!);
  };
  const onLocation = (ch: number) => {
    setHere(ch);
    history.replaceState(history.state, "", hashOf(props.text.info, chapterLabel(ch)));
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
    return lo <= hi ? [lo, index.after(hi)] : null;
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
  const unhighlight = (tagId: string, iv: Interval) => store.erase(tagId, [iv]);
  const selectTag = (id: string) => {
    setActiveTag(id);
    setEraser(false);
  };
  // Another space has other tags: close whatever showed this one's.
  createEffect(
    on(
      () => store.space,
      () => {
        stopPainting();
        setNear(null);
        setSheetTag(null);
        setTagsOpen(false);
        setChips([]);
      },
      { defer: true },
    ),
  );
  // The tag sheet edits: view-only members never get it (no pencils), and lose it if made view-only while it's open.
  createEffect(() => {
    if (store.readOnly()) setSheetTag(null);
  });
  // A selected tag that no longer exists (deleted here or on another device, or its creation undone) is deselected.
  createEffect(() => {
    const id = activeTag();
    if (id && !store.tagById().has(id)) setActiveTag(null);
  });
  const chooseTag = (tag: Tag, erase = false) => {
    const range = selectionRange();
    if (range) {
      const ivs = index.byBook(range); // one piece per book
      if (erase) store.erase(tag.id, ivs);
      else store.paint(tag.id, ivs);
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
      if (near() || help() || menuOpen() || spacesOpen() || sheetTag()) {
        setNear(null);
        setHelp(false);
        setMenuOpen(false);
        setSpacesOpen(false);
        setSheetTag(null);
      } else if (tagsOpen()) setTagsOpen(false); else if (chips().length) setChips([]);
      else stopPainting();
    } else if (key === "e") {
      toggleEraser();
    } else if (key === "[" && here() > 0) {
      step(here() - 1);
    } else if (key === "]" && here() < index.chapters - 1) {
      step(here() + 1);
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
        <button class="icon" classList={{ on: menuOpen() }} onClick={() => setMenuOpen((o) => !o)} title="Menu" aria-label="Menu" aria-expanded={menuOpen()}>
          <MenuIcon />
        </button>
        <Omnibox
          text={props.text}
          store={store}
          label={chapterLabel(here())}
          shortLabel={chapterLabel(here(), "short")}
          ctx={{ book: index.bookOfChapter[here()]!, chapter: here() - index.bookFirstChapter[index.bookOfChapter[here()]!]! + 1 }}
          chips={chips()}
          onChips={(c) => {
            setChips(c);
            setNear(null);
            setSheetTag(null);
          }}
          onGo={goRef}
          badge={props.text.info.abbr}
          badgeTitle={`${props.text.info.name} (menu: Translation)`}
        />
        <div class="actions">
          <Show when={store.mode === "local"} fallback={<SyncDot store={store} />}>
            <button class="local-badge" onClick={() => setHelp((h) => !h)} title="Your tags are only stored in this browser. Click for details.">
              <span aria-hidden="true">⚠</span> <span class="local-text">Local only</span>
            </button>
          </Show>
          <SpaceButton
            store={store}
            open={spacesOpen()}
            onToggle={() => {
              setHelp(false);
              setSpacesOpen((o) => !o);
            }}
          />
        </div>
      </header>
      <Show when={menuOpen()}>
        <Menu
          store={store}
          index={index}
          here={here()}
          onChapter={(ch) => {
            setMenuOpen(false);
            jump(ch, null);
          }}
          recent={liveHistory(store.history())}
          reading={reading()}
          onResume={(e) => {
            setMenuOpen(false);
            resume(e, true);
          }}
          texts={props.texts}
          text={props.text.info}
          onText={(id) => {
            setMenuOpen(false);
            props.onText(id, topVerse());
          }}
          onSpaces={() => {
            setMenuOpen(false);
            setHelp(false);
            setSpacesOpen(true);
          }}
          onHelp={() => {
            setMenuOpen(false);
            setSpacesOpen(false);
            setHelp(true);
          }}
          onClose={() => setMenuOpen(false)}
        />
      </Show>
      <Show when={help()}>
        <Help local={store.mode === "local"} onClose={() => setHelp(false)} />
      </Show>
      <Show when={spacesOpen()}>
        <SpacesCard store={store} onClose={() => setSpacesOpen(false)} />
      </Show>

      <main class="page">
        <Bible
          text={props.text}
          store={store}
          activeTag={activeTag()}
          eraser={eraser()}
          nav={nav()}
          onLocation={onLocation}
          onPosition={onPosition}
          topRef={(f) => (topVerse = f)}
          onInspect={(ord) => {
            // Tags at this word, in palette order (then by name).
            const ids = [...new Set(store.visibleTaggings().filter((t) => t.start <= ord && ord < t.end).map((t) => t.tag))];
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
        onAdding={stopPainting}
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
        <UpdateNotice store={store} />
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
  const stalled = () => (props.store.mode === "local" ? null : props.store.stalled());
  return (
    <Show
      when={stalled()}
      fallback={
        <span class="sync" data-status={props.store.status()} title={title()} aria-label={title()}>
          <Show when={props.store.status() === "offline"}>
            <span class="sync-label">Offline{props.store.pendingCount() ? ` · ${props.store.pendingCount()}` : ""}</span>
          </Show>
        </span>
      }
    >
      {(why) => (
        // The connection gave up by itself (store.ts `connectForever`): changes stay on this device meanwhile.
        <button
          class="sync stalled"
          data-status="offline"
          title={
            why() === "outdated"
              ? "This version of the app is out of date. Reload to sync again; your changes are saved on this device."
              : "The connection kept dropping, so it stopped trying. Your changes are saved on this device."
          }
          onClick={() => (why() === "outdated" ? location.reload() : props.store.retry())}
        >
          <span class="sync-label">{why() === "outdated" ? "Reload to sync" : "Not syncing · Retry"}</span>
        </button>
      )}
    </Show>
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
        <dt>Undo</dt>
        <dd>In the menu (top left), or <kbd>⌘Z</kbd>.</dd>
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
  /** The New tag box opened: nothing is painted while naming one. */
  onAdding: () => void;
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
      if (!props.store.readOnly()) props.onEdit(press.id); // long-press without moving
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
      hidden={props.store.hidden().has(tag.id)}
      onTap={(e) => {
        if (suppressClick) return;
        setAdding(false); // the box keeps focus (wells keep the selection), so its blur never closes it
        props.onChoose(tag, e.altKey);
      }}
      onEdit={props.store.readOnly() ? undefined : () => props.onEdit(tag.id)}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!press?.touch && !props.store.readOnly()) props.onEdit(tag.id); // touch long-press is handled above
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
          when={adding() || props.store.readOnly()}
          fallback={
            <button class="well add" onClick={() => (setAdding(true), props.onAdding())} title="New tag" aria-label="New tag">
              <PlusIcon />
              <Show when={props.store.palette().length === 0}>
                <span class="well-name">New tag</span>
              </Show>
            </button>
          }
        >
          <Show when={!props.store.readOnly()}>
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
        </Show>
      </div>
      <div class="tools">
        <button class="well tool all-tags" onClick={props.onAllTags} title={hidden() > 0 ? `All tags (${hidden()} not in the palette)` : "All tags"} aria-label="All tags">
          <TagsIcon />
          <Show when={hidden() > 0}>
            <span class="badge-count">{hidden()}</span>
          </Show>
        </button>
        <Show when={!props.store.readOnly()}>
        <button class="well tool" classList={{ active: props.eraser }} onMouseDown={keepSelection} onClick={() => (setAdding(false), props.onEraser())} title={props.eraser ? "Eraser — tap to stop" : "Eraser (E)"} aria-label="Eraser" aria-pressed={props.eraser}>
          <EraserIcon />
        </button>
        </Show>
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
  /** Hidden from the text (shown dimmed). */
  hidden: boolean;
  onTap: (e: MouseEvent) => void;
  /** Absent for view-only members. */
  onEdit?: () => void;
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
    <span class="well-group" ref={group} classList={{ active: props.active, dragging: props.dragging, hidden: props.hidden }} style={{ "--c": props.tag.color }}>
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
      <Show when={props.active && props.onEdit}>
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
  const instances = createMemo(() => (props.store.byTag().get(props.tag.id) ?? []).map(intervalOf));
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
          store={props.store}
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
