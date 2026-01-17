//! File operations tool
//!
//! Provides file system operations with structured output.
//! All functions return data structures rather than formatted strings.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Result of reading a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadResult {
    pub path: PathBuf,
    pub content: String,
    pub line_count: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// Result of writing a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWriteResult {
    pub path: PathBuf,
    pub bytes_written: usize,
    pub created: bool,
}

/// Result of editing a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEditResult {
    pub path: PathBuf,
    pub replacements: usize,
    pub old_content: String,
    pub new_content: String,
}

/// A file entry in directory listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<chrono::DateTime<chrono::Utc>>,
}

/// Search match result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub line_content: String,
    pub match_start: usize,
    pub match_end: usize,
}

/// Directory tree node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub children: Vec<TreeNode>,
}

/// File operations service
pub struct FileTools {
    /// Files currently in context
    context: RwLock<HashSet<PathBuf>>,
}

impl Default for FileTools {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTools {
    pub fn new() -> Self {
        Self {
            context: RwLock::new(HashSet::new()),
        }
    }

    /// Read file contents, optionally with line range
    pub fn read(
        &self,
        path: &Path,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> Result<FileReadResult> {
        if !path.exists() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            )));
        }

        let content = fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = start_line.unwrap_or(1).saturating_sub(1);
        let end = end_line.unwrap_or(total_lines).min(total_lines);

        let selected_content = lines[start..end].join("\n");

        Ok(FileReadResult {
            path: path.to_path_buf(),
            content: selected_content,
            line_count: total_lines,
            start_line: start + 1,
            end_line: end,
        })
    }

    /// Write content to file
    pub fn write(&self, path: &Path, content: &str) -> Result<FileWriteResult> {
        let created = !path.exists();

        // Create parent directories if needed
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, content)?;

        Ok(FileWriteResult {
            path: path.to_path_buf(),
            bytes_written: content.len(),
            created,
        })
    }

    /// Edit file by replacing text
    pub fn edit(&self, path: &Path, old_str: &str, new_str: &str) -> Result<FileEditResult> {
        if !path.exists() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            )));
        }

        let old_content = fs::read_to_string(path)?;
        let replacements = old_content.matches(old_str).count();

        if replacements == 0 {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "String to replace not found in file",
            )));
        }

        let new_content = old_content.replace(old_str, new_str);
        fs::write(path, &new_content)?;

        Ok(FileEditResult {
            path: path.to_path_buf(),
            replacements,
            old_content,
            new_content,
        })
    }

    /// Search for pattern in files
    pub fn search(&self, pattern: &str, path: &Path, recursive: bool) -> Result<Vec<SearchMatch>> {
        let regex = regex::Regex::new(pattern)
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Invalid regex: {e}"))))?;

        let mut matches = Vec::new();
        Self::search_in_path(&regex, path, recursive, &mut matches)?;
        Ok(matches)
    }

    fn search_in_path(
        regex: &regex::Regex,
        path: &Path,
        recursive: bool,
        matches: &mut Vec<SearchMatch>,
    ) -> Result<()> {
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(path) {
                for (line_num, line) in content.lines().enumerate() {
                    if let Some(m) = regex.find(line) {
                        matches.push(SearchMatch {
                            path: path.to_path_buf(),
                            line_number: line_num + 1,
                            line_content: line.to_string(),
                            match_start: m.start(),
                            match_end: m.end(),
                        });
                    }
                }
            }
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();
                if entry_path.is_file() {
                    Self::search_in_path(regex, &entry_path, false, matches)?;
                } else if recursive && entry_path.is_dir() {
                    Self::search_in_path(regex, &entry_path, true, matches)?;
                }
            }
        }
        Ok(())
    }

    /// List files in directory
    pub fn list(&self, path: &Path, show_hidden: bool) -> Result<Vec<FileEntry>> {
        if !path.is_dir() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("Not a directory: {}", path.display()),
            )));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            if !show_hidden && name.starts_with('.') {
                continue;
            }

            let metadata = entry.metadata().ok();
            let modified = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(chrono::DateTime::from);

            entries.push(FileEntry {
                path: entry.path(),
                name,
                is_dir: entry.path().is_dir(),
                size: metadata.as_ref().map(|m| m.len()),
                modified,
            });
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    /// Build directory tree
    pub fn tree(&self, path: &Path, max_depth: Option<usize>) -> Result<TreeNode> {
        Self::build_tree(path, 0, max_depth.unwrap_or(3))
    }

    fn build_tree(path: &Path, depth: usize, max_depth: usize) -> Result<TreeNode> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        let mut node = TreeNode {
            path: path.to_path_buf(),
            name,
            is_dir: path.is_dir(),
            children: Vec::new(),
        };

        if path.is_dir()
            && depth < max_depth
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                let entry_name = entry.file_name().to_string_lossy().to_string();
                if !entry_name.starts_with('.')
                    && let Ok(child) = Self::build_tree(&entry_path, depth + 1, max_depth)
                {
                    node.children.push(child);
                }
            }
            node.children.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });
        }

        Ok(node)
    }

    /// Add files to context
    pub fn add_to_context(&self, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let mut context = self
            .context
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;

        let mut added = Vec::new();
        for path in paths {
            if path.exists() {
                context.insert(path.clone());
                added.push(path.clone());
            }
        }
        Ok(added)
    }

    /// Remove files from context
    pub fn remove_from_context(&self, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let mut context = self
            .context
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;

        let mut removed = Vec::new();
        for path in paths {
            if context.remove(path) {
                removed.push(path.clone());
            }
        }
        Ok(removed)
    }

    /// Get current context
    pub fn get_context(&self) -> Result<Vec<PathBuf>> {
        let context = self
            .context
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;
        Ok(context.iter().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "line 1\nline 2\nline 3\n").unwrap();
        fs::write(
            dir.path().join("hello.rs"),
            "fn main() {\n    println!(\"Hello\");\n}\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("subdir/nested.txt"), "nested content").unwrap();
        dir
    }

    #[test]
    fn test_read_file() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let result = tools
            .read(&dir.path().join("test.txt"), None, None)
            .unwrap();
        assert_eq!(result.line_count, 3);
        assert!(result.content.contains("line 1"));
    }

    #[test]
    fn test_read_file_with_range() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let result = tools
            .read(&dir.path().join("test.txt"), Some(2), Some(2))
            .unwrap();
        assert_eq!(result.start_line, 2);
        assert_eq!(result.end_line, 2);
        assert!(result.content.contains("line 2"));
    }

    #[test]
    fn test_write_file() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let path = dir.path().join("new_file.txt");
        let result = tools.write(&path, "new content").unwrap();
        assert!(result.created);
        assert_eq!(result.bytes_written, 11);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
    }

    #[test]
    fn test_list_directory() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let entries = tools.list(dir.path(), false).unwrap();
        assert!(entries.len() >= 3);
        assert!(entries.iter().any(|e| e.name == "test.txt"));
        assert!(entries.iter().any(|e| e.name == "subdir" && e.is_dir));
    }

    #[test]
    fn test_search_files() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let matches = tools.search("line", dir.path(), false).unwrap();
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.line_content.contains("line 1")));
    }

    #[test]
    fn test_context_management() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let file1 = dir.path().join("test.txt");
        let file2 = dir.path().join("hello.rs");
        let paths = vec![file1.clone(), file2.clone()];

        let added = tools.add_to_context(&paths).unwrap();
        assert_eq!(added.len(), 2);

        let context = tools.get_context().unwrap();
        assert_eq!(context.len(), 2);

        let removed = tools.remove_from_context(&[file1]).unwrap();
        assert_eq!(removed.len(), 1);

        let context = tools.get_context().unwrap();
        assert_eq!(context.len(), 1);
    }

    #[test]
    fn test_tree() {
        let dir = setup_test_dir();
        let tools = FileTools::new();
        let tree = tools.tree(dir.path(), Some(2)).unwrap();
        assert!(tree.is_dir);
        assert!(!tree.children.is_empty());
    }
}
