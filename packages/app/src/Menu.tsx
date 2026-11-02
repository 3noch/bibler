// The menu (top-left button), a drawer so the top bar fits a phone. Its header holds the things seldom needed as
// icons: Back/Forward (this tab's jumps, as the browser's buttons; an installed app has none), the text's font and
// size (reading.ts; "Aa" opens them under the header) and help. Below it:
// account, space, the translation, recent reading (history.ts) and the contents (books -> chapters).
import { createSignal, For, Show } from "solid-js";
import { BOOKS, formatInterval, type HistoryEntry, type TextIndex } from "@bible-tags/scripture";
import type { TagStore } from "./store.ts";
import type { TextInfo } from "./text.ts";
import { AccountSection } from "./Account.tsx";
import { Version } from "./Version.tsx";
import { FONTS, SIZES, awake, font, layout, resetSize, setAwake, setFont, setLayout, size, stepSize } from "./reading.ts";
import { BackIcon, CheckIcon, CloseIcon, CloudIcon, ForwardIcon, GroupIcon, HelpIcon, NextIcon, OfflineIcon } from "./icons.tsx";

export function Menu(props: {
  store: TagStore;
  index: TextIndex;
  /** Global chapter being read. */
  here: number;
  onChapter: (ch: number) => void;
  /** Reading history, newest first, and this tab's entry. */
  recent: HistoryEntry[];
  reading: string | null;
  onResume: (e: HistoryEntry) => void;
  /** The texts on offer and the one being read. */
  texts: TextInfo[];
  text: TextInfo;
  onText: (id: string) => void;
  onSpaces: () => void;
  /** Where Back and Forward go (null: nowhere; `to` names the place when known). */
  back: { to?: string } | null;
  forward: { to?: string } | null;
  onBack: () => void;
  onForward: () => void;
  onHelp: () => void;
  /** Change how the text looks, keeping the verse at the top. */
  onReading: (change: () => void) => void;
  onClose: () => void;
}) {
  const s = props.store;
  const [reading, setReading] = createSignal(false);
  return (
    <>
      <div class="scrim" onClick={props.onClose} />
      <aside class="menu" role="dialog" aria-label="Menu">
        <div class="menu-head">
          <button class="icon" onClick={props.onClose} aria-label="Close menu">
            <CloseIcon />
          </button>
          <div class="menu-tools">
            <button class="icon" disabled={!props.back} onClick={props.onBack} aria-label={props.back?.to ? `Back to ${props.back.to}` : "Back"} title={props.back?.to ? `Back to ${props.back.to}` : "Back"}>
              <BackIcon />
            </button>
            <button class="icon" disabled={!props.forward} onClick={props.onForward} aria-label={props.forward?.to ? `Forward to ${props.forward.to}` : "Forward"} title={props.forward?.to ? `Forward to ${props.forward.to}` : "Forward"}>
              <ForwardIcon />
            </button>
            <span class="menu-tools-gap" />
            <button
              class="icon text-icon"
              aria-pressed={reading()}
              aria-expanded={reading()}
              onClick={() => setReading(!reading())}
              aria-label="Text font and size"
              title="Text font and size"
            >
              Aa
            </button>
            <button class="icon" onClick={props.onHelp} aria-label="Help" title="Help">
              <HelpIcon />
            </button>
          </div>
        </div>
        <Show when={reading()}>
          <Reading onReading={props.onReading} limited={!!props.text.limits} />
        </Show>
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
        <Texts texts={props.texts} text={props.text} onText={props.onText} />
        <Recent index={props.index} recent={props.recent} reading={props.reading} onResume={props.onResume} />
        <Contents index={props.index} here={props.here} onChapter={props.onChapter} />
        <Version server={s.mode === "server" ? s.serverBuild() : undefined} />
      </aside>
    </>
  );
}

/** "Genesis 12:3–15:4": from where the entry's sitting began to where it got (or one verse). */
function span(ix: TextIndex, e: HistoryEntry): string {
  const [a, b] = [ix.slotOf(Math.min(e.anchor.ord, e.last.ord)), ix.slotOf(Math.max(e.anchor.ord, e.last.ord))];
  return formatInterval(ix, [ix.verseStart[a]!, ix.slotEnd(b)]);
}

function ago(at: number): string {
  const m = Math.round((Date.now() - at) / 60_000);
  if (m < 1) return "just now";
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  return d === 1 ? "yesterday" : d < 30 ? `${d} days ago` : new Date(at).toLocaleDateString();
}

/** The translation being read, and the others on offer with their rights lines. Tags show only on the text they
 *  were made in (another text's ordinals mean other words). */
function Texts(props: { texts: TextInfo[]; text: TextInfo; onText: (id: string) => void }) {
  const [open, setOpen] = createSignal(false);
  return (
    <nav class="menu-section texts" aria-label="Translation">
      <button class="menu-row" aria-expanded={open()} onClick={() => setOpen(!open())}>
        <span class="menu-label">
          {props.text.name}
          <span class="muted">Translation · {props.text.abbr}</span>
        </span>
        <NextIcon />
      </button>
      <Show when={open()}>
        <ul>
          <For each={props.texts}>
            {(t) => (
              <li>
                <button class="text-row" classList={{ current: t.id === props.text.id }} aria-current={t.id === props.text.id} onClick={() => props.onText(t.id)}>
                  <span class="text-name">
                    {t.name} <span class="muted">{t.abbr}</span>
                    <Show when={t.id === props.text.id}>
                      <span class="text-reading">
                        <CheckIcon /> Reading
                      </span>
                    </Show>
                  </span>
                  <span class="muted rights">{t.rights}</span>
                  <span class="muted text-access">
                    {t.limits ? <CloudIcon /> : <OfflineIcon />}
                    {t.limits ? "Online only, a page at a time (its license)" : "Can be read offline"}
                  </span>
                </button>
              </li>
            )}
          </For>
        </ul>
        <p class="muted texts-note">Tags show in the translation they were made in.</p>
      </Show>
    </nav>
  );
}

/** The text's font and size (this device's: reading.ts). */
function Reading(props: { onReading: (change: () => void) => void; limited: boolean }) {
  const percent = () => `${Math.round(SIZES[size()]! * 100)}%`;
  return (
    <section class="menu-section reading" aria-label="Text font and size">
      <div class="choices" role="group" aria-label="Font">
        <For each={FONTS}>
          {(f) => (
            <button aria-pressed={font() === f.id} onClick={() => props.onReading(() => setFont(f.id))}>
              <span class="sample" style={{ "font-family": f.stack }}>
                Aa
              </span>
              {f.name}
            </button>
          )}
        </For>
      </div>
      <div class="choices layout" role="group" aria-label="Layout">
        <For each={[["scroll", "Scroll"], ["pages", "Chapter by chapter"]] as const}>
          {([id, name]) => (
            <button
              aria-pressed={(props.limited ? "pages" : layout()) === id}
              disabled={props.limited}
              onClick={() => props.onReading(() => setLayout(id))}
            >
              {name}
            </button>
          )}
        </For>
      </div>
      <Show when={props.limited}>
        <p class="muted layout-note">This translation is read a page at a time: its license limits how much may show at once.</p>
      </Show>
      <div class="size-row" role="group" aria-label="Text size">
        <button disabled={size() === 0} onClick={() => props.onReading(() => stepSize(-1))} aria-label="Smaller text">
          <span class="small">A</span>
        </button>
        <button class="size-value" onClick={() => props.onReading(resetSize)} title="Back to 100%" aria-label={`Text size ${percent()}; reset`}>
          {percent()}
        </button>
        <button disabled={size() === SIZES.length - 1} onClick={() => props.onReading(() => stepSize(1))} aria-label="Larger text">
          <span class="large">A</span>
        </button>
      </div>
      <Show when={"wakeLock" in navigator}>
        <label class="awake-row">
          <input type="checkbox" checked={awake()} onChange={(e) => setAwake(e.currentTarget.checked)} />
          Keep the screen on while reading
        </label>
      </Show>
    </section>
  );
}

/** Recent reading: each track's last sitting, newest first; choosing one goes back to where it left off. */
function Recent(props: { index: TextIndex; recent: HistoryEntry[]; reading: string | null; onResume: (e: HistoryEntry) => void }) {
  const [open, setOpen] = createSignal(false);
  return (
    <Show when={props.recent.length > 0}>
      <nav class="menu-section recent" aria-label="Recent reading">
        <button class="menu-row" aria-expanded={open()} onClick={() => setOpen(!open())}>
          <span class="menu-label">
            Recent reading
            <span class="muted">{props.recent.length} {props.recent.length === 1 ? "place" : "places"}</span>
          </span>
          <NextIcon />
        </button>
        <Show when={open()}>
          <ul>
            <For each={props.recent}>
              {(e) => (
                <li>
                  <button class="recent-row" classList={{ current: e.id === props.reading }} onClick={() => props.onResume(e)}>
                    {span(props.index, e)}
                    <span class="muted">{ago(Math.max(e.anchor.at, e.last.at))}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </nav>
    </Show>
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
