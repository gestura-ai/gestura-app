//! System Tools for Gestura
//!
//! This module provides output-agnostic system tools that can be used by both
//! the CLI and GUI interfaces. All tools return structured data rather than
//! formatted strings, allowing each interface to present results appropriately.
//!
//! # Tools
//! - [`file`]: File system operations (read, write, edit, search, list, tree)
//! - [`shell`]: Shell command execution
//! - [`git`]: Git repository operations
//! - [`code`]: Code analysis and navigation
//! - [`web`]: Web fetching and search
//! - [`permissions`]: Permission management for tool access
//! - [`registry`]: Tool registry for listing available tools

pub mod code;
pub mod file;
pub mod git;
pub mod permissions;
pub mod registry;
pub mod shell;
pub mod web;

pub use code::CodeTools;
pub use file::FileTools;
pub use git::GitTools;
pub use permissions::PermissionManager;
pub use registry::{
    ToolDefinition, all_tools, find_tool, looks_like_capabilities_question,
    looks_like_tools_question, render_capabilities, render_tool_detail, render_tools_overview,
};
pub use shell::ShellTools;
pub use web::WebTools;
