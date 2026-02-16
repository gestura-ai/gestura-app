//! Core chat session data types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use gestura_core_pipeline::Message;

/// Source of an end-user message.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageSource {
    /// Text input.
    #[default]
    Text,
    /// Voice input (transcribed).
    Voice,
    /// System-generated (internal).
    System,
}

/// A message in a persisted conversation history.
///
/// This is a superset of `gestura_core_pipeline::Message` with additional UI metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// Message role: "user", "assistant", or "tool".
    pub role: String,
    /// Message content.
    pub content: String,
    /// Tool call ID (for tool messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Thinking content (for extended thinking UIs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Timestamp in UTC.
    pub timestamp: DateTime<Utc>,
    /// Message source.
    #[serde(default)]
    pub source: MessageSource,
}

impl ConversationMessage {
    /// Convert to a pipeline `Message` (dropping UI-only fields).
    pub fn to_pipeline_message(&self) -> Message {
        Message {
            role: self.role.clone(),
            content: self.content.clone(),
            tool_call_id: self.tool_call_id.clone(),
            thinking: self.thinking.clone(),
        }
    }
}

/// Tool call record for session history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToolCall {
    /// Tool call ID.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Tool arguments (JSON string).
    pub arguments: String,
    /// Tool result.
    pub result: String,
    /// Whether the call succeeded.
    pub success: bool,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Timestamp.
    pub timestamp: DateTime<Utc>,
}

/// Session-scoped LLM configuration override.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionLlmConfig {
    /// Override LLM provider for this session (e.g., "openai", "anthropic", "ollama").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Override model for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Session-scoped Voice/STT configuration override.
///
/// The GUI/CLI may allow users to override speech-to-text settings for a single
/// chat session without changing the global `AppConfig.voice` defaults.
///
/// ## Field interpretation
/// - `provider`: STT provider id (currently `"local"`, `"openai"`, or `"none"`).
/// - `model`: Provider-specific model selector.
///   - When the effective provider is `"openai"`, this is an OpenAI model id
///     (e.g. `"whisper-1"`, `"gpt-4o-transcribe"`).
///   - When the effective provider is `"local"`, this is either:
///     - a full filesystem path to a whisper.cpp-compatible model file, or
///     - a filename to be resolved under the configured models directory.
///
/// Empty/whitespace-only strings should be treated as `None` by consumers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionVoiceConfig {
    /// Override STT provider for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Override STT model for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Permission level for tool execution.
///
/// Note: Phase 3 will consolidate this with the core permission/policy model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionPermissionLevel {
    /// Read-only access - no file writes, no shell commands.
    Sandbox,
    /// Ask before write operations.
    #[default]
    Restricted,
    /// Full access.
    Full,
}

impl SessionPermissionLevel {
    /// Convert to the pipeline permission level.
    pub fn to_pipeline(self) -> gestura_core_foundation::PermissionLevel {
        match self {
            Self::Sandbox => gestura_core_foundation::PermissionLevel::Sandbox,
            Self::Restricted => gestura_core_foundation::PermissionLevel::Restricted,
            Self::Full => gestura_core_foundation::PermissionLevel::Full,
        }
    }
}

impl std::fmt::Display for SessionPermissionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sandbox => write!(f, "sandbox"),
            Self::Restricted => write!(f, "restricted"),
            Self::Full => write!(f, "full"),
        }
    }
}

impl std::str::FromStr for SessionPermissionLevel {
    type Err = String;

    /// Parse a permission level (case-insensitive, accepts hyphens/underscores).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let norm = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match norm.as_str() {
            "sandbox" => Ok(Self::Sandbox),
            "restricted" => Ok(Self::Restricted),
            "full" | "full_permissions" => Ok(Self::Full),
            _ => Err(format!(
                "Unknown permission level: '{}'. Expected: sandbox, restricted, full",
                s
            )),
        }
    }
}

/// Session-scoped tool availability settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionToolSettings {
    /// Permission level for this session.
    #[serde(default)]
    pub permission_level: SessionPermissionLevel,
    /// Enabled tools for this session (tool name -> enabled).
    #[serde(default)]
    pub enabled_tools: std::collections::HashMap<String, bool>,
}

// NOTE: `SessionToolSettings::from_global_permissions` and `from_global_config`
// live in the `gestura-core` facade as an extension trait because they depend on
// `AppConfig` / `GlobalPermissionSettings` which remain in core's config module.

/// Persisted session state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    /// Conversation history.
    #[serde(default)]
    pub messages: Vec<ConversationMessage>,
    /// Tool call history.
    #[serde(default)]
    pub tool_calls: Vec<SessionToolCall>,
    /// Total tokens used in this session (best-effort).
    #[serde(default)]
    pub total_tokens: u64,
    /// Last context cache key (for smart context reduction).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_cache_key: Option<String>,
    /// Workspace directory for sandboxed file/shell operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Session-scoped LLM configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_config: Option<SessionLlmConfig>,
    /// Session-scoped voice configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_config: Option<SessionVoiceConfig>,
    /// Session-scoped tool settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_settings: Option<SessionToolSettings>,
    /// Paused execution state for resumable sessions.
    ///
    /// When the user pauses (cancels) a streaming response, the execution state
    /// is captured here so it can be resumed later via `@continue` (CLI) or the
    /// resume button (GUI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_execution: Option<gestura_core_pipeline::types::PausedExecutionState>,
}

impl SessionState {
    /// Create a new session state with a workspace directory.
    pub fn with_workspace(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir: Some(workspace_dir),
            ..Default::default()
        }
    }

    /// Get the most recent messages in the session.
    pub fn get_recent_messages(&self, limit: usize) -> Vec<&ConversationMessage> {
        let start = self.messages.len().saturating_sub(limit);
        self.messages.iter().skip(start).collect()
    }

    /// Add a user message.
    pub fn add_user_message(&mut self, content: &str, source: MessageSource) {
        self.messages.push(ConversationMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking: None,
            timestamp: Utc::now(),
            source,
        });
    }

    /// Add an assistant message.
    pub fn add_assistant_message(&mut self, content: &str, thinking: Option<String>) {
        self.messages.push(ConversationMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking,
            timestamp: Utc::now(),
            source: MessageSource::System,
        });
    }

    /// Add a tool result message.
    pub fn add_tool_message(&mut self, tool_call_id: &str, content: &str) {
        self.messages.push(ConversationMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            tool_call_id: Some(tool_call_id.to_string()),
            thinking: None,
            timestamp: Utc::now(),
            source: MessageSource::System,
        });
    }

    /// Record a tool call.
    pub fn record_tool_call(&mut self, call: SessionToolCall) {
        self.tool_calls.push(call);
    }

    /// Convert to pipeline messages.
    pub fn to_pipeline_messages(&self) -> Vec<Message> {
        self.messages
            .iter()
            .map(ConversationMessage::to_pipeline_message)
            .collect()
    }
}

/// A persisted chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    /// Unique session id.
    pub id: String,
    /// Human-friendly title.
    pub title: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last activity time.
    pub last_active: DateTime<Utc>,
    /// Optional model hint (primarily for CLI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Unified session state (conversation history, tool calls, configs, etc.).
    #[serde(default)]
    pub state: SessionState,
}

impl ChatSession {
    /// Create a new session with an auto-generated sandbox workspace.
    pub fn new_sandbox(model: Option<String>) -> Result<Self, gestura_core_foundation::AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let workspace = crate::session_workspace::SessionWorkspace::create_sandbox(&id)
            .map_err(|e| gestura_core_foundation::AppError::Session(e.to_string()))?;

        Ok(Self {
            title: "New Chat".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            model,
            state: SessionState::with_workspace(workspace.root),
            id,
        })
    }

    /// Create a new session using an existing directory as its workspace.
    pub fn new_with_workspace(
        workspace_dir: PathBuf,
        model: Option<String>,
    ) -> Result<Self, gestura_core_foundation::AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let workspace =
            crate::session_workspace::SessionWorkspace::from_directory(&id, workspace_dir)
                .map_err(|e| gestura_core_foundation::AppError::Session(e.to_string()))?;

        Ok(Self {
            title: "New Chat".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            model,
            state: SessionState::with_workspace(workspace.root),
            id,
        })
    }

    /// Append a user message.
    pub fn add_user_message(&mut self, content: &str, source: MessageSource) {
        self.state.add_user_message(content, source);
        self.last_active = Utc::now();
        if self.title == "New Chat" {
            self.title = content
                .lines()
                .next()
                .unwrap_or("New Chat")
                .trim()
                .chars()
                .take(80)
                .collect();
            if self.title.is_empty() {
                self.title = "New Chat".to_string();
            }
        }
    }

    /// Append an assistant message.
    pub fn add_assistant_message(&mut self, content: &str, thinking: Option<String>) {
        self.state.add_assistant_message(content, thinking);
        self.last_active = Utc::now();
    }

    /// Append a tool result message.
    pub fn add_tool_message(&mut self, tool_call_id: &str, content: &str) {
        self.state.add_tool_message(tool_call_id, content);
        self.last_active = Utc::now();
    }

    /// Return the message count.
    pub fn message_count(&self) -> usize {
        self.state.messages.len()
    }

    /// Return the configured workspace directory.
    pub fn workspace_dir(&self) -> Option<&PathBuf> {
        self.state.workspace_dir.as_ref()
    }

    /// Convert the last `limit` messages into pipeline messages.
    pub fn to_pipeline_messages_limited(&self, limit: usize) -> Vec<Message> {
        let start = self.state.messages.len().saturating_sub(limit);
        self.state.messages[start..]
            .iter()
            .map(ConversationMessage::to_pipeline_message)
            .collect()
    }

    /// Serialize this session as pretty JSON.
    pub fn to_pretty_json(&self) -> Result<String, gestura_core_foundation::AppError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}
