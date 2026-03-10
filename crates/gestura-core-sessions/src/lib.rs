//! Session state, persistence, and workspace lifecycle for Gestura.
//!
//! `gestura-core-sessions` owns the session-oriented domain model used across
//! the application. It covers both conversational/agent state and the workspace
//! boundaries needed to safely persist and resume long-running agent activity.
//!
//! ## Responsibilities
//!
//! - `agent_sessions`: persistent agent sessions, storage backends, and legacy
//!   migration helpers
//! - `session_manager`: authentication/session lifecycle, tokens, and access
//!   management for active sessions
//! - `session_workspace`: session-scoped workspaces for file operations with
//!   security validation and path controls
//!
//! ## Architecture role
//!
//! This crate is the source of truth for session behavior, while higher-level
//! orchestration remains in `gestura-core`. The facade re-exports the stable
//! public entry points under paths such as `gestura_core::agent_sessions::*`,
//! `gestura_core::session_manager::*`, and `gestura_core::session_workspace::*`.
//!
//! ## Why this is separate
//!
//! Isolating session logic into a domain crate keeps persistence and workspace
//! lifecycle concerns independently testable and avoids coupling them to CLI,
//! GUI, or pipeline runtime details.

pub mod agent_sessions;
pub mod session_manager;
pub mod session_workspace;
