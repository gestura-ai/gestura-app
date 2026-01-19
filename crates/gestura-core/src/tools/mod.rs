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
pub mod schemas;
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

use crate::error::Result;
use std::path::Path;

/// Async shell operations for pipeline integration
pub mod shell_async {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Execute a shell command asynchronously
    pub async fn execute_command(command: &str, cwd: Option<&str>) -> Result<String> {
        execute_command_with_options(command, cwd, None, Some(60)).await
    }

    /// Execute a shell command asynchronously with cwd/env/timeout.
    pub async fn execute_command_with_options(
        command: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        let tools = ShellTools::new();
        let cmd = command.to_string();
        let work_dir = cwd.map(PathBuf::from);
        let env_map = env.cloned();
        let timeout = timeout_secs;

        tokio::task::spawn_blocking(move || {
            tools
                .run_with_options(&cmd, work_dir.as_deref(), env_map.as_ref(), timeout)
                .map(|r| {
                    if r.success {
                        r.stdout
                    } else {
                        format!("Error (exit {}): {}", r.exit_code, r.stderr)
                    }
                })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }
}

/// Async file operations for pipeline
pub mod file_async {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    struct ListDirOutput {
        path: String,
        show_hidden: bool,
        truncated: bool,
        entries: Vec<super::file::FileEntry>,
    }

    #[derive(Debug, Serialize)]
    struct TreeDirOutput {
        path: String,
        max_depth: Option<usize>,
        show_hidden: bool,
        tree: super::file::TreeNode,
    }

    #[derive(Debug, Serialize)]
    struct EditFileOutput {
        path: String,
        replacements: usize,
    }

    #[derive(Debug, Serialize)]
    struct SearchOutput {
        pattern: String,
        path: String,
        recursive: bool,
        truncated: bool,
        matches: Vec<super::file::SearchMatch>,
    }

    /// Read a file asynchronously.
    pub async fn read_file(path: &str) -> Result<String> {
        read_file_range(path, None, None).await
    }

    /// Read a file asynchronously with an optional line range.
    pub async fn read_file_range(
        path: &str,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            tools
                .read(Path::new(&path), start_line, end_line)
                .map(|r| r.content)
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Write to a file asynchronously
    pub async fn write_file(path: &str, content: &str) -> Result<()> {
        let path = path.to_string();
        let content = content.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            tools.write(Path::new(&path), &content).map(|_| ())
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Edit a file asynchronously by replacing `old_str` with `new_str`.
    ///
    /// Returns a small JSON payload with the number of replacements.
    pub async fn edit_file(path: &str, old_str: &str, new_str: &str) -> Result<String> {
        let path = path.to_string();
        let old_str = old_str.to_string();
        let new_str = new_str.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let res = tools.edit(Path::new(&path), &old_str, &new_str)?;

            let out = EditFileOutput {
                path,
                replacements: res.replacements,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize edit output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// List directory entries asynchronously.
    ///
    /// Returns a JSON string containing the entries.
    pub async fn list_dir(
        path: &str,
        show_hidden: bool,
        max_entries: Option<usize>,
    ) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let mut entries = tools.list(Path::new(&path), show_hidden)?;
            let max = max_entries.unwrap_or(200);
            let truncated = entries.len() > max;
            if truncated {
                entries.truncate(max);
            }

            let out = ListDirOutput {
                path,
                show_hidden,
                truncated,
                entries,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize list output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Build a directory tree asynchronously.
    ///
    /// Returns a JSON string containing the tree.
    pub async fn tree_dir(
        path: &str,
        max_depth: Option<usize>,
        show_hidden: bool,
    ) -> Result<String> {
        let path = path.to_string();
        let show_hidden_flag = show_hidden;
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let tree = tools.tree(Path::new(&path), max_depth, show_hidden_flag)?;
            let out = TreeDirOutput {
                path,
                max_depth,
                show_hidden: show_hidden_flag,
                tree,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize tree output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Search files for a pattern asynchronously.
    ///
    /// Returns a JSON string containing the matches.
    pub async fn search_files(
        pattern: &str,
        path: &str,
        recursive: bool,
        max_matches: Option<usize>,
    ) -> Result<String> {
        let pattern = pattern.to_string();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let mut matches = tools.search(&pattern, Path::new(&path), recursive)?;
            let max = max_matches.unwrap_or(200);
            let truncated = matches.len() > max;
            if truncated {
                matches.truncate(max);
            }

            let out = SearchOutput {
                pattern,
                path,
                recursive,
                truncated,
                matches,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize search output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }
}

/// Async code operations for pipeline
pub mod code_async {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    struct StatsOutput {
        path: String,
        stats: super::code::CodeStats,
    }

    /// Compute code statistics asynchronously.
    ///
    /// Returns a JSON string with the stats.
    pub async fn stats_dir(path: &str) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = CodeTools::default();
            let stats = tools.stats(Path::new(&path))?;
            let out = StatsOutput { path, stats };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize code stats output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {e}")))
        })?
    }
}

/// Async git operations for pipeline
pub mod git_async {
    use super::*;

    /// Execute a git operation asynchronously
    pub async fn execute_git(operation: &str, path: &str) -> Result<String> {
        let op = operation.to_string();
        let work_path = path.to_string();

        tokio::task::spawn_blocking(move || {
            let tools = GitTools::new(Some(std::path::PathBuf::from(&work_path)));
            match op.as_str() {
                "status" => tools.status().map(|s| {
                    format!(
                        "Branch: {}\nStaged: {} files\nUnstaged: {} files\nUntracked: {} files",
                        s.branch,
                        s.staged.len(),
                        s.unstaged.len(),
                        s.untracked.len()
                    )
                }),
                "log" => tools.log(Some(10), None).map(|commits| {
                    commits
                        .iter()
                        .map(|c| format!("{} - {} ({})", c.short_hash, c.message, c.author))
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                "diff" => tools.diff(false, None),
                "diff-staged" => tools.diff(true, None),
                "branches" => tools.branches(false).map(|branches| {
                    branches
                        .iter()
                        .map(|b| {
                            if b.is_current {
                                format!("* {}", b.name)
                            } else {
                                format!("  {}", b.name)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                _ => Err(crate::error::AppError::Io(std::io::Error::other(format!(
                    "Unknown git operation: {}",
                    op
                )))),
            }
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }
}
