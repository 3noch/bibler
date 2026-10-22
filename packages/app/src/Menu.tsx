// The menu (top-left button): account, space, undo/redo, help and the contents (books -> chapters), in a drawer so
// the top bar fits a phone.
import { createSignal, For, Show } from "solid-js";
import { BOOKS, type TextIndex } from "@bible-tags/scripture";
import type { TagStore } from "./store.ts";
import { AccountSection } from "./Account.tsx";
import { Version } from "./Version.tsx";
import { CloseIcon, GroupIcon, HelpIcon, NextIcon, RedoIcon, UndoIcon } from "./icons.tsx";

export function Menu(props: {
  store: TagStore;
  index: TextIndex;
  /** Global chapter being read. */
  here: number;
  onChapter: (ch: number) => void;
  onSpaces: () => void;
  onHelp: () => void;
  onClose: () => void;
}) {
  const s = props.store;
  return (
    <>
      <div class="scrim" onClick={props.onClose} />
      <aside class="menu" role="dialog" aria-label="Menu">
        <div class="menu-head">
          <button class="icon" onClick={props.onClose} aria-label="Close menu">
            <CloseIcon />
          </button>
        </div>
        <AccountSection store={s} />
        <Show when={s.mode === "server"}>
          <section class="menu-section">
            <button class="menu-row" onClick={props.onSpaces}>
              <GroupIcon />
              <span class="menu-label">
                {s.group()?.name ?? "My tags"}
                <span class="muted">{s.group() ? (s.readOnly() ? "Group · view only" : "Group") : "Groups"}</span>
              </span>
              <NextIcon />
            </button>
          </section>
        </Show>
        <section class="menu-section undo-row">
          <button class="menu-row" disabled={!s.canUndo()} onClick={() => s.undo()} title="Undo (⌘Z)">
            <UndoIcon /> Undo
          </button>
          <button class="menu-row" disabled={!s.canRedo()} onClick={() => s.redo()} title="Redo (⇧⌘Z)">
            <RedoIcon /> Redo
          </button>
        </section>
        <section class="menu-section">
          <button class="menu-row" onClick={props.onHelp}>
            <HelpIcon />
            <span class="menu-label">Help</span>
          </button>
        </section>
        <Contents index={props.index} here={props.here} onChapter={props.onChapter} />
        <Version server={s.mode === "server"} />
      </aside>
    </>
  );
}

/** Books by testament; a book opens its chapter grid (the current book starts open). One-chapter books go straight
 *  to their chapter. */
function Contents(props: { index: TextIndex; here: number; onChapter: (ch: number) => void }) {
  const ix = props.index;
  const [open, setOpen] = createSignal<number | null>(ix.bookOfChapter[props.here]!);
  const books = Array.from({ length: ix.books }, (_, b) => b);
  const testaments = [
    { name: "Old Testament", books: books.filter((b) => ix.bookId[b]! < 70) },
    { name: "New Testament", books: books.filter((b) => ix.bookId[b]! >= 70) },
  ];
  return (
    <nav class="menu-section contents" aria-label="Contents">
      <For each={testaments}>
        {(t) => (
          <>
            <h3>{t.name}</h3>
            <ul>
              <For each={t.books}>
                {(b) => {
                  const n = ix.chapterCount(b);
                  const first = ix.bookFirstChapter[b]!;
                  const current = () => ix.bookOfChapter[props.here] === b;
                  return (
                    <li>
                      <button
                        class="book-row"
                        classList={{ current: current() }}
                        aria-expanded={n > 1 ? open() === b : undefined}
                        onClick={() => (n === 1 ? props.onChapter(first) : setOpen(open() === b ? null : b))}
                      >
                        {BOOKS[b]!.name}
                        <Show when={n > 1}>
                          <span class="muted">{n}</span>
                        </Show>
                      </button>
                      <Show when={n > 1 && open() === b}>
                        <div class="chapter-grid">
                          <For each={Array.from({ length: n }, (_, i) => i)}>
                            {(i) => (
                              <button classList={{ current: props.here === first + i }} onClick={() => props.onChapter(first + i)}>
                                {i + 1}
                              </button>
                            )}
                          </For>
                        </div>
                      </Show>
                    </li>
                  );
                }}
              </For>
            </ul>
          </>
        )}
      </For>
    </nav>
  );
}
