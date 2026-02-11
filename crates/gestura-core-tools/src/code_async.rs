//! Async code operations for pipeline integration
//!
//! Wraps the synchronous [`CodeTools`](crate::code::CodeTools) via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::code::CodeTools;
use crate::error::AppError;
use crate::error::Result;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
struct StatsOutput {
    path: String,
    stats: crate::code::CodeStats,
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
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize code stats output: {e}"
            )))
        })
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {e}"))))?
}
