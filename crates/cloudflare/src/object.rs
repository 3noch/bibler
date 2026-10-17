//! The `Space` Durable Object: one space's SQLite database (`crates/space`, the same SQL as the self-hosted
//! server's `spaces/<id>.sqlite`) and its sync `WebSockets` (`api::protocol`), using the hibernation API so an idle
//! space costs nothing while its sockets stay open.
//!
//! A Durable Object runs one event at a time, and nothing below awaits between reading and writing, so each op (and
//! each replay) is atomic and every socket sees changes in seq order without a hub: a change is sent to every ready
//! socket right after it is stored. `space::apply` checks everything before its first write (there is no rollback).
//!
//! Only the Worker can reach a Durable Object; it authenticates the session, checks the owner's role in the space
//! (`api::groups`, for an early 403) and passes them in headers; the object checks the role again itself (see
//! `/sync`):
//! ```text
//! GET  /sync   (WebSocket upgrade, x-owner, x-space)   the sync protocol
//! POST /kick?owner=                           close that owner's sockets (their role changed or they left)
//! GET  /export?owner=   -> [Row]              an owner's live data (space::export)
//! POST /merge  {owner, from, rows} -> n       copy another owner's exported data in (space::merge)
//! POST /reassign {space, from, to}            `from`'s comments here become `to`'s (space::reassign_author)
//! ```

use api::check::Texts;
use api::protocol::{ClientMsg, ServerMsg, MAX_FRAME, MAX_HELLO_PENDING, REPLAY_PAGE};
use ops::Row;
use serde::{Deserialize, Serialize};
use space::{ApplyError, Db, Value};
use std::cell::RefCell;
use std::rc::Rc;
use worker::{
    durable_object, DurableObject, Env, Request, Response, Result, SqlStorage, SqlStorageValue, State, WebSocket,
    WebSocketIncomingMessage, WebSocketPair,
};

#[durable_object]
pub struct Space {
    state: State,
    env: Env,
    db: Sql,
    texts: Rc<RefCell<Texts>>,
    /// How long changes stay replayable: `space::RETENTION_MS`, or the `RETENTION_MS` var (tests).
    retention_ms: i64,
}

/// Per-socket state, kept across hibernation.
#[derive(Serialize, Deserialize)]
struct Socket {
    /// Who is connected (the author of their ops).
    owner: String,
    /// This space's id, the data owner of its rows.
    space: String,
    /// May send ops (role edit or admin).
    write: bool,
    /// `hello` was answered: the socket gets live changes.
    ready: bool,
}

// No awaits in the socket handlers is the point: each event is atomic.
#[allow(clippy::unused_async_trait_impl)]
impl DurableObject for Space {
    fn new(state: State, env: Env) -> Self {
        let db = Sql(state.storage().sql());
        let retention_ms =
            env.var("RETENTION_MS").ok().and_then(|v| v.to_string().parse().ok()).unwrap_or(space::RETENTION_MS);
        if let Err(e) = space::init(&db) {
            worker::console_error!("space schema: {e}");
        }
        // Events wait until the texts are loaded, so ops are checked (and applied) in arrival order.
        let texts = Rc::new(RefCell::new(Texts::default()));
        let (slot, assets) = (texts.clone(), env.clone());
        drop(state.block_concurrency_while(async move {
            match load_texts(&assets).await {
                Ok(t) => *slot.borrow_mut() = t,
                Err(e) => worker::console_error!("loading texts: {e}"),
            }
            Ok(())
        }));
        Space { state, env, db, texts, retention_ms }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let url = req.url()?;
        match url.path() {
            "/sync" => {
                let h = req.headers();
                let (Some(owner), Some(space)) = (h.get("x-owner")?, h.get("x-space")?) else {
                    return Response::error("no owner", 400);
                };
                // The Worker checked the role, but a removal or role change may have landed since, and its kick
                // found no socket to close. Check again while no other event runs: a later kick finds this socket.
                let (env, o, s) = (self.env.clone(), owner.clone(), space.clone());
                let role = self
                    .state
                    .block_concurrency_while(async move {
                        let db = crate::D1Accounts(env.d1("ACCOUNTS")?);
                        api::groups::role_of(&db, &s, &o).await.map_err(worker::Error::RustError)
                    })
                    .await?;
                let Some(role) = role else { return Response::error("not a member", 403) };
                let write = role.can_write();
                let pair = WebSocketPair::new()?;
                self.state.accept_websocket_with_tags(&pair.server, &[&owner]);
                pair.server.serialize_attachment(Socket { owner, space, write, ready: false })?;
                Response::from_websocket(pair.client)
            }
            "/export" => {
                let owner = url.query_pairs().find(|(k, _)| k == "owner").map(|(_, v)| v.into_owned());
                let Some(owner) = owner else { return Response::error("no owner", 400) };
                Response::from_json(&space::export(&self.db, &owner).map_err(worker::Error::RustError)?)
            }
            "/kick" => {
                let owner = url.query_pairs().find(|(k, _)| k == "owner").map(|(_, v)| v.into_owned());
                for ws in self.state.get_websockets_with_tag(&owner.unwrap_or_default()) {
                    let _ = close(&ws, "membership changed; reconnect");
                }
                Response::ok("")
            }
            "/reassign" => {
                let r: Reassign = req.json().await?;
                let change = space::reassign_author(&self.db, &r.space, &r.from, &r.to, now_ms())
                    .map_err(worker::Error::RustError)?;
                if let Some(c) = &change {
                    self.broadcast(&ServerMsg::Change { change: c }.json());
                }
                Response::ok("")
            }
            "/merge" => {
                let m: Merge = req.json().await?;
                let now = now_ms();
                let changes =
                    space::merge(&self.db, &m.owner, &m.from, &m.rows, now).map_err(worker::Error::RustError)?;
                for c in &changes {
                    self.broadcast(&ServerMsg::Change { change: c }.json());
                }
                Response::from_json(&changes.len())
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn websocket_message(&self, ws: WebSocket, message: WebSocketIncomingMessage) -> Result<()> {
        let WebSocketIncomingMessage::String(text) = message else { return Ok(()) };
        let Some(mut socket) = ws.deserialize_attachment::<Socket>()? else { return close(&ws, "no session") };
        if text.len() > MAX_FRAME {
            return close(&ws, "message too large");
        }
        match (socket.ready, serde_json::from_str::<ClientMsg>(&text)) {
            (false, Ok(ClientMsg::Hello { after, last_op, mut pending })) => {
                let mut after = after;
                // A replay can't be done or trusted (`space::needs_reset`): send the whole state instead.
                if space::needs_reset(&self.db, after, last_op.as_deref()).map_err(worker::Error::RustError)? {
                    pending.truncate(MAX_HELLO_PENDING);
                    let reset = space::reset(&self.db, &socket.space, &pending).map_err(worker::Error::RustError)?;
                    ws.send_with_str(ServerMsg::reset(&reset).json())?;
                    after = reset.seq;
                }
                let seq = self.replay(&ws, after)?;
                ws.send_with_str(ServerMsg::Ready { seq }.json())?;
                socket.ready = true;
                ws.serialize_attachment(socket)
            }
            (true, Ok(ClientMsg::Op { op })) => self.op(&ws, &socket, &op),
            (false, _) => close(&ws, "expected hello"),
            (true, _) => close(&ws, "expected an op"),
        }
    }

    async fn websocket_close(&self, ws: WebSocket, code: usize, _reason: String, _clean: bool) -> Result<()> {
        // 1005/1006 mean "no status"; they may not be sent back.
        let code = u16::try_from(code).ok().filter(|c| !matches!(c, 1005 | 1006)).unwrap_or(1000);
        let _ = ws.close(Some(code), Some("bye"));
        Ok(())
    }

    async fn websocket_error(&self, _ws: WebSocket, _error: worker::Error) -> Result<()> {
        Ok(())
    }
}

#[derive(Deserialize)]
struct Reassign {
    space: String,
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct Merge {
    owner: String,
    from: String,
    rows: Vec<Row>,
}

impl Space {
    /// Send every change after `after`; returns the last seq sent (or `after`).
    fn replay(&self, ws: &WebSocket, mut after: u64) -> Result<u64> {
        loop {
            let page = space::changes_since(&self.db, after, REPLAY_PAGE).map_err(worker::Error::RustError)?;
            for c in &page {
                ws.send_with_str(ServerMsg::Change { change: c }.json())?;
                after = c.seq;
            }
            if page.len() < REPLAY_PAGE {
                return Ok(after);
            }
        }
    }

    fn op(&self, ws: &WebSocket, socket: &Socket, op: &ops::Op) -> Result<()> {
        let reject = |code: &str, message: String| {
            ws.send_with_str(ServerMsg::Reject { op_id: &op.op_id, code, message }.json())
        };
        if !socket.write {
            return reject("read_only", "you can view this group but not change it".into());
        }
        if let Err((code, message)) = api::check::check_op(&self.texts.borrow(), op) {
            return reject(code, message);
        }
        let now = now_ms();
        match space::apply_by(&self.db, &socket.space, &socket.owner, op, now) {
            Ok(applied) => {
                if !applied.duplicate {
                    self.broadcast(&ServerMsg::Change { change: &applied.change }.json());
                    // A little pruning per op keeps up without long-running work.
                    if let Err(e) = space::prune(&self.db, now.saturating_sub(self.retention_ms), space::PRUNE_BATCH) {
                        worker::console_error!("prune: {e}");
                    }
                }
                ws.send_with_str(ServerMsg::Ack { op_id: &op.op_id, seq: applied.change.seq }.json())
            }
            Err(ApplyError::Op(e)) => reject(e.code(), e.to_string()),
            Err(ApplyError::Refused(code)) => reject(code, "only its author can change a comment".into()),
            // Not the op's fault (storage failed): a reject would make the client drop it for good, so close instead;
            // the client reconnects and re-sends it (ops are idempotent).
            Err(ApplyError::Internal(e)) => {
                worker::console_error!("apply {}: {e}", op.op_id);
                close(ws, "storage error; reconnect")
            }
        }
    }

    /// Send to every socket that has finished its replay.
    fn broadcast(&self, json: &str) {
        for ws in self.state.get_websockets() {
            if ws.deserialize_attachment::<Socket>().ok().flatten().is_some_and(|s| s.ready) {
                let _ = ws.send_with_str(json);
            }
        }
    }
}

fn close(ws: &WebSocket, message: &str) -> Result<()> {
    let _ = ws.send_with_str(ServerMsg::Error { message }.json());
    ws.close(Some(1008), Some(message))
}

pub(crate) fn now_ms() -> i64 {
    i64::try_from(worker::Date::now().as_millis()).unwrap_or(i64::MAX)
}

/// The texts listed in `/text/texts.json` (written by `make cloudflare`), read from the static assets.
async fn load_texts(env: &Env) -> Result<Texts> {
    let assets = env.assets("ASSETS")?;
    let get = |path: String| {
        let assets = &assets;
        async move {
            let mut res = assets.fetch(format!("https://assets.local/text/{path}"), None).await?;
            if res.status_code() != 200 {
                return Err(worker::Error::RustError(format!("{path}: HTTP {}", res.status_code())));
            }
            res.bytes().await
        }
    };
    let ids: Vec<String> = serde_json::from_slice(&get("texts.json".into()).await?)?;
    let mut texts = Texts::default();
    for id in ids.iter().filter(|id| api::check::valid_id(id)) {
        let manifest: serde_json::Value = serde_json::from_slice(&get(format!("{id}/manifest.json")).await?)?;
        let index = manifest["index"].as_str().unwrap_or_default();
        let bytes = get(format!("{id}/{index}")).await?;
        texts.add_index(id, &bytes).map_err(worker::Error::RustError)?;
    }
    Ok(texts)
}

/// `space::Db` over Durable Object SQL storage.
struct Sql(SqlStorage);

fn bind(params: &[Value]) -> Vec<SqlStorageValue> {
    params
        .iter()
        .map(|v| match v {
            Value::Null => SqlStorageValue::Null,
            Value::Int(n) => SqlStorageValue::Integer(*n),
            Value::Text(s) => SqlStorageValue::String(s.clone()),
        })
        .collect()
}

impl Db for Sql {
    fn exec(&self, sql: &str, params: &[Value]) -> std::result::Result<(), String> {
        let cursor = self.0.exec(sql, bind(params)).map_err(|e| e.to_string())?;
        // Run the statement to completion.
        cursor.raw().try_for_each(|r| r.map(drop)).map_err(|e| e.to_string())
    }

    fn query(&self, sql: &str, params: &[Value]) -> std::result::Result<Vec<Vec<Value>>, String> {
        let cursor = self.0.exec(sql, bind(params)).map_err(|e| e.to_string())?;
        cursor
            .raw()
            .map(|row| {
                row.map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|v| match v {
                        SqlStorageValue::Null => Ok(Value::Null),
                        SqlStorageValue::Integer(n) => Ok(Value::Int(n)),
                        SqlStorageValue::String(s) => Ok(Value::Text(s)),
                        other => Err(format!("unexpected SQL value {other:?}")),
                    })
                    .collect()
            })
            .collect()
    }
}
