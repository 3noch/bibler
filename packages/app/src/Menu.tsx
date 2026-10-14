// The menu (top-left button): account, space, undo/redo and help, in a drawer so the top bar fits a phone.
import { Show } from "solid-js";
import type { TagStore } from "./store.ts";
import { AccountSection } from "./Account.tsx";
import { CloseIcon, GroupIcon, HelpIcon, NextIcon, RedoIcon, UndoIcon } from "./icons.tsx";

export function Menu(props: { store: TagStore; onSpaces: () => void; onHelp: () => void; onClose: () => void }) {
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
      </aside>
    </>
  );
}
