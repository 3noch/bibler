// Keeps the screen from dimming while reading (Screen Wake Lock API: Chrome, Edge, Safari 16.4+; elsewhere the
// screen dims as usual). Held while the page is visible and the reader is reading: let go IDLE_MS after the last
// scroll, tap or key, so a phone put down still goes to sleep, and taken again by the next one. The browser lets it
// go by itself when the page is hidden. Per device: Aa -> "Keep the screen on" (reading.ts `awake`).
import { createEffect } from "solid-js";
import { awake } from "./reading.ts";

const IDLE_MS = 5 * 60_000;
/** Activity closer together than this doesn't restart the idle timer (scroll events come many a second). */
const THROTTLE_MS = 1000;

let lock: WakeLockSentinel | null = null;
let asking = false;
let idle = 0;
let last = 0;

async function take() {
  if (lock || asking || document.hidden || !awake() || !("wakeLock" in navigator)) return;
  asking = true;
  try {
    const l = await navigator.wakeLock.request("screen");
    l.addEventListener("release", () => {
      if (lock === l) lock = null;
    });
    lock = l;
  } catch {
    // refused (low battery, not allowed here): the screen dims as usual
  } finally {
    asking = false;
  }
}

function release() {
  void lock?.release();
  lock = null;
}

function activity() {
  const now = performance.now();
  if (now - last < THROTTLE_MS && lock) return;
  last = now;
  void take();
  clearTimeout(idle);
  idle = window.setTimeout(release, IDLE_MS);
}

/** Start keeping the screen on while the user reads (once, at startup). */
export function keepAwakeWhileReading() {
  for (const ev of ["scroll", "wheel", "pointerdown", "keydown", "touchstart"]) addEventListener(ev, activity, { passive: true });
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) activity(); // the lock went with the page; coming back is activity
  });
  createEffect(() => (awake() ? activity() : release()));
}
