//! Sessions and the sync WebSocket (protocol: `api::protocol`; session rules: `api::session`).

use crate::hub::Hub;
use crate::store::{Identity, Store};
use crate::texts::Texts;
use api::protocol::{ClientMsg, ServerMsg, MAX_FRAME, REPLAY_PAGE};
use api::session::{random_hex, token_hash};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use space::ApplyError;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub hub: Arc<Hub>,
    pub texts: Texts,
    /// Mark the session cookie `Secure` (when serving TLS).
    pub secure_cookies: bool,
    /// Sign in with Google, if configured.
    pub google: Option<crate::auth::Google>,
    /// This server's public origin (`https://example.org`), for OAuth redirects; default: the request's `Host`.
    pub public_url: Option<String>,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub(crate) fn session_token(headers: &HeaderMap) -> Option<String> {
    api::session::cookie(headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()), api::session::COOKIE)
}

/// Browsers always send `Origin` on cross-site requests and WebSocket handshakes; it must name this host.
pub(crate) fn same_origin(headers: &HeaderMap) -> bool {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    api::session::same_origin(origin, host)
}

pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())
}

pub(crate) async fn owner_of(st: &Arc<AppState>, headers: &HeaderMap) -> Option<String> {
    let hash = token_hash(&session_token(headers)?);
    let store = st.store.clone();
    blocking(move || store.session_owner(&hash)).await.ok()?.ok()?
}

pub(crate) fn internal(e: &str) -> Response {
    tracing::error!("{e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
}

/// The session cookie for `token` (a year; renewed by signing in again).
pub(crate) fn new_session_cookie(st: &AppState, token: &str) -> String {
    api::session::session_cookie(token, st.secure_cookies)
}

/// What `POST /api/session` returns.
fn session_json(st: &AppState, owner: &str, account: Option<&Identity>) -> Json<serde_json::Value> {
    let providers: &[&str] = if st.google.is_some() { &["google"] } else { &[] };
    Json(api::session::session_json(owner, account, providers))
}

/// `POST /api/session`: the current owner and its account (`null` while anonymous), creating an anonymous owner
/// (and its cookie) if needed. `providers` lists the configured sign-in methods.
pub async fn session(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin").into_response();
    }
    if let Some(owner) = owner_of(&st, &headers).await {
        let (store, o) = (st.store.clone(), owner.clone());
        return match blocking(move || store.owner_identity(&o)).await {
            Ok(Ok(account)) => session_json(&st, &owner, account.as_ref()).into_response(),
            Ok(Err(e)) | Err(e) => internal(&e),
        };
    }
    let (owner, token) = match (random_hex(16), random_hex(32)) {
        (Ok(o), Ok(t)) => (o, t),
        (Err(e), _) | (_, Err(e)) => return internal(&e),
    };
    let (store, o, h) = (st.store.clone(), owner.clone(), token_hash(&token));
    let created = blocking(move || {
        let now = now_ms();
        store.create_owner(&o, now)?;
        store.create_session(&h, &o, now)
    })
    .await;
    if let Err(e) | Ok(Err(e)) = created {
        return internal(&e);
    }
    ([(header::SET_COOKIE, new_session_cookie(&st, &token))], session_json(&st, &owner, None)).into_response()
}

/// `GET /api/sync`: upgrade to the sync WebSocket.
pub async fn sync(State(st): State<Arc<AppState>>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin").into_response();
    }
    let Some(owner) = owner_of(&st, &headers).await else {
        return (StatusCode::UNAUTHORIZED, "no session").into_response();
    };
    ws.max_message_size(MAX_FRAME).on_upgrade(move |socket| run_socket(socket, st, owner))
}

type Tx = futures_util::stream::SplitSink<WebSocket, Message>;

async fn send(tx: &mut Tx, msg: &ServerMsg<'_>) -> bool {
    tx.send(Message::Text(msg.json().into())).await.is_ok()
}

/// Send the space's stored changes after `*last` (up to `until`, if given), advancing `*last`.
async fn send_since(st: &Arc<AppState>, space: &str, tx: &mut Tx, last: &mut u64, until: Option<u64>) -> bool {
    loop {
        let (store, o, after) = (st.store.clone(), space.to_string(), *last);
        let page = match blocking(move || store.changes_since(&o, after, REPLAY_PAGE)).await {
            Ok(Ok(p)) => p,
            Ok(Err(e)) | Err(e) => {
                tracing::error!("replay: {e}");
                return false;
            }
        };
        for c in &page {
            if until.is_some_and(|u| c.seq > u) {
                return true;
            }
            if !send(tx, &ServerMsg::Change { change: c }).await {
                return false;
            }
            *last = c.seq;
        }
        if page.len() < REPLAY_PAGE {
            return true;
        }
    }
}

async fn run_socket(socket: WebSocket, st: Arc<AppState>, owner: String) {
    let (mut tx, mut rx) = socket.split();
    let space = owner.clone(); // the owner's personal space (shared spaces will be chosen in `hello`)
                               // Subscribe before replaying, so nothing committed in between is missed (duplicates are skipped by seq).
    let mut live = st.hub.subscribe(&space);
    let mut last = loop {
        match rx.next().await {
            Some(Ok(Message::Text(t))) => {
                if let Ok(ClientMsg::Hello { after }) = serde_json::from_str::<ClientMsg>(&t) {
                    break after;
                }
                let _ = send(&mut tx, &ServerMsg::Error { message: "expected hello" }).await;
                return;
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            _ => return,
        }
    };
    if !send_since(&st, &space, &mut tx, &mut last, None).await || !send(&mut tx, &ServerMsg::Ready { seq: last }).await
    {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if !handle_op(&st, &space, &owner, &mut tx, &t).await {
                        return;
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                _ => return,
            },
            change = live.recv() => match change {
                Ok(c) if c.seq <= last => {}
                Ok(c) if c.seq == last + 1 => {
                    if !send(&mut tx, &ServerMsg::Change { change: &c }).await {
                        return;
                    }
                    last = c.seq;
                }
                // A gap (should not happen; publishes are ordered): fill it from the store.
                Ok(c) => {
                    if !send_since(&st, &space, &mut tx, &mut last, Some(c.seq)).await {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    let _ = send(&mut tx, &ServerMsg::Error { message: "lagged; reconnect" }).await;
                    return;
                }
                Err(RecvError::Closed) => return,
            },
        }
    }
}

/// Apply one client message. Returns false if the socket should close.
async fn handle_op(st: &Arc<AppState>, space: &str, owner: &str, tx: &mut Tx, text: &str) -> bool {
    let op = match serde_json::from_str::<ClientMsg>(text) {
        Ok(ClientMsg::Op { op }) => op,
        Ok(ClientMsg::Hello { .. }) | Err(_) => {
            let _ = send(tx, &ServerMsg::Error { message: "expected an op" }).await;
            return false;
        }
    };
    if let Err((code, message)) = api::check::check_op(&st.texts, &op) {
        return send(tx, &ServerMsg::Reject { op_id: &op.op_id, code, message }).await;
    }
    let (store, s, o, op2) = (st.store.clone(), space.to_string(), owner.to_string(), op.clone());
    match blocking(move || store.apply(&s, &o, &op2, now_ms())).await {
        Ok(Ok(applied)) => send(tx, &ServerMsg::Ack { op_id: &op.op_id, seq: applied.change.seq }).await,
        Ok(Err(ApplyError::Op(e))) => {
            send(tx, &ServerMsg::Reject { op_id: &op.op_id, code: e.code(), message: e.to_string() }).await
        }
        Ok(Err(ApplyError::Internal(e))) | Err(e) => {
            tracing::error!("apply {}: {e}", op.op_id);
            send(tx, &ServerMsg::Reject { op_id: &op.op_id, code: "internal", message: "internal error".into() }).await
        }
    }
}
