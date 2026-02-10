//! Pipeline types for unified LLM interaction
//!
//! This module defines the core types used by the AgentPipeline for processing
//! requests through a unified path regardless of input source (text, voice).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::context::{ContextCategory, ResolvedContext};
use crate::llm_provider::TokenUsage;

pub use gestura_core_foundation::permissions::PermissionLevel;

/// Strategy for handling context window overflow during auto-compaction.
///
/// Different strategies provide different trade-offs between preserving context,
/// performance, and user control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CompactionStrategy {
    /// Summarize older messages into a condensed form (default)
    /// Preserves semantic meaning while reducing token count
    #[default]
    Summarize,
    /// Truncate oldest messages to fit within limit
    /// Simple but loses information
    Truncate,
    /// Clear all history and start fresh
    /// Most aggressive, loses all context
    Clear,
    /// Prompt user to choose action
    /// Gives user control but requires interaction
    Prompt,
    /// Save context to persistent memory bank file and clear history
    /// Preserves context for future retrieval while freeing tokens
    MemoryBank,
}

impl CompactionStrategy {
    /// Parse compaction strategy from string (case-insensitive)
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "summarize" => Self::Summarize,
            "truncate" => Self::Truncate,
            "clear" => Self::Clear,
            "prompt" => Self::Prompt,
            "memorybank" | "memory_bank" | "memory-bank" => Self::MemoryBank,
            _ => Self::default(),
        }
    }

    /// Get human-readable name for the strategy
    pub fn name(&self) -> &'static str {
        match self {
            Self::Summarize => "Summarize",
            Self::Truncate => "Truncate",
            Self::Clear => "Clear",
            Self::Prompt => "Prompt",
            Self::MemoryBank => "Memory Bank",
        }
    }
}

/// A request to be processed by the agent pipeline
#[derive(Debug, Clone)]
pub struct AgentRequest {
    /// The user's input text (transcribed if from voice)
    pub input: String,
    /// Conversation history (role, content pairs)
    pub history: Vec<Message>,
    /// Optional system prompt override
    pub system_prompt: Option<String>,
    /// Whether to use streaming response
    pub streaming: bool,
    /// Maximum tool execution iterations (default: 10)
    pub max_iterations: usize,
    /// Request metadata
    pub metadata: RequestMetadata,
    /// Optional paused execution state to resume from.
    ///
    /// When set, the pipeline will reconstruct the conversational context from the
    /// paused state and continue the agentic loop from where it left off.
    pub resume_from: Option<PausedExecutionState>,
}

/// A message in conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "user", "assistant", "system", "tool"
    pub role: String,
    /// Message content
    pub content: String,
    /// Optional tool call ID (for tool responses)
    pub tool_call_id: Option<String>,
    /// Optional thinking content (for extended thinking)
    pub thinking: Option<String>,
}

impl AsRef<str> for Message {
    fn as_ref(&self) -> &str {
        &self.content
    }
}

/// Metadata about the request source and context
#[derive(Debug, Clone, Default)]
pub struct RequestMetadata {
    /// Source of the request
    pub source: RequestSource,
    /// Session ID if available
    pub session_id: Option<String>,
    /// User ID if available (for A2A)
    pub user_id: Option<String>,
    /// Additional context hints
    pub hints: HashMap<String, String>,
    /// Allowed tools (if empty, all tools are allowed)
    pub allowed_tools: Vec<String>,

    /// Optional per-request override for whether tools may be executed.
    ///
    /// When `None`, the pipeline uses its default tool behavior (controlled by
    /// [`PipelineConfig::enable_tools`]). When `Some(false)`, tool execution is
    /// disabled for this request even if the pipeline is generally configured to
    /// allow tools.
    ///
    /// This is useful for adapter layers (GUI/CLI) that expose legacy commands
    /// which historically performed a single LLM call without tools.
    pub tools_enabled: Option<bool>,
    /// Workspace directory for sandboxed file/shell operations
    /// If None, operations are unrestricted (legacy behavior)
    pub workspace_dir: Option<PathBuf>,
    /// Session-scoped LLM configuration (for agent awareness)
    pub session_llm_config: Option<SessionLlmInfo>,
    /// Session permission level for tool execution
    pub permission_level: PermissionLevel,
}

/// Session-scoped LLM configuration info (for agent awareness)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionLlmInfo {
    /// Current LLM provider (e.g., "anthropic", "openai", "ollama")
    pub provider: String,
    /// Current model name
    pub model: String,
}

/// Source of the request
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestSource {
    /// Text input from GUI
    GuiText,
    /// Voice input from GUI
    GuiVoice,
    /// Text input from CLI TUI
    CliTui,
    /// Text input from CLI basic mode
    CliBasic,
    /// Delegated task from orchestrator
    Orchestrator,
    /// Unknown/default source
    #[default]
    Unknown,
}

/// Response from the agent pipeline
#[derive(Debug, Clone)]
pub struct AgentResponse {
    /// Final response content
    pub content: String,
    /// Any thinking content (extended thinking)
    pub thinking: Option<String>,
    /// Tool calls that were executed
    pub tool_calls: Vec<ToolCallRecord>,
    /// Token usage statistics
    pub usage: Option<TokenUsage>,
    /// Resolved context that was used
    pub context_used: ResolvedContext,
    /// Whether the response was truncated
    pub truncated: bool,
    /// Number of agentic loop iterations
    pub iterations: usize,
}

/// Record of a tool call execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Unique ID for this tool call
    pub id: String,
    /// Tool name
    pub name: String,
    /// Arguments passed to the tool (JSON string)
    pub arguments: String,
    /// Result from tool execution
    pub result: ToolResult,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResult {
    /// Successful execution with output
    Success(String),
    /// Tool execution failed
    Error(String),
    /// Tool was skipped (permission denied, etc.)
    Skipped(String),
}

/// Captured execution state when an agent session is paused.
///
/// This is saved when the user pauses (cancels) a streaming response and enables
/// the session to be resumed from the same point later. It captures the full
/// conversational context, partial output, and pipeline configuration so the
/// resume path can reconstruct the agent request faithfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PausedExecutionState {
    /// The original user input that initiated the paused request.
    pub original_input: String,
    /// System prompt in effect at pause time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Conversation history *before* the paused request (for context reconstruction).
    pub history: Vec<Message>,
    /// Partial assistant text accumulated before the pause.
    pub partial_content: String,
    /// Partial thinking content accumulated before the pause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_thinking: Option<String>,
    /// Tool calls that were executed in the paused request before the pause.
    #[serde(default)]
    pub completed_tool_calls: Vec<ToolCallRecord>,
    /// Zero-based agentic loop iteration the pipeline was on when paused.
    pub iteration: u32,
    /// Source of the original request.
    pub source: RequestSource,
    /// Session ID associated with this paused state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Workspace directory at pause time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Snapshot of the model/provider that was active at pause time.
    ///
    /// If the model or provider changes between pause and resume, the UI can
    /// warn the user about potential inconsistencies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_snapshot: Option<SessionLlmInfo>,
    /// Timestamp when the execution was paused.
    pub paused_at: DateTime<Utc>,
}

impl PausedExecutionState {
    /// Returns `true` if this paused state has any meaningful content to resume.
    pub fn has_content(&self) -> bool {
        !self.partial_content.is_empty() || !self.completed_tool_calls.is_empty()
    }
}

/// Status of token limit check
#[derive(Debug, Clone)]
pub enum TokenLimitStatus {
    /// Within acceptable limits
    Ok { estimated: usize, limit: usize },
    /// Approaching limit (>90%)
    Warning {
        estimated: usize,
        limit: usize,
        percentage: u8,
    },
    /// Exceeded limit
    Exceeded {
        estimated: usize,
        limit: usize,
        overage: usize,
    },
}

/// Configuration for the pipeline
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Maximum tokens for context (model-dependent)
    pub max_context_tokens: usize,
    /// Maximum output tokens
    pub max_output_tokens: usize,
    /// Enable tool execution
    pub enable_tools: bool,
    /// Maximum agentic loop iterations
    pub max_iterations: usize,
    /// Enable context reduction
    pub enable_context_reduction: bool,
    /// Enable fallback to secondary provider
    pub enable_fallback: bool,
    /// Categories to always include
    pub always_include_categories: Vec<ContextCategory>,
    /// Maximum number of history messages to include in prompt
    /// Older messages are dropped to save tokens
    pub max_history_messages: usize,
    /// Log token usage before/after optimization
    pub log_token_usage: bool,
    /// Auto-compaction threshold (0.0-1.0)
    /// When estimated tokens exceed this percentage of the limit,
    /// automatically trigger context summarization.
    /// Default: 0.8 (80% of limit)
    pub auto_compact_threshold: f64,
    /// Strategy to use when auto-compaction is triggered
    /// Default: CompactionStrategy::Summarize
    pub compaction_strategy: CompactionStrategy,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 128_000, // Default for modern models
            max_output_tokens: 4_096,
            enable_tools: true,
            max_iterations: 10,
            enable_context_reduction: true,
            enable_fallback: true,
            always_include_categories: vec![ContextCategory::General],
            max_history_messages: 10,    // Keep last 10 messages by default
            log_token_usage: true,       // Log token usage for debugging
            auto_compact_threshold: 0.8, // Auto-compact at 80% of token limit
            compaction_strategy: CompactionStrategy::Summarize, // Default to summarization
        }
    }
}

impl PipelineConfig {
    /// Get recommended context tokens for a specific provider
    pub fn context_tokens_for_provider(provider: &str) -> usize {
        match provider {
            "anthropic" => 200_000, // Claude 3.5 Sonnet supports 200k
            "openai" => 128_000,    // GPT-4o supports 128k
            "grok" => 131_072,      // Grok-2 supports 131k
            "ollama" => 32_000,     // Conservative default for local models
            _ => 32_000,            // Conservative default
        }
    }

    /// Create config optimized for a specific provider
    pub fn for_provider(provider: &str) -> Self {
        Self {
            max_context_tokens: Self::context_tokens_for_provider(provider),
            ..Default::default()
        }
    }

    /// Apply user settings from `AppConfig.pipeline` to this configuration.
    ///
    /// This method intelligently merges user preferences with provider-specific
    /// defaults, allowing users to customize behavior while still benefiting from
    /// optimized provider defaults.
    ///
    /// # Merge Strategy
    ///
    /// - `max_history_messages`: Always use user setting
    /// - `auto_compact_threshold`: Always use user setting (converted from percentage)
    /// - `compaction_strategy`: Always use user setting
    /// - `log_token_usage`: Always use user setting
    /// - `max_context_tokens`: Only override if user set non-zero value (0 = keep provider default).
    ///   When the user sets a value, it is **clamped** to the base config's limit (typically the
    ///   provider-optimized default) to avoid exceeding provider/model capabilities.
    ///
    /// # Arguments
    ///
    /// * `settings` - User-configured pipeline settings from `AppConfig`
    ///
    /// # Returns
    ///
    /// Self with user settings applied
    ///
    /// # Examples
    ///
    /// ```
    /// use gestura_core::config::PipelineSettings;
    /// use gestura_core::pipeline::{PipelineConfig, CompactionStrategy};
    ///
    /// // Start with provider defaults
    /// let config = PipelineConfig::for_provider("openai");
    /// assert_eq!(config.max_context_tokens, 128_000);
    ///
    /// // Apply user settings
    /// let mut user_settings = PipelineSettings::default();
    /// user_settings.max_history_messages = 20;
    /// user_settings.compaction_strategy = CompactionStrategy::MemoryBank;
    /// user_settings.max_context_tokens = 0; // Keep provider default
    ///
    /// let config = config.with_user_settings(&user_settings);
    /// assert_eq!(config.max_history_messages, 20);
    /// assert_eq!(config.compaction_strategy, CompactionStrategy::MemoryBank);
    /// assert_eq!(config.max_context_tokens, 128_000); // Provider default preserved
    /// ```
    pub fn with_user_settings(mut self, settings: &crate::config::PipelineSettings) -> Self {
        self.max_history_messages = settings.max_history_messages;
        self.auto_compact_threshold = settings.auto_compact_threshold();
        self.compaction_strategy = settings.compaction_strategy;
        self.log_token_usage = settings.log_token_usage;

        // Only override max_context_tokens if user has set a non-zero value.
        //
        // We clamp to the base config's limit (typically a provider-optimized default) so a
        // user configuration cannot accidentally exceed the provider/model context window.
        if settings.max_context_tokens > 0 {
            self.max_context_tokens = settings.max_context_tokens.min(self.max_context_tokens);
        }

        self
    }
}

impl AgentRequest {
    /// Create a new request with minimal configuration
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            history: Vec::new(),
            system_prompt: None,
            streaming: true,
            max_iterations: 10,
            metadata: RequestMetadata::default(),
            resume_from: None,
        }
    }

    /// Set conversation history
    pub fn with_history(mut self, history: Vec<Message>) -> Self {
        self.history = history;
        self
    }

    /// Set system prompt
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set streaming mode
    pub fn with_streaming(mut self, streaming: bool) -> Self {
        self.streaming = streaming;
        self
    }

    /// Set request source
    pub fn with_source(mut self, source: RequestSource) -> Self {
        self.metadata.source = source;
        self
    }

    /// Set session ID
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.metadata.session_id = Some(session_id.into());
        self
    }

    /// Set allowed tools (for orchestrator/delegated tasks)
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.metadata.allowed_tools = tools;
        self
    }

    /// Enable or disable tool execution for this request.
    ///
    /// Setting this to `false` ensures the pipeline will not attempt to execute
    /// tools (even if the model asks) for this request.
    pub fn with_tools_enabled(mut self, enabled: bool) -> Self {
        self.metadata.tools_enabled = Some(enabled);
        self
    }

    /// Set workspace directory for sandboxed operations
    pub fn with_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.metadata.workspace_dir = Some(workspace.into());
        self
    }

    /// Set session LLM configuration (for agent awareness)
    pub fn with_session_llm_config(
        mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        self.metadata.session_llm_config = Some(SessionLlmInfo {
            provider: provider.into(),
            model: model.into(),
        });
        self
    }

    /// Set session permission level for tool execution
    pub fn with_permission_level(mut self, level: PermissionLevel) -> Self {
        self.metadata.permission_level = level;
        self
    }

    /// Set session permission level from string (for backwards compatibility)
    pub fn with_permission_level_str(mut self, level: &str) -> Self {
        self.metadata.permission_level = PermissionLevel::parse(level);
        self
    }

    /// Attach a paused execution state so the pipeline resumes from that point.
    pub fn with_resume_state(mut self, state: PausedExecutionState) -> Self {
        self.resume_from = Some(state);
        self
    }
}

impl AgentResponse {
    /// Create an empty response
    pub fn empty() -> Self {
        Self {
            content: String::new(),
            thinking: None,
            tool_calls: Vec::new(),
            usage: None,
            context_used: ResolvedContext::default(),
            truncated: false,
            iterations: 0,
        }
    }

    /// Create a response with just content
    pub fn with_content(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Self::empty()
        }
    }

    /// Create an error response
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: format!("Error: {}", message.into()),
            ..Self::empty()
        }
    }
}

impl Message {
    /// Create a user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_call_id: None,
            thinking: None,
        }
    }

    /// Create an assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_call_id: None,
            thinking: None,
        }
    }

    /// Create a system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            tool_call_id: None,
            thinking: None,
        }
    }

    /// Create a tool result message
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            thinking: None,
        }
    }
}
