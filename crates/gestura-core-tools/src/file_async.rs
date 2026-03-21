//! Async file operations for pipeline integration
//!
//! Wraps the synchronous [`FileTools`] via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::error::AppError;
use crate::error::Result;
use crate::file::FileTools;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
struct ListDirOutput {
    path: String,
    show_hidden: bool,
    truncated: bool,
    entries: Vec<crate::file::FileEntry>,
}

#[derive(Debug, Serialize)]
struct TreeDirOutput {
    path: String,
    max_depth: Option<usize>,
    show_hidden: bool,
    tree: crate::file::TreeNode,
}

#[derive(Debug, Serialize)]
struct WriteFileOutput {
    path: String,
    bytes_written: usize,
    created: bool,
    changed: bool,
}

#[derive(Debug, Serialize)]
struct EditFileOutput {
    path: String,
    replacements: usize,
    changed: bool,
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    pattern: String,
    path: String,
    recursive: bool,
    truncated: bool,
    matches: Vec<crate::file::SearchMatch>,
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
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Write to a file asynchronously.
pub async fn write_file(path: &str, content: &str) -> Result<String> {
    let path = path.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = FileTools::new();
        let res = tools.write(Path::new(&path), &content)?;

        let out = WriteFileOutput {
            path,
            bytes_written: res.bytes_written,
            created: res.created,
            changed: res.changed,
        };
        serde_json::to_string_pretty(&out).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize write output: {e}"
            )))
        })
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
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
            changed: res.changed,
        };
        serde_json::to_string_pretty(&out).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize edit output: {e}"
            )))
        })
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// List directory entries asynchronously.
///
/// Returns a JSON string containing the entries.
pub async fn list_dir(path: &str, show_hidden: bool, max_entries: Option<usize>) -> Result<String> {
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
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize list output: {e}"
            )))
        })
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Build a directory tree asynchronously.
///
/// Returns a JSON string containing the tree.
pub async fn tree_dir(path: &str, max_depth: Option<usize>, show_hidden: bool) -> Result<String> {
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
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize tree output: {e}"
            )))
        })
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
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
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize search output: {e}"
            )))
        })
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}
