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

/// A durable session activity event captured for replay and export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivityEvent {
    /// Stable event discriminant (for example, `agent-stream-chunk`).
    pub event_type: String,
    /// Optional JSON payload captured at the time of emission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// Timestamp in UTC.
    pub timestamp: DateTime<Utc>,
}

impl SessionActivityEvent {
    /// Create a new activity event stamped with the current time.
    pub fn new(event_type: impl Into<String>, payload: Option<serde_json::Value>) -> Self {
        Self::with_timestamp(event_type, payload, Utc::now())
    }

    /// Create a new activity event with an explicit timestamp.
    pub fn with_timestamp(
        event_type: impl Into<String>,
        payload: Option<serde_json::Value>,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            payload,
            timestamp,
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

/// A synthesized finding retained for the active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryFinding {
    /// Stable local identifier.
    pub id: String,
    /// Short claim or takeaway.
    pub claim: String,
    /// Supporting evidence snippets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    /// Origin of the finding.
    pub source: String,
    /// Confidence score in the range 0.0 to 1.0.
    pub confidence: f32,
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
    /// User-facing runtime narration captured between major steps.
    Narration,
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
    /// Promoted from a synthesized finding.
    Finding,
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
    /// Synthesized findings gathered for the active session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<SessionMemoryFinding>,
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
    const MAX_FINDINGS: usize = 16;
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

    fn collapse_whitespace(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn concise_text(text: &str, max_chars: usize) -> String {
        Self::truncate_text(&Self::collapse_whitespace(text), max_chars)
    }

    fn prettify_label(value: &str) -> String {
        let normalized = value.trim().replace(['_', '-'], " ").to_ascii_lowercase();
        match normalized.as_str() {
            "inprogress" => "in progress".to_string(),
            "notstarted" => "not started".to_string(),
            other => other.to_string(),
        }
    }

    fn argument_value_excerpt(value: &serde_json::Value, max_chars: usize) -> Option<String> {
        match value {
            serde_json::Value::String(text) => {
                let text = Self::concise_text(text, max_chars);
                (!text.is_empty() && text != "None").then_some(text)
            }
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Bool(flag) => Some(flag.to_string()),
            serde_json::Value::Array(items) => {
                let excerpts: Vec<String> = items
                    .iter()
                    .filter_map(|item| Self::argument_value_excerpt(item, 48))
                    .take(3)
                    .collect();
                if excerpts.is_empty() {
                    None
                } else if items.len() > excerpts.len() {
                    Some(format!(
                        "{} (+{} more)",
                        excerpts.join(", "),
                        items.len() - excerpts.len()
                    ))
                } else {
                    Some(excerpts.join(", "))
                }
            }
            serde_json::Value::Object(map) => {
                for key in [
                    "path",
                    "paths",
                    "url",
                    "query",
                    "command",
                    "task_id",
                    "task_ids",
                    "name",
                    "title",
                    "ref",
                    "status",
                    "operation",
                    "text",
                    "prompt",
                ] {
                    if let Some(value) = map.get(key)
                        && let Some(excerpt) = Self::argument_value_excerpt(value, max_chars)
                    {
                        return Some(excerpt);
                    }
                }
                None
            }
            serde_json::Value::Null => None,
        }
    }

    fn extract_argument_focus(arguments: &str) -> Option<String> {
        let trimmed = arguments.trim();
        if trimmed.is_empty() || trimmed == "{}" {
            return None;
        }

        serde_json::from_str::<serde_json::Value>(trimmed)
            .ok()
            .and_then(|value| Self::argument_value_excerpt(&value, 96))
            .or_else(|| {
                let fallback = Self::concise_text(trimmed, 96);
                (!fallback.is_empty()).then_some(fallback)
            })
    }

    fn extract_argument_field(arguments: &str, field: &str) -> Option<String> {
        let value = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
        value
            .get(field)
            .and_then(|value| Self::argument_value_excerpt(value, 96))
    }

    fn finding_confidence_for_narration_stage(stage: Option<&str>) -> f32 {
        match stage.map(|value| value.to_ascii_lowercase()) {
            Some(stage) if stage == "verification" => 0.9,
            Some(stage) if stage == "blocked" => 0.55,
            Some(stage) if stage == "planning" => 0.6,
            Some(stage) if stage == "context" => 0.68,
            Some(stage) if stage == "execution" => 0.74,
            _ => 0.7,
        }
    }

    fn finding_evidence_excerpt(text: &str) -> Option<String> {
        let excerpt = Self::truncate_text(text, 140);
        (!excerpt.is_empty()).then_some(excerpt)
    }

    fn should_remember_tool_call_finding(call: &SessionToolCall) -> bool {
        if !call.success {
            return false;
        }

        match call.name.to_ascii_lowercase().as_str() {
            "web" | "web_search" | "fetch" => true,
            "shell" | "command" => Self::extract_argument_focus(&call.arguments)
                .map(|focus| {
                    let focus = focus.to_ascii_lowercase();
                    [
                        "test", "check", "clippy", "build", "lint", "verify", "validate",
                    ]
                    .iter()
                    .any(|needle| focus.contains(needle))
                })
                .unwrap_or(false),
            _ => false,
        }
    }

    fn tool_call_finding_confidence(call: &SessionToolCall) -> f32 {
        match call.name.to_ascii_lowercase().as_str() {
            "web" | "web_search" | "fetch" => 0.66,
            "shell" | "command" => 0.62,
            _ => 0.58,
        }
    }

    fn tool_call_finding_evidence(call: &SessionToolCall) -> Vec<String> {
        let mut evidence = Vec::new();
        if let Some(focus) = Self::extract_argument_focus(&call.arguments) {
            evidence.push(format!("Observed focus: {focus}"));
        }
        if let Some(excerpt) = Self::finding_evidence_excerpt(&call.result) {
            evidence.push(format!("Observed result excerpt: {excerpt}"));
        }
        evidence.truncate(3);
        evidence
    }

    fn summarize_tool_call(call: &SessionToolCall) -> String {
        let tool_name = call.name.to_ascii_lowercase();
        let focus = Self::extract_argument_focus(&call.arguments);

        let mut summary = if tool_name.contains("task_update_status") {
            let task_id = Self::extract_argument_field(&call.arguments, "task_id")
                .unwrap_or_else(|| "task".to_string());
            let status = Self::extract_argument_field(&call.arguments, "status")
                .map(|value| Self::prettify_label(&value))
                .unwrap_or_else(|| "updated".to_string());
            format!("Updated task {task_id} to {status}")
        } else if tool_name.contains("search") {
            focus
                .map(|value| format!("Searched for {value}"))
                .unwrap_or_else(|| format!("Ran {}", call.name.replace('_', " ")))
        } else if tool_name.contains("read") || tool_name.contains("view") {
            focus
                .map(|value| format!("Inspected {value}"))
                .unwrap_or_else(|| format!("Inspected output from {}", call.name.replace('_', " ")))
        } else if tool_name.contains("write")
            || tool_name.contains("save")
            || tool_name.contains("patch")
            || tool_name.contains("edit")
        {
            focus
                .map(|value| format!("Updated {value}"))
                .unwrap_or_else(|| format!("Updated content with {}", call.name.replace('_', " ")))
        } else if tool_name.contains("fetch") || tool_name == "web" {
            focus
                .map(|value| format!("Fetched {value}"))
                .unwrap_or_else(|| format!("Fetched content with {}", call.name.replace('_', " ")))
        } else if tool_name.contains("shell")
            || tool_name.contains("command")
            || tool_name.contains("terminal")
            || tool_name.contains("process")
        {
            focus
                .map(|value| format!("Ran command {value}"))
                .unwrap_or_else(|| format!("Ran {}", call.name.replace('_', " ")))
        } else if tool_name.contains("task") {
            focus
                .map(|value| format!("Updated task context for {value}"))
                .unwrap_or_else(|| {
                    format!("Updated task state with {}", call.name.replace('_', " "))
                })
        } else {
            focus
                .map(|value| format!("Ran {} for {value}", call.name.replace('_', " ")))
                .unwrap_or_else(|| format!("Ran {}", call.name.replace('_', " ")))
        };

        if !call.success {
            summary.push_str(" (failed)");
        }

        summary
    }

    fn remember_narration_event(&mut self, payload: &serde_json::Value, timestamp: DateTime<Utc>) {
        let Some(payload) = payload.as_object() else {
            return;
        };

        let stage = payload
            .get("stage")
            .and_then(serde_json::Value::as_str)
            .map(Self::prettify_label);
        let title = payload
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(|value| Self::concise_text(value, 96));
        let summary = payload
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .map(|value| Self::concise_text(value, 220));
        let reason = payload
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(|value| Self::concise_text(value, 220));
        let next_step = payload
            .get("next_step")
            .and_then(serde_json::Value::as_str)
            .map(|value| Self::concise_text(value, 160));
        let message = payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(|value| Self::concise_text(value, 220));
        let evidence = payload
            .get("evidence")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|item| Self::concise_text(item, 120))
                    .filter(|item| !item.is_empty())
                    .take(3)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let Some(summary_text) = summary.or(title).or(message) else {
            return;
        };

        if self.timeline.iter().rev().take(3).any(|entry| {
            entry.kind == SessionMemoryEntryKind::Narration && entry.summary == summary_text
        }) {
            return;
        }

        let mut detail_parts = Vec::new();
        if let Some(stage) = stage.as_deref() {
            detail_parts.push(format!("Stage: {stage}"));
        }
        if let Some(reason) = reason.as_deref() {
            detail_parts.push(format!("Why it matters: {reason}"));
        }
        if let Some(next_step) = next_step.as_deref() {
            detail_parts.push(format!("Next: {next_step}"));
            self.add_next_action(next_step.to_string());
        }
        if !evidence.is_empty() {
            detail_parts.push(format!("Evidence: {}", evidence.join(" | ")));
        }

        let detail =
            (!detail_parts.is_empty()).then(|| Self::truncate_text(&detail_parts.join("\n"), 280));

        self.summary = Some(summary_text.clone());
        Self::push_bounded(
            &mut self.timeline,
            SessionMemoryEntry {
                id: Self::new_id(),
                kind: SessionMemoryEntryKind::Narration,
                summary: summary_text.clone(),
                detail,
                tool_call_id: None,
                created_at: timestamp,
            },
            Self::MAX_TIMELINE,
        );

        if !evidence.is_empty() {
            self.remember_finding(
                summary_text.clone(),
                evidence,
                "narration",
                None,
                Self::finding_confidence_for_narration_stage(stage.as_deref()),
            );
        }

        if stage.as_deref() == Some("blocked") {
            self.remember_blocker(summary_text, reason);
        }
    }

    fn remember_task_runtime_state_event(&mut self, payload: &serde_json::Value) {
        let Some(snapshot) = payload
            .get("snapshot")
            .and_then(serde_json::Value::as_object)
        else {
            return;
        };

        let completion_ready = snapshot
            .get("current_task")
            .is_some_and(serde_json::Value::is_null)
            && snapshot
                .get("ready_tasks")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
            && snapshot
                .get("parallel_ready_tasks")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
            && snapshot
                .get("blocked_tasks")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
            && snapshot
                .get("open_tasks")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
            && snapshot
                .get("missing_requirements")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty);

        if completion_ready {
            self.resolve_all_blockers();
        }
    }

    fn trim_formatting_markers(text: &str) -> &str {
        text.trim()
            .trim_start_matches('#')
            .trim()
            .trim_matches(|c: char| matches!(c, '*' | '_' | '`'))
            .trim()
    }

    fn strip_list_prefix(line: &str) -> Option<&str> {
        let trimmed = line.trim();

        for prefix in ["- ", "* ", "+ "] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                return Some(rest.trim());
            }
        }

        let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
        if digit_count > 0 {
            let rest = &trimmed[digit_count..];
            if let Some(without_marker) = rest.strip_prefix(". ") {
                return Some(without_marker.trim());
            }
            if let Some(without_marker) = rest.strip_prefix(") ") {
                return Some(without_marker.trim());
            }
        }

        None
    }

    fn strip_task_checkbox_prefix(line: &str) -> &str {
        for prefix in ["[ ] ", "[x] ", "[X] ", "[-] ", "[/] "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                return rest.trim();
            }
        }

        line.trim()
    }

    fn heading_section(line: &str) -> Option<AssistantMemorySection> {
        let normalized = Self::trim_formatting_markers(line)
            .trim_end_matches(':')
            .trim()
            .to_ascii_lowercase();

        match normalized.as_str() {
            "finding" | "findings" | "key finding" | "key findings" | "research finding"
            | "research findings" | "evidence" => Some(AssistantMemorySection::Finding),
            "decision" | "decisions" => Some(AssistantMemorySection::Decision),
            "blocker" | "blockers" => Some(AssistantMemorySection::Blocker),
            "resolved blocker" | "resolved blockers" | "unblocked" => {
                Some(AssistantMemorySection::ResolvedBlocker)
            }
            "next action" | "next actions" | "next step" | "next steps" => {
                Some(AssistantMemorySection::NextAction)
            }
            "open question" | "open questions" | "question" | "questions" => {
                Some(AssistantMemorySection::OpenQuestion)
            }
            _ => None,
        }
    }

    fn prefixed_section(line: &str) -> Option<(AssistantMemorySection, String)> {
        let normalized = Self::trim_formatting_markers(line);
        let normalized_lower = normalized.to_ascii_lowercase();

        for (prefix, section) in [
            ("finding:", AssistantMemorySection::Finding),
            ("findings:", AssistantMemorySection::Finding),
            ("key finding:", AssistantMemorySection::Finding),
            ("key findings:", AssistantMemorySection::Finding),
            ("research finding:", AssistantMemorySection::Finding),
            ("research findings:", AssistantMemorySection::Finding),
            ("evidence:", AssistantMemorySection::Finding),
            ("decision:", AssistantMemorySection::Decision),
            ("decision -", AssistantMemorySection::Decision),
            ("decision —", AssistantMemorySection::Decision),
            ("blocker:", AssistantMemorySection::Blocker),
            ("blocked:", AssistantMemorySection::Blocker),
            ("blocked by:", AssistantMemorySection::Blocker),
            ("resolved blocker:", AssistantMemorySection::ResolvedBlocker),
            ("unblocked:", AssistantMemorySection::ResolvedBlocker),
            ("next action:", AssistantMemorySection::NextAction),
            ("next actions:", AssistantMemorySection::NextAction),
            ("next step:", AssistantMemorySection::NextAction),
            ("next steps:", AssistantMemorySection::NextAction),
            ("open question:", AssistantMemorySection::OpenQuestion),
            ("open questions:", AssistantMemorySection::OpenQuestion),
            ("question:", AssistantMemorySection::OpenQuestion),
        ] {
            if normalized_lower.starts_with(prefix) {
                let value = normalized[prefix.len()..].trim();
                if !value.is_empty() {
                    return Some((section, value.to_string()));
                }
            }
        }

        None
    }

    fn split_decision_rationale(text: &str) -> (String, Option<String>) {
        let trimmed = text.trim();
        for delimiter in [" because ", " so that "] {
            if let Some((summary, rationale)) = trimmed.split_once(delimiter) {
                let summary = summary.trim();
                let rationale = rationale.trim();
                if !summary.is_empty() && !rationale.is_empty() {
                    return (
                        summary.to_string(),
                        Some(format!("{} {}", delimiter.trim(), rationale)),
                    );
                }
            }
        }

        (trimmed.to_string(), None)
    }

    fn split_blocker_detail(text: &str) -> (String, Option<String>) {
        let trimmed = text.trim();
        for delimiter in [" — ", " – ", " -- "] {
            if let Some((summary, detail)) = trimmed.split_once(delimiter) {
                let summary = summary.trim();
                let detail = detail.trim();
                if !summary.is_empty() && !detail.is_empty() {
                    return (summary.to_string(), Some(detail.to_string()));
                }
            }
        }

        (trimmed.to_string(), None)
    }

    fn split_finding_claim_and_evidence(text: &str) -> (String, Vec<String>) {
        let trimmed = text.trim();
        for delimiter in [" — ", " – ", " -- "] {
            if let Some((claim, evidence)) = trimmed.split_once(delimiter) {
                let claim = claim.trim();
                let evidence = evidence.trim();
                if !claim.is_empty() && !evidence.is_empty() {
                    return (claim.to_string(), vec![evidence.to_string()]);
                }
            }
        }

        (trimmed.to_string(), Vec::new())
    }

    fn store_assistant_memory_item(&mut self, section: AssistantMemorySection, value: &str) {
        let value = Self::truncate_text(value, 220);
        if value.is_empty() {
            return;
        }

        match section {
            AssistantMemorySection::Finding => {
                let (claim, evidence) = Self::split_finding_claim_and_evidence(&value);
                self.remember_finding(claim, evidence, "assistant_summary", None, 0.76);
            }
            AssistantMemorySection::Decision => {
                let (summary, rationale) = Self::split_decision_rationale(&value);
                self.remember_decision(summary, rationale, vec!["assistant_signaled".to_string()]);
            }
            AssistantMemorySection::Blocker => {
                let (summary, detail) = Self::split_blocker_detail(&value);
                self.remember_blocker(summary, detail);
            }
            AssistantMemorySection::ResolvedBlocker => {
                let (summary, _) = Self::split_blocker_detail(&value);
                self.resolve_blocker(&summary);
            }
            AssistantMemorySection::NextAction => self.add_next_action(value),
            AssistantMemorySection::OpenQuestion => self.add_open_question(value),
        }
    }

    fn extract_assistant_memory_signals(&mut self, content: &str) {
        let mut active_section: Option<AssistantMemorySection> = None;
        let mut active_section_paragraph = String::new();

        let flush_active_section_paragraph =
            |working_memory: &mut Self,
             section: Option<AssistantMemorySection>,
             paragraph: &mut String| {
                let Some(section) = section else {
                    paragraph.clear();
                    return;
                };

                let candidate = paragraph.trim();
                if !candidate.is_empty() {
                    working_memory.store_assistant_memory_item(section, candidate);
                }
                paragraph.clear();
            };

        for raw_line in content.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                flush_active_section_paragraph(self, active_section, &mut active_section_paragraph);
                continue;
            }

            if let Some(section) = Self::heading_section(trimmed) {
                flush_active_section_paragraph(self, active_section, &mut active_section_paragraph);
                active_section = Some(section);
                continue;
            }

            let list_or_plain = Self::strip_list_prefix(trimmed).unwrap_or(trimmed);
            let candidate = Self::strip_task_checkbox_prefix(list_or_plain);

            if let Some((section, value)) = Self::prefixed_section(candidate) {
                flush_active_section_paragraph(self, active_section, &mut active_section_paragraph);
                active_section = Some(section);
                self.store_assistant_memory_item(section, &value);
                continue;
            }

            if let Some(section) = active_section
                && Self::strip_list_prefix(trimmed).is_some()
            {
                flush_active_section_paragraph(self, active_section, &mut active_section_paragraph);
                self.store_assistant_memory_item(section, candidate);
                continue;
            }

            if active_section.is_some() {
                if !active_section_paragraph.is_empty() {
                    active_section_paragraph.push(' ');
                }
                active_section_paragraph.push_str(candidate);
                continue;
            }

            flush_active_section_paragraph(self, active_section, &mut active_section_paragraph);
            active_section = None;
        }

        flush_active_section_paragraph(self, active_section, &mut active_section_paragraph);
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

    /// Track a synthesized finding for the active session.
    pub fn remember_finding(
        &mut self,
        claim: impl Into<String>,
        evidence: Vec<String>,
        source: impl Into<String>,
        tool_call_id: Option<String>,
        confidence: f32,
    ) {
        let now = Utc::now();
        let claim = Self::truncate_text(&claim.into(), 220);
        if claim.is_empty() {
            return;
        }

        let evidence = evidence
            .into_iter()
            .filter_map(|item| Self::finding_evidence_excerpt(&item))
            .take(4)
            .collect::<Vec<_>>();
        let source = source.into();
        let confidence = confidence.clamp(0.0, 1.0);
        let tool_call_id_ref = tool_call_id.as_deref();

        if let Some(existing) = self.findings.iter_mut().find(|finding| {
            finding.claim == claim
                || tool_call_id_ref
                    .is_some_and(|call_id| finding.tool_call_id.as_deref() == Some(call_id))
        }) {
            if !evidence.is_empty() {
                existing.evidence = evidence;
            }
            if confidence >= existing.confidence {
                existing.source = source;
            }
            existing.confidence = existing.confidence.max(confidence);
            if existing.tool_call_id.is_none() {
                existing.tool_call_id = tool_call_id;
            }
            existing.updated_at = now;
            return;
        }

        Self::push_bounded(
            &mut self.findings,
            SessionMemoryFinding {
                id: Self::new_id(),
                claim,
                evidence,
                source,
                confidence,
                tool_call_id,
                created_at: now,
                updated_at: now,
            },
            Self::MAX_FINDINGS,
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

    /// Resolve every currently open blocker in the active session.
    pub fn resolve_all_blockers(&mut self) {
        let now = Utc::now();
        for blocker in &mut self.blockers {
            blocker.status = SessionBlockerStatus::Resolved;
            blocker.updated_at = now;
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

        self.extract_assistant_memory_signals(content);
    }

    /// Track a tool call in short-term memory.
    pub fn remember_tool_call(&mut self, call: &SessionToolCall) {
        let summary = Self::summarize_tool_call(call);
        self.remember_resource(
            SessionMemoryResourceKind::ToolCall,
            summary.clone(),
            call.result.clone(),
            "tool_call",
            Some(call.id.clone()),
        );
        Self::push_bounded(
            &mut self.timeline,
            SessionMemoryEntry {
                id: Self::new_id(),
                kind: SessionMemoryEntryKind::ToolInsight,
                summary: summary.clone(),
                detail: Some(Self::truncate_text(&call.result, 220)),
                tool_call_id: Some(call.id.clone()),
                created_at: Utc::now(),
            },
            Self::MAX_TIMELINE,
        );

        if Self::should_remember_tool_call_finding(call) {
            self.remember_finding(
                summary.clone(),
                Self::tool_call_finding_evidence(call),
                call.name.clone(),
                Some(call.id.clone()),
                Self::tool_call_finding_confidence(call),
            );
        }

        let blocker_summary = format!("Tool '{}' failed", call.name);
        if call.success {
            self.resolve_blocker(&blocker_summary);
        } else {
            self.remember_blocker(
                blocker_summary,
                Some(Self::truncate_text(&call.result, 280)),
            );
        }
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

        for finding in &self.findings {
            let haystack = format!(
                "{} {} {} {:.2}",
                finding.claim,
                finding.evidence.join(" "),
                finding.source,
                finding.confidence
            )
            .to_ascii_lowercase();
            let score = if query_lower.is_empty() || haystack.contains(&query_lower) {
                3.4
            } else {
                0.95
            };
            let detail = if finding.evidence.is_empty() {
                format!(
                    " (source={}; confidence={:.2})",
                    finding.source, finding.confidence
                )
            } else {
                format!(
                    " (source={}; confidence={:.2}; evidence={})",
                    finding.source,
                    finding.confidence,
                    finding.evidence.join(" | ")
                )
            };
            scored_sections.push((score, format!("Finding: {}{}", finding.claim, detail)));
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

        for finding in &self.findings {
            let mut tags = vec!["finding".to_string(), finding.source.clone()];
            if finding.tool_call_id.is_some() {
                tags.push("tool_backed".to_string());
            }
            candidates.push(SessionMemoryPromotionCandidate {
                source: SessionMemoryPromotionSource::Finding,
                summary: finding.claim.clone(),
                detail: (!finding.evidence.is_empty()).then(|| {
                    format!(
                        "source={}; confidence={:.2}; evidence={}",
                        finding.source,
                        finding.confidence,
                        finding.evidence.join(" | ")
                    )
                }),
                tags,
                score: 3.3,
            });
        }

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
                || entry.kind == SessionMemoryEntryKind::Narration
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssistantMemorySection {
    Finding,
    Decision,
    Blocker,
    ResolvedBlocker,
    NextAction,
    OpenQuestion,
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

/// Session-scoped experiential reflection override.
///
/// This uses sparse override semantics so sessions inherit the current global
/// reflection default unless they explicitly opt in or out.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionReflectionSettings {
    /// Override whether experiential reflection is enabled for this session.
    ///
    /// - `None` => inherit the current global `AppConfig.pipeline.reflection.enabled`
    /// - `Some(true)` => force reflection on for this session
    /// - `Some(false)` => force reflection off for this session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
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
    /// Replayable session activity timeline used to restore rich UI state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_log: Vec<SessionActivityEvent>,
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
    /// Session-scoped experiential reflection settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_settings: Option<SessionReflectionSettings>,
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
        let timestamp = Utc::now();
        self.messages.push(ConversationMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking: None,
            timestamp,
            source,
        });
        self.activity_log.push(SessionActivityEvent::with_timestamp(
            "session-user-message",
            Some(serde_json::json!({
                "content": content,
                "source": source,
            })),
            timestamp,
        ));
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

    /// Append streamed continuation content to the most recent assistant message.
    ///
    /// Returns `true` when the last conversation entry was an assistant message and
    /// the additional content/thinking was merged into that entry.
    pub fn append_to_last_assistant_message(
        &mut self,
        content: &str,
        thinking: Option<String>,
    ) -> bool {
        let has_content = !content.is_empty();
        let has_thinking = thinking.as_ref().is_some_and(|value| !value.is_empty());
        if !has_content && !has_thinking {
            return false;
        }

        let Some(last_message) = self
            .messages
            .last_mut()
            .filter(|message| message.role == "assistant")
        else {
            return false;
        };

        if has_content {
            last_message.content.push_str(content);
        }

        if let Some(thinking_chunk) = thinking.filter(|value| !value.is_empty()) {
            match &mut last_message.thinking {
                Some(existing) => existing.push_str(&thinking_chunk),
                None => last_message.thinking = Some(thinking_chunk),
            }
        }

        last_message.timestamp = Utc::now();
        self.working_memory
            .remember_assistant_summary(&last_message.content, last_message.thinking.as_deref());
        true
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

    /// Append a replay/export activity event to the session timeline.
    pub fn record_activity_event(
        &mut self,
        event_type: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) {
        let event = SessionActivityEvent::new(event_type, payload);
        if event.event_type == "agent-stream-narration"
            && let Some(payload) = event.payload.as_ref()
        {
            self.working_memory
                .remember_narration_event(payload, event.timestamp);
        } else if event.event_type == "agent-stream-task-state"
            && let Some(payload) = event.payload.as_ref()
        {
            self.working_memory
                .remember_task_runtime_state_event(payload);
        }
        self.activity_log.push(event);
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

    /// Clone this session into a new persisted session with a fresh identity.
    ///
    /// The fork preserves conversation history, model hints, and workspace
    /// association while assigning a new session id and fresh timestamps.
    pub fn fork(&self) -> Self {
        let mut forked = self.clone();
        let now = Utc::now();
        forked.id = uuid::Uuid::new_v4().to_string();
        forked.created_at = now;
        forked.last_active = now;
        forked
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
    use std::time::Duration;

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
        assert_eq!(state.activity_log.len(), 1);
        assert_eq!(state.activity_log[0].event_type, "session-user-message");
        assert!(
            state
                .working_memory
                .resources
                .iter()
                .any(|resource| resource.tool_call_id.as_deref() == Some("tool-1"))
        );
    }

    #[test]
    fn assistant_messages_extract_structured_decisions_blockers_and_actions() {
        let mut state = SessionState::default();

        state.add_assistant_message(
            "Decisions:\n- Keep knowledge injection in the prompt builder because it already owns prompt assembly.\n\nBlockers:\n- Waiting on a reproduction fixture — no failing test case yet.\n\nNext steps:\n- Add a regression test for enabled knowledge injection.\n\nOpen questions:\n- Should working-memory extraction also parse tool-blocked events?",
            None,
        );

        assert_eq!(state.working_memory.decisions.len(), 1);
        assert_eq!(
            state.working_memory.decisions[0].summary,
            "Keep knowledge injection in the prompt builder"
        );
        assert_eq!(
            state.working_memory.decisions[0].rationale.as_deref(),
            Some("because it already owns prompt assembly.")
        );
        assert_eq!(state.working_memory.blockers.len(), 1);
        assert_eq!(
            state.working_memory.blockers[0].summary,
            "Waiting on a reproduction fixture"
        );
        assert_eq!(
            state.working_memory.blockers[0].detail.as_deref(),
            Some("no failing test case yet.")
        );
        assert_eq!(
            state.working_memory.next_actions,
            vec!["Add a regression test for enabled knowledge injection."]
        );
        assert_eq!(
            state.working_memory.open_questions,
            vec!["Should working-memory extraction also parse tool-blocked events?"]
        );
    }

    #[test]
    fn session_fork_assigns_new_identity_and_preserves_history() {
        let workspace_dir =
            std::env::temp_dir().join(format!("gestura-session-fork-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace_dir).unwrap();

        let mut session = AgentSession::new_with_workspace(workspace_dir.clone(), None).unwrap();
        session.title = "Investigate core boundary drift".to_string();
        session.add_user_message("audit CLI session behavior", MessageSource::Text);

        std::thread::sleep(Duration::from_millis(5));
        let forked = session.fork();

        assert_ne!(forked.id, session.id);
        assert_eq!(forked.title, session.title);
        assert_eq!(forked.model, session.model);
        assert_eq!(forked.message_count(), session.message_count());
        assert_eq!(forked.workspace_dir(), session.workspace_dir());
        assert!(forked.created_at >= session.created_at);
        assert!(forked.last_active >= session.last_active);
    }

    #[test]
    fn assistant_messages_extract_findings_and_plaintext_section_body() {
        let mut state = SessionState::default();

        state.add_assistant_message(
            "## Key findings\nThe markdown parser was flattening paragraph lines before rendering them.\n\n### Next steps\nAdd a regression test for preserved line breaks.\n\n### Open questions\nShould narration updates also preserve authored markdown blocks?",
            None,
        );

        assert_eq!(state.working_memory.findings.len(), 1);
        assert_eq!(
            state.working_memory.findings[0].claim,
            "The markdown parser was flattening paragraph lines before rendering them."
        );
        assert_eq!(
            state.working_memory.next_actions,
            vec!["Add a regression test for preserved line breaks."]
        );
        assert_eq!(
            state.working_memory.open_questions,
            vec!["Should narration updates also preserve authored markdown blocks?"]
        );
    }

    #[test]
    fn failed_tool_calls_create_and_resolve_blockers() {
        let mut state = SessionState::default();

        state.record_tool_call(SessionToolCall {
            id: "tool-fail".to_string(),
            name: "cargo-check".to_string(),
            arguments: "{}".to_string(),
            result: "E0432 unresolved import gestura_core::foo".to_string(),
            success: false,
            duration_ms: 45,
            timestamp: Utc::now(),
        });

        assert_eq!(state.working_memory.blockers.len(), 1);
        assert_eq!(
            state.working_memory.blockers[0].summary,
            "Tool 'cargo-check' failed"
        );
        assert_eq!(
            state.working_memory.blockers[0].status,
            SessionBlockerStatus::Open
        );

        state.record_tool_call(SessionToolCall {
            id: "tool-pass".to_string(),
            name: "cargo-check".to_string(),
            arguments: "{}".to_string(),
            result: "Finished dev profile target(s) in 2.1s".to_string(),
            success: true,
            duration_ms: 21,
            timestamp: Utc::now(),
        });

        assert_eq!(state.working_memory.blockers.len(), 1);
        assert_eq!(
            state.working_memory.blockers[0].status,
            SessionBlockerStatus::Resolved
        );
    }

    #[test]
    fn tool_call_timeline_summaries_capture_user_facing_context() {
        let mut state = SessionState::default();

        state.record_tool_call(SessionToolCall {
            id: "tool-search".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "operation": "search",
                "query": "smart home lighting market trends 2025 2026"
            })
            .to_string(),
            result: "Found several market reports".to_string(),
            success: true,
            duration_ms: 18,
            timestamp: Utc::now(),
        });

        let entry = state
            .working_memory
            .timeline
            .last()
            .expect("tool call adds timeline entry");
        assert_eq!(entry.kind, SessionMemoryEntryKind::ToolInsight);
        assert_eq!(
            entry.summary,
            "Searched for smart home lighting market trends 2025 2026"
        );
        assert_eq!(
            state.working_memory.resources[0].label,
            "Searched for smart home lighting market trends 2025 2026"
        );
    }

    #[test]
    fn narration_activity_events_enrich_working_memory() {
        let mut state = SessionState::default();

        state.record_activity_event(
            "agent-stream-narration",
            Some(serde_json::json!({
                "stage": "verification",
                "title": "Locking in the evidence",
                "summary": "I confirmed enough market-growth evidence to draft the SWOT.",
                "reason": "This keeps the final write-up grounded in current market data.",
                "next_step": "Turn the verified research into the final markdown deliverable.",
                "evidence": [
                    "Two recent market reports agree on double-digit CAGR growth.",
                    "Consumer-demand sources reinforce the $30-$80 positioning."
                ]
            })),
        );

        assert_eq!(state.activity_log.len(), 1);
        assert_eq!(state.working_memory.timeline.len(), 1);
        assert_eq!(
            state.working_memory.timeline[0].kind,
            SessionMemoryEntryKind::Narration
        );
        assert_eq!(
            state.working_memory.timeline[0].summary,
            "I confirmed enough market-growth evidence to draft the SWOT."
        );
        assert!(
            state.working_memory.timeline[0]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains(
                    "Next: Turn the verified research into the final markdown deliverable."
                ))
        );
        assert_eq!(
            state.working_memory.next_actions,
            vec!["Turn the verified research into the final markdown deliverable."]
        );
        assert_eq!(state.working_memory.findings.len(), 1);
        assert_eq!(
            state.working_memory.findings[0].claim,
            "I confirmed enough market-growth evidence to draft the SWOT."
        );
        assert_eq!(state.working_memory.findings[0].evidence.len(), 2);
    }

    #[test]
    fn successful_research_tool_calls_capture_structured_findings() {
        let mut state = SessionState::default();

        state.record_tool_call(SessionToolCall {
            id: "tool-search".to_string(),
            name: "web_search".to_string(),
            arguments: r#"{"query":"smart lighting market 2025 consumer drivers"}"#.to_string(),
            result: "Market reports and retail analyses both point to strong consumer demand in smart lighting upgrades.".to_string(),
            success: true,
            duration_ms: 31,
            timestamp: Utc::now(),
        });

        assert_eq!(state.working_memory.findings.len(), 1);
        assert_eq!(
            state.working_memory.findings[0].claim,
            "Searched for smart lighting market 2025 consumer drivers"
        );
        assert!(
            state.working_memory.findings[0]
                .evidence
                .iter()
                .any(|item| item.contains("Observed result excerpt"))
        );
        assert_eq!(
            state
                .promotion_candidates(3)
                .into_iter()
                .find(|candidate| candidate.source == SessionMemoryPromotionSource::Finding)
                .map(|candidate| candidate.summary),
            Some("Searched for smart lighting market 2025 consumer drivers".to_string())
        );
    }

    #[test]
    fn blocked_narration_activity_events_open_blockers() {
        let mut state = SessionState::default();

        state.record_activity_event(
            "agent-stream-narration",
            Some(serde_json::json!({
                "stage": "blocked",
                "summary": "I still need one clean market-size source before finalizing the report.",
                "reason": "The current search results conflict on the 2025 baseline."
            })),
        );

        assert_eq!(state.working_memory.blockers.len(), 1);
        assert_eq!(
            state.working_memory.blockers[0].summary,
            "I still need one clean market-size source before finalizing the report."
        );
        assert_eq!(
            state.working_memory.blockers[0].detail.as_deref(),
            Some("The current search results conflict on the 2025 baseline.")
        );
    }

    #[test]
    fn completion_runtime_snapshot_resolves_open_blockers() {
        let mut state = SessionState::default();

        state.record_activity_event(
            "agent-stream-narration",
            Some(serde_json::json!({
                "stage": "blocked",
                "summary": "I still need one clean market-size source before finalizing the report.",
                "reason": "The current search results conflict on the 2025 baseline."
            })),
        );
        assert_eq!(state.working_memory.blockers.len(), 1);
        assert_eq!(
            state.working_memory.blockers[0].status,
            SessionBlockerStatus::Open
        );

        state.record_activity_event(
            "agent-stream-task-state",
            Some(serde_json::json!({
                "snapshot": {
                    "root_task_id": "root-task",
                    "current_task": null,
                    "ready_tasks": [],
                    "parallel_ready_tasks": [],
                    "blocked_tasks": [],
                    "open_tasks": [],
                    "completed_tasks": [{"id": "verify", "name": "Verify facts", "status": "completed"}],
                    "missing_requirements": [],
                    "status_message": "All tracked work is complete"
                }
            })),
        );

        assert_eq!(
            state.working_memory.blockers[0].status,
            SessionBlockerStatus::Resolved
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

    #[test]
    fn activity_events_are_serialized_for_replay() {
        let mut state = SessionState::default();
        state.record_activity_event(
            "agent-stream-tool-result",
            Some(serde_json::json!({
                "name": "file",
                "success": true,
            })),
        );

        let json = serde_json::to_value(&state).expect("session state serializes");
        let activity_log = json["activity_log"]
            .as_array()
            .expect("activity_log is serialized as an array");

        assert_eq!(activity_log.len(), 1);
        assert_eq!(activity_log[0]["event_type"], "agent-stream-tool-result");
        assert_eq!(activity_log[0]["payload"]["name"], "file");
    }

    #[test]
    fn reflection_settings_round_trip_through_session_json() {
        let mut session = AgentSession::new_sandbox(None).unwrap();
        session.state.reflection_settings = Some(SessionReflectionSettings {
            enabled: Some(false),
        });

        let json = session.to_pretty_json().unwrap();
        let restored: AgentSession = serde_json::from_str(&json).unwrap();

        assert_eq!(
            restored
                .state
                .reflection_settings
                .as_ref()
                .and_then(|settings| settings.enabled),
            Some(false)
        );
    }
}
