//! Shared, dependency-light primitives for the Gestura workspace.
//!
//! `gestura-core-foundation` exists so domain crates can share core models and
//! policies without depending on the larger `gestura-core` facade. This crate is
//! intended to stay small, stable, and broadly reusable across the workspace.
//!
//! ## What belongs here
//!
//! - cross-cutting error and result types
//! - execution-mode and permission primitives
//! - shared event, telemetry, platform, and interaction models
//! - context analysis data structures reused by higher-level crates
//!
//! ## What does not belong here
//!
//! - protocol implementations
//! - tool implementations
//! - pipeline orchestration
//! - GUI or CLI presentation concerns
//!
//! Most application code should still import through `gestura_core::*`, while
//! domain crates may depend on this crate directly when they need a lightweight
//! shared foundation.

pub mod context;
pub mod error;
pub mod events;
pub mod execution_mode;
pub mod interaction;
pub mod model_display;
pub mod outcomes;
pub mod permissions;
pub mod platform;
pub mod secrets;
pub mod stream_error;
pub mod stream_health;
pub mod stream_reconnect;
pub mod telemetry;

pub use context::{
    ContextCategory, EntityType, ExtractedEntity, FileContext, RequestAnalysis, ResolvedContext,
    ToolContext,
};
pub use error::{AppError, Result};
pub use execution_mode::{
    ExecutionMode, ModeConfig, ModeManager, ToolCategory, ToolExecutionCheck, ToolPermission,
};
pub use outcomes::{OutcomeSignal, OutcomeSignalKind};
pub use permissions::PermissionLevel;
