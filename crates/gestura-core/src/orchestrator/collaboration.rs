//! Durable collaboration records for supervisor/team coordination.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AgentRole, ApprovalActorKind, ApprovalScope, TaskArtifactRecord, TaskResult};

/// Structured team-message category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMessageKind {
    /// General status update.
    StatusUpdate,
    /// Clarification request.
    Clarification,
    /// Blocker notification.
    Blocker,
    /// Handoff summary.
    Handoff,
    /// Review feedback.
    ReviewFeedback,
    /// Approval decision note.
    ApprovalDecision,
    /// Explicit review request.
    ReviewRequest,
    /// Explicit approval request.
    ApprovalRequest,
    /// Explicit test validation request.
    TestValidationRequest,
}

/// Actionable collaboration request type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationRequestKind {
    /// A blocker needs escalation or intervention.
    BlockerEscalation,
    /// Work is ready for another owner to pick up.
    Handoff,
    /// Additional clarification is required.
    Clarification,
    /// Review is required before completion.
    ReviewRequest,
    /// Approval is required before progressing.
    ApprovalRequest,
    /// Explicit test validation is required.
    TestValidationRequest,
}

/// Resolution status for an actionable collaboration request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationActionStatus {
    /// The request is open and awaiting action.
    #[default]
    Open,
    /// Someone has acknowledged the request.
    Acknowledged,
    /// The request has been fully resolved.
    Resolved,
    /// The request was answered with a revision request.
    NeedsRevision,
    /// The request was cancelled or superseded.
    Cancelled,
}

/// Aggregate status for a collaboration thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationThreadStatus {
    /// Informational thread with no open action.
    #[default]
    Active,
    /// Waiting on an explicit action.
    ActionRequired,
    /// Needs revision before it can progress.
    NeedsRevision,
    /// All actionable work in the thread is resolved.
    Resolved,
}

/// Escalation severity for a collaboration thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationEscalationLevel {
    /// Informational escalation.
    Info,
    /// Requires prompt attention.
    Warning,
    /// Requires immediate intervention.
    Critical,
}

/// Retention period before resolved threads are auto-archived.
pub const DEFAULT_RESOLVED_THREAD_RETENTION_DAYS: i64 = 7;

/// Draft payload for creating a new actionable request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamActionRequestDraft {
    /// Request category.
    pub kind: CollaborationRequestKind,
    /// Explicit agent recipients.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_for_agent_ids: Vec<String>,
    /// Role-based recipients.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_for_roles: Vec<AgentRole>,
    /// Approval actor kinds allowed to act.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_for_actor_kinds: Vec<ApprovalActorKind>,
    /// Approval scope when this maps to a gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_scope: Option<ApprovalScope>,
    /// Optional request note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Draft escalation payload for a collaboration message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamEscalationDraft {
    /// Severity level.
    pub level: CollaborationEscalationLevel,
    /// Escalating agent if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalated_by_agent_id: Option<String>,
    /// Optional escalation target role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_role: Option<AgentRole>,
    /// Optional escalation note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Draft payload for creating or replying within a collaboration thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMessageDraft {
    /// Optional task identifier this message refers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Message category.
    pub kind: TeamMessageKind,
    /// Sender agent identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_agent_id: Option<String>,
    /// Recipient agent identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_agent_id: Option<String>,
    /// Human-readable content.
    pub content: String,
    /// Optional target thread id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Optional explicit reply target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Optional actionable request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_request: Option<TeamActionRequestDraft>,
    /// Optional escalation metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<TeamEscalationDraft>,
    /// Optional unread markers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unread_by_agent_ids: Vec<String>,
}

/// Actionable request embedded in a collaboration message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamActionRequest {
    /// Stable request identifier.
    pub id: String,
    /// Request category.
    pub kind: CollaborationRequestKind,
    /// Current action status.
    #[serde(default)]
    pub status: CollaborationActionStatus,
    /// When the request was opened.
    pub requested_at: DateTime<Utc>,
    /// Request author if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by_agent_id: Option<String>,
    /// Explicit agent recipients.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_for_agent_ids: Vec<String>,
    /// Role-based recipients.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_for_roles: Vec<AgentRole>,
    /// Approval actor kinds allowed to act.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_for_actor_kinds: Vec<ApprovalActorKind>,
    /// Approval scope when the request is tied to a gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_scope: Option<ApprovalScope>,
    /// Optional request note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// When the request was resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
    /// Who resolved the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_agent_id: Option<String>,
    /// Optional resolution note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
}

impl TeamActionRequest {
    /// Build a new actionable request.
    pub fn new(
        kind: CollaborationRequestKind,
        requested_by_agent_id: Option<String>,
        note: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            status: CollaborationActionStatus::Open,
            requested_at: Utc::now(),
            requested_by_agent_id,
            requested_for_agent_ids: Vec::new(),
            requested_for_roles: Vec::new(),
            requested_for_actor_kinds: Vec::new(),
            approval_scope: None,
            note,
            resolved_at: None,
            resolved_by_agent_id: None,
            resolution_note: None,
        }
    }

    /// Mark the request resolved or otherwise answered.
    pub fn resolve(
        &mut self,
        status: CollaborationActionStatus,
        resolved_by_agent_id: Option<String>,
        resolution_note: Option<String>,
    ) {
        self.status = status;
        if matches!(
            status,
            CollaborationActionStatus::Resolved
                | CollaborationActionStatus::NeedsRevision
                | CollaborationActionStatus::Cancelled
        ) {
            self.resolved_at = Some(Utc::now());
            self.resolved_by_agent_id = resolved_by_agent_id;
            self.resolution_note = resolution_note;
        } else {
            self.resolved_at = None;
            self.resolved_by_agent_id = resolved_by_agent_id;
            self.resolution_note = resolution_note;
        }
    }

    /// Whether the request still requires attention.
    pub fn requires_attention(&self) -> bool {
        matches!(
            self.status,
            CollaborationActionStatus::Open | CollaborationActionStatus::Acknowledged
        )
    }
}

impl TeamActionRequestDraft {
    /// Convert the draft into a persisted actionable request.
    pub fn into_request(self, requested_by_agent_id: Option<String>) -> TeamActionRequest {
        let mut request = TeamActionRequest::new(self.kind, requested_by_agent_id, self.note);
        request.requested_for_agent_ids = self.requested_for_agent_ids;
        request.requested_for_roles = self.requested_for_roles;
        request.requested_for_actor_kinds = self.requested_for_actor_kinds;
        request.approval_scope = self.approval_scope;
        request
    }
}

/// Structured escalation details for a collaboration message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamEscalation {
    /// Severity level.
    pub level: CollaborationEscalationLevel,
    /// When escalation happened.
    pub escalated_at: DateTime<Utc>,
    /// Escalating agent if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalated_by_agent_id: Option<String>,
    /// Optional escalation target role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_role: Option<AgentRole>,
    /// Optional escalation note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TeamEscalationDraft {
    /// Convert the draft into a persisted escalation record.
    pub fn into_escalation(self) -> TeamEscalation {
        TeamEscalation {
            level: self.level,
            escalated_at: Utc::now(),
            escalated_by_agent_id: self.escalated_by_agent_id,
            target_role: self.target_role,
            note: self.note,
        }
    }
}

/// Artifact reference attached to a collaboration message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamArtifactReference {
    /// Related task if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Artifact name.
    pub name: String,
    /// Artifact category.
    pub kind: String,
    /// Artifact URI/path if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Result reference attached to a collaboration message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamResultReference {
    /// Related task id.
    pub task_id: String,
    /// Whether execution succeeded.
    pub success: bool,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Linked artifact names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_names: Vec<String>,
    /// Execution duration.
    pub duration_ms: u64,
}

/// Message exchanged within a supervisor run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMessage {
    /// Message identifier.
    pub id: String,
    /// Run identifier.
    pub run_id: String,
    /// Optional task identifier this message refers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Message kind.
    pub kind: TeamMessageKind,
    /// Sender agent identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_agent_id: Option<String>,
    /// Recipient agent identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_agent_id: Option<String>,
    /// Human-readable content.
    pub content: String,
    /// Stable thread identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Parent message identifier for replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Actionable request metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_request: Option<TeamActionRequest>,
    /// Escalation metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<TeamEscalation>,
    /// Optional result reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_reference: Option<TeamResultReference>,
    /// Linked artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_references: Vec<TeamArtifactReference>,
    /// Agents that have not yet read the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unread_by_agent_ids: Vec<String>,
    /// When this message/thread was archived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Who archived this message/thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_by_agent_id: Option<String>,
    /// Optional archive note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_note: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl TeamMessage {
    /// Build a new team message.
    pub fn new(
        run_id: impl Into<String>,
        task_id: Option<String>,
        kind: TeamMessageKind,
        sender_agent_id: Option<String>,
        recipient_agent_id: Option<String>,
        content: impl Into<String>,
    ) -> Self {
        let id = Uuid::new_v4().to_string();
        Self {
            id: id.clone(),
            run_id: run_id.into(),
            task_id,
            kind,
            sender_agent_id,
            recipient_agent_id,
            content: content.into(),
            thread_id: Some(id),
            reply_to_message_id: None,
            action_request: None,
            escalation: None,
            result_reference: None,
            artifact_references: Vec::new(),
            unread_by_agent_ids: Vec::new(),
            archived_at: None,
            archived_by_agent_id: None,
            archive_note: None,
            created_at: Utc::now(),
        }
    }

    /// Attach thread metadata.
    pub fn with_thread(mut self, thread_id: String, reply_to_message_id: Option<String>) -> Self {
        self.thread_id = Some(thread_id);
        self.reply_to_message_id = reply_to_message_id;
        self
    }

    /// Attach an action request to the message.
    pub fn with_action_request(mut self, action_request: TeamActionRequest) -> Self {
        self.action_request = Some(action_request);
        self
    }

    /// Attach a result reference to the message.
    pub fn with_result_reference(mut self, result_reference: TeamResultReference) -> Self {
        self.result_reference = Some(result_reference);
        self
    }

    /// Attach artifact references to the message.
    pub fn with_artifact_references(
        mut self,
        artifact_references: Vec<TeamArtifactReference>,
    ) -> Self {
        self.artifact_references = artifact_references;
        self
    }

    /// Attach escalation metadata to the message.
    pub fn with_escalation(mut self, escalation: TeamEscalation) -> Self {
        self.escalation = Some(escalation);
        self
    }

    /// Attach unread markers to the message.
    pub fn with_unread_by_agent_ids(mut self, unread_by_agent_ids: Vec<String>) -> Self {
        self.unread_by_agent_ids = unread_by_agent_ids;
        self
    }

    /// Mark the message as archived.
    pub fn archive(&mut self, archived_by_agent_id: Option<String>, archive_note: Option<String>) {
        self.archived_at = Some(Utc::now());
        self.archived_by_agent_id = archived_by_agent_id;
        self.archive_note = archive_note;
    }

    /// Whether this message is archived.
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// Return the stable thread id for grouping.
    pub fn effective_thread_id(&self) -> &str {
        self.thread_id.as_deref().unwrap_or(self.id.as_str())
    }
}

/// Grouped collaboration thread view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamThread {
    /// Stable thread identifier.
    pub id: String,
    /// Owning run identifier.
    pub run_id: String,
    /// Related task if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Dominant message kind for the thread.
    pub kind: TeamMessageKind,
    /// Current aggregate thread status.
    pub status: CollaborationThreadStatus,
    /// Thread creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
    /// Whether the thread is archived.
    pub archived: bool,
    /// When the thread was archived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Count of unread agents across the thread.
    pub unread_count: usize,
    /// Total message count in the thread.
    pub message_count: usize,
    /// Count of actionable messages in the thread.
    pub actionable_message_count: usize,
    /// Whether the thread still requires attention.
    pub requires_attention: bool,
    /// Participating agent identifiers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participant_agent_ids: Vec<String>,
    /// Latest actionable request in the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_action_request: Option<TeamActionRequest>,
    /// Latest result reference in the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_result_reference: Option<TeamResultReference>,
    /// Unique artifact references for the thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_references: Vec<TeamArtifactReference>,
    /// Thread messages in chronological order.
    pub messages: Vec<TeamMessage>,
}

/// Build grouped threads from flat collaboration messages.
pub fn build_team_threads(messages: &[TeamMessage]) -> Vec<TeamThread> {
    build_team_threads_with_options(messages, false)
}

/// Build grouped threads from flat collaboration messages with archive controls.
pub fn build_team_threads_with_options(
    messages: &[TeamMessage],
    include_archived: bool,
) -> Vec<TeamThread> {
    let mut groups = std::collections::BTreeMap::<String, Vec<TeamMessage>>::new();
    for message in messages {
        groups
            .entry(message.effective_thread_id().to_string())
            .or_default()
            .push(message.clone());
    }

    let mut threads = groups
        .into_iter()
        .filter_map(|(thread_id, mut thread_messages)| {
            thread_messages.sort_by(|left, right| left.created_at.cmp(&right.created_at));
            let first = thread_messages.first()?.clone();
            let last = thread_messages.last()?.clone();
            let latest_action_request = thread_messages
                .iter()
                .rev()
                .find_map(|message| message.action_request.clone());
            let latest_result_reference = thread_messages
                .iter()
                .rev()
                .find_map(|message| message.result_reference.clone());
            let mut artifact_references = Vec::new();
            for message in &thread_messages {
                for artifact in &message.artifact_references {
                    if !artifact_references.contains(artifact) {
                        artifact_references.push(artifact.clone());
                    }
                }
            }
            let mut participant_agent_ids = Vec::new();
            for message in &thread_messages {
                for agent_id in [&message.sender_agent_id, &message.recipient_agent_id]
                    .into_iter()
                    .flatten()
                {
                    if !participant_agent_ids.contains(agent_id) {
                        participant_agent_ids.push(agent_id.clone());
                    }
                }
            }
            let archived = thread_messages.iter().all(TeamMessage::is_archived);
            let archived_at = if archived {
                thread_messages
                    .iter()
                    .filter_map(|message| message.archived_at)
                    .max()
            } else {
                None
            };
            let status = match latest_action_request.as_ref().map(|request| request.status) {
                Some(CollaborationActionStatus::Open | CollaborationActionStatus::Acknowledged) => {
                    CollaborationThreadStatus::ActionRequired
                }
                Some(CollaborationActionStatus::NeedsRevision) => {
                    CollaborationThreadStatus::NeedsRevision
                }
                Some(
                    CollaborationActionStatus::Resolved | CollaborationActionStatus::Cancelled,
                ) => CollaborationThreadStatus::Resolved,
                None => CollaborationThreadStatus::Active,
            };

            if archived && !include_archived {
                return None;
            }

            Some(TeamThread {
                id: thread_id,
                run_id: first.run_id.clone(),
                task_id: last.task_id.clone().or(first.task_id.clone()),
                kind: first.kind,
                status,
                created_at: first.created_at,
                updated_at: last.created_at,
                archived,
                archived_at,
                unread_count: thread_messages
                    .iter()
                    .map(|message| message.unread_by_agent_ids.len())
                    .sum(),
                message_count: thread_messages.len(),
                actionable_message_count: thread_messages
                    .iter()
                    .filter(|message| message.action_request.is_some())
                    .count(),
                requires_attention: latest_action_request
                    .as_ref()
                    .is_some_and(TeamActionRequest::requires_attention),
                participant_agent_ids,
                latest_action_request,
                latest_result_reference,
                artifact_references,
                messages: thread_messages,
            })
        })
        .collect::<Vec<_>>();

    threads.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    threads
}

impl TeamArtifactReference {
    /// Create an artifact reference from a task artifact.
    pub fn from_task_artifact(task_id: Option<String>, artifact: &TaskArtifactRecord) -> Self {
        Self {
            task_id,
            name: artifact.name.clone(),
            kind: artifact.kind.clone(),
            uri: artifact.uri.clone(),
            summary: artifact.summary.clone(),
        }
    }
}

impl TeamResultReference {
    /// Create a collaboration result reference from a task result.
    pub fn from_task_result(result: &TaskResult) -> Self {
        Self {
            task_id: result.task_id.clone(),
            success: result.success,
            summary: result.summary.clone(),
            artifact_names: result
                .artifacts
                .iter()
                .map(|artifact| artifact.name.clone())
                .collect(),
            duration_ms: result.duration_ms,
        }
    }
}

impl TeamMessageDraft {
    /// Build a persisted message from a draft.
    pub fn into_message(self, run_id: impl Into<String>) -> TeamMessage {
        let sender_agent_id = self.sender_agent_id.clone();
        let mut message = TeamMessage::new(
            run_id,
            self.task_id,
            self.kind,
            self.sender_agent_id,
            self.recipient_agent_id,
            self.content,
        );
        if let Some(thread_id) = self.thread_id {
            message = message.with_thread(thread_id, self.reply_to_message_id);
        }
        if let Some(action_request) = self.action_request {
            message = message.with_action_request(action_request.into_request(sender_agent_id));
        }
        if let Some(escalation) = self.escalation {
            message = message.with_escalation(escalation.into_escalation());
        }
        let mut unread_by_agent_ids = self.unread_by_agent_ids;
        if let Some(request) = message.action_request.as_ref() {
            for agent_id in &request.requested_for_agent_ids {
                if !unread_by_agent_ids.contains(agent_id) {
                    unread_by_agent_ids.push(agent_id.clone());
                }
            }
        }
        if !unread_by_agent_ids.is_empty() {
            message = message.with_unread_by_agent_ids(unread_by_agent_ids);
        }
        message
    }
}

/// Archive resolved threads older than the configured retention period.
pub fn archive_resolved_threads(
    messages: &mut [TeamMessage],
    archived_by_agent_id: Option<String>,
    retention_days: i64,
) -> usize {
    let cutoff = Utc::now() - Duration::days(retention_days.max(0));
    let thread_ids = build_team_threads_with_options(messages, true)
        .into_iter()
        .filter(|thread| {
            !thread.archived
                && matches!(thread.status, CollaborationThreadStatus::Resolved)
                && thread.updated_at <= cutoff
        })
        .map(|thread| thread.id)
        .collect::<Vec<_>>();

    let mut archived_count = 0;
    for message in messages.iter_mut() {
        if thread_ids
            .iter()
            .any(|thread_id| thread_id == message.effective_thread_id())
        {
            message.archive(
                archived_by_agent_id.clone(),
                Some("Auto-archived after retention period".to_string()),
            );
            archived_count += 1;
        }
    }
    archived_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_team_message_deserializes_with_defaults() {
        let legacy = serde_json::json!({
            "id": "msg-1",
            "run_id": "run-1",
            "kind": "status_update",
            "content": "Legacy message",
            "created_at": "2026-03-10T00:00:00Z"
        });

        let message: TeamMessage = serde_json::from_value(legacy).unwrap();

        assert_eq!(message.id, "msg-1");
        assert_eq!(message.thread_id, None);
        assert!(message.action_request.is_none());
        assert!(message.artifact_references.is_empty());
        assert!(message.unread_by_agent_ids.is_empty());
    }

    #[test]
    fn build_team_threads_groups_replies_and_tracks_resolution() {
        let mut request = TeamActionRequest::new(
            CollaborationRequestKind::ReviewRequest,
            Some("orchestrator".to_string()),
            Some("Need reviewer sign-off".to_string()),
        );
        request.requested_for_actor_kinds = vec![ApprovalActorKind::Reviewer];
        let root = TeamMessage::new(
            "run-1",
            Some("task-1".to_string()),
            TeamMessageKind::ReviewRequest,
            Some("orchestrator".to_string()),
            None,
            "Review requested",
        )
        .with_action_request(request);
        let reply = TeamMessage::new(
            "run-1",
            Some("task-1".to_string()),
            TeamMessageKind::ApprovalDecision,
            Some("reviewer-1".to_string()),
            Some("agent-1".to_string()),
            "Approved",
        )
        .with_thread(
            root.effective_thread_id().to_string(),
            Some(root.id.clone()),
        );
        let mut resolved_root = root.clone();
        resolved_root.action_request.as_mut().unwrap().resolve(
            CollaborationActionStatus::Resolved,
            Some("reviewer-1".to_string()),
            Some("Looks good".to_string()),
        );

        let threads = build_team_threads(&[resolved_root.clone(), reply.clone()]);

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, resolved_root.effective_thread_id());
        assert_eq!(threads[0].messages.len(), 2);
        assert_eq!(threads[0].status, CollaborationThreadStatus::Resolved);
        assert!(!threads[0].requires_attention);
    }
}
