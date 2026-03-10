//! Context data types shared across domain crates.
//!
//! These are pure data structures used by the pipeline, context management,
//! and other subsystems. They live in foundation so that domain crates
//! (`gestura-core-pipeline`, `gestura-core-tools`, etc.) can reference them
//! without depending on the full `gestura-core` facade.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Categories of context that might be needed for a request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCategory {
    /// File system operations (read, write, edit files)
    FileSystem,
    /// Shell command execution
    Shell,
    /// Git operations
    Git,
    /// Code analysis (symbols, references)
    Code,
    /// Web fetching and search
    Web,
    /// Voice and audio processing
    Voice,
    /// Configuration management
    Config,
    /// Session and history
    Session,
    /// Tool introspection
    Tools,
    /// Agent orchestration
    Agent,
    /// MCP protocol operations
    Mcp,
    /// A2A protocol operations
    A2a,
    /// Task management for current session
    Task,
    /// Screen capture and recording (screenshot, screen_record)
    Screen,
    /// General conversation (no specific tools)
    General,
}

/// Request analysis result - determines what context is needed
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestAnalysis {
    /// Original request text
    pub request: String,
    /// Detected categories needed
    pub categories: HashSet<ContextCategory>,
    /// Specific tools that might be needed
    pub suggested_tools: Vec<String>,
    /// Whether this looks like a tool-related request
    pub needs_tools: bool,
    /// Whether this looks like a follow-up question
    pub is_followup: bool,
    /// Extracted entities (file paths, URLs, etc.)
    pub entities: Vec<ExtractedEntity>,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
}

/// An entity extracted from the request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// Type of entity
    pub entity_type: EntityType,
    /// The extracted value
    pub value: String,
    /// Start position in the original text
    pub start: usize,
    /// End position in the original text
    pub end: usize,
}

/// Types of entities that can be extracted
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    FilePath,
    DirectoryPath,
    Url,
    GitBranch,
    GitCommit,
    Command,
    Symbol,
    Language,
    ErrorMessage,
}

/// Context that has been resolved and cached
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedContext {
    /// Categories included in this context
    pub categories: HashSet<ContextCategory>,
    /// Tool definitions (reduced if not needed)
    pub tools: Vec<ToolContext>,
    /// File contents if loaded
    pub files: Vec<FileContext>,
    /// Retrieved memory sections (short-term and long-term) relevant to the request.
    #[serde(default)]
    pub memory_sections: Vec<String>,
    /// Session history (potentially summarized)
    pub history_summary: Option<String>,
    /// Knowledge items activated
    pub knowledge: Vec<String>,
    /// Total estimated tokens
    pub estimated_tokens: usize,
}

/// Minimal tool context for when tools are needed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    /// Tool name
    pub name: String,
    /// Brief description
    pub description: String,
    /// Whether full schema is included
    pub has_full_schema: bool,
}

impl RequestAnalysis {
    /// Create a new empty analysis
    pub fn new(request: impl Into<String>) -> Self {
        Self {
            request: request.into(),
            categories: HashSet::new(),
            suggested_tools: Vec::new(),
            needs_tools: false,
            is_followup: false,
            entities: Vec::new(),
            confidence: 0.0,
        }
    }

    /// Add a detected category
    pub fn with_category(mut self, category: ContextCategory) -> Self {
        self.categories.insert(category);
        self
    }

    /// Add a suggested tool
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.suggested_tools.push(tool.into());
        self.needs_tools = true;
        self
    }

    /// Add an extracted entity
    pub fn with_entity(mut self, entity: ExtractedEntity) -> Self {
        self.entities.push(entity);
        self
    }
}

/// File context loaded for the request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    /// File path
    pub path: String,
    /// Content (potentially truncated)
    pub content: String,
    /// Whether content was truncated
    pub truncated: bool,
    /// Total lines in file
    pub total_lines: usize,
}
