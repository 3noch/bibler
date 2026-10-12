//! Sign in with Google (OAuth 2.0 authorization code + PKCE; the `OpenID` Connect `id_token` gives the identity).
//!
//! ```text
//! GET  /api/auth/google?return=#hash   -> 302 to Google (state + PKCE verifier in a short-lived cookie)
//! GET  /api/auth/google/callback       -> 302 to /#hash (or /?auth_error=code#hash)
//! POST /api/logout                     -> ends the session (the client then starts a new anonymous one)
//! ```
//! The `id_token` comes straight from Google's token endpoint over TLS, so its signature need not be checked
//! (`OpenID` Connect Core 3.1.3.7); its `iss`, `aud`, `exp` and `email_verified` still are.

use crate::session::random_hex;
use crate::Identity;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
}

impl Google {
    /// Google's production endpoints.
    #[must_use]
    pub fn new(client_id: String, client_secret: String) -> Self {
        Google {
            client_id,
            client_secret,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
        }
    }
}

pub const FLOW_COOKIE: &str = "bt_oauth";
pub const PROVIDER: &str = "google";

/// Only a fragment of this app (`#...`, from `location.hash`) may be returned to; anything else becomes "".
#[must_use]
pub fn safe_return(r: &str) -> String {
    let ok = r.starts_with('#') && r.len() <= 200 && r.bytes().all(|b| b.is_ascii_graphic() && b != b'"');
    if ok {
        r.to_string()
    } else {
        String::new()
    }
}

/// Where Google sends the user back, given this app's public origin (`https://example.org`).
#[must_use]
pub fn callback_url(origin: &str) -> String {
    format!("{}/api/auth/google/callback", origin.trim_end_matches('/'))
}

/// The `Set-Cookie` value carrying (or, with `max_age` 0, clearing) the flow state.
#[must_use]
pub fn flow_cookie(value: &str, max_age: u32, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{FLOW_COOKIE}={value}; Path=/api/auth; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

/// Percent-encode a query value.
#[must_use]
pub fn enc(s: &str) -> String {
    s.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

/// Begin signing in: `(Google's consent URL, the flow cookie's value)`.
///
/// # Errors
/// If the system RNG fails.
pub fn start(g: &Google, redirect_uri: &str, ret: &str) -> Result<(String, String), String> {
    let (state, verifier) = (random_hex(16)?, random_hex(32)?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope=openid%20email%20profile&state={state}\
         &code_challenge={challenge}&code_challenge_method=S256&prompt=select_account",
        g.auth_url,
        enc(&g.client_id),
        enc(redirect_uri),
    );
    Ok((url, format!("{state}.{verifier}.{}", URL_SAFE_NO_PAD.encode(safe_return(ret)))))
}

/// The flow cookie's parts: `(state, verifier, return fragment)` (the fragment is "" if absent or unsafe).
#[must_use]
pub fn parse_flow(flow: &str) -> (Option<&str>, Option<&str>, String) {
    let mut parts = flow.splitn(3, '.');
    let (state, verifier, ret) = (parts.next().filter(|s| !s.is_empty()), parts.next(), parts.next());
    let ret = ret.and_then(|r| URL_SAFE_NO_PAD.decode(r).ok()).and_then(|b| String::from_utf8(b).ok());
    (state, verifier, safe_return(&ret.unwrap_or_default()))
}

/// The token request's form fields.
#[must_use]
pub fn token_form(g: &Google, code: &str, redirect_uri: &str, verifier: &str) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("client_id", g.client_id.clone()),
        ("client_secret", g.client_secret.clone()),
        ("redirect_uri", redirect_uri.to_string()),
        ("code_verifier", verifier.to_string()),
    ]
}

/// `application/x-www-form-urlencoded` body.
#[must_use]
pub fn form_body(form: &[(&str, String)]) -> String {
    form.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect::<Vec<_>>().join("&")
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub id_token: String,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    exp: i64,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
    name: Option<String>,
}

/// The identity in an `id_token` received directly from the token endpoint.
///
/// # Errors
/// `bad_token` or `expired` (codes for `?auth_error=`).
pub fn identity_from(id_token: &str, client_id: &str, now_s: i64) -> Result<Identity, &'static str> {
    let payload = id_token.split('.').nth(1).ok_or("bad_token")?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| "bad_token")?;
    let c: Claims = serde_json::from_slice(&bytes).map_err(|_| "bad_token")?;
    if !matches!(c.iss.as_str(), "https://accounts.google.com" | "accounts.google.com") || c.aud != client_id {
        return Err("bad_token");
    }
    if c.exp < now_s {
        return Err("expired");
    }
    Ok(Identity {
        provider: PROVIDER.into(),
        subject: c.sub,
        email: c.email.filter(|_| c.email_verified),
        name: c.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(claims: &serde_json::Value) -> String {
        format!("x.{}.y", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn id_token_checks() {
        let ok = serde_json::json!({"iss": "https://accounts.google.com", "aud": "cid", "sub": "123", "exp": 100,
            "email": "a@b.c", "email_verified": true, "name": "A"});
        let id = identity_from(&token(&ok), "cid", 50).unwrap();
        assert_eq!((id.subject.as_str(), id.email.as_deref()), ("123", Some("a@b.c")));
        assert_eq!(identity_from(&token(&ok), "other", 50), Err("bad_token"));
        assert_eq!(identity_from(&token(&ok), "cid", 101), Err("expired"));
        let mut evil = ok.clone();
        evil["iss"] = "https://evil.example".into();
        assert_eq!(identity_from(&token(&evil), "cid", 50), Err("bad_token"));
        let mut unverified = ok.clone();
        unverified["email_verified"] = false.into();
        assert_eq!(identity_from(&token(&unverified), "cid", 50).unwrap().email, None);
        assert_eq!(identity_from("garbage", "cid", 50), Err("bad_token"));
    }

    #[test]
    fn only_fragments_are_returned_to() {
        assert_eq!(safe_return("#john/3"), "#john/3");
        for bad in ["https://evil.example", "//evil.example", "/x", "#a b", "#\"x"] {
            assert_eq!(safe_return(bad), "", "{bad}");
        }
    }

    #[test]
    fn the_flow_cookie_round_trips() {
        let g = Google::new("c i".into(), "s".into());
        let (url, flow) = start(&g, "http://h/cb", "#john/3").unwrap();
        let (state, verifier, ret) = parse_flow(&flow);
        assert!(url.contains(&format!("state={}", state.unwrap())) && url.contains("client_id=c%20i"));
        assert_eq!((verifier.map(str::len), ret.as_str()), (Some(64), "#john/3"));
        assert_eq!(parse_flow(""), (None, None, String::new()));
        assert_eq!(form_body(&[("a", "x y&".into()), ("b", "1".into())]), "a=x%20y%26&b=1");
    }
}
