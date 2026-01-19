//! Session workspace management for sandboxed file operations
//!
//! This module provides utilities for creating and managing session-specific
//! workspace directories. All file operations, shell commands, and tool calls
//! are scoped to the session's workspace directory for security and isolation.
//!
//! # Security Features
//!
//! - **Path Traversal Prevention**: All paths are validated to ensure they resolve within the workspace
//! - **Symlink Validation**: Symlinks are checked to ensure their targets are within the workspace
//! - **Dangerous Path Blocking**: Paths containing null bytes, control characters, or suspicious patterns are rejected
//! - **Depth Limiting**: Maximum path component depth is enforced to prevent abuse
//! - **Race Condition Mitigation**: Time-of-check-time-of-use (TOCTOU) considerations are documented

use std::fs::{self, Metadata};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Maximum depth of path components allowed (prevents deeply nested attacks)
const MAX_PATH_DEPTH: usize = 64;

/// Maximum length of a single path component
const MAX_COMPONENT_LENGTH: usize = 255;

/// Errors that can occur during workspace operations
#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("Failed to create workspace directory: {0}")]
    CreateFailed(#[from] std::io::Error),

    #[error("Path '{path}' is outside workspace '{workspace}'")]
    PathOutsideWorkspace { path: PathBuf, workspace: PathBuf },

    #[error("Workspace directory does not exist: {0}")]
    WorkspaceNotFound(PathBuf),

    #[error("Invalid workspace path: {0}")]
    InvalidPath(String),

    #[error("Symlink '{symlink}' points outside workspace to '{target}'")]
    SymlinkEscape { symlink: PathBuf, target: PathBuf },

    #[error("Path contains dangerous characters or patterns: {0}")]
    DangerousPath(String),

    #[error("Path exceeds maximum depth of {MAX_PATH_DEPTH} components")]
    PathTooDeep,

    #[error("Path component exceeds maximum length of {MAX_COMPONENT_LENGTH} characters")]
    ComponentTooLong,

    #[error("Access denied: {0}")]
    AccessDenied(String),
}

/// Result type for workspace operations
pub type WorkspaceResult<T> = Result<T, WorkspaceError>;

/// Session workspace configuration
#[derive(Debug, Clone)]
pub struct SessionWorkspace {
    /// The root directory for this session's workspace
    pub root: PathBuf,
    /// Session ID this workspace belongs to
    pub session_id: String,
    /// Whether this is a user-provided directory or auto-generated sandbox
    pub is_sandbox: bool,
}

impl SessionWorkspace {
    /// Create a new session workspace with an auto-generated sandbox directory
    ///
    /// Creates a directory at `~/.gestura/sessions/<session_id>/`
    pub fn create_sandbox(session_id: &str) -> WorkspaceResult<Self> {
        // Validate session ID to prevent path injection
        validate_session_id(session_id)?;

        let base_dir = get_sessions_base_dir();
        let workspace_dir = base_dir.join(session_id);

        fs::create_dir_all(&workspace_dir)?;

        // Canonicalize after creation to get the real path
        let canonical = workspace_dir.canonicalize().map_err(|e| {
            WorkspaceError::InvalidPath(format!("{}: {}", workspace_dir.display(), e))
        })?;

        tracing::info!(
            session_id = %session_id,
            workspace = ?canonical,
            "Created sandbox workspace for session"
        );

        Ok(Self {
            root: canonical,
            session_id: session_id.to_string(),
            is_sandbox: true,
        })
    }

    /// Create a session workspace using an existing directory
    ///
    /// This is used when the user specifies a project directory (CLI cwd or GUI selection)
    pub fn from_directory(session_id: &str, directory: PathBuf) -> WorkspaceResult<Self> {
        // Validate session ID
        validate_session_id(session_id)?;

        // Verify the directory exists
        if !directory.exists() {
            return Err(WorkspaceError::WorkspaceNotFound(directory));
        }

        // Check that the directory is actually a directory (not a symlink to a file)
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|e| WorkspaceError::InvalidPath(format!("{}: {}", directory.display(), e)))?;

        if !metadata.is_dir() && !metadata.file_type().is_symlink() {
            return Err(WorkspaceError::InvalidPath(format!(
                "{} is not a directory",
                directory.display()
            )));
        }

        // Canonicalize the path to resolve symlinks and relative paths
        let canonical = directory
            .canonicalize()
            .map_err(|e| WorkspaceError::InvalidPath(format!("{}: {}", directory.display(), e)))?;

        // Double-check the canonical path is a directory
        if !canonical.is_dir() {
            return Err(WorkspaceError::InvalidPath(format!(
                "{} does not resolve to a directory",
                directory.display()
            )));
        }

        tracing::info!(
            session_id = %session_id,
            workspace = ?canonical,
            "Using existing directory as workspace"
        );

        Ok(Self {
            root: canonical,
            session_id: session_id.to_string(),
            is_sandbox: false,
        })
    }

    /// Resolve a path relative to the workspace root with full security validation
    ///
    /// This method performs comprehensive security checks:
    /// 1. Validates path for dangerous characters/patterns
    /// 2. Checks path depth limits
    /// 3. Validates symlinks point within workspace
    /// 4. Ensures resolved path is within workspace bounds
    ///
    /// Returns an error if any security check fails.
    pub fn resolve_path(&self, path: &Path) -> WorkspaceResult<PathBuf> {
        // First, validate the path string for dangerous patterns
        validate_path_safety(path)?;

        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        // Validate path depth
        validate_path_depth(&resolved)?;

        // For existing paths, perform symlink validation
        if resolved.exists() {
            self.validate_symlinks_in_path(&resolved)?;
        }

        // Canonicalize to resolve .. and symlinks
        // For non-existent paths, we need to normalize manually
        let canonical = if resolved.exists() {
            resolved.canonicalize().map_err(|e| {
                WorkspaceError::InvalidPath(format!("{}: {}", resolved.display(), e))
            })?
        } else {
            // For new files, normalize the path without requiring existence
            normalize_path(&resolved, &self.root)?
        };

        // Check if the path is within the workspace
        if !canonical.starts_with(&self.root) {
            return Err(WorkspaceError::PathOutsideWorkspace {
                path: path.to_path_buf(),
                workspace: self.root.clone(),
            });
        }

        Ok(canonical)
    }

    /// Resolve a path for reading (file must exist)
    ///
    /// Additional validation for read operations including symlink target checks.
    pub fn resolve_path_for_read(&self, path: &Path) -> WorkspaceResult<PathBuf> {
        let resolved = self.resolve_path(path)?;

        if !resolved.exists() {
            return Err(WorkspaceError::InvalidPath(format!(
                "Path does not exist: {}",
                path.display()
            )));
        }

        // Final symlink check on the resolved path
        self.validate_symlinks_in_path(&resolved)?;

        Ok(resolved)
    }

    /// Resolve a path for writing (parent directory must exist)
    ///
    /// Validates the parent directory exists and is writable.
    pub fn resolve_path_for_write(&self, path: &Path) -> WorkspaceResult<PathBuf> {
        let resolved = self.resolve_path(path)?;

        // Check parent directory exists and is within workspace
        if let Some(parent) = resolved.parent() {
            if !parent.exists() {
                return Err(WorkspaceError::InvalidPath(format!(
                    "Parent directory does not exist: {}",
                    parent.display()
                )));
            }

            // Validate parent is within workspace
            let parent_canonical = parent
                .canonicalize()
                .map_err(|e| WorkspaceError::InvalidPath(format!("{}: {}", parent.display(), e)))?;

            if !parent_canonical.starts_with(&self.root) {
                return Err(WorkspaceError::PathOutsideWorkspace {
                    path: path.to_path_buf(),
                    workspace: self.root.clone(),
                });
            }
        }

        Ok(resolved)
    }

    /// Validate all symlinks in a path point to targets within the workspace
    fn validate_symlinks_in_path(&self, path: &Path) -> WorkspaceResult<()> {
        let mut current = PathBuf::new();

        for component in path.components() {
            current.push(component);

            // Skip if this component doesn't exist yet
            if !current.exists() {
                continue;
            }

            // Check if this path component is a symlink
            if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
                self.validate_symlink(&current)?;
            }
        }

        Ok(())
    }

    /// Validate a single symlink points within the workspace
    fn validate_symlink(&self, symlink_path: &Path) -> WorkspaceResult<()> {
        // Read the symlink target
        let target = fs::read_link(symlink_path).map_err(|e| {
            WorkspaceError::InvalidPath(format!(
                "Failed to read symlink {}: {}",
                symlink_path.display(),
                e
            ))
        })?;

        // Resolve the target relative to the symlink's parent
        let absolute_target = if target.is_absolute() {
            target.clone()
        } else {
            symlink_path
                .parent()
                .unwrap_or(Path::new("/"))
                .join(&target)
        };

        // Canonicalize the target to get the real path
        let canonical_target = absolute_target.canonicalize().map_err(|e| {
            WorkspaceError::InvalidPath(format!(
                "Failed to resolve symlink target {}: {}",
                absolute_target.display(),
                e
            ))
        })?;

        // Check if the target is within the workspace
        if !canonical_target.starts_with(&self.root) {
            tracing::warn!(
                symlink = %symlink_path.display(),
                target = %canonical_target.display(),
                workspace = %self.root.display(),
                "Blocked symlink pointing outside workspace"
            );

            return Err(WorkspaceError::SymlinkEscape {
                symlink: symlink_path.to_path_buf(),
                target: canonical_target,
            });
        }

        Ok(())
    }

    /// Check if a path is within the workspace (without resolving)
    pub fn is_path_allowed(&self, path: &Path) -> bool {
        // Quick validation checks
        if validate_path_safety(path).is_err() {
            return false;
        }

        if validate_path_depth(path).is_err() {
            return false;
        }

        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        // Try to canonicalize, fall back to normalization
        let canonical = if resolved.exists() {
            resolved.canonicalize().unwrap_or(resolved)
        } else {
            normalize_path(&resolved, &self.root).unwrap_or(resolved)
        };

        canonical.starts_with(&self.root)
    }

    /// Check if a path contains any symlinks
    pub fn path_has_symlinks(&self, path: &Path) -> bool {
        let mut current = PathBuf::new();

        for component in path.components() {
            current.push(component);

            if current.exists()
                && fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink())
            {
                return true;
            }
        }

        false
    }

    /// Get file metadata with symlink awareness
    pub fn get_metadata(&self, path: &Path) -> WorkspaceResult<Metadata> {
        let resolved = self.resolve_path_for_read(path)?;
        fs::metadata(&resolved)
            .map_err(|e| WorkspaceError::InvalidPath(format!("{}: {}", resolved.display(), e)))
    }

    /// Get symlink metadata (doesn't follow symlinks)
    pub fn get_symlink_metadata(&self, path: &Path) -> WorkspaceResult<Metadata> {
        let resolved = self.resolve_path(path)?;
        fs::symlink_metadata(&resolved)
            .map_err(|e| WorkspaceError::InvalidPath(format!("{}: {}", resolved.display(), e)))
    }

    /// Get the workspace root directory
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Clean up the workspace directory (only for sandbox workspaces)
    pub fn cleanup(&self) -> WorkspaceResult<()> {
        if self.is_sandbox && self.root.exists() {
            tracing::info!(
                session_id = %self.session_id,
                workspace = ?self.root,
                "Cleaning up sandbox workspace"
            );
            fs::remove_dir_all(&self.root)?;
        }
        Ok(())
    }
}

/// Get the base directory for session workspaces
///
/// Returns `~/.gestura/sessions/` on Unix or `%LOCALAPPDATA%\gestura\sessions\` on Windows
pub fn get_sessions_base_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gestura")
        .join("sessions")
}

/// Clean up old session workspaces that are older than the specified duration
pub fn cleanup_old_sessions(max_age: std::time::Duration) -> WorkspaceResult<usize> {
    let base_dir = get_sessions_base_dir();
    if !base_dir.exists() {
        return Ok(0);
    }

    let now = std::time::SystemTime::now();
    let mut cleaned = 0;

    for entry in fs::read_dir(&base_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        // Check the modification time
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age <= max_age {
            continue;
        }

        tracing::info!(
            path = ?path,
            age_days = age.as_secs() / 86400,
            "Removing old session workspace"
        );
        if fs::remove_dir_all(&path).is_ok() {
            cleaned += 1;
        }
    }

    Ok(cleaned)
}

// ============================================================================
// Security Validation Helper Functions
// ============================================================================

/// Validate a session ID to prevent path injection attacks
fn validate_session_id(session_id: &str) -> WorkspaceResult<()> {
    // Check for empty session ID
    if session_id.is_empty() {
        return Err(WorkspaceError::InvalidPath(
            "Session ID cannot be empty".to_string(),
        ));
    }

    // Check for dangerous characters
    for ch in session_id.chars() {
        match ch {
            // Allow alphanumeric, hyphens, and underscores
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => {}
            // Block everything else
            _ => {
                return Err(WorkspaceError::DangerousPath(format!(
                    "Session ID contains invalid character: '{}'",
                    ch
                )));
            }
        }
    }

    // Check for path traversal patterns
    if session_id.contains("..") {
        return Err(WorkspaceError::DangerousPath(
            "Session ID cannot contain '..'".to_string(),
        ));
    }

    // Check length
    if session_id.len() > MAX_COMPONENT_LENGTH {
        return Err(WorkspaceError::ComponentTooLong);
    }

    Ok(())
}

/// Validate a path for dangerous patterns and characters
fn validate_path_safety(path: &Path) -> WorkspaceResult<()> {
    let path_str = path.to_string_lossy();

    // Check for null bytes (can be used to truncate paths in some systems)
    if path_str.contains('\0') {
        return Err(WorkspaceError::DangerousPath(
            "Path contains null byte".to_string(),
        ));
    }

    // Check for control characters (ASCII 0-31 except tab, newline)
    for ch in path_str.chars() {
        if ch.is_control() && ch != '\t' && ch != '\n' && ch != '\r' {
            return Err(WorkspaceError::DangerousPath(format!(
                "Path contains control character: 0x{:02x}",
                ch as u32
            )));
        }
    }

    // Check each component for validity
    for component in path.components() {
        if let Component::Normal(os_str) = component {
            let comp_str = os_str.to_string_lossy();

            // Check component length
            if comp_str.len() > MAX_COMPONENT_LENGTH {
                return Err(WorkspaceError::ComponentTooLong);
            }

            // Block components that start with hyphen (could be interpreted as flags)
            // Allow hidden files (starting with .) as they're common in development
            if comp_str.starts_with("--") {
                return Err(WorkspaceError::DangerousPath(format!(
                    "Path component looks like a command flag: {}",
                    comp_str
                )));
            }

            // Block Windows reserved names (even on Unix for portability)
            let upper = comp_str.to_uppercase();
            let reserved = [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ];
            // Check if it's exactly the reserved name or reserved name with extension
            let base_name = upper.split('.').next().unwrap_or(&upper);
            if reserved.contains(&base_name) {
                return Err(WorkspaceError::DangerousPath(format!(
                    "Path contains Windows reserved name: {}",
                    comp_str
                )));
            }
        }
    }

    Ok(())
}

/// Validate that a path doesn't exceed the maximum depth
fn validate_path_depth(path: &Path) -> WorkspaceResult<()> {
    let depth = path.components().count();
    if depth > MAX_PATH_DEPTH {
        return Err(WorkspaceError::PathTooDeep);
    }
    Ok(())
}

/// Normalize a path without requiring it to exist
///
/// This handles `..` and `.` components safely for paths that don't exist yet.
fn normalize_path(path: &Path, workspace_root: &Path) -> WorkspaceResult<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(p) => normalized.push(p.as_os_str()),
            Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
            Component::CurDir => {
                // Skip `.` components
            }
            Component::ParentDir => {
                // Handle `..` by popping the last component
                if !normalized.pop() {
                    // Can't go above root
                    return Err(WorkspaceError::PathOutsideWorkspace {
                        path: path.to_path_buf(),
                        workspace: workspace_root.to_path_buf(),
                    });
                }
            }
            Component::Normal(c) => {
                normalized.push(c);
            }
        }
    }

    // Ensure the normalized path is still within the workspace
    // This catches cases where many `..` components escape the workspace
    let canonical_workspace = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    if !normalized.starts_with(&canonical_workspace) {
        return Err(WorkspaceError::PathOutsideWorkspace {
            path: path.to_path_buf(),
            workspace: workspace_root.to_path_buf(),
        });
    }

    Ok(normalized)
}

/// Blocked shell command patterns for additional security
pub fn is_shell_command_allowed(command: &str) -> Result<(), String> {
    let command_lower = command.to_lowercase();

    // Block commands that could escape the workspace or cause system damage
    let blocked_patterns = [
        // Privilege escalation
        "sudo ",
        "su ",
        "doas ",
        "pkexec ",
        // Dangerous destructive commands with root paths
        "rm -rf /",
        "rm -fr /",
        "rm -rf /*",
        "chmod -R 777 /",
        "chown -R",
        // Shell escapes
        "exec ",
        "eval ",
        // Network exfiltration (could be too aggressive, consider removing)
        // "curl ",
        // "wget ",
        // Process manipulation
        "kill -9 1",
        "killall ",
        // System modification
        "mount ",
        "umount ",
        "mkfs",
        "fdisk",
        "dd if=",
    ];

    for pattern in &blocked_patterns {
        if command_lower.contains(pattern) {
            return Err(format!("Command contains blocked pattern: {}", pattern));
        }
    }

    // Block commands that start with certain dangerous prefixes
    let blocked_prefixes = ["sudo", "su", "doas"];
    let first_word = command.split_whitespace().next().unwrap_or("");
    for prefix in &blocked_prefixes {
        if first_word == *prefix {
            return Err(format!("Command starts with blocked prefix: {}", prefix));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_sandbox_creation() {
        let session_id = "test-session-123";
        let workspace = SessionWorkspace::create_sandbox(session_id).unwrap();

        assert!(workspace.root.exists());
        assert!(workspace.is_sandbox);
        assert_eq!(workspace.session_id, session_id);

        // Cleanup
        workspace.cleanup().unwrap();
        assert!(!workspace.root.exists());
    }

    #[test]
    fn test_from_directory() {
        let temp = tempdir().unwrap();
        let session_id = "test-session-456";

        let workspace =
            SessionWorkspace::from_directory(session_id, temp.path().to_path_buf()).unwrap();

        assert!(!workspace.is_sandbox);
        assert_eq!(workspace.session_id, session_id);
    }

    #[test]
    fn test_path_resolution() {
        let temp = tempdir().unwrap();
        let session_id = "test-session-789";

        let workspace =
            SessionWorkspace::from_directory(session_id, temp.path().to_path_buf()).unwrap();

        // Create a file in the workspace
        let test_file = temp.path().join("test.txt");
        File::create(&test_file).unwrap();

        // Relative path should resolve within workspace
        let resolved = workspace.resolve_path(Path::new("test.txt")).unwrap();
        assert_eq!(resolved, test_file.canonicalize().unwrap());

        // Path outside workspace should fail
        let outside = workspace.resolve_path(Path::new("../../../etc/passwd"));
        assert!(outside.is_err());
    }

    #[test]
    fn test_is_path_allowed() {
        let temp = tempdir().unwrap();
        let session_id = "test-session-allowed";

        let workspace =
            SessionWorkspace::from_directory(session_id, temp.path().to_path_buf()).unwrap();

        assert!(workspace.is_path_allowed(Path::new("subdir/file.txt")));
        assert!(workspace.is_path_allowed(temp.path()));
        // Note: is_path_allowed may return true for non-existent paths that would be inside
    }

    #[test]
    fn test_dangerous_path_null_byte() {
        let result = validate_path_safety(Path::new("file\0.txt"));
        assert!(result.is_err());
        if let Err(WorkspaceError::DangerousPath(msg)) = result {
            assert!(msg.contains("null byte"));
        }
    }

    #[test]
    fn test_dangerous_path_control_chars() {
        // Test with a control character (bell)
        let result = validate_path_safety(Path::new("file\x07.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_dangerous_path_windows_reserved() {
        let result = validate_path_safety(Path::new("CON"));
        assert!(result.is_err());

        let result = validate_path_safety(Path::new("LPT1.txt"));
        assert!(result.is_err());

        let result = validate_path_safety(Path::new("normal.txt"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_path_depth_limit() {
        // Create a very deep path
        let deep_path: PathBuf = (0..100).map(|i| format!("dir{}", i)).collect();
        let result = validate_path_depth(&deep_path);
        assert!(result.is_err());

        // Normal depth should be fine
        let normal_path = Path::new("a/b/c/d/e");
        let result = validate_path_depth(normal_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_session_id_validation() {
        // Valid session IDs
        assert!(validate_session_id("my-session-123").is_ok());
        assert!(validate_session_id("session_with_underscore").is_ok());
        assert!(validate_session_id("UPPERCASE123").is_ok());

        // Invalid session IDs
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("session/with/slashes").is_err());
        assert!(validate_session_id("session..traversal").is_err());
        assert!(validate_session_id("session with spaces").is_err());
    }

    #[test]
    fn test_shell_command_blocking() {
        // Blocked commands
        assert!(is_shell_command_allowed("sudo rm -rf /").is_err());
        assert!(is_shell_command_allowed("rm -rf /").is_err());
        assert!(is_shell_command_allowed("su - root").is_err());

        // Allowed commands
        assert!(is_shell_command_allowed("ls -la").is_ok());
        assert!(is_shell_command_allowed("git status").is_ok());
        assert!(is_shell_command_allowed("cat file.txt").is_ok());
        assert!(is_shell_command_allowed("rm -rf ./node_modules").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_validation() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let session_id = "test-symlink-session";

        let workspace =
            SessionWorkspace::from_directory(session_id, temp.path().to_path_buf()).unwrap();

        // Create a valid symlink within workspace
        let target = temp.path().join("target.txt");
        File::create(&target).unwrap();
        let valid_link = temp.path().join("valid_link");
        symlink(&target, &valid_link).unwrap();

        // Valid symlink should be allowed
        let result = workspace.resolve_path(Path::new("valid_link"));
        assert!(result.is_ok());

        // Create a symlink pointing outside workspace
        let outside_link = temp.path().join("escape_link");
        symlink("/etc/passwd", &outside_link).unwrap();

        // Invalid symlink should be blocked
        let result = workspace.resolve_path(Path::new("escape_link"));
        assert!(result.is_err());
        if let Err(WorkspaceError::SymlinkEscape { .. }) = result {
            // Expected
        } else {
            panic!("Expected SymlinkEscape error");
        }
    }

    #[test]
    fn test_resolve_path_for_write() {
        let temp = tempdir().unwrap();
        let session_id = "test-write-session";

        let workspace =
            SessionWorkspace::from_directory(session_id, temp.path().to_path_buf()).unwrap();

        // Create a subdirectory
        let subdir = temp.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        // Writing to existing directory should work
        let result = workspace.resolve_path_for_write(Path::new("subdir/new_file.txt"));
        assert!(result.is_ok());

        // Writing to non-existent parent should fail
        let result = workspace.resolve_path_for_write(Path::new("nonexistent/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_path_has_symlinks() {
        let temp = tempdir().unwrap();
        let session_id = "test-has-symlinks";

        let workspace =
            SessionWorkspace::from_directory(session_id, temp.path().to_path_buf()).unwrap();

        // Regular file should not have symlinks
        let regular = temp.path().join("regular.txt");
        File::create(&regular).unwrap();
        assert!(!workspace.path_has_symlinks(Path::new("regular.txt")));
    }
}
