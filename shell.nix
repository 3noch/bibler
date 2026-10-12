# Dev shell: node + tools for `make dev`. Rust comes from rustup (rust-toolchain.toml), not from here, so the
# toolchain stays the one the repo pins.
{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  packages = [
    pkgs.nodejs_22
    pkgs.gnumake
    pkgs.wrangler # make cloudflare-dev / cloudflare-deploy
  ];
}
