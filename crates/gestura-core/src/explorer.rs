//! Workspace file explorer helpers.
//!
//! Platform-independent business logic for listing directories, validating
//! relative paths, and normalising git-porcelain change paths.  GUI and CLI
//! presentation layers should delegate here and only add transport-specific
//! wrappers (Tauri commands, TUI rendering, …).

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned by the explorer helpers.
#[derive(Debug, thiserror::Error)]
pub enum ExplorerError {
    /// The workspace root directory has not been configured.
    #[error("workspace root is not set")]
    MissingRoot,
    /// The supplied relative path is syntactically invalid (absolute, contains
    /// `..`, etc.).
    #[error("invalid relative path: {0}")]
    InvalidRelPath(String),
    /// The resolved path escapes the workspace root (e.g. via symlinks).
    #[error("path escapes workspace root")]
    PathEscapesRoot,
    /// The resolved path does not point to a directory.
    #[error("not a directory: {0}")]
    NotADirectory(String),
    /// Underlying I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerEntryKind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
}

/// A single entry returned by [`list_dir`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerEntry {
    /// File/directory name (leaf component only).
    pub name: String,
    /// Forward-slash separated path relative to the workspace root.
    pub rel_path: String,
    /// Whether this entry is a file or directory.
    pub kind: ExplorerEntryKind,
    /// Whether the entry is a symbolic link.
    pub is_symlink: bool,
}

/// Response payload for a directory listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerListDirResponse {
    /// Absolute path of the workspace root.
    pub root: String,
    /// Relative directory that was listed.
    pub dir_rel: String,
    /// Entries in the directory.
    pub entries: Vec<ExplorerEntry>,
    /// `true` when `max_entries` was reached before the full listing.
    pub truncated: bool,
}

/// Response payload for the workspace root query.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerRootResponse {
    /// Absolute path of the workspace root.
    pub root: String,
    /// Whether a `.git` directory exists at the root.
    pub is_git_repo: bool,
}

/// Kind of change reported by `git status --porcelain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerGitChangeKind {
    /// Newly added.
    Added,
    /// Modified content.
    Modified,
    /// Deleted.
    Deleted,
    /// Renamed.
    Renamed,
    /// Copied.
    Copied,
    /// Untracked (only in `unstaged`).
    Untracked,
    /// Unknown status character.
    Unknown,
}

/// Combined staged/unstaged/untracked status for a single path.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerGitPathStatus {
    /// Staged index change kind (if any).
    pub staged: Option<ExplorerGitChangeKind>,
    /// Unstaged worktree change kind (if any).
    pub unstaged: Option<ExplorerGitChangeKind>,
    /// `true` when the path is untracked.
    pub untracked: bool,
}

/// Response payload for the git-status query.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerGitStatusResponse {
    /// Absolute path of the workspace root.
    pub root: String,
    /// Whether a `.git` directory exists at the root.
    pub is_git_repo: bool,
    /// Per-path statuses (key = forward-slash relative path).
    pub paths: HashMap<String, ExplorerGitPathStatus>,
    /// Non-fatal error message (e.g. `git` not installed).
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// Validate that `rel` is a safe, non-escaping relative path.
///
/// Returns the normalised [`PathBuf`] on success, or [`ExplorerError::InvalidRelPath`]
/// if the path is absolute or contains `..` / prefix components.
pub fn ensure_safe_rel_path(rel: &str) -> Result<PathBuf, ExplorerError> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Ok(PathBuf::new());
    }

    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(ExplorerError::InvalidRelPath(rel.to_string()));
    }

    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(_) => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExplorerError::InvalidRelPath(rel.to_string()));
            }
        }
    }

    Ok(p.to_path_buf())
}

/// Convert a [`Path`] into a forward-slash separated string, keeping only
/// `Normal` components.
fn path_to_slash_string(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Normalize a git porcelain path into a safe `rel_path` string.
///
/// - Handles rename/copy entries that contain `old -> new` by keeping the **new** path.
/// - Strips surrounding quotes.
/// - Rejects any path that isn't a safe relative path (e.g. contains `..`).
pub fn normalize_git_change_path(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let raw = raw.trim_matches('"');
    let raw = match raw.rsplit_once(" -> ") {
        Some((_, new)) => new.trim(),
        None => raw,
    };

    let safe = ensure_safe_rel_path(raw).ok()?;
    Some(path_to_slash_string(&safe))
}

/// Canonicalize the workspace root path.
pub fn canonical_root(root: &Path) -> Result<PathBuf, ExplorerError> {
    Ok(std::fs::canonicalize(root)?)
}

/// Resolve `rel` under `root`, ensuring the result stays within the root
/// after symlink resolution.
pub fn resolve_under_root(root: &Path, rel: &str) -> Result<PathBuf, ExplorerError> {
    let rel = ensure_safe_rel_path(rel)?;
    let root_canon = canonical_root(root)?;
    let joined = root_canon.join(rel);
    let target = std::fs::canonicalize(&joined)?;
    if !target.starts_with(&root_canon) {
        return Err(ExplorerError::PathEscapesRoot);
    }
    Ok(target)
}

/// List directory entries under `root/dir_rel`, returning at most `max_entries`.
///
/// Returns `(entries, truncated)` where `truncated` is `true` when the listing
/// was cut short.  Entries are sorted directories-first, then
/// case-insensitively by name.  Symlinks that escape the workspace root are
/// silently omitted.
pub fn list_dir(
    root: &Path,
    dir_rel: &str,
    max_entries: usize,
) -> Result<(Vec<ExplorerEntry>, bool), ExplorerError> {
    let dir_rel = dir_rel.trim();

    let root_canon = canonical_root(root)?;
    let dir_path = if dir_rel.is_empty() {
        root_canon.clone()
    } else {
        resolve_under_root(&root_canon, dir_rel)?
    };

    if !dir_path.is_dir() {
        return Err(ExplorerError::NotADirectory(dir_path.display().to_string()));
    }

    let mut entries = Vec::new();
    let mut truncated = false;

    for (i, e) in std::fs::read_dir(&dir_path)?.enumerate() {
        if i >= max_entries {
            truncated = true;
            break;
        }
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }

        let ft = e.file_type()?;
        let is_symlink = ft.is_symlink();

        // Follow symlinks only enough to determine dir/file, but never allow escape.
        let kind = if ft.is_symlink() {
            match std::fs::canonicalize(e.path()) {
                Ok(target) if target.starts_with(&root_canon) => {
                    if target.is_dir() {
                        ExplorerEntryKind::Dir
                    } else {
                        ExplorerEntryKind::File
                    }
                }
                Ok(_) => {
                    // Symlink escapes root; drop it from the listing entirely.
                    continue;
                }
                Err(_) => ExplorerEntryKind::File,
            }
        } else if ft.is_dir() {
            ExplorerEntryKind::Dir
        } else {
            ExplorerEntryKind::File
        };

        let rel_path = if dir_rel.is_empty() {
            name.clone()
        } else {
            path_to_slash_string(Path::new(dir_rel).join(&name).as_path())
        };

        entries.push(ExplorerEntry {
            name,
            rel_path,
            kind,
            is_symlink,
        });
    }

    entries.sort_by(|a, b| {
        let a_dir = a.kind == ExplorerEntryKind::Dir;
        let b_dir = b.kind == ExplorerEntryKind::Dir;
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    Ok((entries, truncated))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_dir_traversal() {
        assert!(ensure_safe_rel_path("../secret").is_err());
        assert!(ensure_safe_rel_path("a/../../b").is_err());
    }

    #[test]
    fn allows_empty_and_normal_paths() {
        assert_eq!(ensure_safe_rel_path("").unwrap(), PathBuf::new());
        assert_eq!(
            ensure_safe_rel_path("src/lib.rs").unwrap(),
            PathBuf::from("src/lib.rs")
        );
    }

    #[cfg(unix)]
    #[test]
    fn drops_symlinks_that_escape_root() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        symlink(&outside, root.join("escape")).unwrap();
        fs::write(root.join("ok.txt"), "ok").unwrap();

        let (entries, _) = list_dir(&root, "", 100).unwrap();
        assert!(entries.iter().any(|e| e.name == "ok.txt"));
        assert!(!entries.iter().any(|e| e.name == "escape"));
    }

    #[test]
    fn normalize_git_change_path_handles_rename_like_strings() {
        let p = PathBuf::from("old name.rs -> new name.rs");
        assert_eq!(
            normalize_git_change_path(&p).as_deref(),
            Some("new name.rs")
        );

        let p = PathBuf::from("\"old.rs -> new.rs\"");
        assert_eq!(normalize_git_change_path(&p).as_deref(), Some("new.rs"));
    }
}
