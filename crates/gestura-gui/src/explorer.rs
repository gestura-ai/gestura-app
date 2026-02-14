use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ExplorerError {
    #[error("workspace root is not set")]
    MissingRoot,
    #[error("invalid relative path: {0}")]
    InvalidRelPath(String),
    #[error("path escapes workspace root")]
    PathEscapesRoot,
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerEntryKind {
    File,
    Dir,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerEntry {
    pub name: String,
    pub rel_path: String,
    pub kind: ExplorerEntryKind,
    pub is_symlink: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerListDirResponse {
    pub root: String,
    pub dir_rel: String,
    pub entries: Vec<ExplorerEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerRootResponse {
    pub root: String,
    pub is_git_repo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerGitChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerGitPathStatus {
    pub staged: Option<ExplorerGitChangeKind>,
    pub unstaged: Option<ExplorerGitChangeKind>,
    pub untracked: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExplorerGitStatusResponse {
    pub root: String,
    pub is_git_repo: bool,
    pub paths: HashMap<String, ExplorerGitPathStatus>,
    pub error: Option<String>,
}

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

pub fn canonical_root(root: &Path) -> Result<PathBuf, ExplorerError> {
    Ok(std::fs::canonicalize(root)?)
}

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
        let mut is_symlink = ft.is_symlink();

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

        // For non-symlinks, ensure rel_path stays normalized.
        if !is_symlink {
            is_symlink = false;
        }

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
