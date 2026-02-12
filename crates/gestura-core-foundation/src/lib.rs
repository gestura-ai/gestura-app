//! Gestura Core Foundation
//!
//! This crate contains shared primitives that multiple `gestura-core-*` domain crates
//! can depend on without pulling in the full `gestura-core` facade.

pub mod context;
pub mod error;
pub mod events;
pub mod execution_mode;
pub mod interaction;
pub mod model_display;
pub mod permissions;
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
pub use permissions::PermissionLevel;
