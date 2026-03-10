//! Async git operations for pipeline integration
//!
//! Wraps the synchronous [`GitTools`] via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::error::AppError;
use crate::error::Result;
use crate::git::{GitTools, GitWorktreeInfo};
use std::path::{Path, PathBuf};

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

/// Resolve the repository top-level path asynchronously.
pub async fn rev_parse_toplevel(path: impl AsRef<Path>) -> Result<PathBuf> {
    let work_path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || GitTools::new(Some(work_path)).rev_parse_toplevel())
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Get the current branch asynchronously.
pub async fn current_branch(path: impl AsRef<Path>) -> Result<String> {
    let work_path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || GitTools::new(Some(work_path)).current_branch())
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// List worktrees asynchronously.
pub async fn worktree_list(path: impl AsRef<Path>) -> Result<Vec<GitWorktreeInfo>> {
    let work_path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || GitTools::new(Some(work_path)).worktree_list())
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Create a worktree asynchronously.
pub async fn worktree_add(
    path: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    branch: String,
    base_branch: String,
    create_branch: bool,
) -> Result<GitWorktreeInfo> {
    let work_path = path.as_ref().to_path_buf();
    let worktree_path = worktree_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || {
        GitTools::new(Some(work_path)).worktree_add(
            &worktree_path,
            &branch,
            &base_branch,
            create_branch,
        )
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Remove a worktree asynchronously.
pub async fn worktree_remove(
    path: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    force: bool,
) -> Result<()> {
    let work_path = path.as_ref().to_path_buf();
    let worktree_path = worktree_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || {
        GitTools::new(Some(work_path)).worktree_remove(&worktree_path, force)
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Check whether a worktree is clean asynchronously.
pub async fn is_worktree_clean(
    path: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
) -> Result<bool> {
    let work_path = path.as_ref().to_path_buf();
    let worktree_path = worktree_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || {
        GitTools::new(Some(work_path)).is_worktree_clean(&worktree_path)
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}
