//! System Tools Commands
//!
//! Provides built-in system tools for agentic workflows:
//! - `gestura tools file` - File operations
//! - `gestura tools shell` - Shell command execution
//! - `gestura tools git` - Git operations
//! - `gestura tools code` - Code analysis
//! - `gestura tools web` - Web fetching
//! - `gestura tools permissions` - Permission management
//! - `gestura tools screen` - Screen capture and recording

pub mod code;
pub mod file;
pub mod git;
pub mod permissions;
pub mod screen;
pub mod shell;
pub mod web;

use super::Result;

/// Tools category for subcommand routing
pub enum ToolsCategory {
    File(file::FileSubcommand),
    Shell(shell::ShellSubcommand),
    Git(git::GitSubcommand),
    Code(code::CodeSubcommand),
    Web(web::WebSubcommand),
    Permissions(permissions::PermissionsSubcommand),
    Screen(screen::ScreenSubcommand),
}

/// Run a tools subcommand
pub fn run(category: ToolsCategory) -> Result<()> {
    match category {
        ToolsCategory::File(cmd) => file::run(cmd),
        ToolsCategory::Shell(cmd) => shell::run(cmd),
        ToolsCategory::Git(cmd) => git::run(cmd),
        ToolsCategory::Code(cmd) => code::run(cmd),
        ToolsCategory::Web(cmd) => web::run(cmd),
        ToolsCategory::Permissions(cmd) => permissions::run(cmd),
        ToolsCategory::Screen(cmd) => screen::run(cmd),
    }
}
