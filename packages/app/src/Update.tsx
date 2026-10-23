// A new client is out when the client version being served differs from this page's own (`<meta name="build-client">`,
// which changes only when the client does: see vite.config.ts). The served one comes with each sync `ready` and with
// the `pong` the socket asks for when the page comes back after a while (store.ts: a socket that stays open may see the
// client replaced). The offline-only build has no socket, so it fetches `client-version.txt` on coming back instead. An installed app
// resumed from the background never reloads by itself, so: reload while the page is hidden or just after it comes
// back (the user isn't looking at, or hasn't started on, the old one yet), else offer a Reload button. Unsent edits
// survive a reload (outbox); typed text doesn't, so a page with a draft only gets the button.
import { createEffect, createSignal, on, onCleanup, Show } from "solid-js";
import type { TagStore } from "./store.ts";

const MINE = document.querySelector<HTMLMetaElement>('meta[name="build-client"]')?.content ?? ""; // none in dev
const RELOADED = "bible-tags/reloaded-for";
/** How long after coming back to the page an update still reloads it without asking. */
const RESUME_MS = 10_000;
/** Offline-only build: hidden at least this long, coming back asks for the served version (as store.ts's ping). */
const CHECK_AFTER_MS = 30_000;

/** The client version being served now, or null if it can't be had (offline, dev). */
async function served(): Promise<string | null> {
  try {
    const res = await fetch(`${import.meta.env.BASE_URL}client-version.txt`, { cache: "no-store" });
    return res.ok ? (await res.text()).trim() || null : null;
  } catch {
    return null;
  }
}

/** Text typed but not saved: a comment or name being written. */
const drafting = () =>
  [...document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("textarea, input")].some(
    (el) => el === document.activeElement || (el.tagName === "TEXTAREA" && el.value.trim() !== ""),
  );

export function UpdateNotice(props: { store: TagStore }) {
  const [latest, setLatest] = createSignal<string | null>(null); // the latest word on what is served
  createEffect(on(props.store.serverClient, (v) => v && setLatest(v)));
  const available = () => {
    const theirs = latest();
    return !!MINE && !!theirs && theirs !== MINE;
  };
  let shownAt = document.hidden ? 0 : performance.now();
  let hiddenAt: number | null = document.hidden ? performance.now() : null;
  const reload = () => {
    try {
      sessionStorage.setItem(RELOADED, latest() ?? "");
    } catch {
      // private mode: no loop guard, the button still works
    }
    location.reload();
  };
  /** Reload without asking, unless the user may be busy, or a reload for this version already happened (and
   *  didn't take: never loop). */
  const maybeReload = () => {
    if (!available() || drafting()) return;
    let already = false;
    try {
      already = sessionStorage.getItem(RELOADED) === latest();
    } catch {
      already = true;
    }
    if (!already && (document.hidden || performance.now() - shownAt < RESUME_MS)) reload();
  };
  const onVisibility = () => {
    if (document.hidden) hiddenAt ??= performance.now();
    else {
      shownAt = performance.now();
      const check = props.store.mode === "local" && MINE && hiddenAt !== null && shownAt - hiddenAt >= CHECK_AFTER_MS;
      if (check) void served().then((v) => v && setLatest(v));
      hiddenAt = null;
    }
    maybeReload();
  };
  document.addEventListener("visibilitychange", onVisibility);
  onCleanup(() => document.removeEventListener("visibilitychange", onVisibility));
  createEffect(() => {
    if (available()) maybeReload();
  });
  return (
    <Show when={available()}>
      <p class="toast update">
        A new version is available.
        <button onClick={reload}>Reload</button>
      </p>
    </Show>
  );
}
