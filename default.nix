# Production build (the dev workflow is the Makefile; this file does not use it).
#
#   nix-build -A app      result/bin/bible-tags: the server wrapped with the client and text baked in
#   nix-build -A server   just the Rust server        nix-build -A client   just the built client
#   nix-build -A text     the text artifacts          nix-build -A textBuild
#
# Run, terminating TLS in-process (rustls; SIGHUP reloads the cert files after renewal):
#   result/bin/bible-tags --data /var/lib/bible-tags --addr 0.0.0.0:443 \
#     --tls-cert fullchain.pem --tls-key key.pem --http-redirect 0.0.0.0:80
#
# Each derivation only sees the files it needs (lib.fileset), so a client-only change never rebuilds Rust and a
# server change never rebuilds the client or the text. buildRustPackage still rebuilds all crate dependencies on
# every Rust change; if that gets slow, switch the Rust derivations to crane (separate dependency derivation).
#
# Updating dependencies: after changing package-lock.json, set npmDepsHash to the output of
#   nix-shell -p prefetch-npm-deps --run 'prefetch-npm-deps package-lock.json'
{
  pkgs ? import <nixpkgs> { },
}:
let
  inherit (pkgs) lib;
  fs = lib.fileset;
  version = "0.0.0";

  rustFiles = [
    ./Cargo.toml
    ./Cargo.lock
    ./rustfmt.toml
    ./crates
  ];
  rustSrc = extra: fs.toSource {
    root = ./.;
    fileset = fs.unions (rustFiles ++ extra);
  };

  # USFM -> tokens/index tool. Its tests check the golden file against the vendored source.
  textBuild = pkgs.rustPlatform.buildRustPackage {
    pname = "text-build";
    inherit version;
    src = rustSrc [
      ./data/source
      ./fixtures/text
    ];
    cargoLock.lockFile = ./Cargo.lock;
    cargoBuildFlags = [ "-p" "text-build" ];
    cargoTestFlags = [ "-p" "text-build" ];
  };

  # Content-hashed text artifacts: $out/<text_id>/{manifest.json,index.*.bin,*.json}
  text = pkgs.runCommand "bible-tags-text-${version}" { } ''
    ${textBuild}/bin/text-build \
      --src ${./data/source/eng-kjv2006} \
      --golden ${./fixtures/text/kjv-1769-v2.golden.json} \
      --out $out/kjv-1769-v2
  '';

  server = pkgs.rustPlatform.buildRustPackage {
    pname = "bible-tags-server";
    inherit version;
    src = rustSrc [ ./fixtures/ops.json ]; # its tests run the shared ops fixtures through SQLite
    cargoLock.lockFile = ./Cargo.lock;
    cargoBuildFlags = [ "-p" "server" ];
    cargoTestFlags = [ "-p" "server" ];
    # The sync tests run a real server on 127.0.0.1 (Linux sandboxes allow loopback; macOS needs this).
    __darwinAllowLocalNetworking = true;
    meta.mainProgram = "bible-tags-server";
  };

  # Static client bundle (Vite dist/).
  client = pkgs.buildNpmPackage {
    pname = "bible-tags-client";
    inherit version;
    nodejs = pkgs.nodejs_22;
    src = fs.toSource {
      root = ./.;
      fileset = fs.unions [
        ./package.json
        ./package-lock.json
        ./packages/scripture/package.json
        ./packages/scripture/src
        ./packages/app/package.json
        ./packages/app/tsconfig.json
        ./packages/app/vite.config.ts
        ./packages/app/index.html
        ./packages/app/src
        ./packages/app/public
      ];
    };
    npmDepsHash = "sha256-Vcqq0oBzOX0MuWMRwquVGkYeUBoB9wrjZNnwteTqhe0=";
    npmWorkspace = "packages/app";
    installPhase = ''
      runHook preInstall
      cp -r packages/app/dist $out
      runHook postInstall
    '';
  };

  app = pkgs.runCommand "bible-tags-${version}" {
    nativeBuildInputs = [ pkgs.makeWrapper ];
    meta.mainProgram = "bible-tags";
  } ''
    makeWrapper ${lib.getExe server} $out/bin/bible-tags \
      --add-flags "--static ${client} --text ${text}"
  '';
in
{
  inherit
    app
    client
    server
    text
    textBuild
    ;
  default = app;
}
