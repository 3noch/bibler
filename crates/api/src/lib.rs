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

/// The client version in a built `index.html` (`<meta name="build-client" content="...">`, a hash of the client's
/// files written by `packages/app/vite.config.ts`). Each `ready` carries the one the server would serve now, so an
/// open client whose own differs knows a new client was deployed (a server-only change keeps it).
#[must_use]
pub fn client_version(index_html: &str) -> Option<String> {
    let rest = &index_html[index_html.find(r#"<meta name="build-client" content=""#)? + 35..];
    Some(rest[..rest.find('"')?].to_string()).filter(|v| !v.is_empty())
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
    fn client_version_is_read_from_the_meta_tag() {
        let html =
            r#"<head><meta name="build-commit" content="abc"><meta name="build-client" content="0123abcd"></head>"#;
        assert_eq!(super::client_version(html).as_deref(), Some("0123abcd"));
        assert_eq!(super::client_version("<head></head>"), None);
        assert_eq!(super::client_version(r#"<meta name="build-client" content="">"#), None);
    }
}
