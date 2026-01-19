//! A2A (Agent-to-Agent) Protocol Client
//!
//! This module provides an HTTP client for the A2A protocol, enabling
//! agent-to-agent communication following the Linux Foundation A2A spec.

mod client;
mod types;

pub use client::*;
pub use types::*;
