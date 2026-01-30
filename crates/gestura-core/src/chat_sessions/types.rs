//! Core chat session data types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::pipeline::Message;

/// Source of an end-user message.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageSource {
    /// Text input.
    Text,
    /// Voice input (transcribed).
    Voice,
    /// System-generated (internal).
    System,
}

impl Default for MessageSource {
    fn default() -> Self {
        Self::Text
    }
}

/// A message in a persisted conversation history.
///
/// This is a superset of `crate::pipeline::Message` with additional UI metadata.
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
    pub fn to_pipeline(self) -> crate::pipeline::PermissionLevel {
        match self {
            Self::Sandbox => crate::pipeline::PermissionLevel::Sandbox,
            Self::Restricted => crate::pipeline::PermissionLevel::Restricted,
            Self::Full => crate::pipeline::PermissionLevel::Full,
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

impl SessionToolSettings {
    /// Create session tool settings from the global permission settings.
    ///
    /// New sessions often inherit their initial permission level and tool enablement
    /// from the global application configuration.
    pub fn from_global_permissions(settings: &crate::config::GlobalPermissionSettings) -> Self {
        use crate::config::GlobalPermissionLevel;

        let permission_level = match settings.default_level {
            GlobalPermissionLevel::Sandbox => SessionPermissionLevel::Sandbox,
            GlobalPermissionLevel::Restricted => SessionPermissionLevel::Restricted,
            GlobalPermissionLevel::Full => SessionPermissionLevel::Full,
        };

        Self {
            permission_level,
            enabled_tools: settings.default_enabled_tools.clone(),
        }
    }

    /// Convenience helper to derive session tool settings from the full app config.
    pub fn from_global_config(config: &crate::config::AppConfig) -> Self {
        Self::from_global_permissions(&config.permissions)
    }
}

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
    ///
    /// This is primarily used for building an LLM context window without
    /// cloning message content. The returned slice is ordered from oldest to
    /// newest within the requested window.
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
    pub fn new_sandbox(model: Option<String>) -> Result<Self, crate::error::AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let workspace = crate::SessionWorkspace::create_sandbox(&id)
            .map_err(|e| crate::error::AppError::Session(e.to_string()))?;

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
    ) -> Result<Self, crate::error::AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let workspace = crate::SessionWorkspace::from_directory(&id, workspace_dir)
            .map_err(|e| crate::error::AppError::Session(e.to_string()))?;

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
            // Derive an initial title from the first user input (best-effort).
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
    pub fn to_pretty_json(&self) -> Result<String, crate::error::AppError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalPermissionLevel, GlobalPermissionSettings};
    use std::collections::HashMap;

    #[test]
    fn session_tool_settings_from_global_permissions_maps_level_and_tools() {
        let mut tools = HashMap::new();
        tools.insert("file".to_string(), true);
        tools.insert("shell".to_string(), false);

        let settings = GlobalPermissionSettings {
            default_level: GlobalPermissionLevel::Sandbox,
            default_enabled_tools: tools.clone(),
        };

        let session_tools = SessionToolSettings::from_global_permissions(&settings);
        assert_eq!(
            session_tools.permission_level,
            SessionPermissionLevel::Sandbox
        );
        assert_eq!(session_tools.enabled_tools, tools);
    }
}
