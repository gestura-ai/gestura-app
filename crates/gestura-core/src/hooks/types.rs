//! Hook configuration and context types.
//!
//! Config types (`HookEvent`, `HookCommandTemplate`, `HookDefinition`,
//! `HooksSettings`) are defined in `gestura-core-config` and re-exported here
//! so that downstream code can keep using `crate::hooks::*` unchanged.
//!
//! The runtime-only `HookContext` type stays defined here because it has no
//! serialization and is not part of the persisted configuration.

use std::path::PathBuf;

// Re-export serializable config types from the config crate.
pub use gestura_core_config::hooks_types::{
    HookCommandTemplate, HookDefinition, HookEvent, HooksSettings,
};

/// Runtime context provided to hooks.
///
/// Note: this is intentionally flexible and uses optional fields so we can
/// evolve what data is available as we integrate deeper into the pipeline.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// Workspace root (if available).
    pub workspace_dir: Option<PathBuf>,
    /// Session id (if available).
    pub session_id: Option<String>,
    /// Tool name (for tool events).
    pub tool_name: Option<String>,
    /// Tool arguments (stringified JSON).
    pub tool_arguments_json: Option<String>,
    /// Tool success flag (for post-tool hooks).
    pub tool_success: Option<bool>,
    /// Tool output (for post-tool hooks).
    pub tool_output: Option<String>,
    /// Pipeline prompt (for pipeline events).
    pub pipeline_prompt: Option<String>,
}
