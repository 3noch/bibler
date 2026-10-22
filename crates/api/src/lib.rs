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

/// The file next to the client's `index.html` that holds its version (a hash of the client's files, written by
/// `packages/app/vite.config.ts`; the page has the same in `<meta name="build-client">`). Each `ready` carries the one
/// the server would serve now, so an open client whose own differs knows a new client was deployed (a server-only
/// change keeps it).
pub const CLIENT_VERSION_FILE: &str = "client-version.txt";

/// The version in a [`CLIENT_VERSION_FILE`]'s contents.
#[must_use]
pub fn client_version(file: &str) -> Option<String> {
    Some(file.trim().to_string()).filter(|v| !v.is_empty())
}

/// What `GET /api/health` returns: `{ok, version, commit, committed_at}`.
#[must_use]
pub fn health(version: &str) -> serde_json::Value {
    let b = build();
    serde_json::json!({ "ok": true, "version": version, "commit": b.commit, "committed_at": b.committed_at })
}

#[cfg(test)]
mod tests {
    #[test]
    fn client_version_is_the_file_trimmed() {
        assert_eq!(super::client_version("0123abcd\n").as_deref(), Some("0123abcd"));
        assert_eq!(super::client_version(" \n"), None);
    }
}
