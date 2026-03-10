//! Async code operations for pipeline integration
//!
//! Wraps the synchronous [`CodeTools`] via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::code::{CodeTools, EditOp};
use crate::error::AppError;
use crate::error::Result;
use std::path::Path;

/// Serialize a value to pretty JSON, wrapping errors.
fn to_json<T: serde::Serialize>(val: &T) -> Result<String> {
    serde_json::to_string_pretty(val)
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Serialize error: {e}"))))
}

/// Shorthand join-error mapper.
fn join_err(e: tokio::task::JoinError) -> AppError {
    AppError::Io(std::io::Error::other(format!("Task join error: {e}")))
}

/// Compute code statistics for `path` (file or directory).
pub async fn stats_dir(path: &str) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let stats = tools.stats(Path::new(&path))?;
        to_json(&serde_json::json!({ "path": path, "stats": stats }))
    })
    .await
    .map_err(join_err)?
}

/// Generate a repository map for `path` up to `max_depth` directory levels.
pub async fn map(path: &str, max_depth: usize) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.repository_map(Path::new(&path), max_depth)?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Extract top-level symbols from a file.
pub async fn symbols(path: &str) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.symbols(Path::new(&path))?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Find all references to `symbol` under `root`.
pub async fn references(symbol: &str, root: &str) -> Result<String> {
    let symbol = symbol.to_string();
    let root = root.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.references(&symbol, Path::new(&root))?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Find the first definition of `symbol` under `root`.
pub async fn definition(symbol: &str, root: &str) -> Result<String> {
    let symbol = symbol.to_string();
    let root = root.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.definition(&symbol, Path::new(&root))?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Parse Cargo dependencies from the manifest at `path`.
pub async fn deps(path: &str) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.cargo_dependencies(Path::new(&path))?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Run `cargo clippy` in `path`, optionally applying fixes.
pub async fn lint(path: &str, fix: bool) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.cargo_clippy(Path::new(&path), fix)?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Run `cargo test` in `path`, with an optional test name filter.
pub async fn test(path: &str, filter: Option<String>) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.cargo_test(Path::new(&path), filter.as_deref())?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Find files matching `pattern` under `root` (glob syntax).
pub async fn glob_search(pattern: &str, root: &str, max_results: usize) -> Result<String> {
    let pattern = pattern.to_string();
    let root = root.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.glob_search(&pattern, Path::new(&root), max_results)?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Search file contents under `root` for lines matching `pattern` (regex).
pub async fn grep(
    pattern: &str,
    root: &str,
    file_glob: Option<String>,
    context_lines: usize,
    case_sensitive: bool,
    max_matches: usize,
) -> Result<String> {
    let pattern = pattern.to_string();
    let root = root.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.grep(
            &pattern,
            Path::new(&root),
            file_glob.as_deref(),
            context_lines,
            case_sensitive,
            max_matches,
        )?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Read multiple files in a single call.
pub async fn batch_read(paths: Vec<String>) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let result = tools.batch_read(&path_refs);
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Apply multiple str-replace edits across files.
pub async fn batch_edit(edits: Vec<EditOp>) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.batch_edit(&edits);
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}

/// Return a structured outline of symbols in `path`.
pub async fn outline(path: &str) -> Result<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let tools = CodeTools::default();
        let result = tools.outline(Path::new(&path))?;
        to_json(&result)
    })
    .await
    .map_err(join_err)?
}
