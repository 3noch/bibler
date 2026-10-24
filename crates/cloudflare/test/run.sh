#!/usr/bin/env bash
# End-to-end tests of the Cloudflare deployment: starts `wrangler dev` on 127.0.0.1 with throwaway state and a
# fake Google, runs lazy.mjs (first: it counts the Durable Objects that stored anything), sync.mjs, auth.mjs and
# groups.mjs against it; then restarts it with no retention (every change but the newest is pruned) and a minimum client protocol of 2 for
# retention.mjs. Run via `make cloudflare-test` (assets built first).
set -euo pipefail
cd "$(dirname "$0")/.."
PORT=${PORT:-18787} FAKE=${FAKE:-18799}
STATE=$(mktemp -d) LOG=$(mktemp)
trap 'kill $(jobs -p) 2>/dev/null; wait 2>/dev/null; rm -rf "$STATE" "$LOG"' EXIT

# Start wrangler dev with these extra arguments on fresh state, and wait until it is ready.
serve() {
  rm -rf "${STATE:?}"/* && : >"$LOG"
  wrangler dev --ip 127.0.0.1 --port "$PORT" --persist-to "$STATE" --show-interactive-dev-session=false \
    --var GOOGLE_CLIENT_ID:cid --var GOOGLE_CLIENT_SECRET:sec \
    --var "GOOGLE_AUTH_URL:http://127.0.0.1:$FAKE/auth" --var "GOOGLE_TOKEN_URL:http://127.0.0.1:$FAKE/token" \
    "$@" >"$LOG" 2>&1 &
  for _ in $(seq 1 300); do
    grep -q "Ready on" "$LOG" && return
    kill -0 $! 2>/dev/null || { cat "$LOG"; exit 1; }
    sleep 1
  done
  cat "$LOG"
  echo "wrangler dev did not start"
  exit 1
}
stop() {
  kill $(jobs -p) 2>/dev/null || true
  wait 2>/dev/null || true
}

serve
node test/lazy.mjs "http://127.0.0.1:$PORT" "$STATE"
node test/sync.mjs "http://127.0.0.1:$PORT"
node test/auth.mjs "http://127.0.0.1:$PORT" "$FAKE"
node test/groups.mjs "http://127.0.0.1:$PORT"
stop
serve --var RETENTION_MS:0 --var MIN_CLIENT_PROTOCOL:2
node test/retention.mjs "http://127.0.0.1:$PORT"
