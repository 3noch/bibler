// A text its license limits (text-build `limits`; here the BSB as a stand-in) is read a page at a time: its book
// files aren't served, only pages within its limits (500 verses or half a book). Other texts are served whole.
import assert from "node:assert/strict";
const B = process.argv[2];
const get = (p) => fetch(B + p);

const m = await (await get("/text/bsb-restricted-v1/manifest.json")).json();
assert.deepEqual(m.limits, { verses: 500, book_percent: 50 });
assert.equal((await get(`/text/bsb-restricted-v1/${m.index}`)).status, 200, "its index (no words) is served");
assert.equal((await get(`/text/bsb-restricted-v1/${m.books[0].file}`)).status, 404, "its book files aren't");

const page = await get("/api/text/bsb-restricted-v1/page?from=0&to=2"); // Genesis 1: the title slot, 1:1, 1:2
assert.equal(page.status, 200);
assert.equal(page.headers.get("cache-control"), "no-store");
const json = await page.json();
assert.equal(json.books[0].words.slice(0, 4).join(" "), "In the beginning God");
assert.equal((await get("/api/text/bsb-restricted-v1/page?from=0&to=600")).status, 403, "over 500 verses");
assert.equal((await get("/api/text/bsb-restricted-v1/page?from=5&to=2")).status, 400);
assert.equal((await get("/api/text/bsb-2026-v1/page?from=0&to=2")).status, 404, "a text served whole has no pages");

const open = await (await get("/text/bsb-2026-v1/manifest.json")).json();
assert.equal((await get(`/text/bsb-2026-v1/${open.books[0].file}`)).status, 200, "the BSB itself is served whole");
console.log("cloudflare pages e2e: ok");
