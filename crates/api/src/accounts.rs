//! Accounts: owners, sessions and external identities (`accounts.sqlite` self-hosted, D1 on Cloudflare), and who a
//! sign-in makes you.
//!
//! Everyone starts as an anonymous owner (`POST /api/session`). Signing in either makes the current anonymous owner
//! an account (a new identity) or, if the identity already has an account, **merges** the anonymous owner's live
//! data into it (`space::merge`; the anonymous data stays where it was) and switches the session to that account.
//! Signing in while signed in to another account just switches accounts. Every sign-in issues a new session token
//! (no session fixation).

/// An external sign-in identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub provider: String,
    /// The provider's stable user id (never the email, which can change).
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Idempotent; statements separated by `;` (no PRAGMAs, so D1 accepts it too).
pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS owners (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES owners(id),
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_by_owner ON sessions (owner_id);
CREATE TABLE IF NOT EXISTS identities (
  provider TEXT NOT NULL,
  subject TEXT NOT NULL,
  owner_id TEXT NOT NULL REFERENCES owners(id),
  email TEXT,
  name TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (provider, subject)
);
CREATE INDEX IF NOT EXISTS identities_by_owner ON identities (owner_id);
";

/// Every statement of the accounts schema (this module's and [`crate::groups::SCHEMA`]), in order.
#[must_use]
pub fn schema_statements() -> Vec<&'static str> {
    [SCHEMA, crate::groups::SCHEMA].iter().flat_map(|s| s.split(';')).filter(|s| !s.trim().is_empty()).collect()
}

// Positional parameters, in the order given.
/// `(id, created_at)`
pub const CREATE_OWNER: &str = "INSERT INTO owners (id, created_at) VALUES (?, ?)";
/// `(token_hash, owner_id, created_at)`
pub const CREATE_SESSION: &str = "INSERT INTO sessions (token_hash, owner_id, created_at) VALUES (?, ?, ?)";
/// `(token_hash)` -> `owner_id`
pub const SESSION_OWNER: &str = "SELECT owner_id FROM sessions WHERE token_hash = ?";
/// `(token_hash)`
pub const DELETE_SESSION: &str = "DELETE FROM sessions WHERE token_hash = ?";
/// `(provider, subject)` -> `owner_id`
pub const IDENTITY_OWNER: &str = "SELECT owner_id FROM identities WHERE provider = ? AND subject = ?";
/// `(provider, subject, owner_id, email, name, created_at)`
pub const ADD_IDENTITY: &str =
    "INSERT INTO identities (provider, subject, owner_id, email, name, created_at) VALUES (?, ?, ?, ?, ?, ?)";
/// `(owner_id)` -> `provider, subject, email, name`: the identity an owner signs in with (the first one).
pub const OWNER_IDENTITY: &str = "SELECT provider, subject, email, name FROM identities WHERE owner_id = ?
 ORDER BY created_at, provider LIMIT 1";

/// Who signing in makes the session's owner.
#[derive(Debug, PartialEq, Eq)]
pub enum SignIn {
    /// The identity's existing account; first merge this anonymous owner's data into it, if `Some`.
    Existing { owner: String, merge_from: Option<String> },
    /// Attach the identity to the current anonymous owner, which becomes the account.
    Link(String),
    /// A new owner with this identity (signed in elsewhere, or no session).
    New,
}

/// `current`: the session's owner, if any; `anonymous`: it has no identity; `existing`: the identity's owner.
#[must_use]
pub fn sign_in(current: Option<&str>, anonymous: bool, existing: Option<&str>) -> SignIn {
    match (current, existing) {
        (_, Some(x)) => SignIn::Existing {
            owner: x.to_string(),
            merge_from: current.filter(|a| anonymous && *a != x).map(str::to_string),
        },
        (Some(a), None) if anonymous => SignIn::Link(a.to_string()),
        (_, None) => SignIn::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_rules() {
        let merge = SignIn::Existing { owner: "x".into(), merge_from: Some("a".into()) };
        assert_eq!(sign_in(Some("a"), true, Some("x")), merge);
        assert_eq!(sign_in(Some("x"), false, Some("x")), SignIn::Existing { owner: "x".into(), merge_from: None });
        assert_eq!(sign_in(Some("y"), false, Some("x")), SignIn::Existing { owner: "x".into(), merge_from: None });
        assert_eq!(sign_in(None, false, Some("x")), SignIn::Existing { owner: "x".into(), merge_from: None });
        assert_eq!(sign_in(Some("a"), true, None), SignIn::Link("a".into()));
        assert_eq!(sign_in(Some("y"), false, None), SignIn::New);
        assert_eq!(sign_in(None, false, None), SignIn::New);
    }
}
