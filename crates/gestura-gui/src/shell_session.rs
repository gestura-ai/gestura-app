//! Shell session management for sandboxed agent sessions
//!
//! Provides cross-platform terminal spawning with Gestura CLI integration,
//! similar to GitHub Codex or Claude's sandboxed code interface.

use crate::sandbox::{SandboxConfig, create_default_sandbox};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use uuid::Uuid;

/// Result type for shell session operations
pub type ShellResult<T> = Result<T, ShellSessionError>;

/// Errors that can occur during shell session operations
#[derive(Debug)]
pub enum ShellSessionError {
    /// Failed to spawn terminal process
    SpawnFailed(std::io::Error),
    /// Terminal emulator not found
    TerminalNotFound(String),
    /// Working directory not found or inaccessible
    WorkingDirectoryInvalid(PathBuf),
    /// Failed to create a fallback working directory
    WorkingDirectoryCreateFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    /// CLI not installed
    CliNotInstalled,
}

impl std::fmt::Display for ShellSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e) => write!(f, "Failed to spawn terminal: {}", e),
            Self::TerminalNotFound(name) => write!(f, "Terminal emulator not found: {}", name),
            Self::WorkingDirectoryInvalid(path) => {
                write!(f, "Working directory invalid: {:?}", path)
            }
            Self::WorkingDirectoryCreateFailed { path, source } => write!(
                f,
                "Failed to create working directory {}: {}",
                path.display(),
                source
            ),
            Self::CliNotInstalled => write!(f, "Gestura CLI is not installed"),
        }
    }
}

impl std::error::Error for ShellSessionError {}

/// Shell session configuration
#[derive(Debug, Clone)]
pub struct ShellSessionConfig {
    /// Session identifier
    pub id: String,
    /// Working directory for the shell
    pub working_directory: Option<PathBuf>,
    /// Environment variables to set in the shell
    pub env_vars: HashMap<String, String>,
    /// Sandbox configuration for resource limits
    pub sandbox: SandboxConfig,
}

impl Default for ShellSessionConfig {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            working_directory: std::env::current_dir().ok(),
            env_vars: HashMap::new(),
            sandbox: create_default_sandbox("shell-agent"),
        }
    }
}

impl ShellSessionConfig {
    /// Create a new shell session configuration with a specific working directory
    pub fn with_working_directory(mut self, path: PathBuf) -> Self {
        self.working_directory = Some(path);
        self
    }

    /// Add an environment variable to the session
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }
}

/// Find the path to the Gestura CLI binary
///
/// Checks common installation locations:
/// - /usr/local/bin/gestura (PKG install)
/// - /opt/homebrew/bin/gestura (Homebrew on Apple Silicon)
/// - User's PATH
fn find_gestura_cli() -> Option<PathBuf> {
    // Check absolute paths first (more reliable for GUI apps)
    let known_paths = [
        "/usr/local/bin/gestura",
        "/opt/homebrew/bin/gestura",
        "/usr/bin/gestura",
    ];

    for path in known_paths {
        let p = PathBuf::from(path);
        if p.exists() {
            tracing::info!("Found Gestura CLI at: {:?}", p);
            return Some(p);
        }
    }

    // Fall back to PATH lookup using `which`
    if let Ok(output) = Command::new("which").arg("gestura").output()
        && output.status.success()
    {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            let p = PathBuf::from(&path_str);
            if p.exists() {
                tracing::info!("Found Gestura CLI via PATH: {:?}", p);
                return Some(p);
            }
        }
    }

    tracing::warn!("Gestura CLI not found in any known location");
    None
}

/// Check if the Gestura CLI is installed and accessible
pub fn is_cli_installed() -> bool {
    find_gestura_cli().is_some()
}

/// Open a shell session in the platform's default terminal
///
/// This spawns a new terminal window with:
/// - Working directory set to the specified or current directory
/// - Gestura environment variables configured
/// - Access to the `gestura` CLI commands
pub fn open_shell_session(config: ShellSessionConfig) -> ShellResult<()> {
    let working_dir = resolve_working_directory(&config)?;

    tracing::info!("Opening shell session {} in {:?}", config.id, working_dir);

    // Platform-specific terminal launching
    #[cfg(target_os = "macos")]
    return open_macos_terminal(&working_dir, &config);

    #[cfg(target_os = "windows")]
    return open_windows_terminal(&working_dir, &config);

    #[cfg(target_os = "linux")]
    return open_linux_terminal(&working_dir, &config);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        tracing::error!("Unsupported platform for shell sessions");
        Err(ShellSessionError::TerminalNotFound(
            "Unsupported platform".to_string(),
        ))
    }
}

fn resolve_working_directory(config: &ShellSessionConfig) -> ShellResult<PathBuf> {
    if let Some(dir) = config.working_directory.as_ref() {
        if is_usable_working_dir(dir) {
            return Ok(dir.clone());
        }
        tracing::warn!(
            "Ignoring unusable working directory {:?} for shell session {}",
            dir,
            config.id
        );
    }

    // Fall back to current working directory if it is usable.
    if let Ok(cwd) = std::env::current_dir()
        && is_usable_working_dir(&cwd)
    {
        return Ok(cwd);
    }

    // Fall back to user's home directory if available.
    if let Some(home) = dirs::home_dir()
        && is_usable_working_dir(&home)
    {
        return Ok(home);
    }

    // Final fallback: create a dedicated session directory.
    create_shell_session_directory(&config.id)
}

fn is_usable_working_dir(dir: &Path) -> bool {
    if is_forbidden_working_dir(dir) {
        return false;
    }
    dir.exists() && dir.is_dir()
}

fn is_forbidden_working_dir(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        dir == Path::new("/")
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        false
    }
}

fn create_shell_session_directory(session_id: &str) -> ShellResult<PathBuf> {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("gestura").join("shell_sessions").join(session_id);

    tracing::debug!(
        session_id = %session_id,
        base_dir = ?base,
        target_dir = ?dir,
        "Creating shell session directory"
    );

    fs::create_dir_all(&dir).map_err(|e| {
        tracing::error!(
            session_id = %session_id,
            target_dir = ?dir,
            error = %e,
            "Failed to create shell session directory"
        );
        ShellSessionError::WorkingDirectoryCreateFailed {
            path: dir.clone(),
            source: e,
        }
    })?;

    tracing::debug!(
        session_id = %session_id,
        target_dir = ?dir,
        "Shell session directory created successfully"
    );
    Ok(dir)
}

/// Open terminal on macOS using Terminal.app or iTerm2
#[cfg(target_os = "macos")]
fn open_macos_terminal(working_dir: &Path, config: &ShellSessionConfig) -> ShellResult<()> {
    let dir_str = sh_escape_single_quotes(working_dir.to_string_lossy().as_ref());

    // Build environment setup script
    let env_setup = build_env_script(&config.env_vars, &config.id);

    // Build the initialization command that will run in the new terminal
    let init_cmd = format!(
        r#"cd '{}' && {} && echo '🚀 Gestura Shell Session: {}' && echo 'Type "gestura --help" for available commands' && exec $SHELL"#,
        dir_str, env_setup, config.id
    );

    // Try iTerm2 first (if installed), then fall back to Terminal.app
    if is_iterm_installed() {
        open_iterm(&init_cmd)
    } else {
        open_terminal_app(&init_cmd)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn sh_escape_single_quotes(input: &str) -> String {
    // Safe embedding for single-quoted strings in sh: close quote, escape, reopen.
    input.replace('\'', "'\\''")
}

/// Check if iTerm2 is installed
#[cfg(target_os = "macos")]
fn is_iterm_installed() -> bool {
    PathBuf::from("/Applications/iTerm.app").exists()
}

/// Open iTerm2 with the given command
#[cfg(target_os = "macos")]
fn open_iterm(init_cmd: &str) -> ShellResult<()> {
    // Escape single quotes for AppleScript
    let escaped_cmd = init_cmd.replace('\\', "\\\\").replace('"', "\\\"");

    let script = format!(
        r#"
        tell application "iTerm"
            activate
            create window with default profile
            tell current session of current window
                write text "{}"
            end tell
        end tell
        "#,
        escaped_cmd
    );

    Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .spawn()
        .map_err(ShellSessionError::SpawnFailed)?;

    tracing::info!("Opened iTerm2 shell session");
    Ok(())
}

/// Open Terminal.app with the given command
#[cfg(target_os = "macos")]
fn open_terminal_app(init_cmd: &str) -> ShellResult<()> {
    // Escape for AppleScript
    let escaped_cmd = init_cmd.replace('\\', "\\\\").replace('"', "\\\"");

    let script = format!(
        r#"
        tell application "Terminal"
            activate
            do script "{}"
        end tell
        "#,
        escaped_cmd
    );

    Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .spawn()
        .map_err(ShellSessionError::SpawnFailed)?;

    tracing::info!("Opened Terminal.app shell session");
    Ok(())
}

/// Open terminal on Windows using Windows Terminal or Command Prompt
#[cfg(target_os = "windows")]
fn open_windows_terminal(working_dir: &Path, config: &ShellSessionConfig) -> ShellResult<()> {
    let dir_str = working_dir.to_string_lossy();

    // Build environment setup for Windows
    let env_setup = build_env_script_windows(&config.env_vars, &config.id);

    // Try Windows Terminal first (wt.exe), then fall back to cmd.exe
    if is_windows_terminal_installed() {
        // Windows Terminal
        Command::new("wt.exe")
            .args(["-d", &dir_str, "cmd", "/k", &env_setup])
            .spawn()
            .map_err(ShellSessionError::SpawnFailed)?;

        tracing::info!("Opened Windows Terminal shell session");
    } else {
        // Fallback to cmd.exe
        Command::new("cmd")
            .args(["/k", &format!("cd /d \"{}\" && {}", dir_str, env_setup)])
            .spawn()
            .map_err(ShellSessionError::SpawnFailed)?;

        tracing::info!("Opened Command Prompt shell session");
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn is_windows_terminal_installed() -> bool {
    Command::new("where")
        .arg("wt.exe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn build_env_script_windows(env_vars: &HashMap<String, String>, session_id: &str) -> String {
    let mut script = String::new();

    // Set session ID
    script.push_str(&format!("set GESTURA_SESSION_ID={} && ", session_id));
    script.push_str("set GESTURA_SANDBOX=1 && ");

    // Set custom environment variables
    for (key, value) in env_vars {
        script.push_str(&format!("set {}={} && ", key, value));
    }

    script.push_str(
        "echo Gestura Shell Session Ready && echo Type 'gestura --help' for available commands",
    );
    script
}

/// Open terminal on Linux using the default terminal emulator
#[cfg(target_os = "linux")]
fn open_linux_terminal(working_dir: &Path, config: &ShellSessionConfig) -> ShellResult<()> {
    let dir_str = sh_escape_single_quotes(working_dir.to_string_lossy().as_ref());

    // Build environment setup script
    let env_setup = build_env_script(&config.env_vars, &config.id);

    let init_cmd = format!(
        r#"cd '{}' && {} && echo '🚀 Gestura Shell Session: {}' && exec $SHELL"#,
        dir_str, env_setup, config.id
    );

    // Try common terminal emulators in order of preference
    let terminals = [
        ("gnome-terminal", vec!["--", "bash", "-c", &init_cmd]),
        ("konsole", vec!["-e", "bash", "-c", &init_cmd]),
        (
            "xfce4-terminal",
            vec!["-e", &format!("bash -c '{}'", init_cmd)],
        ),
        ("xterm", vec!["-e", "bash", "-c", &init_cmd]),
    ];

    for (terminal, args) in terminals {
        if let Ok(mut child) = Command::new(terminal).args(&args).spawn() {
            // Check if it actually started
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => continue,
                Ok(None) | Ok(Some(_)) => {
                    tracing::info!("Opened {} shell session", terminal);
                    return Ok(());
                }
                Err(_) => continue,
            }
        }
    }

    Err(ShellSessionError::TerminalNotFound(
        "No supported terminal emulator found".to_string(),
    ))
}

/// Build environment setup script for Unix shells
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_env_script(env_vars: &HashMap<String, String>, session_id: &str) -> String {
    let mut script = String::new();

    // Set session ID and sandbox marker
    script.push_str(&format!("export GESTURA_SESSION_ID='{}' && ", session_id));
    script.push_str("export GESTURA_SANDBOX=1 && ");

    // Set custom environment variables
    for (key, value) in env_vars {
        script.push_str(&format!("export {}='{}' && ", key, value));
    }

    // Trim trailing " && "
    if script.ends_with(" && ") {
        script.truncate(script.len() - 4);
    }

    script
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_session_config_default() {
        let config = ShellSessionConfig::default();
        assert!(!config.id.is_empty());
    }

    #[test]
    fn test_shell_session_config_with_env() {
        let config = ShellSessionConfig::default().with_env("TEST_VAR", "test_value");
        assert_eq!(
            config.env_vars.get("TEST_VAR"),
            Some(&"test_value".to_string())
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_working_directory_falls_back_from_root() {
        let config = ShellSessionConfig::default().with_working_directory(PathBuf::from("/"));
        let resolved = resolve_working_directory(&config).expect("should resolve");
        assert_ne!(resolved, PathBuf::from("/"));
        assert!(resolved.exists());
        assert!(resolved.is_dir());
    }

    #[test]
    fn test_resolve_working_directory_falls_back_from_missing_dir() {
        let missing = std::env::temp_dir()
            .join("gestura_missing_dir_test")
            .join(Uuid::new_v4().to_string());
        let config = ShellSessionConfig::default().with_working_directory(missing);
        let resolved = resolve_working_directory(&config).expect("should resolve");
        assert!(resolved.exists());
        assert!(resolved.is_dir());
    }

    #[test]
    fn test_find_cli_returns_option() {
        // Just test that it doesn't panic
        let _ = is_cli_installed();
    }
}
