//! Shell command execution tool
//!
//! Provides shell command execution with structured output.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Result of executing a shell command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
    pub duration_ms: u64,
}

/// Command history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub command: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// Shell command service
pub struct ShellTools {
    history: RwLock<Vec<HistoryEntry>>,
    last_output: RwLock<Option<CommandResult>>,
}

impl Default for ShellTools {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellTools {
    pub fn new() -> Self {
        Self {
            history: RwLock::new(Vec::new()),
            last_output: RwLock::new(None),
        }
    }

    /// Execute a shell command
    pub fn run(&self, command: &str, timeout_secs: Option<u64>) -> Result<CommandResult> {
        self.run_with_options(command, None, None, timeout_secs)
    }

    /// Execute a shell command with working directory and environment.
    ///
    /// This method enforces a timeout to avoid "hanging" tool calls.
    pub fn run_with_options(
        &self,
        command: &str,
        cwd: Option<&Path>,
        env: Option<&HashMap<String, String>>,
        timeout_secs: Option<u64>,
    ) -> Result<CommandResult> {
        let timeout = timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(300));
        let start = Instant::now();

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let Some(env_map) = env {
            cmd.envs(env_map);
        }

        let mut child = cmd.spawn().map_err(AppError::Io)?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Io(std::io::Error::other("Missing child stdout")))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Io(std::io::Error::other("Missing child stderr")))?;

        let out_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        });
        let err_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            buf
        });

        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(AppError::Io)? {
                break status;
            }
            if start.elapsed() >= timeout {
                timed_out = true;
                // Best-effort termination. Even if kill fails, we still try to wait.
                let _ = child.kill();
                break child.wait().map_err(AppError::Io)?;
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        let stdout_bytes = out_handle.join().unwrap_or_default();
        let stderr_bytes = err_handle.join().unwrap_or_default();

        let duration = start.elapsed();
        let exit_code = if timed_out {
            // Conventional "timeout" code.
            124
        } else {
            status.code().unwrap_or(-1)
        };

        let mut stderr_str = String::from_utf8_lossy(&stderr_bytes).to_string();
        if timed_out {
            if !stderr_str.ends_with('\n') && !stderr_str.is_empty() {
                stderr_str.push('\n');
            }
            stderr_str.push_str("Command timed out");
        }

        let result = CommandResult {
            command: command.to_string(),
            stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
            stderr: stderr_str,
            exit_code,
            success: !timed_out && status.success(),
            duration_ms: duration.as_millis() as u64,
        };

        // Store in history
        if let Ok(mut history) = self.history.write() {
            history.push(HistoryEntry {
                command: command.to_string(),
                timestamp: chrono::Utc::now(),
                exit_code,
                duration_ms: result.duration_ms,
            });
        }

        // Store as last output
        if let Ok(mut last) = self.last_output.write() {
            *last = Some(result.clone());
        }

        Ok(result)
    }

    /// Test a command (parse without executing)
    pub fn test(&self, command: &str) -> Result<CommandTestResult> {
        // Use shell to check syntax without executing
        let output = Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(AppError::Io)?;

        Ok(CommandTestResult {
            command: command.to_string(),
            valid: output.status.success(),
            error: if output.status.success() {
                None
            } else {
                Some(String::from_utf8_lossy(&output.stderr).to_string())
            },
        })
    }

    /// Get command history
    pub fn history(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>> {
        let history = self
            .history
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;

        let limit = limit.unwrap_or(50);
        Ok(history.iter().rev().take(limit).cloned().collect())
    }

    /// Get last command output
    pub fn last(&self) -> Result<Option<CommandResult>> {
        let last = self
            .last_output
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;
        Ok(last.clone())
    }
}

/// Result of testing a command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandTestResult {
    pub command: String,
    pub valid: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_echo() {
        let tools = ShellTools::new();
        let result = tools.run("echo hello", None).unwrap();
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn test_run_failing_command() {
        let tools = ShellTools::new();
        let result = tools.run("exit 1", None).unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_history() {
        let tools = ShellTools::new();
        tools.run("echo test1", None).unwrap();
        tools.run("echo test2", None).unwrap();

        let history = tools.history(None).unwrap();
        assert_eq!(history.len(), 2);
        assert!(history[0].command.contains("echo test2"));
    }

    #[test]
    fn test_last() {
        let tools = ShellTools::new();
        assert!(tools.last().unwrap().is_none());

        tools.run("echo last", None).unwrap();
        let last = tools.last().unwrap().unwrap();
        assert!(last.stdout.contains("last"));
    }

    #[test]
    fn test_test_command() {
        let tools = ShellTools::new();
        let result = tools.test("echo").unwrap();
        assert!(result.valid);
        assert!(result.error.is_none());
    }
}
