//! Async shell operations for pipeline integration
//!
//! Wraps the synchronous [`ShellTools`] via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::error::{AppError, Result};
use crate::shell::ShellTools;
use std::collections::HashMap;
use std::path::PathBuf;

/// Execute a shell command asynchronously.
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
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}
