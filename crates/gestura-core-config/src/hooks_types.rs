//! Hook configuration types used in [`AppConfig`].
//!
//! These are the serializable hook definitions that appear in
//! `~/.gestura/config.yaml`. Runtime types like `HookContext` remain
//! in `gestura-core::hooks`.

use serde::{Deserialize, Serialize};

/// A hook event.
///
/// Hooks are executed when their configured event is emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Emitted before the pipeline begins executing.
    PrePipeline,
    /// Emitted after the pipeline finishes executing.
    PostPipeline,
    /// Emitted right before a tool call is executed.
    PreTool,
    /// Emitted after a tool call completes.
    PostTool,
}

/// A command template for a hook.
///
/// `program` and each entry in `args` are template-expanded using `{{key}}`
/// placeholders from a `HookContext`. Unknown variables resolve to the empty
/// string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookCommandTemplate {
    /// Program/binary to execute.
    pub program: String,
    /// Arguments passed to the program.
    #[serde(default)]
    pub args: Vec<String>,
}

/// A single hook definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookDefinition {
    /// Friendly name for UI/debugging.
    pub name: String,
    /// Event this hook listens to.
    pub event: HookEvent,
    /// Command template executed for the event.
    pub command: HookCommandTemplate,
}

/// Global hooks settings.
///
/// These live in [`AppConfig`](crate::AppConfig) and are designed to be
/// compatible with YAML schema evolution (new fields should be
/// `#[serde(default)]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksSettings {
    /// Global enable switch.
    pub enabled: bool,
    /// Allow-list of programs that hooks are permitted to execute.
    ///
    /// Safety: empty by default.
    pub allowed_programs: Vec<String>,
    /// Hook definitions.
    pub hooks: Vec<HookDefinition>,
    /// Maximum time to allow a hook process to run.
    pub timeout_ms: u64,
    /// Maximum number of bytes to retain from stdout/stderr.
    pub max_output_bytes: usize,
}

impl Default for HooksSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_programs: Vec::new(),
            hooks: Vec::new(),
            timeout_ms: 5_000,
            max_output_bytes: 64 * 1024,
        }
    }
}
