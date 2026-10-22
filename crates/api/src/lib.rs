//! What both deployments say to the client, with no I/O: the self-hosted server (`crates/server`: axum + rusqlite)
//! and the Cloudflare Worker (`crates/cloudflare`: D1 + one Durable Object per space). Both store a space with
//! `crates/space`; this crate is everything around it (the sync protocol, op checks, sessions, Google sign-in,
//! the accounts schema and sign-in rules), so the two cannot drift apart.

pub mod accounts;
pub mod check;
pub mod google;
pub mod groups;
pub mod protocol;
pub mod session;

pub use accounts::Identity;

/// What `GET /api/health` returns: `{ok, version, commit, committed_at}`; the client's menu shows the server's commit
/// next to its own (the two are built and deployed separately, so they can differ). `commit` and `committed_at`
/// (ISO 8601) come from the environment at run time if set (the nix build's wrapper sets them, so a new commit
/// doesn't rebuild the server), else from the build (`build.rs`); empty when neither knew them.
#[must_use]
pub fn health(version: &str) -> serde_json::Value {
    let var = |name: &str, built: &str| std::env::var(name).unwrap_or_else(|_| built.to_string());
    serde_json::json!({
        "ok": true,
        "version": version,
        "commit": var("BUILD_COMMIT", env!("BUILD_COMMIT")),
        "committed_at": var("BUILD_COMMITTED_AT", env!("BUILD_COMMITTED_AT")),
    })
}
