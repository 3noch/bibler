# bible-tags: project context

A Bible reader where the user **tags** text (not highlights). Tags are the point: colored, renamable,
commentable, recallable. Text is the main UI element; tags sit in a palette (like paint wells); the user
"paints" a selected tag over words, or selects nothing and uses normal text selection.

Stack: **Solid.js** client (plain Vite SPA, no SSR), **Rust** server (axum + tokio + rusqlite, SQLite WAL),
local-first. No WASM (decided: duplicate small algorithms in TS and Rust, keep them honest with shared fixtures).

## Git
- Commit to `main` by default whenever a piece of work is done and checked (no feature branches during initial
  development). **Push only when the user says so.**

## Dev workflow (Makefile) and deployment (default.nix)
- **GitHub Pages (offline-only build)**: `make pages` -> `build/pages/` (client built with `VITE_SYNC=local`,
  `VITE_BASE=./`, plus the text artifacts and `.nojekyll`); `make pages-serve` previews it. `.github/workflows/pages.yml`
  builds and deploys it on push to `main` (repo Settings -> Pages -> Source: GitHub Actions). No server there: the
  app shows a "Local only" badge and help note; data lives only in that browser (see `packages/app/CLAUDE.md`).
- `default.nix` (independent of the Makefile): `textBuild` + `text` (runCommand), `server`
  (buildRustPackage, `-p server`), `client` (buildNpmPackage, `npmWorkspace = packages/app`; update
  `npmDepsHash` with `prefetch-npm-deps package-lock.json` after lockfile changes), `app` = server wrapped with
  `--static ${client} --text ${text}`. Per-derivation `lib.fileset` sources, so client changes never rebuild Rust.
  If Rust rebuilds get slow, move to crane. Uses `<nixpkgs>` (26.11, rustc 1.98); pin it before real deploys.
  Verified 2026-10-10 on macOS only; Linux (deploy target) not yet built.
- Server (`crates/server`, binary `bible-tags-server`): axum; `/api/*` (health only so far), `/text/*`
  (text-build output), SPA fallback to `index.html`; compression (br/gzip); `Cache-Control` immutable for
  content-hashed files (`/assets/*`, `name.<16hex>.ext`), `no-cache` otherwise. TLS is in-process rustls (ring
  provider, so no cmake/aws-lc) via `--tls-cert/--tls-key`; SIGHUP reloads the cert; `--http-redirect ADDR` adds a
  plain-HTTP 308-to-HTTPS listener. No ACME client yet (renew externally + SIGHUP).

## Dev environment and checks
- Rust: `rust-toolchain.toml` = `stable` (+ rustfmt, clippy); verified on 1.99.0. `rustfmt.toml` = 120 cols.
  `clippy::pedantic` is on workspace-wide (`[workspace.lints]`, each crate `lints.workspace = true`) and `make check`
  runs clippy with `-D warnings`. Narrowing casts in text-build go through `narrow32/16/8`, guarded by exact
  capacity checks in the builder (tested at the u16 chapter boundary); don't add bare `as` casts.
  Run `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets` (workspace-wide).
- Node >= 22.18 (native type stripping). Not installed globally; the user installs tools via `nix-env`.
  Ephemeral: `nix-shell -p nodejs_22 typescript --run 'node --test "test/*.test.ts" && tsc -p tsconfig.json'`
  from `packages/scripture`.
- Both normalize suites pass (28 shared fixtures + 200x150-step property run each). Mutation-checked on
  2026-10-10: adjacency, survivor age, survivor id tie-break, comment move, split-piece age, erase-abut,
  no-op update, and each output-array sort order are all caught in both languages.
- Mutation testing: do it on a COPY of the workspace in the scratchpad, never in-place, and use
  `cargo test --no-fail-fast` (otherwise a fixtures failure hides whether the property test catches it).
- A fixture failure in one language means a port bug or spec ambiguity; never "fix" it by editing fixtures
  without understanding why.

## Architecture: three layers
1. **Scripture text**: immutable, static, pre-tokenized, content-hashed files cached by a service worker. Not synced.
2. **User data** (tags, taggings, comments, palette, overlap styles): small, mutable, synced. Only layer needing sync.
3. **Derived indexes** (per-verse segments, nearby-instance results, verse buckets): in-memory memos on the client,
   never synced.

"Instant" painting = optimistic local writes. Server round trips must never be on the paint path.
Live updates (decided 2026-10-10; sync engine: see "Sync engine choice" below):
no live-query/IVM engine on the client. `normalize` already emits explicit deltas (create/update/delete/move), so
derived indexes are maintained by hand from changesets: per-tag sorted interval arrays + verse-bucket index with one
Solid signal per verse; local optimistic and server changes go through one `applyChangeset` inside `batch()`.
Parameterized queries (nearest instances to a moving position p, A&B leapfrog) are recomputed per call
(microseconds) because IVM maintains results under data changes, not under changing parameters.
Feldera-style server-side IVM is for later **cross-user aggregates**, which ARE planned long-term (e.g.
"most-tagged verses across all users"); build those server-side, periodic or incremental, when needed.

## Data model
Text is addressed by **word ordinals**: sparse u32 `ord = bookId * 2^24 + local` (decided 2026-10-10). Book ids
are stable (OT 2..40, NT 70..96, 41..69 reserved for deuterocanon, from the source's file numbering), so adding
books never shifts existing ordinals and a text fix only touches one book's ordinals. Gaps between books hold no
words (`end - start` is not a word count across books). JS must use `bookIdOf`/`localOf`, never `>>` (ids >= 128
exceed int32). ~790k words in KJV. All intervals are **half-open `[start, end)`**.
Anchors are keyed by `text_id` (versioned, e.g. `kjv-1769-v1`); other translations get their own ordinal spaces.

```sql
tags(id ULID PK, owner_id, name, description, color, created_at, updated_at, deleted_at, UNIQUE(owner_id, id))
  -- name unique per owner, case-insensitive, among non-deleted
taggings(id ULID PK, owner_id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into,
         FOREIGN KEY (owner_id, tag_id) REFERENCES tags(owner_id, id))
  -- index (owner_id, tag_id, start_ord)
comments(id, owner_id, tagging_id, body, created_at, edited_at, deleted_at)
palette(owner_id, slot, tag_id)                 -- user's top-N quick-select
overlap_styles(owner_id, combo_key, style_json) -- combo_key = sorted tag ids joined
changes(owner_id, seq, op_id, kind, payload, created_at, PRIMARY KEY(owner_id, seq), UNIQUE(owner_id, op_id))
```
- Tag identity is an opaque ULID + owner_id; name/color/description are mutable attributes. The composite FK is the
  namespacing guarantee (two users' "life" tags never collide). Owner always comes from the session, never the payload.
- Soft deletes everywhere (sync + undo). Undo/redo (Ctrl-Z) is a day-one requirement.
- Future cross-user overlay: additive `tag_id -> concept_id` equivalence mapping. Not now.
- Taggings have NO descriptions. Annotating a subsection = create another tag and comment on that. Overlap comments
  are deferred (and may be unnecessary).

## Core invariant and `normalize`
Per `(owner, tag)`, taggings are **disjoint and non-adjacent** (`a.end < b.start`). Implemented by the pure,
deterministic `normalize(existing, intent) -> Changeset` (TS: `packages/scripture/src/normalize.ts`,
Rust: `crates/normalize/src/lib.rs`).
- **Paint** over/abutting same-tag taggings merges them. Survivor = oldest by `(created_at, id)`. Others are
  tombstoned with `merged_into`, and their comments show on the survivor (displayed by timestamp). Decided
  2026-10-11: that re-pointing is **logical**: comment rows keep their `tagging` and readers follow the
  `merged_into` chain (`resolve_tagging` / `resolveTagging`), so paints never write comment rows and undoing a merge
  still works after a comment was edited. `normalize`'s `move_comments` output is therefore unused by ops.
- **Erase** trims/deletes/splits. Left piece keeps id + comments; right piece gets client-supplied `split_id` and the
  original `created_at`. Plain delete keeps comments on the tombstone (so undo restores both).
- Ids it may need (`new_id`, `split_id`) are client-supplied in the intent so optimistic == canonical.
- ids are ASCII (ULIDs) so TS and Rust string ordering agree.
- Callers may pass all of a tag's taggings or only overlapping/abutting ones (locality is a tested property).

## Sync (DONE, step 5; own event log per space)
- **Ops** (`crates/ops` Rust = canonical, `packages/scripture/src/ops.ts` = optimistic twin; spec
  `fixtures/ops.json`, generated by `fixtures/ops.py` with hand-written expectations, 78 cases): `paint`, `erase`,
  `tag_create`, `tag_update`, `tag_delete`, `palette_set`, `revert`, `comment_create/update/delete`, each with a
  client `op_id` and client-supplied ids/`now`. `effect(rows, op)` -> **Change** = row `{before, after}` pairs sorted
  by (table tag<tagging<palette<comment, id);
  soft deletes everywhere. Paint/erase wrap `normalize` (per tag AND text_id). Tag names: trimmed (Unicode
  White_Space), 1-100 code points, unique case-insensitively among live tags. Palette <= 9 live tags, one row per owner.
  Comments: body trimmed, 1-10,000 code points (`bad_body`); created on the live end of the target's merge chain
  (`unknown_tagging`, `tagging_deleted`); update sets `edited_at` (same body = no-op); deletes are soft. Erase and
  tag_delete leave comments on the tombstone (hidden; undo brings them back). No revert rule is needed for comments.
- **Undo/redo = `revert(op)`**: apply a change's `before` rows (a created row becomes a tombstone) only if every row
  is still exactly its `after` (else `conflict`), and the result keeps the invariants (no overlap/abut with taggings
  outside the change, no live tagging of a deleted tag, no deleted tag left in the palette, unique names).
  Redo = revert of the revert. A property test (Rust 300x120, TS 150x100) checks invariants after every random op
  and that undoing everything restores the start; it found the palette rule.
- **Spaces** (decided 2026-10-11, for Cloudflare Durable Objects): a **space** = one SQLite database = one sync
  stream with ONE `seq` (`changes.seq INTEGER PRIMARY KEY`; `changes.owner_id` = author). Today every owner has a
  personal space whose id is the owner id. The schema stays multi-owner (every row keeps `owner_id`; ops only see
  their owner's rows; op ids and tag names are per owner; ids are unique per space, a collision with another
  member's row -> `id_exists`) so groups can later share a space. Schema + `apply`/`changes_since`/`import` live in
  `crates/space` over a tiny synchronous `Db` trait (positional `?` params; rusqlite impl behind the `rusqlite`
  feature), so the self-hosted server and the Durable Object run the same SQL. No PRAGMAs in the shared schema (DOs
  reject them); schema version is in a `meta` table. `apply` does every check before its first write (DOs have no
  rollback); tested by asserting `total_changes()` is unchanged after each rejected fixture op.
- **Server** (`crates/server`): `--data DIR` = `accounts.sqlite` (owners, sessions; D1 on Cloudflare) +
  `spaces/<id>.sqlite`. One writer thread holds every write connection (<= 256 open, then reopened), per-space read
  pools; each op is one transaction (idempotent by `(owner, op_id)`). Commit listener publishes to `Hub` (per-space
  broadcast) **on the writer thread**, so sockets see seq order. `--migrate-from OLD.db` imports the pre-spaces
  single database once (only if `--data` doesn't exist yet; into a temp dir, then renamed); personal spaces keep the
  old per-owner seqs, so clients' caches stay valid. The Makefile passes it when `build/dev.db` exists.
  Ordinals are validated against the text index (`bad_text`), ids against `[0-9A-Za-z_-]{1,64}` (`bad_id`).
  The shared ops fixtures also run through SQLite (`crates/space/tests/fixtures.rs`, which also tests a two-owner
  space; mutation-checked 2026-10-11: owner check, per-space seq, write-before-checks, duplicate scope).
- **Protocol** (`/api/sync` WebSocket, documented in `sync.rs`): `hello{after}` -> replayed changes -> `ready` ->
  live changes; `op` -> `ack`/`reject{code}`; the change itself is the confirmation. Lag -> `error` + reconnect.
- **Auth placeholder**: `POST /api/session` creates an anonymous owner + HttpOnly SameSite=Lax cookie (Secure under
  TLS; only sha256(token) stored). Same-origin `Origin` required on both endpoints. Real auth will attach accounts
  to owners later.
- **Client** (`SyncState` in `packages/scripture/src/syncState.ts`, pure; `packages/app/src/store.ts` wraps it):
  view = confirmed base + pending ops replayed (rebase on every change; ops that no longer apply are dropped with a
  notice). Undo/redo steps = op-id lists (a multi-op step undoes in reverse). Persistence: shared base cache
  (localStorage, never regresses seq), shared **outbox** of unconfirmed ops (union across tabs, Web Lock, adopted by
  other tabs; any tab may re-send since ops are idempotent), per-tab undo (sessionStorage). Reconnect with backoff
  and on `online`. The local-only prototype's data is imported once.
- Tests: end-to-end WebSocket tests (fan-out, replay, duplicates, rejects, owner isolation, revert, auth/origin,
  restart); a client convergence property test (3 clients, random edits/delivery/disconnects, 120 seeds);
  browser-tested with two tabs incl. offline -> reconnect. Comments are in the fixtures, both property tests (with
  "every op kind applied" assertions) and the 3-client convergence test; mutation-checked 2026-10-11 (8 mutations,
  caught in both languages). Not yet: real auth, rate limits, retention of the changes log.

## Sync engine choice
- Decided 2026-10-10: our own per-owner event log (requirement: free OSS, self-hostable, no paid tier). The
  evaluation (Zero, LiveStore, Electric, Jazz, PowerSync, ...) is in `docs/sync-evaluation.md`; read it before revisiting.

## Tag queries
A query is an AND of a set S of tags. An **instance** of S = a maximal run of words where every tag in S covers
the word (superset semantics: other tags may also be present). OPEN: whether to also offer "A&B and nothing else".
- Single tag: instances are the taggings themselves (so comments show). N nearest after p:
  `start_ord > p ORDER BY start_ord LIMIT N`; before p: `start_ord < p ORDER BY start_ord DESC LIMIT N`.
  Works because same-tag taggings are disjoint (start order == end order).
- Combination (A&B...): intersection runs, annotated with contributing taggings. Use a **leapfrog seek** across the
  members' sorted interval lists (binary/galloping search per list) to find next/prev instance without
  materializing the full intersection; or iterate the rarest tag and probe the others. In SQL: range-overlap join
  `a.start < b.end AND b.start < a.end` ordered by distance, LIMIT N.
- Cross-tag "everything overlapping this window": R*Tree (1D) or `start_ord BETWEEN lo - maxLen AND hi` + filter;
  on the client use a **verse-bucket index** (verse -> ids of taggings overlapping it).
- Bitsets/Roaring were considered and **rejected** for the client (data is sparse; run lists/intervals win). Revisit
  Roaring-with-run-containers only for a server holding many users' tags.
- NOT supported, by design: NOT and OR.

## Text (frozen): see `crates/text-build/CLAUDE.md` and `docs/text-format.md`
- **Never edit or refresh `data/source/eng-kjv2006/`.** The tokenization (hence every user's ordinals) is
  frozen by `fixtures/text/kjv-1769-v1.golden.json`; changing it needs a new `text_id` plus an ordinal remap.

## Testing philosophy
Shared JSON fixtures are the behavioral spec for anything duplicated across TS and Rust. Property tests
(randomized, seeded, against a brute-force model) in both languages. Mutation-check new suites: deliberately break the
implementation and confirm tests fail. Don't claim something is verified unless it was actually run.

## Next steps
- Real auth (attach accounts to anonymous owners).
- Reader: pairs within 3+ tag combinations in the Nearby panel, overlap style rules, a book/chapter picker.
- Touch painting was tested only with synthesized PointerEvents; try it on a real phone.
