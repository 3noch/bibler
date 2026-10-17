// Service worker (installable PWA + offline reading). Generated into the build by `serviceWorker()` in
// vite.config.ts, which replaces the two placeholders below; never registered in dev.
// Paths are relative to this file, which sits at the app's base (site root, or /<repo>/ on GitHub Pages).
//   app shell (index, hashed assets, icons)  precached per version; old versions are deleted on activate
//   text (index + every book, ~1.7 MB gz)    precached when the page asks (a "precache-text" message, sent once
//                                            the worker is ready) into a cache keyed by the text id; the
//                                            content-hashed files never change, so they are served cache-first.
//                                            Not during activate: the page's requests wait while a worker that
//                                            controls it is still activating, so a slow download stalled the app
//   /api/*                                   never touched (sync goes over a WebSocket anyway)
const VERSION = "__VERSION__";
const SHELL_FILES = __SHELL__;
const TEXT_ID = "__TEXT_ID__";
const SHELL = `shell-${VERSION}`;
const TEXT = `text-${TEXT_ID}`;
const HASHED = /\/assets\/|\.[0-9a-f]{16}\.[a-z]+$/;

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(SHELL).then((c) => c.addAll(["./", ...SHELL_FILES])).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    (async () => {
      for (const k of await caches.keys()) if (k !== SHELL && k !== TEXT) await caches.delete(k);
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (e) => {
  if (e.data === "precache-text") e.waitUntil(precacheText());
});

async function precacheText() {
  const base = `text/${TEXT_ID}/`;
  const cache = await caches.open(TEXT);
  const res = await fetch(base + "manifest.json", { cache: "no-cache" });
  if (!res.ok) return;
  await cache.put(base + "manifest.json", res.clone());
  const m = await res.json();
  const todo = [m.index, ...m.books.map((b) => b.file)].map((f) => base + f);
  // A few at a time; whatever fails is retried on the next visit.
  const next = async () => {
    for (let url = todo.shift(); url; url = todo.shift()) if (!(await cache.match(url))) await cache.add(url).catch(() => {});
  };
  await Promise.all(Array.from({ length: 4 }, next));
}

self.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (req.method !== "GET" || url.origin !== location.origin || url.pathname.includes("/api/")) return;
  if (req.mode === "navigate") e.respondWith(networkFirst(req, SHELL, "./"));
  else if (HASHED.test(url.pathname)) e.respondWith(cacheFirst(req, url.pathname.includes("/text/") ? TEXT : SHELL));
  else e.respondWith(networkFirst(req, url.pathname.includes("/text/") ? TEXT : SHELL));
});

async function cacheFirst(req, name) {
  const hit = await caches.match(req);
  if (hit) return hit;
  const res = await fetch(req);
  if (res.ok) (await caches.open(name)).put(req, res.clone());
  return res;
}

// Online: always the latest (so a deploy shows up on the next load). Offline: the cached copy.
async function networkFirst(req, name, fallback) {
  try {
    const res = await fetch(req);
    if (res.ok) (await caches.open(name)).put(fallback ?? req, res.clone());
    return res;
  } catch (err) {
    const hit = await caches.match(fallback ?? req);
    if (hit) return hit;
    throw err;
  }
}
