//! Built-in tools, schemas, permissions, and tool policy for Gestura.
//!
//! This domain crate owns the implementation of Gestura's built-in tools and
//! the shared machinery around them: registry construction, permission checks,
//! policy evaluation, provider schema generation, and async wrappers used by the
//! higher-level pipeline.
//!
//! ## Design role
//!
//! This crate is intentionally independent from the `gestura-core` facade so it
//! can stay workable with lower coupling and faster iteration. The public stable
//! import path for most consumers remains `gestura_core::tools::*`, which
//! re-exports these modules from the facade crate.
//!
//! ## High-signal modules
//!
//! - `file`, `shell`, `git`, `web`, `screen`: built-in tool implementations
//! - `registry`: built-in tool catalog and discovery helpers
//! - `schemas`: provider-specific tool schemas for OpenAI, Anthropic, and Gemini
//! - `permissions`: permission management and audit-friendly checks
//! - `policy`: policy evaluation helpers layered on top of permissions
//!
//! Pipeline orchestration and higher-level request handling stay in
//! `gestura-core`; this crate focuses on the tools domain itself.

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
pub mod gui;
pub mod mcp_manager;
pub mod permissions;
pub mod policy;
pub mod registry;
pub mod schemas;
pub mod screen;
pub mod shell;
pub mod tool_confirmation;
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
