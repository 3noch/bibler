# bibler

A Bible reader built around **tags** instead of highlights: paint colored, named tags over words, then find every
place a tag (or a combination of tags) appears.

- **Read** the KJV as one continuous scroll; jump anywhere by typing a reference (`Jn 3:16`, `Rom 8`, `Ps 23`).
- **Tag** by picking a color from the dock and dragging over words (tap or long-press on phones).
- **Look up** any tagged words to see nearby places with the same tags, as text snippets.
- **Search** tags and combinations, optionally within books or groups (Gospels, Pauline epistles, …).
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

## Deploy

- **GitHub Pages** (no server; tags stay in the visitor's browser): pushing to `main` runs
  `.github/workflows/pages.yml`. One-time setup: *Settings → Pages → Source: GitHub Actions*.
- **Server** (tags stored in SQLite on the server):
  ```sh
  nix-build -A app
  result/bin/bible-tags --data /var/lib/bible-tags --addr 0.0.0.0:443 \
    --tls-cert fullchain.pem --tls-key key.pem --http-redirect 0.0.0.0:80
  ```

## Sign in with Google (optional)

1. Google Cloud Console → *APIs & Services → Credentials → Create credentials → OAuth client ID*, type
   *Web application*.
2. Add an *Authorized redirect URI* per origin: `https://your.host/api/auth/google/callback`, and for
   `make dev`, `http://localhost:5173/api/auth/google/callback`.
3. Start the server with `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in its environment (`make dev` passes
   them through). Behind a proxy that terminates TLS, add `--public-url https://your.host`.

Without them the app works the same, just with no sign-in button.

## Layout

| Path | Contents |
|---|---|
| `packages/app` | Solid client |
| `packages/scripture` | Shared TS core: text index, reference parser, tag edits, sync, queries |
| `crates/server` | HTTP + sync WebSocket server (SQLite, TLS, sign-in) |
| `crates/space` | One user's (or later a group's) database: schema and sync, shared with the Cloudflare port |
| `crates/ops`, `crates/normalize` | Rust side of the shared core |
| `crates/text-build` | Turns the KJV source into the app's text files |
| `data/source` | Vendored KJV (1769) text — never edit; tags depend on it |
| `fixtures/`, `docs/` | Shared test cases and specs |

Design notes and decisions live in `CLAUDE.md` and `docs/`.
