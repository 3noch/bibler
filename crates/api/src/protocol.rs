//! The sync WebSocket (`GET /api/sync`), JSON text frames:
//! ```text
//! client: {"type":"hello","after":seq,"pending":[op_id]}  first message: the last seq the client has, and the
//!                                                  ops it holds unconfirmed (only used for a reset)
//!         {"type":"op","op":{op_id,kind,...}}      an ops::Op (idempotent by op_id)
//! server: {"type":"change","change":{seq,...}}     every change after `after`, then live, in seq order
//!         {"type":"reset","seq":n,"rows":[..],"changes":[],"applied":[op_id]}   instead of the replay when
//!                                                  changes after `after` were pruned: the whole state at seq n;
//!                                                  `applied` = which of `pending` are already in it
//!         {"type":"ready","seq":n}                 replay finished
//!         {"type":"ack","op_id":..,"seq":n}        op applied (its change arrives as a `change` message, unless
//!                                                  it was applied long ago: seq <= the client's, change pruned)
//!         {"type":"reject","op_id":..,"code":..,"message":..}   op refused; it had no effect
//!         {"type":"error","message":..}            then the server closes; the client reconnects with hello
//! ```

use ops::{Change, Op, Row};
use serde::{Deserialize, Serialize};

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
    Change { change: &'a Change },
    Ready { seq: u64 },
    Reset { seq: u64, rows: &'a [Row], changes: &'a [Change], applied: &'a [String] },
    Ack { op_id: &'a str, seq: u64 },
    Reject { op_id: &'a str, code: &'a str, message: String },
    Error { message: &'a str },
}

impl ServerMsg<'_> {
    #[must_use]
    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl<'a> ServerMsg<'a> {
    /// The `reset` message for a [`space::Reset`] (no changes: they were pruned).
    #[must_use]
    pub fn reset(r: &'a space::Reset) -> Self {
        ServerMsg::Reset { seq: r.seq, rows: &r.rows, changes: &[], applied: &r.applied }
    }
}
