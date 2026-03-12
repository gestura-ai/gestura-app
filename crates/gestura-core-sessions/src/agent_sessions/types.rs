//! Core agent session data types.

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

/// Resource kind tracked in session working memory.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemoryResourceKind {
    /// User or assistant message-derived resource.
    Message,
    /// Tool execution result.
    ToolCall,
    /// File path or file content reference.
    File,
    /// Command or terminal output reference.
    Command,
    /// Web/documentation reference.
    Web,
    /// Task or workflow reference.
    Task,
    /// Knowledge or policy reference.
    Knowledge,
    /// Fallback classification.
    Other,
}

/// A resource remembered for the active session only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryResource {
    /// Stable local identifier.
    pub id: String,
    /// Resource classification.
    pub kind: SessionMemoryResourceKind,
    /// Human-readable resource label.
    pub label: String,
    /// Resource value or summary.
    pub value: String,
    /// Origin of the resource.
    pub source: String,
    /// Associated tool call if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// A durable decision made during the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryDecision {
    /// Stable local identifier.
    pub id: String,
    /// Optional external linkage identifier for durable learning records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    /// Short decision summary.
    pub summary: String,
    /// Optional rationale or supporting detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Tags used for targeted retrieval.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Status for a remembered blocker.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionBlockerStatus {
    /// Blocker is currently active.
    Open,
    /// Blocker has been resolved.
    Resolved,
}

/// A blocker tracked in short-term memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryBlocker {
    /// Stable local identifier.
    pub id: String,
    /// Short blocker summary.
    pub summary: String,
    /// Optional supporting detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Current blocker status.
    pub status: SessionBlockerStatus,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Timeline entry kind for short-term memory.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemoryEntryKind {
    /// User request or goal.
    UserGoal,
    /// Assistant-produced synthesis.
    AssistantSummary,
    /// Tool-derived insight.
    ToolInsight,
    /// Handoff or checkpoint note.
    Handoff,
}

/// Source bucket for promoting short-term memory into durable memory.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemoryPromotionSource {
    /// Promoted from a remembered resource.
    Resource,
    /// Promoted from a remembered decision.
    Decision,
    /// Promoted from a remembered blocker.
    Blocker,
    /// Promoted from a timeline entry.
    Timeline,
    /// Promoted from suggested next actions.
    NextAction,
}

/// High-value short-term memory candidate for durable promotion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryPromotionCandidate {
    /// Source bucket for the candidate.
    pub source: SessionMemoryPromotionSource,
    /// Short summary suitable for prompt or durable storage.
    pub summary: String,
    /// Optional detail for richer promotion records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Suggested retrieval tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Relative priority score.
    pub score: f32,
}

/// A compact timeline entry retained in session-local memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryEntry {
    /// Stable local identifier.
    pub id: String,
    /// Entry classification.
    pub kind: SessionMemoryEntryKind,
    /// Short summary used for retrieval.
    pub summary: String,
    /// Optional detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Related tool call if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Session-scoped working memory for the active task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionWorkingMemory {
    /// Optional higher-level directive identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_id: Option<String>,
    /// Optional active task identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_task_id: Option<String>,
    /// Rolling summary of the current work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Suggested next actions for the current session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<String>,
    /// Open questions discovered while working.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_questions: Vec<String>,
    /// Resources gathered for the active session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<SessionMemoryResource>,
    /// Decisions made during the session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<SessionMemoryDecision>,
    /// Known blockers for the session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<SessionMemoryBlocker>,
    /// Compact timeline used for retrieval and handoff generation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<SessionMemoryEntry>,
}

impl SessionWorkingMemory {
    const MAX_RESOURCES: usize = 24;
    const MAX_DECISIONS: usize = 16;
    const MAX_BLOCKERS: usize = 12;
    const MAX_TIMELINE: usize = 24;
    const MAX_NEXT_ACTIONS: usize = 10;
    const MAX_OPEN_QUESTIONS: usize = 10;

    fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn truncate_text(text: &str, max_chars: usize) -> String {
        let trimmed = text.trim();
        if trimmed.chars().count() <= max_chars {
            return trimmed.to_string();
        }

        let truncated: String = trimmed.chars().take(max_chars).collect();
        format!("{}…", truncated.trim_end())
    }

    fn push_bounded<T>(items: &mut Vec<T>, item: T, max_len: usize) {
        items.push(item);
        if items.len() > max_len {
            let excess = items.len() - max_len;
            items.drain(0..excess);
        }
    }

    /// Track a resource for the active session.
    pub fn remember_resource(
        &mut self,
        kind: SessionMemoryResourceKind,
        label: impl Into<String>,
        value: impl Into<String>,
        source: impl Into<String>,
        tool_call_id: Option<String>,
    ) {
        let now = Utc::now();
        let label = label.into();
        let value = Self::truncate_text(&value.into(), 280);
        let source = source.into();

        if let Some(existing) = self.resources.iter_mut().find(|resource| {
            resource.kind == kind
                && resource.label == label
                && resource.tool_call_id == tool_call_id
        }) {
            existing.value = value;
            existing.source = source;
            existing.updated_at = now;
            return;
        }

        Self::push_bounded(
            &mut self.resources,
            SessionMemoryResource {
                id: Self::new_id(),
                kind,
                label,
                value,
                source,
                tool_call_id,
                created_at: now,
                updated_at: now,
            },
            Self::MAX_RESOURCES,
        );
    }

    /// Track a decision for the active session.
    pub fn remember_decision(
        &mut self,
        summary: impl Into<String>,
        rationale: Option<String>,
        tags: Vec<String>,
    ) {
        self.remember_linked_decision(summary, rationale, tags, None);
    }

    /// Track a decision with an optional durable linkage identifier.
    pub fn remember_linked_decision(
        &mut self,
        summary: impl Into<String>,
        rationale: Option<String>,
        tags: Vec<String>,
        reference_id: Option<String>,
    ) {
        let now = Utc::now();
        let summary = Self::truncate_text(&summary.into(), 180);

        if let Some(existing) = self.decisions.iter_mut().find(|decision| {
            reference_id
                .as_ref()
                .zip(decision.reference_id.as_ref())
                .is_some_and(|(expected, current)| expected == current)
                || decision.summary == summary
        }) {
            existing.reference_id = reference_id;
            existing.rationale = rationale;
            existing.tags = tags;
            existing.updated_at = now;
            return;
        }

        Self::push_bounded(
            &mut self.decisions,
            SessionMemoryDecision {
                id: Self::new_id(),
                reference_id,
                summary,
                rationale,
                tags,
                created_at: now,
                updated_at: now,
            },
            Self::MAX_DECISIONS,
        );
    }

    /// Track or refresh a blocker in the active session.
    pub fn remember_blocker(&mut self, summary: impl Into<String>, detail: Option<String>) {
        let now = Utc::now();
        let summary = Self::truncate_text(&summary.into(), 180);

        if let Some(existing) = self
            .blockers
            .iter_mut()
            .find(|blocker| blocker.summary == summary)
        {
            existing.detail = detail;
            existing.status = SessionBlockerStatus::Open;
            existing.updated_at = now;
            return;
        }

        Self::push_bounded(
            &mut self.blockers,
            SessionMemoryBlocker {
                id: Self::new_id(),
                summary,
                detail,
                status: SessionBlockerStatus::Open,
                created_at: now,
                updated_at: now,
            },
            Self::MAX_BLOCKERS,
        );
    }

    /// Resolve a blocker previously tracked in the active session.
    pub fn resolve_blocker(&mut self, summary: &str) {
        if let Some(blocker) = self
            .blockers
            .iter_mut()
            .find(|blocker| blocker.summary == summary)
        {
            blocker.status = SessionBlockerStatus::Resolved;
            blocker.updated_at = Utc::now();
        }
    }

    /// Record a next action for the current session.
    pub fn add_next_action(&mut self, action: impl Into<String>) {
        let action = Self::truncate_text(&action.into(), 160);
        if !action.is_empty() && !self.next_actions.iter().any(|existing| existing == &action) {
            Self::push_bounded(&mut self.next_actions, action, Self::MAX_NEXT_ACTIONS);
        }
    }

    /// Record an open question for the current session.
    pub fn add_open_question(&mut self, question: impl Into<String>) {
        let question = Self::truncate_text(&question.into(), 160);
        if !question.is_empty()
            && !self
                .open_questions
                .iter()
                .any(|existing| existing == &question)
        {
            Self::push_bounded(&mut self.open_questions, question, Self::MAX_OPEN_QUESTIONS);
        }
    }

    /// Track the current user goal.
    pub fn remember_user_goal(&mut self, content: &str) {
        let summary = Self::truncate_text(content, 180);
        if summary.is_empty() {
            return;
        }

        self.summary = Some(summary.clone());
        Self::push_bounded(
            &mut self.timeline,
            SessionMemoryEntry {
                id: Self::new_id(),
                kind: SessionMemoryEntryKind::UserGoal,
                summary,
                detail: None,
                tool_call_id: None,
                created_at: Utc::now(),
            },
            Self::MAX_TIMELINE,
        );
    }

    /// Track an assistant-produced summary for short-term retrieval.
    pub fn remember_assistant_summary(&mut self, content: &str, thinking: Option<&str>) {
        let summary = Self::truncate_text(content, 220);
        if summary.is_empty() {
            return;
        }

        self.summary = Some(summary.clone());
        Self::push_bounded(
            &mut self.timeline,
            SessionMemoryEntry {
                id: Self::new_id(),
                kind: SessionMemoryEntryKind::AssistantSummary,
                summary,
                detail: thinking.map(|value| Self::truncate_text(value, 220)),
                tool_call_id: None,
                created_at: Utc::now(),
            },
            Self::MAX_TIMELINE,
        );
    }

    /// Track a tool call in short-term memory.
    pub fn remember_tool_call(&mut self, call: &SessionToolCall) {
        let summary = format!("{} executed", call.name);
        self.remember_resource(
            SessionMemoryResourceKind::ToolCall,
            call.name.clone(),
            call.result.clone(),
            "tool_call",
            Some(call.id.clone()),
        );
        Self::push_bounded(
            &mut self.timeline,
            SessionMemoryEntry {
                id: Self::new_id(),
                kind: SessionMemoryEntryKind::ToolInsight,
                summary,
                detail: Some(Self::truncate_text(&call.result, 220)),
                tool_call_id: Some(call.id.clone()),
                created_at: Utc::now(),
            },
            Self::MAX_TIMELINE,
        );
    }

    /// Track a tool result message in short-term memory.
    pub fn remember_tool_result(&mut self, tool_call_id: &str, content: &str) {
        self.remember_resource(
            SessionMemoryResourceKind::ToolCall,
            format!("Tool result {tool_call_id}"),
            content,
            "tool_result",
            Some(tool_call_id.to_string()),
        );
    }

    /// Build targeted short-term memory sections for prompt context.
    pub fn relevant_sections(&self, query: &str, limit: usize) -> Vec<String> {
        let query_lower = query.to_ascii_lowercase();
        let mut scored_sections: Vec<(f32, String)> = Vec::new();

        if let Some(summary) = &self.summary {
            let score = if summary.to_ascii_lowercase().contains(&query_lower) {
                4.0
            } else {
                1.5
            };
            scored_sections.push((score, format!("Working summary: {summary}")));
        }

        for resource in &self.resources {
            let haystack = format!("{} {} {}", resource.label, resource.value, resource.source)
                .to_ascii_lowercase();
            let score = if query_lower.is_empty() || haystack.contains(&query_lower) {
                3.0
            } else {
                0.8
            };
            scored_sections.push((
                score,
                format!(
                    "Resource [{}]: {} => {}",
                    resource.source, resource.label, resource.value
                ),
            ));
        }

        for decision in &self.decisions {
            let haystack = format!(
                "{} {} {}",
                decision.summary,
                decision.rationale.clone().unwrap_or_default(),
                decision.tags.join(" ")
            )
            .to_ascii_lowercase();
            let score = if query_lower.is_empty() || haystack.contains(&query_lower) {
                3.8
            } else {
                1.0
            };
            let detail = decision
                .rationale
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            scored_sections.push((score, format!("Decision: {}{}", decision.summary, detail)));
        }

        for blocker in &self.blockers {
            let haystack = format!(
                "{} {} {:?}",
                blocker.summary,
                blocker.detail.clone().unwrap_or_default(),
                blocker.status
            )
            .to_ascii_lowercase();
            let score = if query_lower.is_empty() || haystack.contains(&query_lower) {
                3.6
            } else {
                0.9
            };
            let detail = blocker
                .detail
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            scored_sections.push((
                score,
                format!(
                    "Blocker [{:?}]: {}{}",
                    blocker.status, blocker.summary, detail
                ),
            ));
        }

        for entry in &self.timeline {
            let haystack = format!(
                "{} {} {:?}",
                entry.summary,
                entry.detail.clone().unwrap_or_default(),
                entry.kind
            )
            .to_ascii_lowercase();
            let score = if query_lower.is_empty() || haystack.contains(&query_lower) {
                2.8
            } else {
                0.7
            };
            let detail = entry
                .detail
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            scored_sections.push((score, format!("Timeline: {}{}", entry.summary, detail)));
        }

        if !self.next_actions.is_empty() {
            scored_sections.push((
                2.0,
                format!("Next actions: {}", self.next_actions.join("; ")),
            ));
        }

        if !self.open_questions.is_empty() {
            scored_sections.push((
                2.0,
                format!("Open questions: {}", self.open_questions.join("; ")),
            ));
        }

        scored_sections.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored_sections
            .into_iter()
            .take(limit)
            .map(|(_, section)| section)
            .collect()
    }

    /// Select high-value short-term memory items for durable promotion.
    pub fn promotion_candidates(&self, limit: usize) -> Vec<SessionMemoryPromotionCandidate> {
        let mut candidates = Vec::new();

        for decision in &self.decisions {
            let mut tags = decision.tags.clone();
            if tags.is_empty() {
                tags.push("decision".to_string());
            }
            candidates.push(SessionMemoryPromotionCandidate {
                source: SessionMemoryPromotionSource::Decision,
                summary: decision.summary.clone(),
                detail: decision.rationale.clone(),
                tags,
                score: 4.0,
            });
        }

        for blocker in &self.blockers {
            if blocker.status == SessionBlockerStatus::Resolved {
                continue;
            }
            candidates.push(SessionMemoryPromotionCandidate {
                source: SessionMemoryPromotionSource::Blocker,
                summary: blocker.summary.clone(),
                detail: blocker.detail.clone(),
                tags: vec!["blocker".to_string()],
                score: 3.6,
            });
        }

        for resource in &self.resources {
            candidates.push(SessionMemoryPromotionCandidate {
                source: SessionMemoryPromotionSource::Resource,
                summary: format!("{}: {}", resource.label, resource.value),
                detail: Some(format!(
                    "kind={:?}; source={}",
                    resource.kind, resource.source
                )),
                tags: vec![
                    resource.source.clone(),
                    format!("{:?}", resource.kind).to_ascii_lowercase(),
                ],
                score: 3.0,
            });
        }

        for entry in &self.timeline {
            if entry.kind == SessionMemoryEntryKind::AssistantSummary
                || entry.kind == SessionMemoryEntryKind::Handoff
            {
                candidates.push(SessionMemoryPromotionCandidate {
                    source: SessionMemoryPromotionSource::Timeline,
                    summary: entry.summary.clone(),
                    detail: entry.detail.clone(),
                    tags: vec![format!("{:?}", entry.kind).to_ascii_lowercase()],
                    score: 2.4,
                });
            }
        }

        for action in &self.next_actions {
            candidates.push(SessionMemoryPromotionCandidate {
                source: SessionMemoryPromotionSource::NextAction,
                summary: action.clone(),
                detail: None,
                tags: vec!["next_action".to_string()],
                score: 1.8,
            });
        }

        candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        candidates.truncate(limit);
        candidates
    }
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
/// agent session without changing the global `AppConfig.voice` defaults.
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
    /// Short-term working memory for this session.
    #[serde(default)]
    pub working_memory: SessionWorkingMemory,
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
        self.working_memory.remember_user_goal(content);
    }

    /// Add an assistant message.
    pub fn add_assistant_message(&mut self, content: &str, thinking: Option<String>) {
        self.messages.push(ConversationMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking: thinking.clone(),
            timestamp: Utc::now(),
            source: MessageSource::System,
        });
        self.working_memory
            .remember_assistant_summary(content, thinking.as_deref());
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
        self.working_memory
            .remember_tool_result(tool_call_id, content);
    }

    /// Record a tool call.
    pub fn record_tool_call(&mut self, call: SessionToolCall) {
        self.working_memory.remember_tool_call(&call);
        self.tool_calls.push(call);
    }

    /// Add a decision to session working memory.
    pub fn remember_decision(
        &mut self,
        summary: impl Into<String>,
        rationale: Option<String>,
        tags: Vec<String>,
    ) {
        self.working_memory
            .remember_decision(summary, rationale, tags);
    }

    /// Add or refresh a blocker in session working memory.
    pub fn remember_blocker(&mut self, summary: impl Into<String>, detail: Option<String>) {
        self.working_memory.remember_blocker(summary, detail);
    }

    /// Resolve an existing blocker in session working memory.
    pub fn resolve_blocker(&mut self, summary: &str) {
        self.working_memory.resolve_blocker(summary);
    }

    /// Track an explicit resource in session working memory.
    pub fn remember_resource(
        &mut self,
        kind: SessionMemoryResourceKind,
        label: impl Into<String>,
        value: impl Into<String>,
        source: impl Into<String>,
    ) {
        self.working_memory
            .remember_resource(kind, label, value, source, None);
    }

    /// Return prompt-ready short-term memory sections relevant to the query.
    pub fn relevant_working_memory_sections(&self, query: &str, limit: usize) -> Vec<String> {
        self.working_memory.relevant_sections(query, limit)
    }

    /// Return high-value short-term memory candidates for durable promotion.
    pub fn promotion_candidates(&self, limit: usize) -> Vec<SessionMemoryPromotionCandidate> {
        self.working_memory.promotion_candidates(limit)
    }

    /// Convert to pipeline messages.
    pub fn to_pipeline_messages(&self) -> Vec<Message> {
        self.messages
            .iter()
            .map(ConversationMessage::to_pipeline_message)
            .collect()
    }
}

/// A persisted agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
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

impl AgentSession {
    /// Create a new session with an auto-generated sandbox workspace.
    pub fn new_sandbox(model: Option<String>) -> Result<Self, gestura_core_foundation::AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let workspace = crate::session_workspace::SessionWorkspace::create_sandbox(&id)
            .map_err(|e| gestura_core_foundation::AppError::Session(e.to_string()))?;

        Ok(Self {
            title: "New Session".to_string(),
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
            title: "New Session".to_string(),
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
        if self.title == "New Session" {
            self.title = content
                .lines()
                .next()
                .unwrap_or("New Session")
                .trim()
                .chars()
                .take(80)
                .collect();
            if self.title.is_empty() {
                self.title = "New Session".to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_mutations_update_working_memory() {
        let mut state = SessionState::default();

        state.add_user_message("Investigate memory workflow", MessageSource::Text);
        state.add_assistant_message(
            "I will inspect the memory-bank and session modules.",
            Some("Need to compare durable vs session state".to_string()),
        );
        state.record_tool_call(SessionToolCall {
            id: "tool-1".to_string(),
            name: "codebase-retrieval".to_string(),
            arguments: "{}".to_string(),
            result: "Found memory bank implementation".to_string(),
            success: true,
            duration_ms: 12,
            timestamp: Utc::now(),
        });
        state.add_tool_message("tool-1", "Relevant files collected");

        assert!(state.working_memory.summary.is_some());
        assert!(!state.working_memory.timeline.is_empty());
        assert!(!state.working_memory.resources.is_empty());
        assert!(
            state
                .working_memory
                .resources
                .iter()
                .any(|resource| resource.tool_call_id.as_deref() == Some("tool-1"))
        );
    }

    #[test]
    fn relevant_working_memory_sections_prioritize_matches() {
        let mut state = SessionState::default();
        state.remember_decision(
            "Adopt scoped long-term memory",
            Some("Shared memory should remain directive-scoped".to_string()),
            vec!["memory".to_string(), "directive".to_string()],
        );
        state.remember_blocker(
            "Need prompt-budget controls",
            Some("Avoid flooding context with low-value memory".to_string()),
        );
        state.remember_resource(
            SessionMemoryResourceKind::File,
            "memory_bank.rs",
            "Durable memory entry schema",
            "view",
        );

        let sections = state.relevant_working_memory_sections("directive memory", 3);
        assert_eq!(sections.len(), 3);
        assert!(sections.iter().any(|section| section.contains("Decision")));
    }

    #[test]
    fn promotion_candidates_prioritize_decisions_and_blockers() {
        let mut state = SessionState::default();
        state.remember_decision(
            "Adopt directive-scoped durable memory",
            Some("Cross-agent coordination needs shared context".to_string()),
            vec!["memory".to_string(), "directive".to_string()],
        );
        state.remember_blocker(
            "Need reflection gate for promotions",
            Some("Avoid noisy long-term memory".to_string()),
        );

        let candidates = state.promotion_candidates(2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].source, SessionMemoryPromotionSource::Decision);
        assert!(
            candidates
                .iter()
                .any(|candidate| { candidate.source == SessionMemoryPromotionSource::Blocker })
        );
    }

    #[test]
    fn remember_linked_decision_updates_existing_reference() {
        let mut working_memory = SessionWorkingMemory::default();
        working_memory.remember_linked_decision(
            "Reflection: verify assumptions first",
            Some("Initial rationale".to_string()),
            vec!["reflection".to_string()],
            Some("reflection-1".to_string()),
        );
        working_memory.remember_linked_decision(
            "Reflection: verify assumptions first",
            Some("Updated rationale".to_string()),
            vec!["reflection".to_string(), "review_approved".to_string()],
            Some("reflection-1".to_string()),
        );

        assert_eq!(working_memory.decisions.len(), 1);
        assert_eq!(
            working_memory.decisions[0].reference_id.as_deref(),
            Some("reflection-1")
        );
        assert_eq!(
            working_memory.decisions[0].rationale.as_deref(),
            Some("Updated rationale")
        );
        assert!(
            working_memory.decisions[0]
                .tags
                .contains(&"review_approved".to_string())
        );
    }

    #[test]
    fn working_memory_round_trips_through_session_json() {
        let mut session = AgentSession::new_sandbox(None).unwrap();
        session.state.remember_blocker(
            "Need orchestrator handoff summary",
            Some("Subagent outputs should promote selectively".to_string()),
        );

        let json = session.to_pretty_json().unwrap();
        let restored: AgentSession = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.state.working_memory.blockers.len(), 1);
        assert_eq!(
            restored.state.working_memory.blockers[0].summary,
            "Need orchestrator handoff summary"
        );
    }
}
