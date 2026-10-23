// Service worker (installable PWA + offline reading). Generated into the build by `serviceWorker()` in
// vite.config.ts, which replaces the two placeholders below; never registered in dev.
// Paths are relative to this file, which sits at the app's base (site root, or /<repo>/ on GitHub Pages).
//   app shell (index, hashed assets, icons)  precached per version; old versions are deleted on activate
//   text/texts.json (the texts on offer)     network-first into its own cache (kept across versions)
//   each text (index + every book, ~1.8 MB   one cache per text, `text-<id>`. The page names the texts this device
//   gz)                                      keeps (a "keep-texts" message: the few most recently read, see
//                                            src/text.ts): each is downloaded whole, files an older build of it left
//                                            are deleted, and every other text's cache is dropped. Its
//                                            content-hashed files never change, so they are served cache-first.
//                                            Not during activate: the page's requests wait while a worker that
//                                            controls it is still activating, so a slow download stalled the app
//   /api/*, client-version.txt               never touched (sync goes over a WebSocket anyway; the version file
//                                            must say what is served now, never a cached answer)
const VERSION = "__VERSION__";
const SHELL_FILES = __SHELL__;
const SHELL = `shell-${VERSION}`;
const LIST = "texts";
const TEXT = "text-";
const HASHED = /\/assets\/|\.[0-9a-f]{16}\.[a-z]+$/;
/** `.../text/<id>/<file>` -> id */
const TEXT_FILE = /\/text\/([^/]+)\/[^/]+$/;

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(SHELL).then((c) => c.addAll(["./", ...SHELL_FILES])).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    (async () => {
      for (const k of await caches.keys()) if (k !== SHELL && k !== LIST && !k.startsWith(TEXT)) await caches.delete(k);
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (e) => {
  if (e.data?.type === "keep-texts" && Array.isArray(e.data.texts)) e.waitUntil(keepTexts(e.data.texts.map(String)));
});

// One at a time: two pages (or a quick switch) must not interleave their deletions with a download.
let queue = Promise.resolve();
function keepTexts(ids) {
  queue = queue.then(async () => {
    for (const k of await caches.keys()) if (k.startsWith(TEXT) && !ids.includes(k.slice(TEXT.length))) await caches.delete(k);
    for (const id of ids) await precacheText(id);
  }).catch(() => {});
  return queue;
}

async function precacheText(id) {
  const base = new URL(`text/${id}/`, self.registration.scope).href;
  const cache = await caches.open(TEXT + id);
  const res = await fetch(base + "manifest.json", { cache: "no-cache" }).catch(() => null);
  if (!res?.ok) return;
  await cache.put(base + "manifest.json", res.clone());
  const m = await res.json();
  const want = new Set([base + "manifest.json", ...[m.index, ...m.books.map((b) => b.file)].map((f) => base + f)]);
  // Files of an older build of this text (a heading fixed, say: new hashes) are no longer read.
  for (const req of await cache.keys()) if (!want.has(req.url)) await cache.delete(req);
  const todo = [...want];
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
  if (url.pathname.endsWith("/client-version.txt")) return;
  if (req.mode === "navigate") return e.respondWith(networkFirst(req, SHELL, "./"));
  if (url.pathname.endsWith("/text/texts.json")) return e.respondWith(networkFirst(req, LIST));
  const text = TEXT_FILE.exec(url.pathname);
  const name = text ? TEXT + decodeURIComponent(text[1]) : SHELL;
  e.respondWith(HASHED.test(url.pathname) ? cacheFirst(req, name) : networkFirst(req, name));
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
