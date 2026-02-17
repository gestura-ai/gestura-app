//! Hook execution backends.

use async_trait::async_trait;
use std::time::Instant;
use tokio::process::Command;

use gestura_core_foundation::error::{AppError, Result};

/// Output produced by an executed hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutput {
    /// Exit code, if available.
    pub exit_code: Option<i32>,
    /// Captured stdout (possibly truncated).
    pub stdout: String,
    /// Captured stderr (possibly truncated).
    pub stderr: String,
    /// Elapsed time in milliseconds.
    pub duration_ms: u64,
}

/// A hook executor.
///
/// This trait exists to make the hook engine testable without spawning real
/// processes (though we do use process execution in unit tests on Unix).
#[async_trait]
pub trait HookExecutor: Send + Sync {
    /// Execute a command.
    async fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
        max_output_bytes: usize,
    ) -> Result<HookOutput>;
}

/// Executor that spawns local OS processes.
pub struct ProcessHookExecutor;

#[async_trait]
impl HookExecutor for ProcessHookExecutor {
    async fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
        max_output_bytes: usize,
    ) -> Result<HookOutput> {
        let start = Instant::now();

        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let output = cmd.output().await.map_err(AppError::from)?;

        let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if stdout.len() > max_output_bytes {
            stdout.truncate(max_output_bytes);
        }
        if stderr.len() > max_output_bytes {
            stderr.truncate(max_output_bytes);
        }

        Ok(HookOutput {
            exit_code: output.status.code(),
            stdout,
            stderr,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}
