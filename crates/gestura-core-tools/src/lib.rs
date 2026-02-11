//! Gestura Core Tools
//!
//! Domain crate containing implementations for Gestura's built-in tools.
//!
//! This crate intentionally does **not** depend on the `gestura-core` facade.
//! The facade crate re-exports these modules to preserve stable public paths.

/// Error compatibility module.
///
/// Many tool implementations historically imported `crate::error::{AppError, Result}`.
/// We keep that shape here so the code remains portable across crates.
pub mod error {
    pub use gestura_core_foundation::error::{AppError, Result};
}

pub mod config;

pub mod code;
pub mod file;
pub mod git;
pub mod permissions;
pub mod policy;
pub mod registry;
pub mod schemas;
pub mod screen;
pub mod shell;
pub mod tool_inspection;
pub mod web;

// Async wrappers for pipeline / GUI integration
pub mod code_async;
pub mod file_async;
pub mod git_async;
pub mod screen_async;
pub mod shell_async;

pub use registry::{
    ToolDefinition, all_tools, find_tool, looks_like_capabilities_question,
    looks_like_tools_question, render_tool_detail, render_tools_overview,
};
