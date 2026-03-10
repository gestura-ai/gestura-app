//! Async git operations for pipeline integration
//!
//! Wraps the synchronous [`GitTools`] via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::error::AppError;
use crate::error::Result;
use crate::git::GitTools;

/// Execute a git operation asynchronously.
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
            _ => Err(AppError::Io(std::io::Error::other(format!(
                "Unknown git operation: {}",
                op
            )))),
        }
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}
