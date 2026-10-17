#!/usr/bin/env bash
# Build and deploy from Cloudflare's Workers Builds (Cloudflare builds each push itself; setup in README.md).
# Its build image has Node and npm but no Rust, so this installs the toolchain from rust-toolchain.toml, builds the
# assets and worker-build with the Makefile, then runs wrangler with the arguments given:
#   ./ci.sh deploy --env qa         (QA's deploy command; root directory crates/cloudflare)
#   ./ci.sh deploy                  (production's; refused until wrangler.toml sets routes)
#   ./ci.sh deploy --env qa --dry-run   (a local trial: builds everything, uploads nothing)
set -euo pipefail
cd "$(dirname "$0")/../.."
WRANGLER_VERSION=4.148.0

# Production serves only its custom domain (wrangler.toml `routes`): refuse until one is set, like the Makefile's
# cloudflare-deploy. Checked first so a misconfigured build fails before the long build.
if [[ "${1:-}" == deploy && " $* " != *" --env"* && " $* " != *" -e "* ]] \
  && ! grep -q '^routes' crates/cloudflare/wrangler.toml; then
  echo "Set production's domain (routes) in crates/cloudflare/wrangler.toml first; for QA use ./ci.sh deploy --env qa" >&2
  exit 1
fi

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
  # shellcheck disable=SC1091
  . "${CARGO_HOME:-$HOME/.cargo}/env"
fi
rustup toolchain install # the toolchain, components and wasm target named in rust-toolchain.toml
make cloudflare-assets build/tools/bin/worker-build
cd crates/cloudflare
PATH="$PWD/../../build/tools/bin:$PATH" npx --yes "wrangler@$WRANGLER_VERSION" "$@"
