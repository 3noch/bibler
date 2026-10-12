#!/usr/bin/env bash
# End-to-end tests of the Cloudflare deployment: starts `wrangler dev` on 127.0.0.1 with throwaway state and a
# fake Google, runs sync.mjs and auth.mjs against it, stops it. Run via `make cloudflare-test` (assets built first).
set -euo pipefail
cd "$(dirname "$0")/.."
PORT=${PORT:-18787} FAKE=${FAKE:-18799}
STATE=$(mktemp -d) LOG=$(mktemp)
trap 'kill $(jobs -p) 2>/dev/null; wait 2>/dev/null; rm -rf "$STATE" "$LOG"' EXIT
wrangler dev --ip 127.0.0.1 --port "$PORT" --persist-to "$STATE" --show-interactive-dev-session=false \
  --var GOOGLE_CLIENT_ID:cid --var GOOGLE_CLIENT_SECRET:sec \
  --var "GOOGLE_AUTH_URL:http://127.0.0.1:$FAKE/auth" --var "GOOGLE_TOKEN_URL:http://127.0.0.1:$FAKE/token" \
  >"$LOG" 2>&1 &
for _ in $(seq 1 300); do
  grep -q "Ready on" "$LOG" && break
  kill -0 %1 2>/dev/null || { cat "$LOG"; exit 1; }
  sleep 1
done
grep -q "Ready on" "$LOG" || { cat "$LOG"; echo "wrangler dev did not start"; exit 1; }
node test/sync.mjs "http://127.0.0.1:$PORT"
node test/auth.mjs "http://127.0.0.1:$PORT" "$FAKE"
node test/groups.mjs "http://127.0.0.1:$PORT"
