//! Sign in with Google (flow and rules: `api::google`, `api::accounts`).

use crate::store::{Identity, Store};
use crate::sync::{blocking, internal, new_session_cookie, now_ms, owner_of, same_origin, session_token, AppState};
use api::accounts::SignIn;
pub use api::google::Google;
use api::google::{callback_url as callback_for, flow_cookie as flow_value, parse_flow, TokenResponse};
use api::session::{random_hex, token_hash};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::sync::Arc;

fn redirect(to: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to.to_string())]).into_response()
}

/// Where Google sends the user back: `--public-url`, else this request's own origin.
fn callback_url(st: &AppState, headers: &HeaderMap) -> String {
    let origin = st.public_url.clone().unwrap_or_else(|| {
        let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
        format!("{}://{host}", if st.secure_cookies { "https" } else { "http" })
    });
    callback_for(&origin)
}

fn flow_cookie(st: &AppState, value: &str, max_age: u32) -> HeaderValue {
    HeaderValue::from_str(&flow_value(value, max_age, st.secure_cookies))
        .unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    api::session::cookie(headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()), name)
}

#[derive(Deserialize)]
pub struct StartQuery {
    #[serde(rename = "return", default)]
    ret: String,
}

/// `GET /api/auth/google`: off to Google's consent screen.
pub async fn google_start(State(st): State<Arc<AppState>>, headers: HeaderMap, q: Query<StartQuery>) -> Response {
    let Some(g) = &st.google else { return (StatusCode::NOT_FOUND, "sign-in is not configured").into_response() };
    let (url, flow) = match api::google::start(g, &callback_url(&st, &headers), &q.ret) {
        Ok(x) => x,
        Err(e) => return internal(&e),
    };
    let mut res = redirect(&url);
    res.headers_mut().insert(header::SET_COOKIE, flow_cookie(&st, &flow, 600));
    res
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /api/auth/google/callback`: finish signing in, then back to the app.
pub async fn google_callback(State(st): State<Arc<AppState>>, headers: HeaderMap, q: Query<CallbackQuery>) -> Response {
    let Some(g) = st.google.clone() else {
        return (StatusCode::NOT_FOUND, "sign-in is not configured").into_response();
    };
    let flow = cookie(&headers, api::google::FLOW_COOKIE).unwrap_or_default();
    let (state, verifier, ret) = parse_flow(&flow);
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
    let form = api::google::token_form(&g, &code, &callback_url(&st, &headers), verifier);
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
    let identity = match api::google::identity_from(&token.id_token, &g.client_id, now_ms() / 1000) {
        Ok(i) => i,
        Err(code) => return fail(code),
    };
    let current = owner_of(&st, &headers).await;
    let old_session = session_token(&headers).map(|t| token_hash(&t));
    let token = match sign_in(&st, current, identity, old_session).await {
        Ok(t) => t,
        Err(e) => {
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

/// Decide the account for `identity` given the session's current owner (merging an anonymous owner into an
/// existing account: its personal data, its groups and its comments in them), and open a new session for it.
/// Returns the new session token.
async fn sign_in(
    st: &Arc<AppState>,
    current: Option<String>,
    identity: Identity,
    old: Option<String>,
) -> Result<String, String> {
    let store = st.store.clone();
    let (owner, merged) = blocking(move || account_for(store.as_ref(), current.as_deref(), &identity)).await??;
    if let Some(from) = merged {
        carry_groups(st, &from, &owner).await?;
    }
    let store = st.store.clone();
    blocking(move || {
        let now = now_ms();
        if let Some(h) = old {
            store.delete_session(&h)?;
        }
        let token = random_hex(32)?;
        store.create_session(&token_hash(&token), &owner, now)?;
        Ok(token)
    })
    .await?
}

/// The account `identity` signs in to; also the anonymous owner merged into it, if any (its personal data is
/// copied here; `carry_groups` does the rest).
fn account_for(
    store: &dyn Store,
    current: Option<&str>,
    identity: &Identity,
) -> Result<(String, Option<String>), String> {
    let now = now_ms();
    let existing = store.identity_owner(&identity.provider, &identity.subject)?;
    let anonymous = match current {
        Some(c) => store.owner_identity(c)?.is_none(),
        None => false,
    };
    Ok(match api::accounts::sign_in(current, anonymous, existing.as_deref()) {
        SignIn::Existing { owner, merge_from } => {
            if let Some(a) = &merge_from {
                // Personal spaces: the space id is the owner id.
                let n = store.merge(a, a, &owner, &owner, now)?;
                tracing::info!("merged {n} changes from anonymous owner {a} into {owner}");
            }
            (owner, merge_from)
        }
        SignIn::Link(a) => {
            store.add_identity(identity, &a, now)?;
            (a, None)
        }
        SignIn::New => {
            let o = random_hex(16)?;
            store.create_owner(&o, now)?;
            store.add_identity(identity, &o, now)?;
            (o, None)
        }
    })
}

/// The anonymous owner `from`, merged into account `to`, may have joined (or created) groups: its comments there
/// become `to`'s (so they stay editable), its sockets there are closed, then its memberships move to `to`.
async fn carry_groups(st: &Arc<AppState>, from: &str, to: &str) -> Result<(), String> {
    let db = crate::groups::Accounts(st.store.clone());
    let now = now_ms();
    for space in api::groups::spaces_of(&db, from).await? {
        let (store, s, f, t) = (st.store.clone(), space.clone(), from.to_string(), to.to_string());
        blocking(move || store.reassign_author(&s, &f, &t, now)).await??;
        st.hub.kick(&space, from);
    }
    api::groups::move_memberships(&db, from, to, now).await?;
    Ok(())
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
    let clear = api::session::clear_session_cookie(st.secure_cookies);
    ([(header::SET_COOKIE, clear)], StatusCode::NO_CONTENT).into_response()
}
