//! Session cookies. `POST /api/session` creates an anonymous owner and sets an `HttpOnly` session cookie (only the
//! token's sha256 is stored). The owner always comes from the session, never from a payload. Session, sync and
//! logout require a same-origin `Origin` (CSRF / cross-site WebSocket hijacking).

use crate::Identity;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

pub const COOKIE: &str = "bt_session";

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// `n` random bytes as hex.
///
/// # Errors
/// If the system RNG fails.
pub fn random_hex(n: usize) -> Result<String, String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(hex(&buf))
}

#[must_use]
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// The value of cookie `name` in `Cookie` header values.
pub fn cookie<'a>(headers: impl IntoIterator<Item = &'a str>, name: &str) -> Option<String> {
    headers.into_iter().flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name && !v.is_empty()).then(|| v.to_string())
    })
}

/// Browsers always send `Origin` on cross-site requests and WebSocket handshakes; it must name this host.
#[must_use]
pub fn same_origin(origin: Option<&str>, host: &str) -> bool {
    let Some(origin) = origin else { return true };
    origin.split_once("://").is_some_and(|(_, rest)| rest == host)
}

/// The session cookie for `token` (a year; renewed by signing in again).
#[must_use]
pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000{secure}")
}

#[must_use]
pub fn clear_session_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}

/// What `POST /api/session` returns: `{owner, account: {provider, email, name} | null, providers}`.
#[must_use]
pub fn session_json(owner: &str, account: Option<&Identity>, providers: &[&str]) -> serde_json::Value {
    let account = account.map(|a| serde_json::json!({ "provider": a.provider, "email": a.email, "name": a.name }));
    serde_json::json!({ "owner": owner, "account": account, "providers": providers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_and_origins() {
        assert_eq!(cookie(["a=1; bt_session=xyz", "c=3"], COOKIE).as_deref(), Some("xyz"));
        assert_eq!(cookie(["bt_session="], COOKIE), None);
        assert!(same_origin(None, "h"));
        assert!(same_origin(Some("https://h:1"), "h:1"));
        assert!(!same_origin(Some("https://evil"), "h"));
    }
}
