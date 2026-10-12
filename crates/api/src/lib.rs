//! What both deployments say to the client, with no I/O: the self-hosted server (`crates/server`: axum + rusqlite)
//! and the Cloudflare Worker (`crates/cloudflare`: D1 + one Durable Object per space). Both store a space with
//! `crates/space`; this crate is everything around it (the sync protocol, op checks, sessions, Google sign-in,
//! the accounts schema and sign-in rules), so the two cannot drift apart.

pub mod accounts;
pub mod check;
pub mod google;
pub mod protocol;
pub mod session;

pub use accounts::Identity;
