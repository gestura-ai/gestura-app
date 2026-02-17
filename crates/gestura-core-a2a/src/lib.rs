//! A2A (Agent-to-Agent) protocol.
//!
//! This crate provides:
//! - A transport-agnostic JSON-RPC **protocol server** (`A2AServer`) intended to be
//!   hosted by shell crates (GUI/CLI) over HTTP/SSE.
//! - An HTTP **client** (`A2AClient`) for calling remote A2A agents.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::a2a::*`.

mod client;
mod server;
mod types;

pub use client::*;
pub use server::*;
pub use types::*;
