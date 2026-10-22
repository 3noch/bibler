// A new client is out when the client version the server serves (each sync `ready`) differs from this page's own
// (`<meta name="build-client">`, which changes only when the client does: see vite.config.ts). An installed app
// resumed from the background never reloads by itself, so: reload while the page is hidden or just after it comes
// back (the user isn't looking at, or hasn't started on, the old one yet), else offer a Reload button. Unsent edits
// survive a reload (outbox); typed text doesn't, so a page with a draft only gets the button.
import { createEffect, onCleanup, Show } from "solid-js";
import type { TagStore } from "./store.ts";

const MINE = document.querySelector<HTMLMetaElement>('meta[name="build-client"]')?.content ?? ""; // none in dev
const RELOADED = "bible-tags/reloaded-for";
/** How long after coming back to the page an update still reloads it without asking. */
const RESUME_MS = 10_000;

/** Text typed but not saved: a comment or name being written. */
const drafting = () =>
  [...document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("textarea, input")].some(
    (el) => el === document.activeElement || (el.tagName === "TEXTAREA" && el.value.trim() !== ""),
  );

export function UpdateNotice(props: { store: TagStore }) {
  const available = () => {
    const theirs = props.store.serverClient();
    return !!MINE && !!theirs && theirs !== MINE;
  };
  let shownAt = document.hidden ? 0 : performance.now();
  const reload = () => {
    try {
      sessionStorage.setItem(RELOADED, props.store.serverClient() ?? "");
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
      already = sessionStorage.getItem(RELOADED) === props.store.serverClient();
    } catch {
      already = true;
    }
    if (!already && (document.hidden || performance.now() - shownAt < RESUME_MS)) reload();
  };
  const onVisibility = () => {
    if (!document.hidden) shownAt = performance.now();
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
