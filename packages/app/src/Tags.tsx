// All tags: find or create a tag, pick one to paint with, pin/unpin it in the palette, or open its editor. The
// palette holds at most MAX_PALETTE tags; everything else lives here.
import { createMemo, createSignal, For, Show } from "solid-js";
import { MAX_PALETTE } from "@bible-tags/scripture";
import type { Tag, TagStore } from "./store.ts";
import { CloseIcon, PencilIcon, PinIcon, PlusIcon } from "./icons.tsx";

export function TagsSheet(props: {
  store: TagStore;
  active: string | null;
  onChoose: (tag: Tag) => void;
  /** A tag was created here (it becomes the active tag). */
  onCreated: (tag: Tag) => void;
  onEdit: (id: string) => void;
  onClose: () => void;
}) {
  const [filter, setFilter] = createSignal("");
  const pinned = createMemo(() => new Set(props.store.paletteIds()));
  const full = () => props.store.paletteIds().length >= MAX_PALETTE;
  const rows = createMemo(() => {
    const q = filter().trim().toLowerCase();
    const pal = props.store.paletteIds();
    return props.store
      .tags()
      .filter((t) => t.name.toLowerCase().includes(q))
      .sort((a, b) => {
        const pa = pal.indexOf(a.id);
        const pb = pal.indexOf(b.id);
        if (pa >= 0 || pb >= 0) return (pa < 0 ? Infinity : pa) - (pb < 0 ? Infinity : pb);
        return a.name.localeCompare(b.name);
      });
  });
  const togglePin = (t: Tag) => {
    const pal = props.store.paletteIds();
    if (pal.includes(t.id)) props.store.setPalette(pal.filter((id) => id !== t.id));
    else if (!full()) props.store.setPalette([...pal, t.id]);
  };
  const places = (id: string) => props.store.byTag().get(id)?.length ?? 0;
  /** The typed name, if no tag has it yet (names are unique case-insensitively). */
  const newName = createMemo(() => {
    const name = filter().trim();
    return name && !props.store.tags().some((t) => t.name.toLowerCase() === name.toLowerCase()) ? name : null;
  });
  const create = () => {
    const name = newName();
    if (!name) return;
    const tag = props.store.createTag(name);
    if (tag) {
      setFilter("");
      props.onCreated(tag);
    }
  };

  return (
    <aside class="sheet tags-sheet" role="dialog" aria-label="All tags">
      <div class="sheet-head">
        <div class="search-title">
          <h2>Tags</h2>
          <p class="muted">
            {props.store.tags().length} · {props.store.paletteIds().length}/{MAX_PALETTE} in the palette
          </p>
        </div>
        <button class="icon" onClick={props.onClose} aria-label="Close">
          <CloseIcon />
        </button>
      </div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const exact = rows().find((t) => t.name.toLowerCase() === filter().trim().toLowerCase());
          if (newName()) create();
          else if (exact) props.onChoose(exact);
        }}
      >
        <input
          class="tags-filter"
          placeholder="Find or create a tag…"
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
          aria-label="Find or create a tag"
          ref={(el) => queueMicrotask(() => el.focus())}
        />
      </form>
      <ul class="tags-list nearby-body">
        <Show when={newName()}>
          <li class="create-row">
            <button class="tag-row" onClick={create}>
              <span class="create-icon" aria-hidden="true">
                <PlusIcon />
              </span>
              <span class="nearby-name">Create “{newName()}”</span>
              <kbd>↵</kbd>
            </button>
          </li>
        </Show>
        <For each={rows()} fallback={<Show when={!newName()}><li class="empty">No tags yet — type a name above to create one.</li></Show>}>
          {(t) => (
            <li classList={{ active: props.active === t.id }} style={{ "--c": t.color }}>
              <button class="tag-row" onClick={() => props.onChoose(t)} title={`Paint with ${t.name}`}>
                <span class="swatch" />
                <span class="nearby-name">{t.name}</span>
                <span class="muted">{places(t.id)}</span>
              </button>
              <button
                class="icon small pin"
                classList={{ on: pinned().has(t.id) }}
                disabled={!pinned().has(t.id) && full()}
                onClick={() => togglePin(t)}
                title={pinned().has(t.id) ? "Remove from palette" : full() ? `The palette holds ${MAX_PALETTE} tags; unpin one first` : "Add to palette"}
                aria-label={pinned().has(t.id) ? `Unpin ${t.name}` : `Pin ${t.name}`}
                aria-pressed={pinned().has(t.id)}
              >
                <PinIcon filled={pinned().has(t.id)} />
              </button>
              <button class="icon small" onClick={() => props.onEdit(t.id)} aria-label={`Edit ${t.name}`} title="Edit">
                <PencilIcon />
              </button>
            </li>
          )}
        </For>
      </ul>
    </aside>
  );
}
