//! Sessions and the sync WebSocket (protocol: `api::protocol`; session rules: `api::session`).

use crate::hub::{Event, Hub};
use crate::store::{Identity, Store};
use crate::texts::Texts;
use api::groups::Role;
use api::protocol::{ClientMsg, ServerMsg, MAX_FRAME, MAX_HELLO_PENDING, REPLAY_PAGE};
use api::session::{random_hex, token_hash};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
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
    /// The client's `api::CLIENT_VERSION_FILE` (in `--static`), read at each `ready`, so replacing the client
    /// without a restart is noticed too.
    pub client_version: Option<std::path::PathBuf>,
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

#[derive(serde::Deserialize)]
pub struct SyncQuery {
    /// The space to sync: a group the owner belongs to, or (default) their personal space.
    space: Option<String>,
}

/// `GET /api/sync?space=`: upgrade to the sync WebSocket.
pub async fn sync(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<SyncQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin").into_response();
    }
    let Some(owner) = owner_of(&st, &headers).await else {
        return (StatusCode::UNAUTHORIZED, "no session").into_response();
    };
    let space = q.space.unwrap_or_else(|| owner.clone());
    if !crate::store::valid_space(&space) {
        return (StatusCode::BAD_REQUEST, "bad space").into_response();
    }
    // An early refusal; the socket checks again once it can hear kicks (`run_socket`).
    match api::groups::role_of(&crate::groups::Accounts(st.store.clone()), &space, &owner).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::FORBIDDEN, "not a member").into_response(),
        Err(e) => return internal(&e),
    }
    ws.max_message_size(MAX_FRAME).on_upgrade(move |socket| run_socket(socket, st, owner, space))
}

type Tx = futures_util::stream::SplitSink<WebSocket, Message>;

async fn send(tx: &mut Tx, msg: &ServerMsg<'_>) -> bool {
    tx.send(Message::Text(msg.json().into())).await.is_ok()
}

/// The client version served now (`--static`'s `api::CLIENT_VERSION_FILE`, read each time, so a client replaced
/// without a restart is noticed).
async fn client_version(st: &AppState) -> Option<String> {
    let path = st.client_version.as_ref()?;
    tokio::fs::read_to_string(path).await.ok().and_then(|v| api::client_version(&v))
}

/// Reading history is kept for accounts only (an anonymous owner's stays on its device): send an account its whole
/// history here. Returns whether the owner is an account, or `None` if the socket should close.
async fn send_history(st: &AppState, space: &str, owner: &str, tx: &mut Tx) -> Option<bool> {
    let (store, o) = (st.store.clone(), owner.to_string());
    if !matches!(blocking(move || store.owner_identity(&o)).await, Ok(Ok(Some(_)))) {
        return Some(false);
    }
    let (store, s, o) = (st.store.clone(), space.to_string(), owner.to_string());
    match blocking(move || store.history(&s, &o)).await {
        Ok(Ok(entries)) => send(tx, &ServerMsg::History { entries: &entries }).await.then_some(true),
        Ok(Err(e)) | Err(e) => {
            tracing::error!("history: {e}");
            Some(true)
        }
    }
}

/// Store a member's history entries (any role; accounts only, others are ignored) and pass what changed to their
/// sockets, this one included (merging is idempotent).
async fn put_history(st: &AppState, space: &str, owner: &str, account: bool, entries: Vec<space::HistoryEntry>) {
    let entries = api::protocol::check_history(entries, now_ms());
    if !account || entries.is_empty() {
        return;
    }
    let (store, s, o) = (st.store.clone(), space.to_string(), owner.to_string());
    match blocking(move || store.put_history(&s, &o, entries)).await {
        Ok(Ok(changed)) if !changed.is_empty() => st.hub.history(space, owner, changed),
        Ok(Ok(_)) => {}
        Ok(Err(e)) | Err(e) => tracing::error!("history: {e}"),
    }
}

async fn pong(st: &AppState, tx: &mut Tx) -> bool {
    send(tx, &ServerMsg::Pong { client: client_version(st).await.as_deref() }).await
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

/// If the client at `*last` can't be replayed to (`space::needs_reset`), send the whole state instead, advancing
/// `*last`. False if the socket should close.
async fn reset_if_needed(
    st: &Arc<AppState>,
    space: &str,
    tx: &mut Tx,
    last: &mut u64,
    last_op: Option<String>,
    pending: &[String],
) -> bool {
    let (store, s, after, pending) = (st.store.clone(), space.to_string(), *last, pending.to_vec());
    match blocking(move || store.catch_up(&s, after, last_op.as_deref(), &pending)).await {
        Ok(Ok(None)) => true,
        Ok(Ok(Some(reset))) => {
            *last = reset.seq;
            send(tx, &ServerMsg::reset(&reset)).await
        }
        Ok(Err(e)) | Err(e) => {
            tracing::error!("catch up: {e}");
            false
        }
    }
}

async fn run_socket(socket: WebSocket, st: Arc<AppState>, owner: String, space: String) {
    let (mut tx, mut rx) = socket.split();
    // Subscribe before replaying, so nothing committed in between is missed (duplicates are skipped by seq).
    let mut live = st.hub.subscribe(&space);
    // The role was checked before the upgrade, but a removal or role change may have landed since, and its kick
    // reached no one (we weren't subscribed). Check again now that a later kick will reach us.
    let role = match api::groups::role_of(&crate::groups::Accounts(st.store.clone()), &space, &owner).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            let _ = send(&mut tx, &ServerMsg::Error { message: "not a member" }).await;
            return;
        }
        Err(e) => {
            tracing::error!("role of {owner} in {space}: {e}");
            return;
        }
    };
    let (mut last, last_op, mut pending) = loop {
        match rx.next().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMsg>(&t) {
                Ok(ClientMsg::Hello { after, last_op, pending }) => break (after, last_op, pending),
                Ok(ClientMsg::Ping) => {
                    if !pong(&st, &mut tx).await {
                        return;
                    }
                }
                _ => {
                    let _ = send(&mut tx, &ServerMsg::Error { message: "expected hello" }).await;
                    return;
                }
            },
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            _ => return,
        }
    };
    pending.truncate(MAX_HELLO_PENDING);
    if !reset_if_needed(&st, &space, &mut tx, &mut last, last_op, &pending).await {
        return;
    }
    if !send_since(&st, &space, &mut tx, &mut last, None).await {
        // The replay reads page by page while other sockets' ops keep pruning: if its next changes were pruned
        // meanwhile, a reset carries on from there (rather than closing, for a reset on reconnecting).
        let before = last;
        // (No `last_op`: the changes up to `last` came from here just now.)
        if !reset_if_needed(&st, &space, &mut tx, &mut last, None, &pending).await
            || last == before
            || !send_since(&st, &space, &mut tx, &mut last, None).await
        {
            return;
        }
    }
    let client = client_version(&st).await;
    if !send(&mut tx, &ServerMsg::Ready { seq: last, build: api::build(), client: client.as_deref() }).await {
        return;
    }
    let Some(account) = send_history(&st, &space, &owner, &mut tx).await else { return };
    loop {
        tokio::select! {
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if !handle_op(&st, &space, &owner, role, account, &mut tx, &t).await {
                        return;
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                _ => return,
            },
            event = live.recv() => match event {
                Ok(Event::Kick(who)) => {
                    if who == owner {
                        let _ = send(&mut tx, &ServerMsg::Error { message: "membership changed; reconnect" }).await;
                        return;
                    }
                }
                Ok(Event::History(who, entries)) => {
                    if who == owner && !send(&mut tx, &ServerMsg::History { entries: &entries }).await {
                        return;
                    }
                }
                Ok(Event::Change(c)) if c.seq <= last => {}
                Ok(Event::Change(c)) if c.seq == last + 1 => {
                    if !send(&mut tx, &ServerMsg::Change { change: &c }).await {
                        return;
                    }
                    last = c.seq;
                }
                // A gap (should not happen; publishes are ordered): fill it from the store.
                Ok(Event::Change(c)) => {
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

/// Handle one client message after `ready`. Returns false if the socket should close.
async fn handle_op(
    st: &Arc<AppState>,
    space: &str,
    owner: &str,
    role: Role,
    account: bool,
    tx: &mut Tx,
    text: &str,
) -> bool {
    let op = match serde_json::from_str::<ClientMsg>(text) {
        Ok(ClientMsg::Op { op }) => op,
        Ok(ClientMsg::Ping) => return pong(st, tx).await,
        Ok(ClientMsg::History { entries }) => {
            put_history(st, space, owner, account, entries).await;
            return true;
        }
        Ok(ClientMsg::Hello { .. }) | Err(_) => {
            let _ = send(tx, &ServerMsg::Error { message: "expected an op" }).await;
            return false;
        }
    };
    if !role.can_write() {
        let message = "you can view this group but not change it".into();
        return send(tx, &ServerMsg::Reject { op_id: &op.op_id, code: "read_only", message }).await;
    }
    if let Err((code, message)) = api::check::check_op(&st.texts, &op) {
        return send(tx, &ServerMsg::Reject { op_id: &op.op_id, code, message }).await;
    }
    let (store, s, o, op2) = (st.store.clone(), space.to_string(), owner.to_string(), op.clone());
    match blocking(move || store.apply(&s, &o, &op2, now_ms())).await {
        Ok(Ok(applied)) => send(tx, &ServerMsg::Ack { op_id: &op.op_id, seq: applied.change.seq }).await,
        Ok(Err(ApplyError::Op(e))) => {
            send(tx, &ServerMsg::Reject { op_id: &op.op_id, code: e.code(), message: e.to_string() }).await
        }
        Ok(Err(ApplyError::Refused(code))) => {
            let message = "only its author can change a comment".into();
            send(tx, &ServerMsg::Reject { op_id: &op.op_id, code, message }).await
        }
        // Not the op's fault (storage failed): a reject would make the client drop it for good, so close instead;
        // the client reconnects and re-sends it (ops are idempotent).
        Ok(Err(ApplyError::Internal(e))) | Err(e) => {
            tracing::error!("apply {}: {e}", op.op_id);
            let _ = send(tx, &ServerMsg::Error { message: "storage error; reconnect" }).await;
            false
        }
    }
}
