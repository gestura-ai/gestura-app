//! Pipeline types for unified LLM interaction
//!
//! This module defines the core types used by the AgentPipeline for processing
//! requests through a unified path regardless of input source (text, voice).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::context::{ContextCategory, ResolvedContext};
use crate::llm_provider::TokenUsage;

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
    /// Workspace directory for sandboxed file/shell operations
    /// If None, operations are unrestricted (legacy behavior)
    pub workspace_dir: Option<PathBuf>,
}

/// Source of the request
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
        }
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

    /// Set workspace directory for sandboxed operations
    pub fn with_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.metadata.workspace_dir = Some(workspace.into());
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
