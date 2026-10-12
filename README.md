# bibler

A Bible reader built around **tags** instead of highlights: paint colored, named tags over words, then find every
place a tag (or a combination of tags) appears.

- **Read** the KJV as one continuous scroll; jump anywhere by typing a reference (`Jn 3:16`, `Rom 8`, `Ps 23`).
- **Tag** by picking a color from the dock and dragging over words (tap or long-press on phones).
- **Look up** any tagged words to see nearby places with the same tags, as text snippets.
- **Search** tags and combinations, optionally within books or groups (Gospels, Pauline epistles, …).
- **Hide** tags you don't want to see right now (Hide all / Show all, then pick).
- **Groups**: tag and comment together. Invite people with a link that lets them edit or only view; switch between
  your own tags and each group's from the top bar.
- **Undo** everything; edits work offline and sync to the server when it comes back.
- **Sign in with Google** (optional) to keep tags safe and use them on every device; tags made before signing in
  come along.

Stack: Solid.js client, Rust server (axum + SQLite), shared TypeScript/Rust core kept in step by common test fixtures.

## Run it

```sh
nix-shell      # provides node; Rust comes from rustup
make dev       # http://localhost:5173
```

| Command | What it does |
|---|---|
| `make dev` | Dev server with live reload (Vite on :5173, Rust API on :8080) |
| `make serve` | Production-like build on http://localhost:8080 |
| `make test` / `make check` | All tests / formatting, lints, type checks |
| `make pages` | Static, server-less build in `build/pages/` (`make pages-serve` to preview) |
| `make cloudflare-dev` | The Cloudflare version run locally by wrangler on http://localhost:8787 (`make cloudflare-test` tests it) |

## Deploy

- **GitHub Pages** (no server; tags stay in the visitor's browser): pushing to `main` runs
  `.github/workflows/pages.yml`. One-time setup: *Settings → Pages → Source: GitHub Actions*.
- **Server** (tags stored in SQLite on the server):
  ```sh
  nix-build -A app
  result/bin/bible-tags --data /var/lib/bible-tags --addr 0.0.0.0:443 \
    --tls-cert fullchain.pem --tls-key key.pem --http-redirect 0.0.0.0:80
  ```

- **Cloudflare** (free plan; Workers + D1 + Durable Objects, one database per user). Needs `wrangler` (in
  `nix-shell`) and a Cloudflare account. One-time setup:
  ```sh
  cd crates/cloudflare
  wrangler login
  wrangler d1 create bible-tags-accounts   # put the printed database_id into wrangler.toml
  ```
  Then `make cloudflare-deploy` from the repo root. It prints the `*.workers.dev` URL; a custom domain can be added
  in the dashboard (*Workers → bible-tags → Settings → Domains*). For sign-in, run
  `wrangler secret put GOOGLE_CLIENT_ID` and `wrangler secret put GOOGLE_CLIENT_SECRET` in `crates/cloudflare`,
  and add `https://<your url>/api/auth/google/callback` to the OAuth client (below).

## Sign in with Google (optional)

1. Google Cloud Console → *APIs & Services → Credentials → Create credentials → OAuth client ID*, type
   *Web application*.
2. Add an *Authorized redirect URI* per origin: `https://your.host/api/auth/google/callback`, and for
   `make dev`, `http://localhost:5173/api/auth/google/callback`.
3. Start the server with `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in its environment (`make dev` passes
   them through). Behind a proxy that terminates TLS, add `--public-url https://your.host`. On Cloudflare they are
   Worker secrets (above); for `make cloudflare-dev`, put them in `crates/cloudflare/.dev.vars` as `KEY=value`
   lines and register `http://localhost:8787/api/auth/google/callback`.

Without them the app works the same, just with no sign-in button.

## Layout

| Path | Contents |
|---|---|
| `packages/app` | Solid client |
| `packages/scripture` | Shared TS core: text index, reference parser, tag edits, sync, queries |
| `crates/server` | Self-hosted HTTP + sync WebSocket server (SQLite, TLS, sign-in) |
| `crates/cloudflare` | The same API on Cloudflare: a Worker, D1 for accounts, a Durable Object per user |
| `crates/space` | One user's (or later a group's) database: schema and sync, used by both servers |
| `crates/api` | Everything else both servers share: sync protocol, checks, sessions, Google sign-in |
| `crates/ops`, `crates/normalize` | Rust side of the shared core |
| `crates/text-build` | Turns the KJV source into the app's text files |
| `data/source` | Vendored KJV (1769) text — never edit; tags depend on it |
| `fixtures/`, `docs/` | Shared test cases and specs |

Design notes and decisions live in `CLAUDE.md` and `docs/`.
