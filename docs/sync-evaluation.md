# Sync framework evaluation

(Moved from the root CLAUDE.md.)

## Sync framework evaluation (2026-10-10; requirement: free OSS, self-hostable, no paid tier)
Our needs: instant optimistic paint, real offline use (writes too), canonical `normalize` result, undo/redo, mostly
per-user data (cross-user aggregates later, server-side). Licenses checked on npm 2026-10-10.
- **Zero** (Rocicorp, Apache-2.0, 1.0 in 2026, v1.10): optimistic mutators run client-side, then authoritatively
  on your push endpoint, with rebase; reactive ZQL queries; permissions. BUT: **no offline writes by design**
  (rejected when disconnected; "not designed for long periods offline"), requires Postgres (logical replication)
  + zero-cache (Docker) + an API server; server mutators are TS (other languages possible but the protocol is
  undocumented). Best fit only if we drop offline writes.
- **LiveStore** (Apache-2.0, v0.4, pre-1.0): event-sourced: events synced with a server-defined total order
  (seqNum/parentSeqNum), clients rebase pending events and materialize them into local SQLite (WASM). Offline-first.
  Its sweet spot is per-user data with little sharing, which is us. Sync backends: Cloudflare, Electric, S2, or a
  custom provider (needs: events ordered by cursor + push notification), which we could write in Rust over our
  `changes` table. This is structurally the same design we already planned (intent = event, normalize =
  materializer, per-owner seq). Risk: pre-1.0.
- **Electric** (Apache-2.0): read-path only (Postgres shapes -> clients over HTTP); writes are your own API.
  Doesn't remove the work we'd do anyway. **TanStack DB** (MIT): client reactive collections + optimistic
  mutations, backend-agnostic; possible client layer, but Solid's own reactivity already covers this.
- **Jazz** (MIT, 0.20): CRDT CoValues + self-hostable sync server. CRDT merges don't preserve the per-tag
  disjointness invariant under concurrent offline edits (it would need repair after each merge). Poor fit.
- Rejected: **PowerSync** server is FSL (source-available, Apache after 2 years; client SDKs Apache), so not OSS
  by the requirement, though technically the strongest offline SQLite sync. **InstantDB** (Apache-2.0) hosted
  service ends Aug 2027, team joining OpenAI, so its future is unclear. **Triplit** is AGPL and community-maintained
  since its team joined Supabase (2025). **Replicache** has proprietary terms (superseded by Zero). **RxDB** core
  is Apache but key plugins are paid.
- **Decision (2026-10-10, user: "continue with our plan"): our own per-owner event log** (the LiveStore model). The
  LiveStore spike was skipped; revisit only if client-side persistence/devtools become a burden. Before building the client store, spike
  LiveStore with a Rust sync provider (~1 day) to see whether its client (rebase, persistence, devtools) saves
  real work; adopt it only if that holds and the spike shows it isn't fighting our in-memory interval indexes.
  Cross-user aggregates are a server concern either way (replay canonical changes into aggregate tables).
