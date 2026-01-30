//! A2A (Agent-to-Agent) protocol.
//!
//! This module provides:
//! - A transport-agnostic JSON-RPC **protocol server** (`A2AServer`) intended to be
//!   hosted by shell crates (GUI/CLI) over HTTP/SSE.
//! - An HTTP **client** (`A2AClient`) for calling remote A2A agents.
//!
//! Shared protocol types live in [`types`](crate::a2a::types).

mod client;
mod server;
mod types;

pub use client::*;
pub use server::*;
pub use types::*;
