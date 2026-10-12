//! Sign in with Google (OAuth 2.0 authorization code + PKCE; the `OpenID` Connect `id_token` gives the identity).
//!
//! Accounts attach to owners: everyone starts as an anonymous owner (`POST /api/session`), and signing in either
//! makes the current anonymous owner an account (a new identity) or, if the identity already has an account,
//! **merges** the anonymous owner's live data into it (`space::merge`; the anonymous data stays where it was) and
//! switches the session to that account. Signing in while signed in to another account just switches accounts.
//! Every sign-in issues a new session token (no session fixation).
//!
//! ```text
//! GET  /api/auth/google?return=#hash   -> 302 to Google (state + PKCE verifier in a short-lived cookie)
//! GET  /api/auth/google/callback       -> 302 to /#hash (or /?auth_error=code#hash)
//! POST /api/logout                     -> ends the session (the client then starts a new anonymous one)
//! ```
//! The `id_token` comes straight from Google's token endpoint over TLS, so its signature need not be checked
//! (`OpenID` Connect Core 3.1.3.7); its `iss`, `aud`, `exp` and `email_verified` still are.

use crate::store::{Identity, Store};
use crate::sync::{
    blocking, internal, new_session_cookie, now_ms, owner_of, random_hex, same_origin, session_token, token_hash,
    AppState,
};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::Arc;

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

const FLOW_COOKIE: &str = "bt_oauth";
const PROVIDER: &str = "google";

fn redirect(to: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to.to_string())]).into_response()
}

/// Only a fragment of this app (`#...`, from `location.hash`) may be returned to; anything else becomes "".
fn safe_return(r: &str) -> String {
    let ok = r.starts_with('#') && r.len() <= 200 && r.bytes().all(|b| b.is_ascii_graphic() && b != b'"');
    if ok {
        r.to_string()
    } else {
        String::new()
    }
}

/// Where Google sends the user back: `--public-url`, else this request's own origin.
fn callback_url(st: &AppState, headers: &HeaderMap) -> String {
    let origin = st.public_url.clone().unwrap_or_else(|| {
        let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
        format!("{}://{host}", if st.secure_cookies { "https" } else { "http" })
    });
    format!("{}/api/auth/google/callback", origin.trim_end_matches('/'))
}

fn flow_cookie(st: &AppState, value: &str, max_age: u32) -> HeaderValue {
    let secure = if st.secure_cookies { "; Secure" } else { "" };
    let c = format!("{FLOW_COOKIE}={value}; Path=/api/auth; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}");
    HeaderValue::from_str(&c).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name && !v.is_empty()).then(|| v.to_string())
    })
}

#[derive(Deserialize)]
pub struct StartQuery {
    #[serde(rename = "return", default)]
    ret: String,
}

/// `GET /api/auth/google`: off to Google's consent screen.
pub async fn google_start(State(st): State<Arc<AppState>>, headers: HeaderMap, q: Query<StartQuery>) -> Response {
    let Some(g) = &st.google else { return (StatusCode::NOT_FOUND, "sign-in is not configured").into_response() };
    let (state, verifier) = match (random_hex(16), random_hex(32)) {
        (Ok(s), Ok(v)) => (s, v),
        (Err(e), _) | (_, Err(e)) => return internal(&e),
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let ret = URL_SAFE_NO_PAD.encode(safe_return(&q.ret));
    let url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope=openid%20email%20profile&state={state}\
         &code_challenge={challenge}&code_challenge_method=S256&prompt=select_account",
        g.auth_url,
        enc(&g.client_id),
        enc(&callback_url(&st, &headers)),
    );
    let mut res = redirect(&url);
    res.headers_mut().insert(header::SET_COOKIE, flow_cookie(&st, &format!("{state}.{verifier}.{ret}"), 600));
    res
}

/// Percent-encode a query value.
fn enc(s: &str) -> String {
    s.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
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
fn identity_from(id_token: &str, client_id: &str, now_s: i64) -> Result<Identity, &'static str> {
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

/// `GET /api/auth/google/callback`: finish signing in, then back to the app.
pub async fn google_callback(State(st): State<Arc<AppState>>, headers: HeaderMap, q: Query<CallbackQuery>) -> Response {
    let Some(g) = st.google.clone() else {
        return (StatusCode::NOT_FOUND, "sign-in is not configured").into_response();
    };
    let flow = cookie(&headers, FLOW_COOKIE).unwrap_or_default();
    let mut parts = flow.splitn(3, '.');
    let (state, verifier, ret) = (parts.next(), parts.next(), parts.next());
    let ret = ret.and_then(|r| URL_SAFE_NO_PAD.decode(r).ok()).and_then(|b| String::from_utf8(b).ok());
    let ret = safe_return(&ret.unwrap_or_default());
    let fail = |code: &str| {
        let mut res = redirect(&format!("/?auth_error={code}{ret}"));
        res.headers_mut().insert(header::SET_COOKIE, flow_cookie(&st, "", 0));
        res
    };
    if q.error.is_some() {
        return fail("cancelled");
    }
    let (Some(code), Some(state), Some(verifier)) = (q.code.clone(), state, verifier) else { return fail("expired") };
    if q.state.as_deref() != Some(state) {
        return fail("expired");
    }
    let form = [
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("client_id", g.client_id.clone()),
        ("client_secret", g.client_secret.clone()),
        ("redirect_uri", callback_url(&st, &headers)),
        ("code_verifier", verifier.to_string()),
    ];
    let token_url = g.token_url.clone();
    let token = blocking(move || -> Result<TokenResponse, String> {
        let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut res = ureq::post(&token_url).send_form(form).map_err(|e| e.to_string())?;
        res.body_mut().read_json::<TokenResponse>().map_err(|e| e.to_string())
    })
    .await;
    let token = match token {
        Ok(Ok(t)) => t,
        Ok(Err(e)) | Err(e) => {
            tracing::warn!("google token exchange: {e}");
            return fail("provider");
        }
    };
    let identity = match identity_from(&token.id_token, &g.client_id, now_ms() / 1000) {
        Ok(i) => i,
        Err(code) => return fail(code),
    };
    let current = owner_of(&st, &headers).await;
    let old_session = session_token(&headers).map(|t| token_hash(&t));
    let store = st.store.clone();
    let signed_in =
        blocking(move || sign_in(store.as_ref(), current.as_deref(), &identity, old_session.as_deref())).await;
    let token = match signed_in {
        Ok(Ok(t)) => t,
        Ok(Err(e)) | Err(e) => {
            tracing::error!("sign-in: {e}");
            return fail("internal");
        }
    };
    let mut res = redirect(&format!("/{ret}"));
    res.headers_mut().append(header::SET_COOKIE, flow_cookie(&st, "", 0));
    if let Ok(v) = HeaderValue::from_str(&new_session_cookie(&st, &token)) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
    res
}

/// Decide the account for `identity` given the session's current owner, and open a new session for it.
/// Returns the new session token.
fn sign_in(store: &dyn Store, current: Option<&str>, identity: &Identity, old: Option<&str>) -> Result<String, String> {
    let now = now_ms();
    let existing = store.identity_owner(&identity.provider, &identity.subject)?;
    let anonymous = match current {
        Some(c) => store.owner_identity(c)?.is_none(),
        None => false,
    };
    let owner = match (current, existing) {
        (_, Some(x)) => {
            if let Some(a) = current.filter(|a| anonymous && *a != x) {
                // Personal spaces: the space id is the owner id.
                let n = store.merge(a, a, &x, &x, now)?;
                tracing::info!("merged {n} changes from anonymous owner {a} into {x}");
            }
            x
        }
        (Some(a), None) if anonymous => {
            store.add_identity(identity, a, now)?;
            a.to_string()
        }
        (_, None) => {
            let o = random_hex(16)?;
            store.create_owner(&o, now)?;
            store.add_identity(identity, &o, now)?;
            o
        }
    };
    if let Some(h) = old {
        store.delete_session(h)?;
    }
    let token = random_hex(32)?;
    store.create_session(&token_hash(&token), &owner, now)?;
    Ok(token)
}

/// `POST /api/logout`.
pub async fn logout(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin").into_response();
    }
    if let Some(t) = session_token(&headers) {
        let (store, h) = (st.store.clone(), token_hash(&t));
        if let Ok(Err(e)) | Err(e) = blocking(move || store.delete_session(&h)).await {
            return internal(&e);
        }
    }
    let secure = if st.secure_cookies { "; Secure" } else { "" };
    let clear = format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}", crate::sync::COOKIE);
    ([(header::SET_COOKIE, clear)], StatusCode::NO_CONTENT).into_response()
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
}
