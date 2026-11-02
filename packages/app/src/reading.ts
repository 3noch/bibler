// How the Bible text looks: its font and size, chosen per device (a phone at arm's length wants another size than a
// laptop, and devices have different fonts installed), kept in localStorage, not synced. Applied as CSS variables on
// the root (`--read-font`, `--read-scale`), which only the reader's styles use: the top bar and menus keep theirs.
// The fonts are system ones (nothing to download, so offline reading is unchanged); each stack falls back to a
// close relative where the first choice isn't installed. Also the layout: one continuous scroll, or a chapter at a
// time (pages); a text its license limits is always read in pages (Bible.tsx).
import { createSignal } from "solid-js";

export interface ReadingFont {
  id: string;
  name: string;
  stack: string;
}

export const FONTS: ReadingFont[] = [
  { id: "serif", name: "Serif", stack: "var(--serif)" },
  { id: "book", name: "Book", stack: 'Charter, "Bitstream Charter", "Sitka Text", Cambria, Georgia, serif' },
  { id: "sans", name: "Sans", stack: "var(--sans)" },
];

/** Text sizes, as multiples of the default. */
export const SIZES = [0.8, 0.9, 1, 1.1, 1.2, 1.35, 1.5, 1.7, 1.9];
const DEFAULT_SIZE = SIZES.indexOf(1);

const KEY = "bible-tags/reading/v1";

export type Layout = "scroll" | "pages";

function load(): { font: string; size: number; layout: Layout; awake: boolean } {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null") as { font?: unknown; size?: unknown; layout?: unknown; awake?: unknown } | null;
    const font = FONTS.some((f) => f.id === v?.font) ? (v!.font as string) : FONTS[0]!.id;
    const size = typeof v?.size === "number" && SIZES.includes(v.size) ? SIZES.indexOf(v.size) : DEFAULT_SIZE;
    return { font, size, layout: v?.layout === "pages" ? "pages" : "scroll", awake: v?.awake !== false };
  } catch {
    return { font: FONTS[0]!.id, size: DEFAULT_SIZE, layout: "scroll", awake: true };
  }
}

const initial = load();
const [font, setFontSignal] = createSignal(initial.font);
/** Index into SIZES. */
const [size, setSizeSignal] = createSignal(initial.size);
const [layout, setLayoutSignal] = createSignal<Layout>(initial.layout);
/** Keep the screen from dimming while reading (wake.ts). */
const [awake, setAwakeSignal] = createSignal(initial.awake);
export { awake, font, layout, size };

function apply() {
  const root = document.documentElement.style;
  root.setProperty("--read-font", (FONTS.find((f) => f.id === font()) ?? FONTS[0]!).stack);
  root.setProperty("--read-scale", String(SIZES[size()]));
  try {
    localStorage.setItem(KEY, JSON.stringify({ font: font(), size: SIZES[size()], layout: layout(), awake: awake() }));
  } catch {
    // private mode: this visit only
  }
}
apply();

export function setFont(id: string) {
  setFontSignal(id);
  apply();
}

/** Step the size by `by` (clamped to SIZES). */
export function stepSize(by: number) {
  setSizeSignal(Math.min(SIZES.length - 1, Math.max(0, size() + by)));
  apply();
}

export function setAwake(on: boolean) {
  setAwakeSignal(on);
  apply();
}

export function setLayout(l: Layout) {
  setLayoutSignal(l);
  apply();
}

export function resetSize() {
  setSizeSignal(DEFAULT_SIZE);
  apply();
}
