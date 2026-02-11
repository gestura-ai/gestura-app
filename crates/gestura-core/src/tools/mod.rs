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

pub mod registry;
pub mod schemas;

// Tool implementations live in the `gestura-core-tools` domain crate.
// We re-export them here to preserve stable public paths like
// `gestura_core::tools::shell::ShellTools`.
pub use gestura_core_tools::{code, file, git, permissions, policy, screen, shell, web};

pub use code::CodeTools;
pub use file::FileTools;
pub use git::GitTools;
pub use permissions::PermissionManager;
pub use registry::{
    ToolDefinition, all_tools, find_tool, looks_like_capabilities_question,
    looks_like_tools_question, render_capabilities, render_tool_detail, render_tools_overview,
};
pub use screen::ScreenTools;
pub use shell::ShellTools;
pub use web::WebTools;

// Async wrappers live in the `gestura-core-tools` domain crate.
// We re-export them here to preserve stable `gestura_core::tools::*` paths.
pub use gestura_core_tools::{code_async, file_async, git_async, screen_async, shell_async};

/// Streaming shell execution for real-time output to the frontend.
///
/// Unlike `shell_async`, which blocks until the command finishes and returns a
/// single `String`, this module uses `tokio::process::Command` to spawn the
/// process asynchronously and forwards stdout/stderr line-by-line via
/// `StreamChunk::ShellOutput` events.  A `StreamChunk::ShellLifecycle` event is
/// emitted at process start and again on completion/failure.
pub mod shell_streaming {
    use crate::streaming::{ShellOutputStream, ShellProcessState, StreamChunk};
    #[cfg(unix)]
    use nix::libc;
    use std::collections::HashMap;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;
    use tokio::sync::mpsc;

    /// Spawn a shell command and stream its output.
    ///
    /// Returns the final `CommandResult`-like summary after the process exits.
    /// `tx` receives `ShellOutput` and `ShellLifecycle` chunks in real time.
    pub async fn execute_streaming(
        command: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        timeout_secs: Option<u64>,
        tx: mpsc::Sender<StreamChunk>,
    ) -> crate::error::Result<StreamingCommandResult> {
        let process_id = uuid::Uuid::new_v4().to_string();
        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(300));

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let Some(env_map) = env {
            cmd.envs(env_map);
        }

        let cwd_owned = cwd.map(String::from);
        let cmd_str = command.to_string();
        let start = std::time::Instant::now();

        let mut child = cmd.spawn().map_err(crate::error::AppError::Io)?;

        // Register the process for control operations (stop/pause/resume).
        let os_pid = child.id().unwrap_or(0);
        register_process(&process_id, os_pid, command, cwd, env, timeout_secs).await;

        // Emit lifecycle: started
        let _ = tx
            .send(StreamChunk::ShellLifecycle {
                process_id: process_id.clone(),
                state: ShellProcessState::Started,
                exit_code: None,
                duration_ms: None,
                command: cmd_str.clone(),
                cwd: cwd_owned.clone(),
            })
            .await;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| crate::error::AppError::Io(std::io::Error::other("no stdout")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| crate::error::AppError::Io(std::io::Error::other("no stderr")))?;

        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let tx_out = tx.clone();
        let pid_out = process_id.clone();
        let stdout_task = tokio::spawn(async move {
            let mut lines = stdout_reader.lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&line);
                collected.push('\n');
                let _ = tx_out
                    .send(StreamChunk::ShellOutput {
                        process_id: pid_out.clone(),
                        stream: ShellOutputStream::Stdout,
                        data: format!("{line}\n"),
                    })
                    .await;
            }
            collected
        });

        let tx_err = tx.clone();
        let pid_err = process_id.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = stderr_reader.lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&line);
                collected.push('\n');
                let _ = tx_err
                    .send(StreamChunk::ShellOutput {
                        process_id: pid_err.clone(),
                        stream: ShellOutputStream::Stderr,
                        data: format!("{line}\n"),
                    })
                    .await;
            }
            collected
        });

        // Wait for process with timeout
        let timed_out;
        let status = tokio::select! {
            result = child.wait() => {
                timed_out = false;
                result.map_err(crate::error::AppError::Io)?
            }
            _ = tokio::time::sleep(timeout) => {
                timed_out = true;
                // Best-effort kill
                let _ = child.kill().await;
                child.wait().await.map_err(crate::error::AppError::Io)?
            }
        };

        // Wait for readers to finish
        let stdout_text = stdout_task.await.unwrap_or_default();
        let stderr_text = stderr_task.await.unwrap_or_default();

        let duration_ms = start.elapsed().as_millis() as u64;
        let exit_code = if timed_out {
            124
        } else {
            status.code().unwrap_or(-1)
        };
        let success = !timed_out && status.success();

        let final_state = if success {
            ShellProcessState::Completed
        } else {
            ShellProcessState::Failed
        };

        // Unregister the process now that it has exited.
        unregister_process(&process_id).await;

        // Emit lifecycle: completed/failed
        let _ = tx
            .send(StreamChunk::ShellLifecycle {
                process_id: process_id.clone(),
                state: final_state,
                exit_code: Some(exit_code),
                duration_ms: Some(duration_ms),
                command: cmd_str.clone(),
                cwd: cwd_owned,
            })
            .await;

        Ok(StreamingCommandResult {
            process_id,
            command: cmd_str,
            stdout: stdout_text,
            stderr: stderr_text,
            exit_code,
            success,
            duration_ms,
        })
    }

    /// Summary of a streaming command execution.
    #[derive(Debug, Clone)]
    pub struct StreamingCommandResult {
        pub process_id: String,
        pub command: String,
        pub stdout: String,
        pub stderr: String,
        pub exit_code: i32,
        pub success: bool,
        pub duration_ms: u64,
    }

    // ------------------------------------------------------------------
    // Process manager — global registry for controlling running shells
    // ------------------------------------------------------------------

    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    /// Handle to a running shell process with enough info to control it.
    struct ProcessEntry {
        /// Raw OS process ID — used for POSIX signal delivery.
        pid: u32,
        /// The original command (for re-run support).
        command: String,
        /// Working directory (for re-run support).
        cwd: Option<String>,
        /// Environment (for re-run support).
        env: Option<HashMap<String, String>>,
        /// Timeout (for re-run support).
        timeout_secs: Option<u64>,
    }

    /// Global process manager instance.
    static PROCESS_MANAGER: OnceLock<Mutex<HashMap<String, ProcessEntry>>> = OnceLock::new();

    fn process_map() -> &'static Mutex<HashMap<String, ProcessEntry>> {
        PROCESS_MANAGER.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Register a running process so it can be controlled later.
    pub(crate) async fn register_process(
        process_id: &str,
        pid: u32,
        command: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        timeout_secs: Option<u64>,
    ) {
        let mut map = process_map().lock().await;
        map.insert(
            process_id.to_string(),
            ProcessEntry {
                pid,
                command: command.to_string(),
                cwd: cwd.map(String::from),
                env: env.cloned(),
                timeout_secs,
            },
        );
    }

    /// Unregister a process (called when it exits).
    pub(crate) async fn unregister_process(process_id: &str) {
        let mut map = process_map().lock().await;
        map.remove(process_id);
    }

    /// Send SIGTERM to a running shell process, then SIGKILL after 3 s.
    pub async fn stop_process(process_id: &str) -> crate::error::Result<()> {
        let pid = {
            let map = process_map().lock().await;
            map.get(process_id).map(|e| e.pid).ok_or_else(|| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "No running process with id {process_id}"
                )))
            })?
        };

        // SIGTERM first
        #[cfg(unix)]
        {
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            // Give it 3 seconds, then SIGKILL
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            });
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            return Err(crate::error::AppError::Io(std::io::Error::other(
                "Process signals not supported on this platform",
            )));
        }
        Ok(())
    }

    /// Send SIGSTOP (pause) to a running shell process.
    #[cfg(unix)]
    pub async fn pause_process(process_id: &str) -> crate::error::Result<()> {
        let pid = {
            let map = process_map().lock().await;
            map.get(process_id).map(|e| e.pid).ok_or_else(|| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "No running process with id {process_id}"
                )))
            })?
        };
        unsafe { libc::kill(pid as i32, libc::SIGSTOP) };
        Ok(())
    }

    /// Send SIGCONT (resume) to a paused shell process.
    #[cfg(unix)]
    pub async fn resume_process(process_id: &str) -> crate::error::Result<()> {
        let pid = {
            let map = process_map().lock().await;
            map.get(process_id).map(|e| e.pid).ok_or_else(|| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "No running process with id {process_id}"
                )))
            })?
        };
        unsafe { libc::kill(pid as i32, libc::SIGCONT) };
        Ok(())
    }

    /// Retrieve info needed to re-run a command (from a previous process_id).
    /// Returns `(command, cwd, env, timeout_secs)` if the process was registered.
    pub async fn get_rerun_info(
        process_id: &str,
    ) -> Option<(
        String,
        Option<String>,
        Option<HashMap<String, String>>,
        Option<u64>,
    )> {
        let map = process_map().lock().await;
        map.get(process_id).map(|e| {
            (
                e.command.clone(),
                e.cwd.clone(),
                e.env.clone(),
                e.timeout_secs,
            )
        })
    }
}

// screen_async module has been moved to gestura-core-tools.
// The `pub use gestura_core_tools::screen_async;` re-export above makes it available.
