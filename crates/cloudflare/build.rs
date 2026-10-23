//! Compiles in the version of the client deployed beside this Worker (`CLIENT_VERSION`): one `wrangler deploy`
//! uploads both, and the Makefile builds the assets before wrangler builds the Worker, so the client can't change
//! without this code changing too (and a code deploy restarts every Durable Object). Empty without the assets (e.g.
//! `make check`'s clippy).
fn main() {
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/cloudflare/assets/client-version.txt");
    println!("cargo:rerun-if-changed={}", file.display());
    let version = std::fs::read_to_string(&file).unwrap_or_default();
    println!("cargo:rustc-env=CLIENT_VERSION={}", version.trim());
}
