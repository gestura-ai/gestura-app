//! Memory Bank - Persistent context storage for conversation history
//!
//! This module implements the Memory Bank concept inspired by Kilo Code's approach.
//! It provides persistent storage of conversation context in human-readable markdown
//! files that can be searched and retrieved across sessions.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur during memory bank operations
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::{MemoryBankError, load_from_memory_bank};
/// use std::path::Path;
///
/// # async fn example() -> Result<(), MemoryBankError> {
/// match load_from_memory_bank(Path::new("invalid.md")).await {
///     Err(MemoryBankError::Io(e)) => println!("File not found: {}", e),
///     Err(MemoryBankError::Parse(msg)) => println!("Invalid format: {}", msg),
///     Err(MemoryBankError::DirectoryNotFound(path)) => println!("Directory not found: {}", path.display()),
///     Err(MemoryBankError::InvalidEntryPath { file_path, memory_dir }) => {
///         println!(
///             "Invalid entry path: {} (expected under {})",
///             file_path.display(),
///             memory_dir.display()
///         )
///     }
///     Ok(entry) => println!("Loaded: {}", entry.summary),
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Error)]
pub enum MemoryBankError {
    /// I/O error during file operations (e.g., file not found, permission denied)
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Error parsing markdown content (e.g., missing required fields, invalid format)
    #[error("Parse error: {0}")]
    Parse(String),
    /// Memory bank directory not found at the expected location
    #[error("Memory bank directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    /// An entry path was provided that is not within the workspace's memory bank directory
    #[error("Invalid memory bank entry path: {file_path} (expected under {memory_dir})")]
    InvalidEntryPath {
        file_path: PathBuf,
        memory_dir: PathBuf,
    },
}

/// A single entry in the memory bank representing a saved conversation context
///
/// Each entry contains metadata (timestamp, session ID, summary) and the full
/// conversation content. Entries are stored as markdown files in `.gestura/memory/`
/// and can be searched and retrieved across sessions.
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::MemoryBankEntry;
///
/// let entry = MemoryBankEntry::new(
///     "session-123".to_string(),
///     "Implemented user authentication".to_string(),
///     "User: How do I add auth?\nAssistant: Here's how...".to_string(),
/// );
///
/// let markdown = entry.to_markdown();
/// let filename = entry.generate_filename(); // e.g., "memory_20260121_143022_session-1.md"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBankEntry {
    /// Timestamp when this entry was created (UTC)
    pub timestamp: DateTime<Utc>,
    /// Session ID that created this entry (used for grouping related conversations)
    pub session_id: String,
    /// Optional category for grouping/filtering entries (e.g., "project", "personal", "research")
    pub category: Option<String>,
    /// Brief summary of the conversation (used for search and display)
    pub summary: String,
    /// Full conversation context in markdown format
    pub content: String,
    /// File path where this entry is stored (not serialized, populated on load)
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

impl PartialEq for MemoryBankEntry {
    fn eq(&self, other: &Self) -> bool {
        // `file_path` is an on-disk detail populated on load and is intentionally
        // excluded from equality so entries compare based on their semantic content.
        self.timestamp == other.timestamp
            && self.session_id == other.session_id
            && self.category == other.category
            && self.summary == other.summary
            && self.content == other.content
    }
}

impl Eq for MemoryBankEntry {}

impl MemoryBankEntry {
    /// Create a new memory bank entry with the current timestamp
    ///
    /// # Arguments
    ///
    /// * `session_id` - Unique identifier for the session that created this entry
    /// * `summary` - Brief description of the conversation (used for search)
    /// * `content` - Full conversation context in markdown format
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use gestura_core::memory_bank::MemoryBankEntry;
    ///
    /// let entry = MemoryBankEntry::new(
    ///     "session-abc123".to_string(),
    ///     "Fixed authentication bug".to_string(),
    ///     "User: Auth is broken\nAssistant: Here's the fix...".to_string(),
    /// );
    /// ```
    pub fn new(session_id: String, summary: String, content: String) -> Self {
        Self {
            timestamp: Utc::now(),
            session_id,
            category: None,
            summary,
            content,
            file_path: None,
        }
    }

    /// Convert entry to markdown format for file storage
    ///
    /// The markdown format includes metadata headers and the full context:
    /// ```markdown
    /// # Memory Bank Entry
    ///
    /// **Timestamp**: 2026-01-21 14:30:22 UTC
    /// **Session ID**: session-abc123
    /// **Category**: engineering   # optional
    /// **Summary**: Fixed authentication bug
    ///
    /// ## Context
    ///
    /// [conversation content here]
    /// ```
    pub fn to_markdown(&self) -> String {
        let category_line = self
            .category
            .as_deref()
            .map(|c| format!("**Category**: {}\n", c))
            .unwrap_or_default();
        format!(
            "# Memory Bank Entry\n\n\
             **Timestamp**: {}\n\
             **Session ID**: {}\n\
             {}\
             **Summary**: {}\n\n\
             ## Context\n\n\
             {}\n",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            self.session_id,
            category_line,
            self.summary,
            self.content
        )
    }

    /// Parse entry from markdown format
    ///
    /// # Arguments
    ///
    /// * `markdown` - Markdown content to parse
    /// * `file_path` - Optional path to the source file (stored in the entry)
    ///
    /// # Returns
    ///
    /// Parsed memory bank entry
    ///
    /// # Errors
    ///
    /// Returns `MemoryBankError::Parse` if required fields are missing or invalid
    pub fn from_markdown(
        markdown: &str,
        file_path: Option<PathBuf>,
    ) -> Result<Self, MemoryBankError> {
        let lines: Vec<&str> = markdown.lines().collect();

        let mut timestamp = None;
        let mut session_id = None;
        let mut category: Option<String> = None;
        let mut summary = None;
        let mut content_start = None;

        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("**Timestamp**:") {
                let ts_str = line.trim_start_matches("**Timestamp**:").trim();
                // Parse timestamp - remove " UTC" suffix and parse as naive datetime, then assume UTC
                let ts_str_clean = ts_str.trim_end_matches(" UTC");
                timestamp =
                    chrono::NaiveDateTime::parse_from_str(ts_str_clean, "%Y-%m-%d %H:%M:%S")
                        .ok()
                        .map(|ndt| Utc.from_utc_datetime(&ndt));
            } else if line.starts_with("**Session ID**:") {
                session_id = Some(
                    line.trim_start_matches("**Session ID**:")
                        .trim()
                        .to_string(),
                );
            } else if line.starts_with("**Category**:") {
                let v = line.trim_start_matches("**Category**:").trim();
                if !v.is_empty() {
                    category = Some(v.to_string());
                }
            } else if line.starts_with("**Summary**:") {
                summary = Some(line.trim_start_matches("**Summary**:").trim().to_string());
            } else if line.starts_with("## Context") {
                content_start = Some(i + 2); // Skip the header and blank line
                break;
            }
        }

        let timestamp =
            timestamp.ok_or_else(|| MemoryBankError::Parse("Missing timestamp".to_string()))?;
        let session_id =
            session_id.ok_or_else(|| MemoryBankError::Parse("Missing session ID".to_string()))?;
        let summary =
            summary.ok_or_else(|| MemoryBankError::Parse("Missing summary".to_string()))?;
        let content_start = content_start
            .ok_or_else(|| MemoryBankError::Parse("Missing context section".to_string()))?;

        let content = lines[content_start..].join("\n");

        Ok(Self {
            timestamp,
            session_id,
            category,
            summary,
            content,
            file_path,
        })
    }

    /// Generate a unique filename for this entry
    ///
    /// Format: `memory_YYYYMMDD_HHMMSS_<session-prefix>.md`
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use gestura_core::memory_bank::MemoryBankEntry;
    ///
    /// let entry = MemoryBankEntry::new(
    ///     "session-abc123".to_string(),
    ///     "Summary".to_string(),
    ///     "Content".to_string(),
    /// );
    ///
    /// let filename = entry.generate_filename();
    /// // e.g., "memory_20260121_143022_session-a.md"
    /// assert!(filename.starts_with("memory_"));
    /// assert!(filename.ends_with(".md"));
    /// ```
    pub fn generate_filename(&self) -> String {
        format!(
            "memory_{}_{}.md",
            self.timestamp.format("%Y%m%d_%H%M%S"),
            &self.session_id[..20.min(self.session_id.len())]
        )
    }
}

/// Get the memory bank directory path for a workspace
///
/// Returns the path to `.gestura/memory/` within the workspace directory.
/// This directory is created automatically when saving entries.
///
/// # Arguments
///
/// * `workspace_dir` - Root directory of the workspace
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::get_memory_bank_dir;
/// use std::path::Path;
///
/// let workspace = Path::new("/home/user/project");
/// let memory_dir = get_memory_bank_dir(workspace);
/// assert_eq!(memory_dir, Path::new("/home/user/project/.gestura/memory"));
/// ```
pub fn get_memory_bank_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(".gestura").join("memory")
}

/// Ensure the memory bank directory exists, creating it if necessary
///
/// # Arguments
///
/// * `workspace_dir` - Root directory of the workspace
///
/// # Returns
///
/// Path to the memory bank directory
///
/// # Errors
///
/// Returns `MemoryBankError::Io` if directory creation fails
pub async fn ensure_memory_bank_dir(workspace_dir: &Path) -> Result<PathBuf, MemoryBankError> {
    let dir = get_memory_bank_dir(workspace_dir);
    tokio::fs::create_dir_all(&dir).await?;
    Ok(dir)
}

/// Save a memory bank entry to disk as a markdown file
///
/// Creates the `.gestura/memory/` directory if it doesn't exist, then writes
/// the entry as a markdown file with a timestamp-based filename.
///
/// # Arguments
///
/// * `workspace_dir` - The workspace directory (memory will be saved to `.gestura/memory/`)
/// * `entry` - The memory bank entry to save
///
/// # Returns
///
/// Path to the saved file
///
/// # Errors
///
/// Returns `MemoryBankError::Io` if directory creation or file write fails
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::{MemoryBankEntry, save_to_memory_bank};
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let entry = MemoryBankEntry::new(
///     "session-123".to_string(),
///     "Fixed bug".to_string(),
///     "Conversation content...".to_string(),
/// );
///
/// let workspace = Path::new("/home/user/project");
/// let file_path = save_to_memory_bank(workspace, &entry).await?;
/// println!("Saved to: {}", file_path.display());
/// # Ok(())
/// # }
/// ```
pub async fn save_to_memory_bank(
    workspace_dir: &Path,
    entry: &MemoryBankEntry,
) -> Result<PathBuf, MemoryBankError> {
    let dir = ensure_memory_bank_dir(workspace_dir).await?;
    let filename = entry.generate_filename();
    let file_path = dir.join(&filename);

    let markdown = entry.to_markdown();
    tokio::fs::write(&file_path, markdown).await?;

    tracing::info!(
        file_path = %file_path.display(),
        session_id = %entry.session_id,
        "Saved memory bank entry"
    );

    Ok(file_path)
}

/// Load a memory bank entry from a markdown file
///
/// # Arguments
///
/// * `file_path` - Path to the memory bank markdown file
///
/// # Returns
///
/// The loaded memory bank entry with `file_path` populated
///
/// # Errors
///
/// Returns `MemoryBankError::Io` if file read fails, or `MemoryBankError::Parse`
/// if the markdown format is invalid
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::load_from_memory_bank;
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let file_path = Path::new(".gestura/memory/memory_20260121_143022_session-a.md");
/// let entry = load_from_memory_bank(file_path).await?;
/// println!("Loaded: {}", entry.summary);
/// # Ok(())
/// # }
/// ```
pub async fn load_from_memory_bank(file_path: &Path) -> Result<MemoryBankEntry, MemoryBankError> {
    let markdown = tokio::fs::read_to_string(file_path).await?;
    MemoryBankEntry::from_markdown(&markdown, Some(file_path.to_path_buf()))
}

/// List all memory bank entries in a workspace
///
/// Scans the `.gestura/memory/` directory for all markdown files and loads them.
/// Invalid or corrupted files are logged as warnings and skipped.
///
/// # Arguments
///
/// * `workspace_dir` - The workspace directory containing `.gestura/memory/`
///
/// # Returns
///
/// Vector of all memory bank entries, sorted by timestamp (newest first)
///
/// # Errors
///
/// Returns `MemoryBankError::Io` if directory read fails. Individual file
/// parse errors are logged but don't fail the entire operation.
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::list_memory_bank;
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let workspace = Path::new("/home/user/project");
/// let entries = list_memory_bank(workspace).await?;
///
/// for entry in entries {
///     println!("{}: {}", entry.timestamp, entry.summary);
/// }
/// # Ok(())
/// # }
/// ```
pub async fn list_memory_bank(
    workspace_dir: &Path,
) -> Result<Vec<MemoryBankEntry>, MemoryBankError> {
    let dir = get_memory_bank_dir(workspace_dir);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&dir).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            match load_from_memory_bank(&path).await {
                Ok(mem_entry) => entries.push(mem_entry),
                Err(e) => {
                    tracing::warn!(
                        file_path = %path.display(),
                        error = %e,
                        "Failed to load memory bank entry"
                    );
                }
            }
        }
    }

    // Sort by timestamp, newest first
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(entries)
}

async fn ensure_memory_bank_dir_exists(workspace_dir: &Path) -> Result<PathBuf, MemoryBankError> {
    let dir = get_memory_bank_dir(workspace_dir);
    if tokio::fs::try_exists(&dir).await? {
        Ok(dir)
    } else {
        Err(MemoryBankError::DirectoryNotFound(dir))
    }
}

async fn validate_entry_path(
    workspace_dir: &Path,
    file_path: &Path,
) -> Result<PathBuf, MemoryBankError> {
    let dir = ensure_memory_bank_dir_exists(workspace_dir).await?;
    let dir_canon = tokio::fs::canonicalize(&dir).await?;
    let file_canon = tokio::fs::canonicalize(file_path).await?;

    if file_canon.extension().and_then(|s| s.to_str()) != Some("md") {
        return Err(MemoryBankError::Parse(
            "Memory bank entries must be markdown (.md) files".to_string(),
        ));
    }

    if !file_canon.starts_with(&dir_canon) {
        return Err(MemoryBankError::InvalidEntryPath {
            file_path: file_canon,
            memory_dir: dir_canon,
        });
    }

    Ok(file_canon)
}

async fn atomic_write_file(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Missing file name"))?
        .to_string_lossy();

    let tmp_path = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    tokio::fs::write(&tmp_path, contents).await?;

    match tokio::fs::rename(&tmp_path, path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // On some platforms (notably Windows), rename won't overwrite an existing file.
            // Best-effort fallback: remove destination, then rename again.
            let _ = tokio::fs::remove_file(path).await;
            let rename2 = tokio::fs::rename(&tmp_path, path).await;
            if rename2.is_err() {
                let _ = tokio::fs::remove_file(&tmp_path).await;
            }
            rename2.map_err(|_| e)
        }
    }
}

/// Delete a single memory bank entry.
///
/// This will validate that the entry path is a markdown file located under
/// the workspace's `.gestura/memory/` directory.
pub async fn delete_memory_bank_entry(
    workspace_dir: &Path,
    file_path: &Path,
) -> Result<(), MemoryBankError> {
    let file_path = validate_entry_path(workspace_dir, file_path).await?;
    tokio::fs::remove_file(&file_path).await?;
    Ok(())
}

/// Update a single memory bank entry in-place.
///
/// This will validate that `file_path` is a markdown file located under the
/// workspace's `.gestura/memory/` directory, then rewrite the file contents.
pub async fn update_memory_bank_entry(
    workspace_dir: &Path,
    file_path: &Path,
    entry: &MemoryBankEntry,
) -> Result<(), MemoryBankError> {
    let file_path = validate_entry_path(workspace_dir, file_path).await?;
    let markdown = entry.to_markdown();
    atomic_write_file(&file_path, &markdown).await?;
    Ok(())
}

/// Search memory bank entries for relevant content
///
/// Performs a case-insensitive substring search across both the summary and
/// content fields of all memory bank entries. Results are sorted by timestamp
/// (newest first) and limited to the specified count.
///
/// # Arguments
///
/// * `workspace_dir` - The workspace directory containing `.gestura/memory/`
/// * `query` - Search query string (case-insensitive)
/// * `limit` - Maximum number of results to return
///
/// # Returns
///
/// Vector of matching memory bank entries, sorted by timestamp (newest first)
///
/// # Errors
///
/// Returns `MemoryBankError::Io` if directory read fails
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::search_memory_bank;
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let workspace = Path::new("/home/user/project");
/// let results = search_memory_bank(workspace, "authentication", 5).await?;
///
/// for entry in results {
///     println!("Found: {} - {}", entry.timestamp, entry.summary);
/// }
/// # Ok(())
/// # }
/// ```
pub async fn search_memory_bank(
    workspace_dir: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<MemoryBankEntry>, MemoryBankError> {
    let all_entries = list_memory_bank(workspace_dir).await?;
    let query_lower = query.to_lowercase();

    let mut matching_entries: Vec<MemoryBankEntry> = all_entries
        .into_iter()
        .filter(|entry| {
            entry.summary.to_lowercase().contains(&query_lower)
                || entry.content.to_lowercase().contains(&query_lower)
                || entry
                    .category
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase().contains(&query_lower))
        })
        .take(limit)
        .collect();

    // Already sorted by timestamp from list_memory_bank
    matching_entries.truncate(limit);

    Ok(matching_entries)
}

/// Clear all memory bank entries in a workspace
///
/// Deletes all markdown files in the `.gestura/memory/` directory. This operation
/// is irreversible and should be used with caution.
///
/// # Arguments
///
/// * `workspace_dir` - The workspace directory containing `.gestura/memory/`
///
/// # Returns
///
/// Number of entries deleted
///
/// # Errors
///
/// Returns `MemoryBankError::Io` if directory read or file deletion fails
///
/// # Examples
///
/// ```rust,ignore
/// use gestura_core::memory_bank::clear_memory_bank;
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let workspace = Path::new("/home/user/project");
/// let count = clear_memory_bank(workspace).await?;
/// println!("Deleted {} memory bank entries", count);
/// # Ok(())
/// # }
/// ```
pub async fn clear_memory_bank(workspace_dir: &Path) -> Result<usize, MemoryBankError> {
    let dir = get_memory_bank_dir(workspace_dir);

    if !dir.exists() {
        return Ok(0);
    }

    let mut count = 0;
    let mut read_dir = tokio::fs::read_dir(&dir).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            tokio::fs::remove_file(&path).await?;
            count += 1;
        }
    }

    tracing::info!(count = count, "Cleared memory bank entries");

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_ensure_memory_bank_dir() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Ensure directory is created
        let memory_dir = ensure_memory_bank_dir(workspace_path).await.unwrap();

        assert!(memory_dir.exists(), "Memory bank directory should exist");
        assert_eq!(
            memory_dir,
            workspace_path.join(".gestura").join("memory"),
            "Memory bank directory should be at correct path"
        );
    }

    #[tokio::test]
    async fn test_save_and_load_memory_bank() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Create an entry
        let entry = MemoryBankEntry::new(
            "test-session-123".to_string(),
            "Implemented user authentication feature".to_string(),
            "User: How do I add authentication?\nAssistant: Here's how to implement JWT auth..."
                .to_string(),
        );

        // Save to memory bank
        let file_path = save_to_memory_bank(workspace_path, &entry).await.unwrap();

        assert!(file_path.exists(), "Memory bank file should exist");
        assert_eq!(
            file_path.extension().and_then(|s| s.to_str()),
            Some("md"),
            "Memory bank file should have .md extension"
        );

        // Load from memory bank
        let loaded_entry = load_from_memory_bank(&file_path).await.unwrap();

        assert_eq!(loaded_entry.session_id, entry.session_id);
        assert_eq!(loaded_entry.summary, entry.summary);
        assert_eq!(loaded_entry.content, entry.content);
        assert!(
            loaded_entry.file_path.is_some(),
            "Loaded entry should have file path"
        );
    }

    #[tokio::test]
    async fn test_list_memory_bank() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Initially empty
        let entries = list_memory_bank(workspace_path).await.unwrap();
        assert_eq!(entries.len(), 0, "Memory bank should be empty initially");

        // Save multiple entries with unique session IDs to avoid filename collisions
        for i in 0..3 {
            let entry = MemoryBankEntry::new(
                format!("session-unique-{:03}", i),
                format!("Summary {}", i),
                format!("Content {}", i),
            );
            save_to_memory_bank(workspace_path, &entry).await.unwrap();
            // Small delay to ensure different timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // List should return all entries
        let entries = list_memory_bank(workspace_path).await.unwrap();
        assert_eq!(entries.len(), 3, "Memory bank should contain 3 entries");

        // Entries should be sorted by timestamp (newest first)
        for i in 0..entries.len() - 1 {
            assert!(
                entries[i].timestamp >= entries[i + 1].timestamp,
                "Entries should be sorted by timestamp (newest first)"
            );
        }
    }

    #[tokio::test]
    async fn test_search_memory_bank() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Save entries with different content and unique session IDs
        let entries_data = vec![
            (
                "session-auth-001",
                "Implemented user authentication",
                "Developer asked about JWT authentication and OAuth2 flows",
            ),
            (
                "session-db-002",
                "Fixed database bug",
                "Team reported slow queries in PostgreSQL database",
            ),
            (
                "session-profile-003",
                "Added user profile",
                "Client wanted to add user profile pictures and bio fields",
            ),
        ];

        for (session_id, summary, content) in entries_data {
            let entry = MemoryBankEntry::new(
                session_id.to_string(),
                summary.to_string(),
                content.to_string(),
            );
            save_to_memory_bank(workspace_path, &entry).await.unwrap();
            // Small delay to ensure different timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Search for "authentication"
        let results = search_memory_bank(workspace_path, "authentication", 10)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "Should find 1 entry matching 'authentication'"
        );
        assert_eq!(results[0].session_id, "session-auth-001");

        // Search for "user"
        let results = search_memory_bank(workspace_path, "user", 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 2, "Should find 2 entries matching 'user'");

        // Search with limit
        let results = search_memory_bank(workspace_path, "user", 1).await.unwrap();
        assert_eq!(results.len(), 1, "Should respect limit parameter");

        // Search for non-existent term
        let results = search_memory_bank(workspace_path, "nonexistent", 10)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            0,
            "Should find 0 entries for non-existent term"
        );
    }

    #[tokio::test]
    async fn test_category_roundtrip_and_search() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        let mut entry = MemoryBankEntry::new(
            "session-cat-001".to_string(),
            "Entry with category".to_string(),
            "Some content".to_string(),
        );
        entry.category = Some("research".to_string());

        let file_path = save_to_memory_bank(workspace_path, &entry).await.unwrap();
        let loaded = load_from_memory_bank(&file_path).await.unwrap();
        assert_eq!(loaded.category.as_deref(), Some("research"));

        let results = search_memory_bank(workspace_path, "research", 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "session-cat-001");
    }

    #[tokio::test]
    async fn test_update_and_delete_memory_bank_entry() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        let mut entry = MemoryBankEntry::new(
            "session-edit-001".to_string(),
            "Original summary".to_string(),
            "Original content".to_string(),
        );
        entry.category = Some("initial".to_string());

        let file_path = save_to_memory_bank(workspace_path, &entry).await.unwrap();

        // Update
        let mut updated = load_from_memory_bank(&file_path).await.unwrap();
        updated.summary = "Updated summary".to_string();
        updated.content = "Updated content".to_string();
        updated.category = Some("updated".to_string());

        update_memory_bank_entry(workspace_path, &file_path, &updated)
            .await
            .unwrap();

        let reloaded = load_from_memory_bank(&file_path).await.unwrap();
        assert_eq!(reloaded.summary, "Updated summary");
        assert_eq!(reloaded.content, "Updated content");
        assert_eq!(reloaded.category.as_deref(), Some("updated"));

        // Delete
        delete_memory_bank_entry(workspace_path, &file_path)
            .await
            .unwrap();
        assert!(!file_path.exists());

        let entries = list_memory_bank(workspace_path).await.unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[tokio::test]
    async fn test_update_rejects_outside_memory_dir() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Ensure memory bank dir exists so the error is about path validation.
        let entry = MemoryBankEntry::new(
            "session-init".to_string(),
            "Init".to_string(),
            "Init".to_string(),
        );
        let _ = save_to_memory_bank(workspace_path, &entry).await.unwrap();

        let outside_path = workspace_path.join("not-in-memory.md");
        tokio::fs::write(&outside_path, "# Not a memory entry")
            .await
            .unwrap();

        // We don't actually need a valid loaded entry here; just an entry payload.
        let entry_payload = MemoryBankEntry::new(
            "session".to_string(),
            "Summary".to_string(),
            "Content".to_string(),
        );

        let err = update_memory_bank_entry(workspace_path, &outside_path, &entry_payload)
            .await
            .unwrap_err();
        match err {
            MemoryBankError::InvalidEntryPath { .. } => {}
            other => panic!("Expected InvalidEntryPath, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_clear_memory_bank() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Save multiple entries with unique session IDs
        for i in 0..5 {
            let entry = MemoryBankEntry::new(
                format!("session-clear-{:03}", i),
                format!("Summary {}", i),
                format!("Content {}", i),
            );
            save_to_memory_bank(workspace_path, &entry).await.unwrap();
            // Small delay to ensure different timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Verify entries exist
        let entries = list_memory_bank(workspace_path).await.unwrap();
        assert_eq!(entries.len(), 5, "Should have 5 entries before clear");

        // Clear memory bank
        let count = clear_memory_bank(workspace_path).await.unwrap();
        assert_eq!(count, 5, "Should clear 5 entries");

        // Verify entries are gone
        let entries = list_memory_bank(workspace_path).await.unwrap();
        assert_eq!(entries.len(), 0, "Memory bank should be empty after clear");

        // Clear again should return 0
        let count = clear_memory_bank(workspace_path).await.unwrap();
        assert_eq!(count, 0, "Clearing empty memory bank should return 0");
    }

    #[tokio::test]
    async fn test_markdown_format() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Create an entry with specific content
        let entry = MemoryBankEntry::new(
            "test-session".to_string(),
            "Test summary".to_string(),
            "Line 1\nLine 2\nLine 3".to_string(),
        );

        // Save to memory bank
        let file_path = save_to_memory_bank(workspace_path, &entry).await.unwrap();

        // Read raw markdown file
        let markdown = tokio::fs::read_to_string(&file_path).await.unwrap();

        println!("Generated markdown:\n{}", markdown);

        // Verify markdown format
        assert!(
            markdown.contains("# Memory Bank Entry"),
            "Should have title"
        );
        assert!(
            markdown.contains("**Session ID**:"),
            "Should have session ID field"
        );
        assert!(
            markdown.contains("**Timestamp**:"),
            "Should have timestamp field"
        );
        assert!(
            markdown.contains("**Summary**:"),
            "Should have summary field"
        );
        assert!(
            markdown.contains("## Context"),
            "Should have context section"
        );
        assert!(
            markdown.contains("test-session"),
            "Should contain session ID value"
        );
        assert!(
            markdown.contains("Test summary"),
            "Should contain summary value"
        );
        assert!(
            markdown.contains("Line 1\nLine 2\nLine 3"),
            "Should contain content value"
        );
    }
}
