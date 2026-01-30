//! Scripting engine - thin wrapper over gestura_core::scripting
//!
//! This module provides re-exports from gestura-core's scripting module.
//! All script loading, validation, and execution logic lives in core.

// Re-export core scripting types
pub use gestura_core::scripting::{
    Script, ScriptContext, ScriptExecutionResult, ScriptLanguage, ScriptPermission, ScriptTrigger,
    ScriptingEngine, get_scripting_engine,
};
