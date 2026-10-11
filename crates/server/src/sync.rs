//! Sessions and the sync WebSocket.
//!
//! Auth is a placeholder until real auth exists: `POST /api/session` creates an anonymous owner and sets an
//! `HttpOnly` session cookie (only the token's sha256 is stored). The owner always comes from the session, never
//! from a payload. Both endpoints require a same-origin `Origin` (CSRF / cross-site WebSocket hijacking).
//!
//! Protocol (JSON text frames):
//! ```text
//! client: {"type":"hello","after":seq}             first message: the last seq the client has
//!         {"type":"op","op":{op_id,kind,...}}      an ops::Op (idempotent by op_id)
//! server: {"type":"change","change":{seq,...}}     every change after `after`, then live, in seq order
//!         {"type":"ready","seq":n}                 replay finished
//!         {"type":"ack","op_id":..,"seq":n}        op applied (its change arrives as a `change` message)
//!         {"type":"reject","op_id":..,"code":..,"message":..}   op refused; it had no effect
//!         {"type":"error","message":..}            then the server closes; the client reconnects with hello
//! ```

use crate::hub::Hub;
use crate::store::{ApplyError, Store};
use crate::texts::Texts;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use ops::{Change, Intent, Op};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub hub: Arc<Hub>,
    pub texts: Texts,
    /// Mark the session cookie `Secure` (when serving TLS).
    pub secure_cookies: bool,
}

const COOKIE: &str = "bt_session";
const REPLAY_PAGE: usize = 500;
/// Incoming frames: an op is small, except a comment (<= 10,000 code points, up to 4 bytes each, plus JSON escaping).
const MAX_FRAME: usize = 128 * 1024;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn random_hex(n: usize) -> Result<String, String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(hex(&buf))
}

fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == COOKIE && !v.is_empty()).then(|| v.to_string())
    })
}

/// Browsers always send `Origin` on cross-site requests and WebSocket handshakes; it must name this host.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    origin.split_once("://").is_some_and(|(_, rest)| rest == host)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())
}

async fn owner_of(st: &Arc<AppState>, headers: &HeaderMap) -> Option<String> {
    let hash = token_hash(&session_token(headers)?);
    let store = st.store.clone();
    blocking(move || store.session_owner(&hash)).await.ok()?.ok()?
}

fn internal(e: &str) -> Response {
    tracing::error!("{e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
}

/// `POST /api/session`: the current owner, creating an anonymous one (and its cookie) if needed.
pub async fn session(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin").into_response();
    }
    if let Some(owner) = owner_of(&st, &headers).await {
        return Json(serde_json::json!({ "owner": owner })).into_response();
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
    let secure = if st.secure_cookies { "; Secure" } else { "" };
    let cookie = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000{secure}");
    ([(header::SET_COOKIE, cookie)], Json(serde_json::json!({ "owner": owner }))).into_response()
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

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Hello { after: u64 },
    Op { op: Op },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg<'a> {
    Change { change: &'a Change },
    Ready { seq: u64 },
    Ack { op_id: &'a str, seq: u64 },
    Reject { op_id: &'a str, code: &'a str, message: String },
    Error { message: &'a str },
}

type Tx = futures_util::stream::SplitSink<WebSocket, Message>;

async fn send(tx: &mut Tx, msg: &ServerMsg<'_>) -> bool {
    match serde_json::to_string(msg) {
        Ok(json) => tx.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

/// Send stored changes after `*last` (up to `until`, if given), advancing `*last`.
async fn send_since(st: &Arc<AppState>, owner: &str, tx: &mut Tx, last: &mut u64, until: Option<u64>) -> bool {
    loop {
        let (store, o, after) = (st.store.clone(), owner.to_string(), *last);
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
    // Subscribe before replaying, so nothing committed in between is missed (duplicates are skipped by seq).
    let mut live = st.hub.subscribe(&owner);
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
    if !send_since(&st, &owner, &mut tx, &mut last, None).await || !send(&mut tx, &ServerMsg::Ready { seq: last }).await
    {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if !handle_op(&st, &owner, &mut tx, &t).await {
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
                    if !send_since(&st, &owner, &mut tx, &mut last, Some(c.seq)).await {
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

/// Ids are client-generated (ULIDs): bound their shape so they cannot be abused as storage.
fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn check_op(st: &AppState, op: &Op) -> Result<(), (&'static str, String)> {
    let ids: Vec<&str> = match &op.intent {
        Intent::Paint { tag, new_id, .. } => vec![tag, new_id],
        Intent::Erase { tag, split_id, .. } => {
            [Some(tag.as_str()), split_id.as_deref()].into_iter().flatten().collect()
        }
        Intent::TagCreate { id, .. }
        | Intent::TagUpdate { id, .. }
        | Intent::TagDelete { id, .. }
        | Intent::CommentUpdate { id, .. }
        | Intent::CommentDelete { id, .. } => vec![id],
        Intent::CommentCreate { id, tagging, .. } => vec![id, tagging],
        Intent::PaletteSet { tags, .. } => tags.iter().map(String::as_str).collect(),
        Intent::Revert { target, .. } => vec![target],
    };
    if !valid_id(&op.op_id) || !ids.into_iter().all(valid_id) {
        return Err(("bad_id", "ids must be 1-64 characters of [0-9A-Za-z_-]".into()));
    }
    if let Intent::Paint { text_id, start, end, .. } | Intent::Erase { text_id, start, end, .. } = &op.intent {
        st.texts.check(text_id, *start, *end).map_err(|m| ("bad_text", m))?;
    }
    Ok(())
}

/// Apply one client message. Returns false if the socket should close.
async fn handle_op(st: &Arc<AppState>, owner: &str, tx: &mut Tx, text: &str) -> bool {
    let op = match serde_json::from_str::<ClientMsg>(text) {
        Ok(ClientMsg::Op { op }) => op,
        Ok(ClientMsg::Hello { .. }) | Err(_) => {
            let _ = send(tx, &ServerMsg::Error { message: "expected an op" }).await;
            return false;
        }
    };
    if let Err((code, message)) = check_op(st, &op) {
        return send(tx, &ServerMsg::Reject { op_id: &op.op_id, code, message }).await;
    }
    let (store, o, op2) = (st.store.clone(), owner.to_string(), op.clone());
    match blocking(move || store.apply(&o, &op2, now_ms())).await {
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
