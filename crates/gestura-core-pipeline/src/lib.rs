//! Gestura Core Pipeline
//!
//! This crate contains the pipeline public types and persona prompt used by
//! the agent pipeline. The `AgentPipeline` implementation itself remains in
//! `gestura-core` because it orchestrates many core modules.

pub mod persona;
pub mod reflection;
pub mod reflection_eval;
pub mod types;

// Re-export key types at crate root for convenience.
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
