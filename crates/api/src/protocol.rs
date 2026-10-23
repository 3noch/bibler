//! The sync WebSocket (`GET /api/sync`), JSON text frames:
//! ```text
//! client: {"type":"hello","after":seq,"last_op":op_id,"pending":[op_id]}  first message: the last seq the
//!                                                  client has and that change's op (so a space restored from an
//!                                                  older backup is noticed), and the ops it holds unconfirmed
//!                                                  (only used for a reset)
//!         {"type":"op","op":{op_id,kind,...}}      an ops::Op (idempotent by op_id)
//!         {"type":"ping"}                          at any time, exactly this text: answered `{"type":"pong"}`
//!                                                  (the client checks its socket is still alive after resuming)
//! server: {"type":"change","change":{seq,...}}     every change after `after`, then live, in seq order
//!         {"type":"reset","seq":n,"last_op":..,"rows":[..],"changes":[],"applied":[op_id]}   instead of the
//!                                                  replay when one can't be trusted or done (`space::needs_reset`:
//!                                                  changes after `after` were pruned, or the space has a different
//!                                                  change at `after` or none): the whole state at seq n;
//!                                                  `applied` = which of `pending` are already in it
//!         {"type":"ready","seq":n,"build":..,"client":..}   replay finished
//!         {"type":"ack","op_id":..,"seq":n}        op applied (its change arrives as a `change` message, unless
//!                                                  it was applied long ago: seq <= the client's, change pruned)
//!         {"type":"reject","op_id":..,"code":..,"message":..}   op refused for good (the client drops it); it had
//!                                                  no effect. A storage failure is an `error` instead, so the op is
//!                                                  re-sent after reconnecting
//!         {"type":"error","message":..}            then the server closes; the client reconnects with hello
//! ```

use ops::{Change, Op, Row};
use serde::{Deserialize, Serialize};

/// A heartbeat, matched as exact text (the Durable Object answers it with an auto-response, without waking).
pub const PING: &str = r#"{"type":"ping"}"#;
/// The answer to [`PING`].
pub const PONG: &str = r#"{"type":"pong"}"#;

/// Changes per replay query.
pub const REPLAY_PAGE: usize = 500;
/// `hello.pending` op ids considered at most (a client rarely holds more).
pub const MAX_HELLO_PENDING: usize = 1000;
/// Incoming frames: an op is small, except a comment (<= 10,000 code points, up to 4 bytes each, plus JSON escaping).
pub const MAX_FRAME: usize = 128 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        after: u64,
        /// The op of the client's change at `after` (absent from older clients: then only a missing change counts).
        #[serde(default)]
        last_op: Option<String>,
        #[serde(default)]
        pending: Vec<String>,
    },
    Op {
        op: Op,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg<'a> {
    Change {
        change: &'a Change,
    },
    /// The replay is done; `build` is the server's (`crate::build`), `client` the client version it serves now
    /// (`crate::client_version`; none in dev, where Vite serves the client).
    Ready {
        seq: u64,
        build: &'a crate::Build,
        #[serde(skip_serializing_if = "Option::is_none")]
        client: Option<&'a str>,
    },
    Reset {
        seq: u64,
        last_op: Option<&'a str>,
        rows: &'a [Row],
        changes: &'a [Change],
        applied: &'a [String],
    },
    Ack {
        op_id: &'a str,
        seq: u64,
    },
    Reject {
        op_id: &'a str,
        code: &'a str,
        message: String,
    },
    Error {
        message: &'a str,
    },
}

impl ServerMsg<'_> {
    #[must_use]
    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl<'a> ServerMsg<'a> {
    /// The `reset` message for a [`space::Reset`] (no changes: they may be pruned).
    #[must_use]
    pub fn reset(r: &'a space::Reset) -> Self {
        ServerMsg::Reset { seq: r.seq, last_op: r.last_op.as_deref(), rows: &r.rows, changes: &[], applied: &r.applied }
    }
}
