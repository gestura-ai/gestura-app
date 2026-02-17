//! Session management domain crate for Gestura.
//!
//! This crate provides:
//! - **Agent sessions**: persistent session model, file-backed store, legacy migration
//! - **Session manager**: authentication sessions, tokens, access control
//! - **Session workspace**: sandboxed file-operation workspaces with security validation

pub mod agent_sessions;
pub mod session_manager;
pub mod session_workspace;
