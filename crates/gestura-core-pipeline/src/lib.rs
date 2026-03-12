//! Public pipeline types, persona, and reflection models for Gestura.
//!
//! `gestura-core-pipeline` defines the stable data model shared across the
//! agent execution stack: request/response types, persona defaults, compaction
//! strategy, paused-execution state, and reflection/evaluation structures.
//!
//! ## Design role
//!
//! This crate intentionally contains the *types and prompt assets* of the
//! pipeline rather than the full runtime implementation. The concrete pipeline
//! orchestration lives in `gestura-core`, where it can coordinate tools,
//! context, streaming, sessions, and provider selection.
//!
//! In other words:
//!
//! - this crate owns the pipeline vocabulary
//! - `gestura-core` owns the pipeline execution engine
//!
//! ## High-signal exports
//!
//! - `AgentRequest`, `AgentResponse`: the primary request/response types used by
//!   CLI, GUI, and tests
//! - `RequestSource`, `RequestMetadata`: request origin and routing metadata
//! - `PipelineConfig`: runtime configuration for pipeline execution
//! - `CompactionStrategy`: context-compaction policy shared with sessions
//! - `PausedExecutionState`: resumable execution state for confirmation flows
//! - `default_system_prompt`: the default runtime persona prompt
//! - `reflection` and `reflection_eval`: quality signals and evaluation helpers
//!
//! ## Architecture boundary
//!
//! Downstream code should usually import these types through
//! `gestura_core::pipeline::*` so the facade remains the stable public entry
//! point. Depending on this crate directly is most appropriate when working on
//! pipeline data structures, persona content, or reflection logic in isolation.
//!
//! ## Documentation direction
//!
//! The goal is for `cargo doc` to surface pipeline concepts here, while more
//! operational or end-user workflow documentation stays outside the API docs.

pub mod persona;
pub mod reflection;
pub mod reflection_eval;
pub mod types;

// Re-export key types at crate root for convenience.
pub use gestura_core_foundation::outcomes::{OutcomeSignal, OutcomeSignalKind};
pub use persona::default_system_prompt;
pub use reflection::{AgentReflection, QualitySignals, ReflectionConfig};
pub use reflection_eval::{
    ReflectionEvalCase, ReflectionEvalReport, ReflectionEvalSummary, ReflectionEvalToolOutcome,
    ReflectionEvalToolResult, ReflectionEvalTurn, builtin_reflection_eval_cases,
    evaluate_reflection_case, evaluate_reflection_cases,
};
pub use types::{
    AgentRequest, AgentResponse, CompactionStrategy, Message, PausedExecutionState, PipelineConfig,
    RequestMetadata, RequestSource, SessionLlmInfo, TokenLimitStatus, ToolCallRecord, ToolResult,
};
