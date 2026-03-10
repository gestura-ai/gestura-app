//! Agent-to-Agent protocol types, client, and server for Gestura.
//!
//! `gestura-core-a2a` implements Gestura's A2A domain: a transport-agnostic
//! JSON-RPC server model, an HTTP client for remote agents, and the typed task
//! lifecycle/provenance structures needed for cross-agent delegation.
//!
//! ## Main concepts
//!
//! - `A2AServer`: protocol server that shells can expose over HTTP/SSE
//! - `A2AClient`: HTTP client for discovery, task creation, retries, status, and cancellation
//! - `AgentCard` / `Skill`: remote capability advertisement
//! - `A2ATask`, `TaskStatus`, `RemoteTaskContract`: typed remote-task lifecycle
//! - `TaskProvenance` / `TaskAuditEvent`: provenance and audit metadata
//!
//! ## Architecture boundary
//!
//! This crate owns protocol logic and domain types. Listener setup, shell
//! transport hosting, and GUI/CLI presentation concerns remain in the shell
//! crates that embed it.
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
