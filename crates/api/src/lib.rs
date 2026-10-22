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

/// The commit this server was built from and its time (ISO 8601), sent in every sync `ready` (the client's menu
/// shows it next to its own: the two are built and deployed separately, so they can differ) and by `/api/health`.
/// From the environment at run time if set (the nix build's wrapper sets them, so a new commit doesn't rebuild the
/// server), else from the build (`build.rs`); empty when neither knew them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Build {
    pub commit: String,
    pub committed_at: String,
}

/// This server's [`Build`].
#[must_use]
pub fn build() -> &'static Build {
    static BUILD: std::sync::LazyLock<Build> = std::sync::LazyLock::new(|| {
        let var = |name: &str, built: &str| std::env::var(name).unwrap_or_else(|_| built.to_string());
        Build {
            commit: var("BUILD_COMMIT", env!("BUILD_COMMIT")),
            committed_at: var("BUILD_COMMITTED_AT", env!("BUILD_COMMITTED_AT")),
        }
    });
    &BUILD
}

/// What `GET /api/health` returns: `{ok, version, commit, committed_at}`.
#[must_use]
pub fn health(version: &str) -> serde_json::Value {
    let b = build();
    serde_json::json!({ "ok": true, "version": version, "commit": b.commit, "committed_at": b.committed_at })
}
