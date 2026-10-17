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
- **Cloudflare**: `make cloudflare-dev` (wrangler dev, :8787), `make cloudflare-test`, `make cloudflare-qa`,
  `make cloudflare-deploy`. Decided 2026-10-12: QA = `[env.qa]` (Worker `bible-tags-qa` on workers.dev, own D1
  `bible-tags-qa-accounts`, own DOs since DO namespaces are per Worker); production = top level (Worker `bible-tags`,
  custom domain only: `workers_dev`/`preview_urls` false, deploy refuses until `routes` is set). Bindings aren't
  inherited by wrangler environments, so `[env.qa]` repeats them; secrets are per environment;
  all rebuild `build/cloudflare/assets` (client in server mode + `build/text` + `texts.json` + `_headers`).
  Deploy on push = Cloudflare Workers Builds (dashboard; README): root `crates/cloudflare`, deploy command
  `./ci.sh deploy --env qa`; `ci.sh` installs Rust via rustup (the build image has none), runs the Makefile and
  wrangler (pinned version in the script). Trial-run from a fresh clone with `--dry-run` (~4 min, nothing cached).
  `make check` also runs clippy for `wasm32-unknown-unknown` on `crates/cloudflare` (it compiles natively too).
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
  (text-build output), SPA fallback to `index.html` (not under `/assets`, where it would be cached as immutable); compression (br/gzip); `Cache-Control` immutable for
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
Text is addressed by **word ordinals** that are addresses: u32 `bookId << 24 | chapter << 16 | verse << 8 | word`
(decided 2026-10-12, replacing book-local word counts; no data was migrated, there were no users). Book ids are
stable (OT 2..40, NT 70..96, 41..69 reserved for deuterocanon, from the source's file numbering); chapter/verse are
as printed (verse 0 = title); word = position in the verse. So the same verse has the same ordinals in any text
with the same versification (cross-translation tags project by verse), and a text can be indexed without shipping
its words. Ordinals are sparse: never subtract them to count words or walk them one by one (`TextIndex.pos`).
All intervals are **half-open `[start, end)`** and **tile**: an end is the next word's ordinal (`TextIndex.after`)
or the book's end, never `last + 1` inside a book, so neighbouring verses still abut. **Taggings never span books**
(2026-10-17, from a review: erasing part of one that did left pieces starting or ending between books, on no word):
the server requires the end to be a word of the start's book or that book's end, and the client cuts strokes and
selections with `TextIndex.byBook` (one tagging per book, one undo step). JS must use
`bookIdOf`/`makeOrd`, never `>>` (ids >= 128 exceed int32). ~791k words in KJV; **hyphens split words**
(`Beth-` + `el`, the first part flagged `joined`).
Anchors are keyed by `text_id` (versioned, now `kjv-1769-v2`); the client ignores taggings of other texts.
Planned for licensed translations (user, 2026-10-12): build their index ahead of time from a licensed copy and
treat it as static data (the index holds no text); if it can't be published as a file, store it in the shared
database and let clients cache it forever. Text itself is fetched at runtime within the license's caching terms.

Each space's SQLite database (`space::SCHEMA`; the same SQL on the server and in Durable Objects) holds the current
state in normalized tables plus the change log. The tables are the source of truth (`apply` checks ops against them
and writes rows + one change in one transaction); the log is only for sync replay, undo and op dedupe.
```sql
meta(key PK, value)                       -- 'schema' version, 'pruned_through' (retention)
members(owner_id PK, created_at)          -- owners with rows in this space
tags(id PK, owner_id -> members, name, color, description (unused), created_at, updated_at, deleted_at,
     UNIQUE(owner_id, id))
  -- unique index (owner_id, lower(name)) among live tags: an ASCII backstop; ops::effect enforces the Unicode rule
taggings(id PK, owner_id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into,
         FOREIGN KEY (owner_id, tag_id) REFERENCES tags(owner_id, id), CHECK (start_ord < end_ord))
  -- index (owner_id, tag_id, text_id, start_ord) among live taggings
comments(id PK, owner_id -> members, tagging_id -> taggings, body, created_at, edited_at, deleted_at, author_id)
  -- index (owner_id, tagging_id); author_id (schema v2) = the member who wrote it
palettes(owner_id PK -> members, tags JSON array of tag ids (<= 9, read/written whole), updated_at)
changes(seq INTEGER PK (one per space), owner_id (data owner), op_id, kind, payload JSON {before, after rows}
        ('' once pruned), created_at, UNIQUE(owner_id, op_id))
```
Not built: per-combination overlap styles (an `overlap_styles` table was sketched early; see Next steps).
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
  stream with ONE `seq` (`changes.seq INTEGER PRIMARY KEY`; `changes.owner_id` = the data owner, i.e. the space id in a group, so op ids are per space). Today every owner has a
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
  Ordinals are validated against the text index (`bad_text`: start is a word, end is a word or a book end), ids
  against `[0-9A-Za-z_-]{1,64}` (`bad_id`).
  The shared ops fixtures also run through SQLite (`crates/space/tests/fixtures.rs`, which also tests a two-owner
  space; mutation-checked 2026-10-11: owner check, per-space seq, write-before-checks, duplicate scope).
- **Cloudflare** (`crates/cloudflare`, done 2026-10-11; workers-rs 0.8.7, built for wasm32 by worker-build 0.8.7
  installed into `build/tools`; wrangler from nix): same API and protocol as the self-hosted server. Worker routes
  `/api/*` (everything else is Workers Static Assets: client + text, no SPA fallback (`not_found_handling =
  "none"`: the app lives at `/` with hash routes, and an `index.html` answer to a missing `/assets/*` was cached as
  immutable), `_headers` for caching); accounts
  in D1 (binding `ACCOUNTS`, schema created once per isolate from `api::accounts::SCHEMA`); one SQLite-backed Durable
  Object `Space` per space (`id_from_name(space)`), running `crates/space` over `SqlStorage` with the WebSocket
  hibernation API. The Worker authenticates and forwards the upgrade with `x-owner`. The DO has no hub: no awaits
  between apply and broadcast, so each event is atomic and every ready socket gets changes in seq order (a socket is
  "ready" after its hello replay, kept in its attachment). Texts for `bad_text` are loaded from the assets
  (`/text/texts.json`, written by `make cloudflare-assets`) inside `block_concurrency_while` in the constructor, so
  ops are never checked out of order. Sign-in merge = `GET /export` on the anonymous space's DO, `POST /merge` on
  the account's. `GOOGLE_AUTH_URL`/`GOOGLE_TOKEN_URL` vars exist only for the tests. Tested end to end by
  `make cloudflare-test` (`crates/cloudflare/test/`: wrangler dev on 127.0.0.1 with throwaway state + a fake
  Google: sync, replay, rejects, isolation, auth/origin, logout, sign-in, merge incl. live push, forged/cancelled
  callbacks); browser-checked (paint, wipe local storage, reload, tagging comes back). NOT yet deployed (needs the
  user's Cloudflare account; `wrangler.toml` has a placeholder D1 id).
- **Groups** (done 2026-10-12; the user asked for "something working" to review and tweak): a group is a space
  whose rows all belong to the group (data owner = space id, exactly as a personal space's id is its owner's), so
  members share tags, palette and comments. Ops are applied with `space::apply_by(db, space, author, op)`: new
  comments get `author` (an optional field on `CommentRow`, stamped by the server, never by `effect`, so the fixtures
  are unchanged; comments table gained `author_id`, schema v2 via `ALTER TABLE` in `space::init`), and a change may
  only modify comments its author wrote (`ApplyError::Refused("not_author")`). Accounts DB has `groups`,
  `memberships(space_id, owner_id, role)`, `invites(token, space_id, role)`; roles admin/edit/read; one reusable
  invite link per role (a new one revokes the old; tokens stored plain so admins can re-copy them). HTTP API and
  rules in `api::groups::handle` over the async `AccountsDb` trait (rusqlite via the blocking pool on the server, D1
  on Cloudflare), tested in `crates/api/tests/groups.rs`. The socket picks its space with `/api/sync?space=`; the
  role is checked at upgrade (403 if not a member) and again once the socket can receive kicks (server: after
  subscribing to the hub; Durable Object: inside `block_concurrency_while` before accepting), so a removal racing a
  connect can't leave a socket with its old role; it is then fixed for the connection; read-only sockets get every change
  but ops are rejected `read_only`. Role changes and removals disconnect that member (`Reply::kicks`: hub `Event::Kick`
  on the server, DO `POST /kick` + socket tags on Cloudflare) so they reconnect with the new role. Tested: server
  `groups_share_rows_with_roles`, Cloudflare `test/groups.mjs`, browser (create group, invite links, a scripted
  second member's paint + comment arriving live with author, view-only join via `?join=` with live updates and no
  edit controls, switching spaces). Not yet: group-level comment moderation (admins can't delete others' comments),
  transferring/deleting a group, showing who painted what, rate limits on joins. Removing a member does NOT revoke
  the invite link they joined with, so they can rejoin with it (found in review 2026-10-17); the user chose not to
  rotate links on removal since others may rely on them: see the TODO in Next steps.
- **Retention** (done 2026-10-17): changes stay replayable/revertible for 30 days (`space::RETENTION_MS`; server
  `--retention-days`, Worker var `RETENTION_MS` for tests). Pruning runs online, never as a job: after each applied
  (non-duplicate) op the space prunes at most `PRUNE_BATCH` (64) of its oldest changes (user's choice: no long work in
  one request; since each op adds one change, pruning keeps up; idle spaces don't prune and don't need to). A pruned
  change keeps its row (seq, owner, op id, kind) with an empty payload, so a re-sent op is a duplicate forever (acked
  with its old seq) but can't be replayed or reverted (`unknown_op`). Pruning is a prefix: `meta.pruned_through`.
  `hello{after, pending}` with `after < pruned_through` gets `reset{seq, rows (every row incl. tombstones: comments
  follow merge chains), changes: [], applied (which of `pending` are already applied)}` instead of a replay; the
  client confirms `applied` silently, rebases the rest, drops undo steps it can't revert, and treats an `ack` at or
  below its seq as confirmation. Tested: `crates/space/tests/retention.rs` (mutation-checked, 6/6), server
  `a_client_behind_the_retention_window_gets_a_reset`, Cloudflare `test/retention.mjs` (second wrangler run with
  `RETENTION_MS:0`), TS sync tests (mutation-checked, 4/4).
- **Shared edge logic** (`crates/api`, no I/O): sync protocol messages, `check_op`/`Texts`, session cookies,
  Google OAuth helpers, accounts schema + SQL + the sign-in rule (`accounts::sign_in`). Both servers use it; keep
  server-specific code to I/O only.
: `hello{after, pending}` -> replayed changes (or a `reset`) -> `ready` ->
  live changes; `op` -> `ack`/`reject{code}`; the change itself is the confirmation. Lag -> `error` + reconnect.
- **Sessions**: `POST /api/session` returns `{owner, account, providers}`, creating an anonymous owner + HttpOnly
  SameSite=Lax cookie (Secure under TLS; only sha256(token) stored) if needed. Same-origin `Origin` required on
  session/sync/logout.
- **Sign in with Google** (`api::google` + `crates/server/src/auth.rs`, done 2026-10-11): authorization code + PKCE, state + verifier
  + return hash in a 10-minute `bt_oauth` cookie (Path=/api/auth); `id_token` read from the token endpoint response
  (no signature check needed, per OIDC; iss/aud/exp/email_verified checked); HTTP via `ureq` (rustls + ring, no
  aws-lc). Accounts = owners with rows in `identities(provider, subject)`. Rules: anonymous + new identity -> the
  owner becomes the account; anonymous + existing account -> `space::merge` copies the anonymous owner's live data
  into the account (same-named tags combined case-insensitively, taggings painted so overlaps merge, comments
  follow, palette appended while room; original timestamps; idempotent op ids `merge-<from>-<row id>`; the
  anonymous space is left as it was) and the session switches; its groups come along (2026-10-17, from a review:
  they used to be lost): in each, `space::reassign_author` makes the account the author of its comments as an
  ordinary change (kind `reassign`, so open clients get it and the comments stay editable), its sockets there are
  kicked, then `groups::move_memberships` moves memberships (higher role wins) and `created_by`; signed in + other identity -> switch (new account if
  unknown). Every sign-in rotates the session token. `POST /api/logout` deletes the session; the client then clears
  its base/outbox/undo and reloads. Config: env `GOOGLE_CLIENT_ID`/`GOOGLE_CLIENT_SECRET` (not flags), optional
  `--public-url`; redirect URI = `<origin>/api/auth/google/callback`. Client (`Account.tsx`): top-bar person button
  (initial when signed in) -> card with "Sign in with Google" or name/email + Sign out; sign-in is refused while
  ops are unsent (`?auth_error=` from the callback becomes a notice). Tested: `tests/auth.rs` against a fake token
  endpoint (PKCE, linking, merge, rotation, logout, forged/stale callbacks); UI checked in the browser (anonymous
  card, redirect URL, signed-in card via a hand-inserted identity, sign-out). NOT yet tried against real Google
  (needs the user's OAuth client).
- **Client** (`SyncState` in `packages/scripture/src/syncState.ts`, pure; `packages/app/src/store.ts` wraps it):
  view = confirmed base + pending ops replayed (rebase on every change; ops that no longer apply are dropped with a
  notice). Undo/redo steps = op-id lists (a multi-op step undoes in reverse). Persistence: shared base cache
  (localStorage, never regresses seq), shared **outbox** of unconfirmed ops (union across tabs, Web Lock, adopted by
  other tabs; any tab may re-send since ops are idempotent), per-tab undo (sessionStorage). Reconnect with backoff
  and on `online`.
- Tests: end-to-end WebSocket tests (fan-out, replay, duplicates, rejects, owner isolation, revert, auth/origin,
  restart); a client convergence property test (3 clients, random edits/delivery/disconnects, 120 seeds);
  browser-tested with two tabs incl. offline -> reconnect. Comments are in the fixtures, both property tests (with
  "every op kind applied" assertions) and the 3-client convergence test; mutation-checked 2026-10-11 (8 mutations,
  caught in both languages). Not yet: rate limits.

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
  frozen by `fixtures/text/kjv-1769-v2.golden.json` once anyone has data; changing it then needs a new `text_id`
  plus an ordinal remap.

## Testing philosophy
Shared JSON fixtures are the behavioral spec for anything duplicated across TS and Rust. Property tests
(randomized, seeded, against a brute-force model) in both languages. Mutation-check new suites: deliberately break the
implementation and confirm tests fail. Don't claim something is verified unless it was actually run.

## Next steps
- TODO (user, 2026-10-17): think through removal + anonymous members + write-enabled invite links. A removed member
  can rejoin with the same link, and anonymous joins are free to repeat. Options: require sign-in for write-enabled
  (edit/admin) links, a per-group ban list keyed by account, or per-person invites. Decide before groups go public.
- Groups follow-ups (user will review first): see the Groups bullet's "Not yet".
- Deploy to Cloudflare, QA first (user: `wrangler login`, two `wrangler d1 create`, a domain, secrets; README) and try sign-in with a real
  Google OAuth client. Then: rate limits.
- Install-as-app affordance (deferred 2026-10-11): Install button via `beforeinstallprompt`, iOS "Add to Home
  Screen" hint, `navigator.storage.persist()`.
- Reader: pairs within 3+ tag combinations in the Nearby panel, overlap style rules, a book/chapter picker.
- Touch painting was tested only with synthesized PointerEvents; try it on a real phone.
