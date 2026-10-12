# Dev workflow. Needs cargo (rustup) and node >= 22.18 on PATH (`nix-shell` provides node; see shell.nix).
# Production builds are default.nix, which does not use this file.
#
#   make dev     Rust server (plain HTTP :8080, data build/dev-data/) + Vite (HMR :5173, proxies /api incl. the sync
#                WebSocket, and /text). Open http://localhost:5173
#   make serve   production-like: release server serving the built client + text on http://localhost:8080
#   make test    all tests (Rust + TS)        make check   fmt, clippy, typecheck
#   make nix     nix-build default.nix        make clean   remove build outputs
#   make pages   offline-only static site (no server; data stays in the browser) in build/pages/, as deployed to
#                GitHub Pages by .github/workflows/pages.yml;   make pages-serve  previews it on :8000

SHELL := bash
TEXT_ID := kjv-1769-v1
TEXT := build/text/$(TEXT_ID)/manifest.json
SERVER_ADDR ?= 127.0.0.1:8080
NPM := npm
# Data directories; a pre-spaces database (build/dev.db, build/serve.db) is imported once if it exists.
DEV_DATA := --data build/dev-data $(if $(wildcard build/dev.db),--migrate-from build/dev.db)
SERVE_DATA := --data build/serve-data $(if $(wildcard build/serve.db),--migrate-from build/serve.db)

.PHONY: dev dev-server dev-client serve build client server text test test-rust test-ts check fmt nix clean node-check pages pages-serve

dev: $(TEXT) node_modules/.installed
	@echo "==> http://localhost:5173  (API/text served by the Rust dev server on $(SERVER_ADDR))"
	@trap 'kill 0' INT TERM EXIT; \
	  cargo run -q -p server -- --addr $(SERVER_ADDR) --text build/text $(DEV_DATA) & \
	  BACKEND=http://$(SERVER_ADDR) $(NPM) run -w @bible-tags/app dev & \
	  wait

dev-server: $(TEXT)
	cargo run -p server -- --addr $(SERVER_ADDR) --text build/text $(DEV_DATA)

dev-client: node_modules/.installed
	BACKEND=http://$(SERVER_ADDR) $(NPM) run -w @bible-tags/app dev

serve: build
	@echo "==> http://$(SERVER_ADDR)"
	./target/release/bible-tags-server --addr $(SERVER_ADDR) --text build/text $(SERVE_DATA) --static packages/app/dist

build: client server $(TEXT)

client: node_modules/.installed
	$(NPM) run -w @bible-tags/app build

server:
	cargo build --release -p server

# Text artifacts are rebuilt only when the source text or the tokenizer changes.
text: $(TEXT)
$(TEXT): $(wildcard data/source/eng-kjv2006/*.usfm) $(wildcard crates/text-build/src/*.rs) fixtures/text/$(TEXT_ID).golden.json
	rm -rf build/text/$(TEXT_ID)
	cargo run -q -p text-build --release

node_modules/.installed: package.json package-lock.json packages/app/package.json packages/scripture/package.json | node-check
	$(NPM) ci
	@touch $@

node-check:
	@command -v node >/dev/null || { echo "node not found: run 'nix-shell' (shell.nix) or 'nix-env -iA nixpkgs.nodejs_22'"; exit 1; }

test: test-rust test-ts

test-rust:
	cargo test

test-ts: $(TEXT) node_modules/.installed
	cd packages/scripture && node --test "test/*.test.ts"

check: node_modules/.installed
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	npx tsc -p packages/scripture/tsconfig.json
	npx tsc -p packages/app/tsconfig.json

fmt:
	cargo fmt

nix:
	nix-build default.nix -A app

# Static, server-less build: the client in local mode (VITE_SYNC=local, relative paths) + the text artifacts.
pages: $(TEXT) node_modules/.installed
	rm -rf build/pages
	VITE_SYNC=local VITE_BASE=./ $(NPM) run -w @bible-tags/app build -- --outDir ../../build/pages --emptyOutDir
	mkdir -p build/pages/text
	cp -R build/text/$(TEXT_ID) build/pages/text/
	touch build/pages/.nojekyll

pages-serve: pages
	@echo "==> http://127.0.0.1:8000"
	python3 -m http.server 8000 --bind 127.0.0.1 --directory build/pages

clean:
	rm -rf build packages/app/dist result
