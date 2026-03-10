//! Memory Bank - Persistent context storage for conversation history
//!
//! This module implements the Memory Bank concept inspired by Kilo Code's approach.
//! It provides persistent storage of conversation context in human-readable markdown
//! files that can be searched and retrieved across sessions.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// High-level memory retention domain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Durable long-term memory shared across sessions/agents.
    #[default]
    LongTerm,
    /// Explicitly marked short-term memory persisted for inspection or handoff.
    ShortTerm,
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LongTerm => write!(f, "long_term"),
            Self::ShortTerm => write!(f, "short_term"),
        }
    }
}

impl std::str::FromStr for MemoryKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "long_term" | "long-term" | "longterm" => Ok(Self::LongTerm),
            "short_term" | "short-term" | "shortterm" => Ok(Self::ShortTerm),
            _ => Err(format!("Unknown memory kind: {value}")),
        }
    }
}

/// Typed classification for durable memory records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Process/workflow guidance.
    Procedural,
    /// Stable factual project knowledge.
    Semantic,
    /// Outcome or event history.
    #[default]
    Episodic,
    /// Resource acquisition or references.
    Resource,
    /// Explicit decisions.
    Decision,
    /// Known blockers.
    Blocker,
    /// Handoff/checkpoint material.
    Handoff,
    /// Structured corrective reflection promoted from a failed/suboptimal agent
    /// attempt (ERL-inspired) so future turns can retrieve and reuse it.
    Reflection,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Procedural => write!(f, "procedural"),
            Self::Semantic => write!(f, "semantic"),
            Self::Episodic => write!(f, "episodic"),
            Self::Resource => write!(f, "resource"),
            Self::Decision => write!(f, "decision"),
            Self::Blocker => write!(f, "blocker"),
            Self::Handoff => write!(f, "handoff"),
            Self::Reflection => write!(f, "reflection"),
        }
    }
}

impl std::str::FromStr for MemoryType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "procedural" => Ok(Self::Procedural),
            "semantic" => Ok(Self::Semantic),
            "episodic" => Ok(Self::Episodic),
            "resource" => Ok(Self::Resource),
            "decision" => Ok(Self::Decision),
            "blocker" => Ok(Self::Blocker),
            "handoff" => Ok(Self::Handoff),
            "reflection" => Ok(Self::Reflection),
            _ => Err(format!("Unknown memory type: {value}")),
        }
    }
}

/// Scope for targeted durable-memory retrieval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Single task scope.
    Task,
    /// Single session scope.
    #[default]
    Session,
    /// Shared multi-agent directive scope.
    Directive,
    /// Workspace scope.
    Workspace,
    /// Repository-wide scope.
    Repository,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Task => write!(f, "task"),
            Self::Session => write!(f, "session"),
            Self::Directive => write!(f, "directive"),
            Self::Workspace => write!(f, "workspace"),
            Self::Repository => write!(f, "repository"),
        }
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "task" => Ok(Self::Task),
            "session" => Ok(Self::Session),
            "directive" => Ok(Self::Directive),
            "workspace" => Ok(Self::Workspace),
            "repository" | "repo" => Ok(Self::Repository),
            _ => Err(format!("Unknown memory scope: {value}")),
        }
    }
}

/// Filter options for targeted memory-bank retrieval.
#[derive(Debug, Clone)]
pub struct MemoryBankQuery {
    /// Free-text query used for summary/content/tag matching.
    pub text: Option<String>,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Optional memory-kind restrictions.
    pub kinds: Vec<MemoryKind>,
    /// Optional memory-type restrictions.
    pub memory_types: Vec<MemoryType>,
    /// Optional scope restrictions.
    pub scopes: Vec<MemoryScope>,
    /// Optional session filter.
    pub session_id: Option<String>,
    /// Optional task filter.
    pub task_id: Option<String>,
    /// Optional directive filter.
    pub directive_id: Option<String>,
    /// Optional agent filter.
    pub agent_id: Option<String>,
    /// Optional category filter.
    pub category: Option<String>,
    /// Optional tag filter (any match).
    pub tags: Vec<String>,
    /// Optional minimum confidence threshold.
    pub min_confidence: Option<f32>,
}

impl Default for MemoryBankQuery {
    fn default() -> Self {
        Self {
            text: None,
            limit: 10,
            kinds: Vec::new(),
            memory_types: Vec::new(),
            scopes: Vec::new(),
            session_id: None,
            task_id: None,
            directive_id: None,
            agent_id: None,
            category: None,
            tags: Vec::new(),
            min_confidence: None,
        }
    }
}

impl MemoryBankQuery {
    /// Create a query from free text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::default()
        }
    }

    /// Set result limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Restrict to a specific memory scope.
    pub fn with_scope(mut self, scope: MemoryScope) -> Self {
        self.scopes.push(scope);
        self
    }

    /// Restrict to a specific memory type.
    pub fn with_memory_type(mut self, memory_type: MemoryType) -> Self {
        self.memory_types.push(memory_type);
        self
    }

    /// Restrict to a specific session.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Restrict to a specific task.
    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Restrict to a specific directive.
    pub fn with_directive(mut self, directive_id: impl Into<String>) -> Self {
        self.directive_id = Some(directive_id.into());
        self
    }

    /// Restrict to a specific agent.
    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Restrict to a specific category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Require at least one of the supplied tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Require a minimum confidence value.
    pub fn with_min_confidence(mut self, confidence: f32) -> Self {
        self.min_confidence = Some(confidence);
        self
    }
}

/// Ranked result from targeted memory-bank retrieval.
#[derive(Debug, Clone)]
pub struct MemorySearchResult {
    /// Matching memory entry.
    pub entry: MemoryBankEntry,
    /// Ranking score (higher is better).
    pub score: f32,
    /// Fields that contributed to the match.
    pub matched_fields: Vec<String>,
}

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
    /// Retention domain for the memory.
    #[serde(default)]
    pub memory_kind: MemoryKind,
    /// Typed classification of the memory entry.
    #[serde(default)]
    pub memory_type: MemoryType,
    /// Retrieval scope for the memory entry.
    #[serde(default)]
    pub scope: MemoryScope,
    /// Session ID that created this entry (used for grouping related conversations)
    pub session_id: String,
    /// Optional category for grouping/filtering entries (e.g., "project", "personal", "research")
    pub category: Option<String>,
    /// Optional task identifier associated with the memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Optional higher-level directive identifier associated with the memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_id: Option<String>,
    /// Optional agent identifier that produced or owns the memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Tags used for targeted retrieval.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional originating short-term session when this record was promoted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_from_session_id: Option<String>,
    /// Optional explanation of why the record was promoted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_reason: Option<String>,
    /// Confidence score for retrieval/ranking.
    #[serde(default = "default_memory_confidence")]
    pub confidence: f32,
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
            && self.memory_kind == other.memory_kind
            && self.memory_type == other.memory_type
            && self.scope == other.scope
            && self.session_id == other.session_id
            && self.category == other.category
            && self.task_id == other.task_id
            && self.directive_id == other.directive_id
            && self.agent_id == other.agent_id
            && self.tags == other.tags
            && self.promoted_from_session_id == other.promoted_from_session_id
            && self.promotion_reason == other.promotion_reason
            && (self.confidence - other.confidence).abs() < f32::EPSILON
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
            memory_kind: MemoryKind::LongTerm,
            memory_type: MemoryType::Episodic,
            scope: MemoryScope::Session,
            session_id,
            category: None,
            task_id: None,
            directive_id: None,
            agent_id: None,
            tags: Vec::new(),
            promoted_from_session_id: None,
            promotion_reason: None,
            confidence: default_memory_confidence(),
            summary,
            content,
            file_path: None,
        }
    }

    /// Override the memory type for this entry.
    pub fn with_memory_type(mut self, memory_type: MemoryType) -> Self {
        self.memory_type = memory_type;
        self
    }

    /// Override the memory scope for this entry.
    pub fn with_scope(mut self, scope: MemoryScope) -> Self {
        self.scope = scope;
        self
    }

    /// Attach a category to the entry.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Attach provenance identifiers to the entry.
    pub fn with_provenance(
        mut self,
        task_id: Option<String>,
        directive_id: Option<String>,
        agent_id: Option<String>,
    ) -> Self {
        self.task_id = task_id;
        self.directive_id = directive_id;
        self.agent_id = agent_id;
        self
    }

    /// Attach retrieval tags to the entry.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Mark the entry as promoted from short-term memory.
    pub fn with_promotion(
        mut self,
        promoted_from_session_id: impl Into<String>,
        promotion_reason: impl Into<String>,
    ) -> Self {
        self.promoted_from_session_id = Some(promoted_from_session_id.into());
        self.promotion_reason = Some(promotion_reason.into());
        self
    }

    /// Override the retrieval confidence for this entry.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
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
        let task_id_line = self
            .task_id
            .as_deref()
            .map(|value| format!("**Task ID**: {}\n", value))
            .unwrap_or_default();
        let directive_id_line = self
            .directive_id
            .as_deref()
            .map(|value| format!("**Directive ID**: {}\n", value))
            .unwrap_or_default();
        let agent_id_line = self
            .agent_id
            .as_deref()
            .map(|value| format!("**Agent ID**: {}\n", value))
            .unwrap_or_default();
        let tags_line = if self.tags.is_empty() {
            String::new()
        } else {
            format!("**Tags**: {}\n", self.tags.join(", "))
        };
        let promoted_from_line = self
            .promoted_from_session_id
            .as_deref()
            .map(|value| format!("**Promoted From Session ID**: {}\n", value))
            .unwrap_or_default();
        let promotion_reason_line = self
            .promotion_reason
            .as_deref()
            .map(|value| format!("**Promotion Reason**: {}\n", value))
            .unwrap_or_default();
        format!(
            "# Memory Bank Entry\n\n\
             **Timestamp**: {}\n\
             **Memory Kind**: {}\n\
             **Memory Type**: {}\n\
             **Scope**: {}\n\
             **Session ID**: {}\n\
             {}\
             {}\
             {}\
             {}\
             {}\
             {}\
             {}\
             **Confidence**: {:.2}\n\
             **Summary**: {}\n\n\
             ## Context\n\n\
             {}\n",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            self.memory_kind,
            self.memory_type,
            self.scope,
            self.session_id,
            category_line,
            task_id_line,
            directive_id_line,
            agent_id_line,
            tags_line,
            promoted_from_line,
            promotion_reason_line,
            self.confidence,
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
        let mut memory_kind = MemoryKind::LongTerm;
        let mut memory_type = MemoryType::Episodic;
        let mut scope = MemoryScope::Session;
        let mut session_id = None;
        let mut category: Option<String> = None;
        let mut task_id: Option<String> = None;
        let mut directive_id: Option<String> = None;
        let mut agent_id: Option<String> = None;
        let mut tags: Vec<String> = Vec::new();
        let mut promoted_from_session_id: Option<String> = None;
        let mut promotion_reason: Option<String> = None;
        let mut confidence = default_memory_confidence();
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
            } else if line.starts_with("**Memory Kind**:") {
                let value = line.trim_start_matches("**Memory Kind**:").trim();
                if let Ok(parsed) = value.parse() {
                    memory_kind = parsed;
                }
            } else if line.starts_with("**Memory Type**:") {
                let value = line.trim_start_matches("**Memory Type**:").trim();
                if let Ok(parsed) = value.parse() {
                    memory_type = parsed;
                }
            } else if line.starts_with("**Scope**:") {
                let value = line.trim_start_matches("**Scope**:").trim();
                if let Ok(parsed) = value.parse() {
                    scope = parsed;
                }
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
            } else if line.starts_with("**Task ID**:") {
                let v = line.trim_start_matches("**Task ID**:").trim();
                if !v.is_empty() {
                    task_id = Some(v.to_string());
                }
            } else if line.starts_with("**Directive ID**:") {
                let v = line.trim_start_matches("**Directive ID**:").trim();
                if !v.is_empty() {
                    directive_id = Some(v.to_string());
                }
            } else if line.starts_with("**Agent ID**:") {
                let v = line.trim_start_matches("**Agent ID**:").trim();
                if !v.is_empty() {
                    agent_id = Some(v.to_string());
                }
            } else if line.starts_with("**Tags**:") {
                let raw = line.trim_start_matches("**Tags**:").trim();
                if !raw.is_empty() {
                    tags = raw
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string)
                        .collect();
                }
            } else if line.starts_with("**Promoted From Session ID**:") {
                let v = line
                    .trim_start_matches("**Promoted From Session ID**:")
                    .trim();
                if !v.is_empty() {
                    promoted_from_session_id = Some(v.to_string());
                }
            } else if line.starts_with("**Promotion Reason**:") {
                let v = line.trim_start_matches("**Promotion Reason**:").trim();
                if !v.is_empty() {
                    promotion_reason = Some(v.to_string());
                }
            } else if line.starts_with("**Confidence**:") {
                let value = line.trim_start_matches("**Confidence**:").trim();
                if let Ok(parsed) = value.parse::<f32>() {
                    confidence = parsed.clamp(0.0, 1.0);
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
            memory_kind,
            memory_type,
            scope,
            session_id,
            category,
            task_id,
            directive_id,
            agent_id,
            tags,
            promoted_from_session_id,
            promotion_reason,
            confidence,
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

    /// Return true when the entry satisfies the supplied filter.
    pub fn matches_query(&self, query: &MemoryBankQuery) -> bool {
        if !query.kinds.is_empty() && !query.kinds.contains(&self.memory_kind) {
            return false;
        }
        if !query.memory_types.is_empty() && !query.memory_types.contains(&self.memory_type) {
            return false;
        }
        if !query.scopes.is_empty() && !query.scopes.contains(&self.scope) {
            return false;
        }
        if let Some(session_id) = query.session_id.as_deref()
            && self.session_id != session_id
        {
            return false;
        }
        if let Some(task_id) = query.task_id.as_deref()
            && self.task_id.as_deref() != Some(task_id)
        {
            return false;
        }
        if let Some(directive_id) = query.directive_id.as_deref()
            && self.directive_id.as_deref() != Some(directive_id)
        {
            return false;
        }
        if let Some(agent_id) = query.agent_id.as_deref()
            && self.agent_id.as_deref() != Some(agent_id)
        {
            return false;
        }
        if let Some(category) = query.category.as_deref()
            && self.category.as_deref() != Some(category)
        {
            return false;
        }
        if !query.tags.is_empty()
            && !query.tags.iter().any(|tag| {
                self.tags
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(tag))
            })
        {
            return false;
        }
        if let Some(min_confidence) = query.min_confidence
            && self.confidence < min_confidence
        {
            return false;
        }

        let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return true;
        };

        let searchable_text = self.searchable_text();
        let text_lower = text.to_ascii_lowercase();
        if searchable_text.contains(&text_lower) {
            return true;
        }

        let terms = query_terms(text);
        !terms.is_empty() && terms.iter().all(|term| searchable_text.contains(term))
    }

    /// Rank the entry against a targeted query.
    pub fn score_against_query(&self, query: &MemoryBankQuery) -> MemorySearchResult {
        let text = query
            .text
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let terms = query_terms(&text);
        let mut score = self.confidence.clamp(0.0, 1.0) * 2.0;
        let mut matched_fields = Vec::new();

        if !text.is_empty() {
            if self.summary.to_ascii_lowercase().contains(&text) {
                score += 6.0;
                matched_fields.push("summary".to_string());
            }
            if self.content.to_ascii_lowercase().contains(&text) {
                score += 3.0;
                matched_fields.push("content".to_string());
            }
            if self
                .category
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(&text))
            {
                score += 1.5;
                matched_fields.push("category".to_string());
            }
            if self
                .directive_id
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(&text))
            {
                score += 2.5;
                matched_fields.push("directive_id".to_string());
            }
            if self
                .task_id
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(&text))
            {
                score += 2.0;
                matched_fields.push("task_id".to_string());
            }
            if self
                .agent_id
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(&text))
            {
                score += 1.5;
                matched_fields.push("agent_id".to_string());
            }
            if self
                .tags
                .iter()
                .any(|tag| tag.to_ascii_lowercase().contains(&text))
            {
                score += 2.5;
                matched_fields.push("tags".to_string());
            }

            if !terms.is_empty() {
                let summary_lower = self.summary.to_ascii_lowercase();
                let content_lower = self.content.to_ascii_lowercase();
                let category_lower = self
                    .category
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let tags_lower = self
                    .tags
                    .iter()
                    .map(|tag| tag.to_ascii_lowercase())
                    .collect::<Vec<_>>();

                let summary_matches = terms
                    .iter()
                    .filter(|term| summary_lower.contains(term.as_str()))
                    .count();
                let content_matches = terms
                    .iter()
                    .filter(|term| content_lower.contains(term.as_str()))
                    .count();
                let category_matches = terms
                    .iter()
                    .filter(|term| category_lower.contains(term.as_str()))
                    .count();
                let tag_matches = terms
                    .iter()
                    .filter(|term| tags_lower.iter().any(|tag| tag.contains(term.as_str())))
                    .count();

                score += summary_matches as f32 * 1.5;
                score += content_matches as f32 * 0.75;
                score += category_matches as f32 * 0.5;
                score += tag_matches as f32 * 0.75;

                if summary_matches > 0 && !matched_fields.iter().any(|field| field == "summary") {
                    matched_fields.push("summary".to_string());
                }
                if content_matches > 0 && !matched_fields.iter().any(|field| field == "content") {
                    matched_fields.push("content".to_string());
                }
                if category_matches > 0 && !matched_fields.iter().any(|field| field == "category") {
                    matched_fields.push("category".to_string());
                }
                if tag_matches > 0 && !matched_fields.iter().any(|field| field == "tags") {
                    matched_fields.push("tags".to_string());
                }
            }
        }

        score += match self.scope {
            MemoryScope::Directive => 1.0,
            MemoryScope::Repository => 0.9,
            MemoryScope::Workspace => 0.7,
            MemoryScope::Session => 0.5,
            MemoryScope::Task => 0.4,
        };

        score += match self.memory_type {
            MemoryType::Procedural | MemoryType::Semantic => 0.8,
            MemoryType::Reflection => 0.7,
            MemoryType::Decision | MemoryType::Handoff => 0.6,
            MemoryType::Blocker | MemoryType::Resource => 0.4,
            MemoryType::Episodic => 0.3,
        };

        let age_hours = (Utc::now() - self.timestamp).num_hours().max(0) as f32;
        let recency_boost = (72.0 - age_hours.min(72.0)) / 72.0;
        score += recency_boost;

        MemorySearchResult {
            entry: self.clone(),
            score,
            matched_fields,
        }
    }

    fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {}",
            self.summary,
            self.content,
            self.category.as_deref().unwrap_or_default(),
            self.directive_id.as_deref().unwrap_or_default(),
            self.task_id.as_deref().unwrap_or_default(),
            self.agent_id.as_deref().unwrap_or_default(),
            self.promotion_reason.as_deref().unwrap_or_default(),
            self.tags.join(" "),
        )
        .to_ascii_lowercase()
    }

    /// Render a compact section suitable for prompt context.
    pub fn to_prompt_section(&self, preview_chars: usize) -> String {
        let preview = if self.content.chars().count() > preview_chars {
            format!(
                "{}…",
                self.content
                    .chars()
                    .take(preview_chars)
                    .collect::<String>()
                    .trim_end()
            )
        } else {
            self.content.clone()
        };

        let mut header_parts = vec![
            format!("{}", self.scope),
            format!("{}", self.memory_type),
            format!("confidence {:.2}", self.confidence),
        ];
        if let Some(directive_id) = self.directive_id.as_deref() {
            header_parts.push(format!("directive {directive_id}"));
        }
        if let Some(agent_id) = self.agent_id.as_deref() {
            header_parts.push(format!("agent {agent_id}"));
        }

        format!(
            "### Memory Entry ({})\n**Summary**: {}\n**Metadata**: {}\n\n{}\n",
            self.timestamp.format("%Y-%m-%d %H:%M UTC"),
            self.summary,
            header_parts.join(" | "),
            preview
        )
    }
}

fn default_memory_confidence() -> f32 {
    0.70
}

fn query_terms(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() >= 2)
        .map(|term| term.to_ascii_lowercase())
        .collect()
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
/// content fields of all memory bank entries. Results are sorted using the
/// targeted ranking heuristic and then truncated to the requested limit.
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
    let query = MemoryBankQuery::text(query).with_limit(limit);
    let results = search_memory_bank_with_query(workspace_dir, &query).await?;
    Ok(results.into_iter().map(|result| result.entry).collect())
}

/// Search memory-bank entries using structured filters and ranking.
pub async fn search_memory_bank_with_query(
    workspace_dir: &Path,
    query: &MemoryBankQuery,
) -> Result<Vec<MemorySearchResult>, MemoryBankError> {
    let all_entries = list_memory_bank(workspace_dir).await?;
    let mut matching_entries: Vec<MemorySearchResult> = all_entries
        .into_iter()
        .filter(|entry| entry.matches_query(query))
        .map(|entry| entry.score_against_query(query))
        .collect();

    matching_entries.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.entry.timestamp.cmp(&a.entry.timestamp))
    });
    matching_entries.truncate(query.limit.max(1));

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
    async fn test_typed_metadata_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        let entry = MemoryBankEntry::new(
            "session-directive-001".to_string(),
            "Shared implementation directive".to_string(),
            "Use directive-scoped long-term memory for cross-agent coordination.".to_string(),
        )
        .with_memory_type(MemoryType::Procedural)
        .with_scope(MemoryScope::Directive)
        .with_category("workflow")
        .with_provenance(
            Some("task-42".to_string()),
            Some("directive-memory".to_string()),
            Some("supervisor-agent".to_string()),
        )
        .with_tags(vec!["memory".to_string(), "coordination".to_string()])
        .with_promotion(
            "session-short-1",
            "Promoted after reflection because it applies across agents",
        )
        .with_confidence(0.92);

        let file_path = save_to_memory_bank(workspace_path, &entry).await.unwrap();
        let loaded = load_from_memory_bank(&file_path).await.unwrap();

        assert_eq!(loaded.memory_type, MemoryType::Procedural);
        assert_eq!(loaded.scope, MemoryScope::Directive);
        assert_eq!(loaded.directive_id.as_deref(), Some("directive-memory"));
        assert_eq!(loaded.task_id.as_deref(), Some("task-42"));
        assert_eq!(loaded.agent_id.as_deref(), Some("supervisor-agent"));
        assert_eq!(
            loaded.promoted_from_session_id.as_deref(),
            Some("session-short-1")
        );
        assert_eq!(loaded.tags, vec!["memory", "coordination"]);
        assert!((loaded.confidence - 0.92).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_search_memory_bank_with_query_filters_and_ranks() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        let directive_entry = MemoryBankEntry::new(
            "session-a".to_string(),
            "Directive workflow memory policy".to_string(),
            "Share only durable subagent summaries across the directive.".to_string(),
        )
        .with_memory_type(MemoryType::Procedural)
        .with_scope(MemoryScope::Directive)
        .with_provenance(
            Some("task-a".to_string()),
            Some("directive-1".to_string()),
            Some("agent-a".to_string()),
        )
        .with_tags(vec!["memory".to_string(), "policy".to_string()])
        .with_confidence(0.95);
        save_to_memory_bank(workspace_path, &directive_entry)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let session_entry = MemoryBankEntry::new(
            "session-b".to_string(),
            "Local scratch note".to_string(),
            "Temporary debugging note for one task.".to_string(),
        )
        .with_memory_type(MemoryType::Resource)
        .with_scope(MemoryScope::Session)
        .with_tags(vec!["debug".to_string()])
        .with_confidence(0.40);
        save_to_memory_bank(workspace_path, &session_entry)
            .await
            .unwrap();

        let query = MemoryBankQuery::text("directive memory policy")
            .with_limit(5)
            .with_scope(MemoryScope::Directive)
            .with_memory_type(MemoryType::Procedural)
            .with_directive("directive-1")
            .with_min_confidence(0.70);

        let results = search_memory_bank_with_query(workspace_path, &query)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.summary, "Directive workflow memory policy");
        assert!(results[0].score > 0.0);
        assert!(
            results[0]
                .matched_fields
                .iter()
                .any(|field| field == "summary")
        );
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
