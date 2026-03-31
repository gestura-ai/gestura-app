//! Subagent orchestration (core-owned, tauri-free).
//!
//! This module coordinates delegated tasks across subagents and executes them via the
//! unified [`crate::pipeline::AgentPipeline`].
//!
//! ## Layering
//! - **gestura-core** owns orchestration policy (permissions, task tracking, execution).
//! - Adapters (GUI/CLI) may attach observers to emit UI events, but must not re-implement
//!   orchestration logic.

mod approval;
mod collaboration;
mod environment;
mod persistence;
mod recovery;

use crate::pipeline::sync_task_reflection_outcomes;
use crate::tasks::{TaskBackgroundJob, TaskBackgroundStatus};
use crate::tools::PermissionManager;
use crate::{
    AgentPipeline, AgentRequest, AppConfig, CancellationToken, PausedExecutionState, RequestSource,
    SessionWorkspace, StreamChunk, ToolCallRecord,
};
use crate::{MemoryBankEntry, MemoryScope, MemoryType};
use crate::{TaskManager, TaskStatus};
use chrono::{DateTime, Utc};
use gestura_core_a2a::{
    A2AClient, A2AMessage, A2ATask, Artifact as RemoteArtifact, ArtifactManifestEntry,
    CreateTaskRequest, MessagePart, RemoteTaskContract, RemoteTaskLease, RemoteTaskLeaseRequest,
    RemoteTaskProgress as A2ARemoteTaskProgress, TaskProvenance, TaskStatus as A2ATaskStatus,
};
use gestura_core_foundation::{OutcomeSignal, OutcomeSignalKind};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::{Duration as TokioDuration, sleep};
use tracing::Instrument;
use uuid::Uuid;

use self::persistence::{
    load_persisted_checkpoints, load_persisted_environments, load_persisted_runs,
    persist_checkpoint_to_disk, persist_checkpoint_to_disk_async, persist_environment_to_disk,
    persist_run_to_disk, persist_run_to_disk_async,
};

// Re-export shared task types for convenience and adapter compatibility.
pub use self::approval::{
    ApprovalActor, ApprovalActorKind, ApprovalDecision, ApprovalDecisionKind, ApprovalPolicy,
    ApprovalRequest, ApprovalRequirement, ApprovalScope, ApprovalState, TaskApprovalRecord,
    actor_kind_for_agent_role, default_actor_kind_for_scope,
};
pub use self::collaboration::{
    CollaborationActionStatus, CollaborationEscalationLevel, CollaborationRequestKind,
    CollaborationThreadStatus, DEFAULT_RESOLVED_THREAD_RETENTION_DAYS, TeamActionRequest,
    TeamActionRequestDraft, TeamArtifactReference, TeamEscalation, TeamEscalationDraft,
    TeamMessage, TeamMessageDraft, TeamMessageKind, TeamResultReference, TeamThread,
    archive_resolved_threads, build_team_threads, build_team_threads_with_options,
};
pub use crate::agents::{
    AgentExecutionMode, AgentInfo, AgentRole, AgentSpawnRequest, AgentSpawner, DelegatedTask,
    DelegationBrief, OrchestratorToolCall, RemoteAgentTarget, TaskArtifactRecord, TaskResult,
    TaskTerminalStateHint,
};

/// Execution state for a task managed by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorTaskState {
    /// Accepted but not yet running.
    Queued,
    /// Waiting on dependencies.
    Blocked,
    /// Waiting on approval before execution.
    PendingApproval,
    /// Currently executing.
    Running,
    /// Waiting for review approval after execution.
    ReviewPending,
    /// Waiting for test validation after execution.
    TestPending,
    /// Completed successfully.
    Completed,
    /// Failed execution or gating.
    Failed,
    /// Cancelled before completion.
    Cancelled,
}

/// Persisted lifecycle stage for a delegated-task checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedCheckpointStage {
    /// Task has been queued but not yet dispatched.
    Queued,
    /// Task has been dispatched and has a restart-safe boundary before execution.
    Dispatched,
    /// Task is actively executing.
    Running,
    /// Task completed successfully and the result was published.
    Completed,
    /// Task failed and the terminal failure was published.
    Failed,
    /// Task was cancelled and the terminal state was published.
    Cancelled,
    /// Task is blocked and requires operator/supervisor action.
    Blocked,
}

/// Replay-safety class used during delegated-task restart reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedReplaySafety {
    /// Purely read-only work that may be replayed safely.
    PureReadonly,
    /// Write work that is only safe to replay with explicit idempotency guarantees.
    IdempotentWrite,
    /// Work that should continue from a saved checkpoint rather than replaying.
    CheckpointResumable,
    /// Work whose replay safety is ambiguous and must be operator-gated.
    OperatorGated,
    /// Work that must never be auto-replayed after restart.
    NonReplayableSideEffect,
}

/// Recovery disposition for a delegated-task checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedResumeDisposition {
    /// Resume from a checkpoint boundary.
    ResumeFromCheckpoint,
    /// Restart from a replay-safe boundary.
    RestartFromBoundary,
    /// Require explicit operator action before retrying.
    OperatorInterventionRequired,
    /// No resume action is applicable because the task is terminal.
    NotApplicable,
}

/// Operator actions exposed for delegated-task checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedCheckpointAction {
    /// Resume execution from the last persisted checkpoint boundary.
    ResumeFromCheckpoint,
    /// Clear any saved resume state and restart the task from scratch.
    RestartFromScratch,
    /// Leave the task blocked but record that an operator acknowledged it.
    AcknowledgeBlocked,
}

/// Durable checkpoint record for delegated-task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedTaskCheckpoint {
    /// Stable checkpoint identifier.
    pub id: String,
    /// Owning delegated task id.
    pub task_id: String,
    /// Owning supervisor run id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Owning session id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Agent executing the delegated task.
    pub agent_id: String,
    /// Execution environment id, if assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    /// Execution mode in effect for the task.
    pub execution_mode: AgentExecutionMode,
    /// Current checkpoint stage.
    pub stage: DelegatedCheckpointStage,
    /// Replay-safety classification.
    pub replay_safety: DelegatedReplaySafety,
    /// Restart/resume disposition.
    pub resume_disposition: DelegatedResumeDisposition,
    /// Human-readable description of the last safe boundary.
    pub safe_boundary_label: String,
    /// Workspace used for the task, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Tool calls completed before or at this checkpoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_tool_calls: Vec<OrchestratorToolCall>,
    /// Whether the terminal task result was published to the supervisor state.
    #[serde(default)]
    pub result_published: bool,
    /// Optional terminal or recovery note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Resumable execution state captured at the last safe boundary, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_state: Option<PausedExecutionState>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Compact checkpoint metadata surfaced on workflow task records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedCheckpointSummary {
    /// Current checkpoint stage.
    pub stage: DelegatedCheckpointStage,
    /// Replay-safety classification.
    pub replay_safety: DelegatedReplaySafety,
    /// Restart/resume disposition.
    pub resume_disposition: DelegatedResumeDisposition,
    /// Human-readable label for the latest safe boundary.
    pub safe_boundary_label: String,
    /// Operator actions currently available for this checkpoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_actions: Vec<DelegatedCheckpointAction>,
    /// Optional operator/recovery note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Number of tool calls captured in the checkpoint history.
    #[serde(default)]
    pub completed_tool_call_count: usize,
    /// Whether resumable pipeline state is available.
    #[serde(default)]
    pub has_resume_state: bool,
    /// Whether a terminal result was already published for this checkpoint.
    #[serde(default)]
    pub result_published: bool,
    /// Last update timestamp for the checkpoint.
    pub updated_at: DateTime<Utc>,
}

/// Aggregate status for a supervisor run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorRunStatus {
    /// Drafting/queued.
    Draft,
    /// Some work is actively running.
    Running,
    /// Waiting on approvals or validation gates.
    Waiting,
    /// All tasks completed successfully.
    Completed,
    /// At least one task failed.
    Failed,
    /// Run was cancelled.
    Cancelled,
}

/// Maximum allowed parent -> child supervisor depth.
pub const MAX_CHILD_SUPERVISOR_DEPTH: u8 = 1;
/// Durable memory-bank category used for run/task shared cognition.
pub const SHARED_COGNITION_CATEGORY: &str = "shared_cognition";
/// Stable tag applied to shared-cognition memory entries.
pub const SHARED_COGNITION_TAG: &str = "shared-cognition";

fn default_max_child_supervisor_depth() -> u8 {
    MAX_CHILD_SUPERVISOR_DEPTH
}

fn workflow_run_memory_tag(run_id: &str) -> String {
    format!("workflow-run:{run_id}")
}

fn push_unique_tag(tags: &mut Vec<String>, tag: impl Into<String>) {
    let tag = tag.into();
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
}

fn summarize_shared_cognition(content: &str) -> String {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 96 {
        return collapsed;
    }

    format!(
        "{}…",
        collapsed.chars().take(95).collect::<String>().trim_end()
    )
}

fn shared_cognition_kind_tag(kind: SharedCognitionKind) -> &'static str {
    match kind {
        SharedCognitionKind::Discovery => "shared-cognition:discovery",
        SharedCognitionKind::Blocker => "shared-cognition:blocker",
        SharedCognitionKind::Hypothesis => "shared-cognition:hypothesis",
        SharedCognitionKind::Steering => "shared-cognition:steering",
        SharedCognitionKind::Decision => "shared-cognition:decision",
        SharedCognitionKind::Handoff => "shared-cognition:handoff",
    }
}

fn shared_cognition_memory_type(kind: SharedCognitionKind) -> MemoryType {
    match kind {
        SharedCognitionKind::Blocker => MemoryType::Blocker,
        SharedCognitionKind::Decision => MemoryType::Decision,
        SharedCognitionKind::Handoff => MemoryType::Handoff,
        SharedCognitionKind::Discovery
        | SharedCognitionKind::Hypothesis
        | SharedCognitionKind::Steering => MemoryType::Procedural,
    }
}

fn shared_cognition_confidence(kind: SharedCognitionKind) -> f32 {
    match kind {
        SharedCognitionKind::Blocker => 0.92,
        SharedCognitionKind::Decision => 0.88,
        SharedCognitionKind::Handoff => 0.86,
        SharedCognitionKind::Steering => 0.82,
        SharedCognitionKind::Discovery => 0.78,
        SharedCognitionKind::Hypothesis => 0.74,
    }
}

fn common_directive_id_for_run(run: &SupervisorRun) -> Option<String> {
    let mut directives = run
        .tasks
        .iter()
        .filter_map(|record| record.task.directive_id.as_deref())
        .collect::<Vec<_>>();
    directives.sort_unstable();
    directives.dedup();
    if directives.len() == 1 {
        Some(directives[0].to_string())
    } else {
        None
    }
}

fn shared_cognition_kind_for_message(
    run: &SupervisorRun,
    message: &TeamMessage,
) -> Option<SharedCognitionKind> {
    match message.kind {
        TeamMessageKind::Blocker => Some(SharedCognitionKind::Blocker),
        TeamMessageKind::Handoff => Some(SharedCognitionKind::Handoff),
        TeamMessageKind::ReviewFeedback | TeamMessageKind::ApprovalDecision => {
            Some(SharedCognitionKind::Decision)
        }
        TeamMessageKind::StatusUpdate => {
            if message.sender_agent_id.as_deref() == run.lead_agent_id.as_deref() {
                Some(SharedCognitionKind::Steering)
            } else {
                Some(SharedCognitionKind::Discovery)
            }
        }
        TeamMessageKind::Clarification => {
            if message.sender_agent_id.as_deref() == run.lead_agent_id.as_deref() {
                Some(SharedCognitionKind::Steering)
            } else {
                Some(SharedCognitionKind::Hypothesis)
            }
        }
        TeamMessageKind::ReviewRequest
        | TeamMessageKind::ApprovalRequest
        | TeamMessageKind::TestValidationRequest => None,
    }
}

fn ensure_shared_memory_tags(task: &mut DelegatedTask, run_id: &str) {
    push_unique_tag(&mut task.memory_tags, SHARED_COGNITION_TAG);
    push_unique_tag(&mut task.memory_tags, workflow_run_memory_tag(run_id));
}

/// Structured shared-cognition note persisted on a supervisor run and projected into durable memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedCognitionKind {
    /// Partial discovery or intermediate finding.
    Discovery,
    /// Blocker or impediment that should influence later work.
    Blocker,
    /// Hypothesis or clarification from an executing agent.
    Hypothesis,
    /// Steering or direction from the supervisor.
    Steering,
    /// Decision or review outcome that changes downstream execution.
    Decision,
    /// Handoff note summarizing what the next task should continue from.
    Handoff,
}

/// Durable run-scoped collaboration memory surfaced to workflows and prompt enrichment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedCognitionNote {
    /// Stable note identifier.
    pub id: String,
    /// Owning supervisor run identifier.
    pub run_id: String,
    /// Related delegated task if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Related directive if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_id: Option<String>,
    /// Shared-cognition classification.
    pub kind: SharedCognitionKind,
    /// Original collaboration message kind that produced this note.
    pub message_kind: TeamMessageKind,
    /// Short summary for operator-facing lists.
    pub summary: String,
    /// Full detail persisted for prompt reuse.
    pub detail: String,
    /// Agent that authored the source message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_agent_id: Option<String>,
    /// Optional intended recipient agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_agent_id: Option<String>,
    /// Retrieval tags propagated into durable memory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Heuristic confidence used for durable retrieval ranking.
    pub confidence: f32,
    /// Source collaboration message identifier.
    pub source_message_id: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Task-state counts for a supervisor run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorRunTaskSummary {
    /// Total tasks in the run.
    pub total: usize,
    /// Queued tasks.
    pub queued: usize,
    /// Blocked tasks.
    pub blocked: usize,
    /// Tasks awaiting pre-execution approval.
    pub pending_approval: usize,
    /// Running tasks.
    pub running: usize,
    /// Tasks awaiting review.
    pub review_pending: usize,
    /// Tasks awaiting test validation.
    pub test_pending: usize,
    /// Completed tasks.
    pub completed: usize,
    /// Failed tasks.
    pub failed: usize,
    /// Cancelled tasks.
    pub cancelled: usize,
}

/// Inherited policy applied to tasks created inside a supervisor run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorInheritancePolicy {
    /// Whether child tasks must require pre-execution approval.
    #[serde(default)]
    pub approval_required: bool,
    /// Whether child tasks must require review.
    #[serde(default)]
    pub reviewer_required: bool,
    /// Whether child tasks must require test validation.
    #[serde(default)]
    pub test_required: bool,
    /// Execution mode enforced for child tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<AgentExecutionMode>,
    /// Workspace root propagated to child tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Memory tags appended to child tasks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_tags: Vec<String>,
    /// Human-readable inherited constraints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraint_notes: Vec<String>,
}

impl SupervisorInheritancePolicy {
    /// Apply inherited policy to a delegated task before it is recorded.
    pub fn apply_to_task(&self, task: &mut DelegatedTask) {
        task.approval_required |= self.approval_required;
        task.reviewer_required |= self.reviewer_required;
        task.test_required |= self.test_required;
        if let Some(execution_mode) = self.execution_mode.clone() {
            task.execution_mode = execution_mode;
        }
        if task.workspace_dir.is_none() {
            task.workspace_dir = self.workspace_dir.clone();
        }
        for tag in &self.memory_tags {
            if !task.memory_tags.contains(tag) {
                task.memory_tags.push(tag.clone());
            }
        }
    }
}

/// Parent run reference stored on child supervisor runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorParentRunRef {
    /// Parent supervisor run identifier.
    pub parent_run_id: String,
    /// Optional parent task that initiated the child run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    /// Delegating actor if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_by_agent_id: Option<String>,
    /// Child-run objective inherited from the delegation request.
    pub objective: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Summary stored on parent runs for each child supervisor run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildSupervisorRunSummary {
    /// Child run identifier.
    pub run_id: String,
    /// Human-readable child run label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Child objective.
    pub objective: String,
    /// Lead child supervisor agent if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_agent_id: Option<String>,
    /// Current child run status.
    pub status: SupervisorRunStatus,
    /// Child task summary.
    #[serde(default)]
    pub task_summary: SupervisorRunTaskSummary,
    /// Whether the child needs attention.
    #[serde(default)]
    pub requires_attention: bool,
    /// Child blocked reasons roll-up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_reasons: Vec<String>,
    /// Child creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Child update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Child completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

/// Roll-up state for a run and its direct children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorHierarchySummary {
    /// Depth of this run in the hierarchy.
    #[serde(default)]
    pub depth: u8,
    /// Maximum supported child depth.
    #[serde(default = "default_max_child_supervisor_depth")]
    pub max_depth: u8,
    /// Number of direct child runs.
    #[serde(default)]
    pub child_run_count: usize,
    /// Total tasks across direct child runs.
    #[serde(default)]
    pub descendant_task_count: usize,
    /// Direct children that currently require attention.
    #[serde(default)]
    pub action_required_child_count: usize,
    /// Roll-up status across this run and its children.
    pub rollup_status: SupervisorRunStatus,
    /// Whether any child run requires attention.
    #[serde(default)]
    pub requires_attention: bool,
    /// Aggregate blocked reasons surfaced from children.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_reasons: Vec<String>,
}

/// Request payload for creating a direct child supervisor run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSupervisorRunRequest {
    /// Parent run identifier.
    pub parent_run_id: String,
    /// Optional explicit child run identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Lead agent id for the child supervisor.
    pub lead_agent_id: String,
    /// Child objective/mission statement.
    pub objective: String,
    /// Optional child run display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional parent task that motivated the child run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    /// Optional session override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional workspace override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Whether child tasks require pre-execution approval by default.
    #[serde(default)]
    pub approval_required: bool,
    /// Whether child tasks require review by default.
    #[serde(default)]
    pub reviewer_required: bool,
    /// Whether child tasks require test validation by default.
    #[serde(default)]
    pub test_required: bool,
    /// Execution mode to inherit into child tasks.
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    /// Memory tags appended to child tasks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_tags: Vec<String>,
    /// Human-readable inherited constraints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraint_notes: Vec<String>,
}

/// Durable environment lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentState {
    /// Environment has been requested but not provisioned yet.
    #[default]
    Requested,
    /// Environment resources are being provisioned.
    Provisioning,
    /// Environment is ready to be used.
    Ready,
    /// Environment is actively leased to a running task.
    InUse,
    /// Environment is queued for cleanup.
    CleanupQueued,
    /// Cleanup is running.
    Cleaning,
    /// Environment was archived/retained for inspection.
    Archived,
    /// Environment was removed.
    Removed,
    /// Environment is being reconciled after restart or drift.
    Recovering,
    /// Environment entered a failed state.
    Failed,
}

/// Health assessment for an environment on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentHealth {
    /// Environment is clean and matches expectations.
    Clean,
    /// Environment exists but has uncommitted/unexpected changes.
    Dirty,
    /// Environment path is missing.
    Missing,
    /// Environment path exists but no longer matches the expected shape.
    Drifted,
    /// Environment no longer belongs to an active run/task.
    Orphaned,
    /// Health has not yet been verified.
    #[default]
    Unknown,
}

/// Cleanup behavior for an execution environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CleanupPolicy {
    /// Always keep the environment.
    #[default]
    KeepAlways,
    /// Remove on success.
    RemoveOnSuccess,
    /// Archive on failure.
    ArchiveOnFailure,
    /// Always archive.
    ArchiveAlways,
    /// Remove when clean, archive otherwise.
    RemoveWhenCleanOtherwiseArchive,
}

/// Resulting cleanup action for an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupDisposition {
    /// The environment was kept in place.
    Kept,
    /// The environment was archived/retained.
    Archived,
    /// The environment was removed from disk.
    Removed,
}

/// Cleanup result for an environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResult {
    /// Final cleanup action performed.
    pub disposition: CleanupDisposition,
    /// Completion time of cleanup.
    pub completed_at: DateTime<Utc>,
    /// Retained path when the environment was preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_path: Option<PathBuf>,
    /// Human-readable cleanup summary.
    pub summary: String,
}

/// Recovery status for an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    /// No recovery action is required.
    #[default]
    NotRequired,
    /// Recovery action is pending.
    Pending,
    /// Recovery has reconciled the environment.
    Reconciled,
    /// Manual/operator intervention is required.
    NeedsOperatorAction,
    /// Recovery itself failed.
    Failed,
}

/// Recommended recovery action for an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Nothing to do.
    Noop,
    /// Recreate the missing environment.
    RecreateMissingEnvironment,
    /// Release a stale execution lease.
    ReleaseStaleLease,
    /// Archive a dirty environment for inspection.
    ArchiveDirtyEnvironment,
    /// Queue environment cleanup.
    QueueCleanup,
    /// Block the owning task and surface the issue.
    MarkTaskBlocked,
}

/// Kind of failure observed while managing an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentFailureKind {
    WorkspaceNotFound,
    PathOutsideWorkspace,
    NotGitRepository,
    GitCommandFailed,
    WorktreeAlreadyExists,
    WorktreeCreationFailed,
    WorktreeInvalid,
    WorktreeDirty,
    CleanupDenied,
    LeaseConflict,
    PersistenceError,
    RecoveryError,
}

/// Structured failure details for environment operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentFailure {
    /// Failure kind.
    pub kind: EnvironmentFailureKind,
    /// Human-readable message.
    pub message: String,
    /// Command associated with the failure, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Stderr associated with the failure, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    /// Time the failure occurred.
    pub occurred_at: DateTime<Utc>,
}

/// Lease type held on an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentLeaseKind {
    /// Lease for task execution.
    Execution,
    /// Lease held during recovery.
    Recovery,
    /// Lease held during cleanup.
    Cleanup,
}

/// Active or historical environment lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentLease {
    /// Owning task id.
    pub task_id: String,
    /// Owning agent id.
    pub agent_id: String,
    /// Lease kind.
    pub lease_kind: EnvironmentLeaseKind,
    /// Acquisition timestamp.
    pub acquired_at: DateTime<Utc>,
    /// Release timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<DateTime<Utc>>,
}

/// Git worktree provisioning details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitWorktreeSpec {
    /// Repository root used for git operations.
    pub repo_root: PathBuf,
    /// Base branch used to seed the worktree branch.
    pub base_branch: String,
    /// Generated worktree branch name.
    pub worktree_branch: String,
    /// Filesystem path of the worktree.
    pub worktree_path: PathBuf,
    /// Whether branch creation is allowed when missing.
    pub create_branch_if_missing: bool,
}

/// Durable specification for an execution environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSpec {
    /// Environment id.
    pub id: String,
    /// Execution mode requested by the task.
    pub execution_mode: AgentExecutionMode,
    /// Workspace root associated with the environment.
    pub workspace_root: PathBuf,
    /// Concrete prepared path used for execution.
    pub prepared_path: PathBuf,
    /// Owning session id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Owning run id.
    pub run_id: String,
    /// Owning task id.
    pub task_id: String,
    /// Owning agent id.
    pub agent_id: String,
    /// Cleanup policy.
    pub cleanup_policy: CleanupPolicy,
    /// Whether writes are allowed.
    pub write_access: bool,
    /// Git worktree details when using worktree mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_worktree: Option<GitWorktreeSpec>,
    /// Remote target URL when this is a remote execution surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
}

/// Execution environment bound to a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEnvironment {
    /// Stable environment identifier.
    pub id: String,
    /// Assigned execution mode.
    pub execution_mode: AgentExecutionMode,
    /// Root directory used for execution.
    pub root_dir: PathBuf,
    /// Whether writes are allowed inside the environment.
    pub write_access: bool,
    /// Optional branch or logical branch label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    /// Optional planned worktree path when using git-worktree mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    /// Optional remote URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// Durable environment lifecycle state.
    #[serde(default)]
    pub state: EnvironmentState,
    /// Current environment health.
    #[serde(default)]
    pub health: EnvironmentHealth,
    /// Cleanup behavior for the environment.
    #[serde(default)]
    pub cleanup_policy: CleanupPolicy,
    /// Recovery status for the environment.
    #[serde(default)]
    pub recovery_status: RecoveryStatus,
    /// Recommended recovery action when intervention is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<RecoveryAction>,
    /// Latest structured failure details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<EnvironmentFailure>,
    /// Last cleanup result if cleanup has occurred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_result: Option<CleanupResult>,
}

impl ExecutionEnvironment {
    fn from_record(record: &EnvironmentRecord) -> Self {
        let git_worktree = record.spec.git_worktree.as_ref();
        Self {
            id: record.id.clone(),
            execution_mode: record.spec.execution_mode.clone(),
            root_dir: record.prepared_path.clone(),
            write_access: record.spec.write_access,
            branch_name: git_worktree.map(|spec| spec.worktree_branch.clone()),
            worktree_path: git_worktree.map(|spec| spec.worktree_path.clone()),
            remote_url: record.spec.remote_url.clone(),
            state: record.state,
            health: record.health,
            cleanup_policy: record.spec.cleanup_policy,
            recovery_status: record.recovery_status,
            recovery_action: record.recovery_action,
            failure: record.failure.clone(),
            cleanup_result: record.cleanup_result.clone(),
        }
    }
}

/// Durable persisted record for an execution environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentRecord {
    /// Stable environment identifier.
    pub id: String,
    /// Durable environment specification.
    pub spec: EnvironmentSpec,
    /// Current lifecycle state.
    pub state: EnvironmentState,
    /// Current health assessment.
    pub health: EnvironmentHealth,
    /// Prepared execution path.
    pub prepared_path: PathBuf,
    /// Optional active or historical lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<EnvironmentLease>,
    /// Optional cleanup result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_result: Option<CleanupResult>,
    /// Recovery status.
    #[serde(default)]
    pub recovery_status: RecoveryStatus,
    /// Recommended recovery action, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<RecoveryAction>,
    /// Latest failure details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<EnvironmentFailure>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
    /// Last verification time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<DateTime<Utc>>,
    /// Additional environment metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl EnvironmentRecord {
    fn summary(&self) -> ExecutionEnvironment {
        ExecutionEnvironment::from_record(self)
    }
}

/// Remote progress snapshot mirrored from an A2A task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExecutionProgress {
    /// Optional current stage label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Optional human-readable status message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Percent completion when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// Last remote update time.
    pub updated_at: DateTime<Utc>,
}

/// Local delegated execution progress mirrored from the streaming agent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalExecutionPhase {
    /// Task is queued locally but not yet executing.
    Queued,
    /// Task is actively running.
    Running,
    /// Task is waiting on a sub-phase such as shell execution or reflection.
    Waiting,
    /// Task is blocked and needs intervention.
    Blocked,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was cancelled.
    Cancelled,
}

/// Structured waiting reason for local delegated execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalExecutionWaitingReason {
    /// Waiting for a shell process to finish streaming.
    ShellProcess,
    /// Waiting for reflection/review to finish.
    Reflection,
    /// Waiting for user or policy confirmation before tool execution.
    ToolConfirmation,
    /// Waiting for an environment transition.
    EnvironmentTransition,
}

/// Token usage telemetry captured during local delegated execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalExecutionTokenUsageSnapshot {
    /// Estimated prompt tokens for the current request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<usize>,
    /// Estimated input-token limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Estimated utilization percentage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u8>,
    /// Estimated token-usage status label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Estimated input cost in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Final reported input tokens when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// Final reported output tokens when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// Final reported total tokens when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    /// Model name when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider name when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Environment snapshot carried alongside local delegated telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalExecutionEnvironmentSnapshot {
    /// Current environment state.
    pub state: EnvironmentState,
    /// Current environment health.
    pub health: EnvironmentHealth,
    /// Current recovery status.
    pub recovery_status: RecoveryStatus,
    /// Snapshot timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Local delegated execution progress mirrored from the streaming agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalExecutionProgress {
    /// High-level local execution phase.
    pub phase: LocalExecutionPhase,
    /// Optional structured waiting reason while in a waiting phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<LocalExecutionWaitingReason>,
    /// Optional current stage label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Optional human-readable status message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Percent completion when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// Current agent-loop iteration.
    #[serde(default)]
    pub iteration: u32,
    /// Active tool name when the local agent is inside a tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool_name: Option<String>,
    /// Most recently completed tool name, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_tool_name: Option<String>,
    /// Duration in milliseconds for the most recently completed tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_tool_duration_ms: Option<u64>,
    /// Number of completed tool calls captured so far.
    #[serde(default)]
    pub completed_tool_call_count: usize,
    /// Whether partial user-visible content has been emitted.
    #[serde(default)]
    pub has_partial_content: bool,
    /// Partial content character count emitted so far.
    #[serde(default)]
    pub partial_content_chars: usize,
    /// Whether partial thinking content has been emitted.
    #[serde(default)]
    pub has_partial_thinking: bool,
    /// Partial thinking character count emitted so far.
    #[serde(default)]
    pub partial_thinking_chars: usize,
    /// Token-usage accounting when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<LocalExecutionTokenUsageSnapshot>,
    /// Environment snapshot at the time of this progress update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<LocalExecutionEnvironmentSnapshot>,
    /// Last local update time.
    pub updated_at: DateTime<Utc>,
}

/// Mirrored local execution state for a workflow task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalExecutionRecord {
    /// Latest local execution status.
    pub status: String,
    /// Optional status reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Latest local progress snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<LocalExecutionProgress>,
    /// Last sync timestamp.
    pub last_synced_at: DateTime<Utc>,
}

/// Active workflow-task snapshot surfaced to adapters for live operator views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTaskSnapshot {
    /// The original delegated task definition.
    pub task: DelegatedTask,
    /// Current supervisor state when known.
    pub state: SupervisorTaskState,
    /// Latest mirrored remote execution state, if task runs remotely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_execution: Option<RemoteExecutionRecord>,
    /// Latest mirrored local execution state, if task runs locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_execution: Option<LocalExecutionRecord>,
    /// Current blocked reasons, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_reasons: Vec<String>,
    /// Delegated checkpoint metadata surfaced for operator workflows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<DelegatedCheckpointSummary>,
}

/// Summary of a remote artifact available for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExecutionArtifact {
    /// Artifact display name.
    pub name: String,
    /// Number of message parts in the artifact payload.
    #[serde(default)]
    pub part_count: usize,
    /// Additional artifact metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Compatibility assessment for a remote peer.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteExecutionCompatibility {
    /// Supported task features confirmed by the peer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_features: Vec<String>,
    /// Warnings emitted when degrading to an older peer capability set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Negotiated protocol version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
}

/// Mirrored remote execution state for a workflow task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExecutionRecord {
    /// Target remote agent.
    pub target: RemoteAgentTarget,
    /// Remote task identifier.
    pub remote_task_id: String,
    /// Latest remote task status.
    pub status: String,
    /// Optional status reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Latest remote lease snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<RemoteTaskLease>,
    /// Latest remote progress snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<RemoteExecutionProgress>,
    /// Current remote artifact manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<RemoteExecutionArtifact>,
    /// Provenance details from the remote task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<gestura_core_a2a::TaskProvenance>,
    /// Compatibility assessment for this remote peer.
    #[serde(default)]
    pub compatibility: RemoteExecutionCompatibility,
    /// Last sync timestamp.
    pub last_synced_at: DateTime<Utc>,
}

/// Persistent task record owned by a supervisor run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorTaskRecord {
    /// The original delegated task definition.
    pub task: DelegatedTask,
    /// Current supervisor state.
    pub state: SupervisorTaskState,
    /// Approval tracking for the task.
    pub approval: TaskApprovalRecord,
    /// Stable execution environment id.
    #[serde(default)]
    pub environment_id: String,
    /// Prepared execution environment.
    pub environment: ExecutionEnvironment,
    /// Agent that currently claims or owns the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    /// Number of attempts made.
    #[serde(default)]
    pub attempts: u32,
    /// Reasons the task is blocked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_reasons: Vec<String>,
    /// Latest execution result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<TaskResult>,
    /// Latest mirrored remote execution state, if task runs remotely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_execution: Option<RemoteExecutionRecord>,
    /// Latest mirrored local execution state, if task runs locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_execution: Option<LocalExecutionRecord>,
    /// Task-scoped coordination messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<TeamMessage>,
    /// Delegated checkpoint metadata surfaced for operator workflows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<DelegatedCheckpointSummary>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Execution start time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Completion time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

/// Persistent supervisor run state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorRun {
    /// Run identifier.
    pub id: String,
    /// Optional human-readable run label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional session association.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Workspace root used for persistence/environment prep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Lead agent coordinating the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_agent_id: Option<String>,
    /// Parent run metadata when this is a child supervisor run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run: Option<SupervisorParentRunRef>,
    /// Direct child supervisor runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_runs: Vec<ChildSupervisorRunSummary>,
    /// Depth of this run in the supervisor hierarchy.
    #[serde(default)]
    pub hierarchy_depth: u8,
    /// Maximum supported hierarchy depth.
    #[serde(default = "default_max_child_supervisor_depth")]
    pub max_hierarchy_depth: u8,
    /// Policy inherited by tasks created within this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_policy: Option<SupervisorInheritancePolicy>,
    /// Aggregate run status.
    pub status: SupervisorRunStatus,
    /// Summary of task states for this run.
    #[serde(default)]
    pub task_summary: SupervisorRunTaskSummary,
    /// Roll-up hierarchy state for UI/adapters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy_summary: Option<SupervisorHierarchySummary>,
    /// Tasks that belong to the run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<SupervisorTaskRecord>,
    /// Run-wide messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<TeamMessage>,
    /// Durable supervisor/subagent shared cognition captured during execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_cognition: Vec<SharedCognitionNote>,
    /// Run creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Run completion time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Additional data surfaced to UI/adapters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Agent manager capabilities required by the orchestrator.
///
/// This trait is intentionally small and tauri-free so GUI/CLI wrappers can implement it
/// without pulling adapter dependencies into core.
#[async_trait::async_trait]
pub trait OrchestratorAgentManager: AgentSpawner + Clone + Send + Sync + 'static {
    /// Get status information for a specific agent.
    async fn get_agent_status(&self, id: &str) -> Option<AgentInfo>;

    /// List all active agents.
    async fn list_agents(&self) -> Vec<AgentInfo>;

    /// Update last activity timestamp for an agent.
    async fn update_activity(&self, id: &str);
}

/// Optional observer for adapter-side UI/event wiring.
///
/// Observers MUST NOT perform business logic; they should only emit events / persist
/// presentation-layer state.
#[async_trait::async_trait]
pub trait OrchestratorObserver: Send + Sync {
    /// Called after a task has been accepted and is about to execute.
    async fn on_task_started(&self, task: DelegatedTask);

    /// Called after a task has completed (success or failure).
    async fn on_task_completed(&self, task: DelegatedTask, result: TaskResult);

    /// Called whenever a supervisor run changes.
    async fn on_run_updated(&self, _run: SupervisorRun) {}

    /// Called when a team message is recorded.
    async fn on_team_message(&self, _message: TeamMessage) {}

    /// Called when a collaboration thread changes.
    async fn on_team_thread_updated(&self, _thread: TeamThread) {}

    /// Called when an environment record changes.
    async fn on_environment_updated(&self, _environment: EnvironmentRecord) {}

    /// Called when an environment recovery action is recorded.
    async fn on_environment_recovery(
        &self,
        _environment_id: String,
        _action: RecoveryAction,
        _summary: String,
    ) {
    }

    /// Called when environment cleanup completes.
    async fn on_environment_cleanup(&self, _environment_id: String, _result: CleanupResult) {}
}

#[derive(Clone, Debug)]
struct ActiveTaskControl {
    task: DelegatedTask,
    local_cancel_token: Option<CancellationToken>,
    attempt: u32,
}

#[derive(Debug)]
struct LocalDelegatedExecutionOutcome {
    result: Result<String, String>,
    tool_calls: Vec<OrchestratorToolCall>,
    terminal_state_hint: TaskTerminalStateHint,
    preserve_existing_checkpoint: bool,
}

impl LocalDelegatedExecutionOutcome {
    fn into_task_result(self, task: &DelegatedTask, duration_ms: u64) -> TaskResult {
        TaskResult {
            task_id: task.id.clone(),
            agent_id: task.agent_id.clone(),
            success: self.result.is_ok(),
            run_id: task.run_id.clone(),
            tracking_task_id: task.tracking_task_id.clone(),
            output: self.result.unwrap_or_else(|error| error),
            summary: task.name.clone(),
            tool_calls: self.tool_calls,
            artifacts: Vec::new(),
            terminal_state_hint: Some(self.terminal_state_hint),
            duration_ms,
        }
    }
}

/// Orchestrator for coordinating subagents and delegated task execution.
///
/// The orchestrator is core-owned and does not depend on Tauri. GUI/CLI layers can
/// attach an [`OrchestratorObserver`] to receive lifecycle events.
#[derive(Clone)]
pub struct AgentOrchestrator<M: OrchestratorAgentManager> {
    agent_manager: M,
    permission_manager: Arc<PermissionManager>,
    active_tasks: Arc<Mutex<HashMap<String, ActiveTaskControl>>>,
    supervisor_runs: Arc<Mutex<HashMap<String, SupervisorRun>>>,
    environments: Arc<Mutex<HashMap<String, EnvironmentRecord>>>,
    task_run_index: Arc<Mutex<HashMap<String, String>>>,
    result_tx: mpsc::Sender<TaskResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<TaskResult>>>,
    config: AppConfig,
    observer: Arc<RwLock<Option<Arc<dyn OrchestratorObserver>>>>,
    default_workspace_dir: Option<PathBuf>,
}

impl<M: OrchestratorAgentManager> AgentOrchestrator<M> {
    /// Create a new orchestrator with the given agent manager and application config.
    pub fn new(agent_manager: M, config: AppConfig) -> Self {
        Self::new_with_workspace_root(agent_manager, config, std::env::current_dir().ok())
    }

    /// Create a new orchestrator with an explicit workspace root for persisted state.
    pub fn new_with_workspace_root(
        agent_manager: M,
        config: AppConfig,
        default_workspace_dir: Option<PathBuf>,
    ) -> Self {
        let (result_tx, result_rx) = mpsc::channel(100);
        let orchestrator = Self {
            agent_manager,
            permission_manager: Arc::new(PermissionManager::new()),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            supervisor_runs: Arc::new(Mutex::new(HashMap::new())),
            environments: Arc::new(Mutex::new(HashMap::new())),
            task_run_index: Arc::new(Mutex::new(HashMap::new())),
            result_tx,
            result_rx: Arc::new(Mutex::new(result_rx)),
            config,
            observer: Arc::new(RwLock::new(None)),
            default_workspace_dir,
        };
        orchestrator.bootstrap_persisted_state();
        orchestrator
    }

    /// Attach an observer used for adapter-side event emission.
    ///
    /// This is intentionally async and uses interior mutability so adapters (GUI/CLI)
    /// can attach observers after construction (e.g., once a Tauri `AppHandle` exists)
    /// without requiring `&mut self`.
    pub async fn set_observer(&self, observer: Arc<dyn OrchestratorObserver>) {
        *self.observer.write().await = Some(observer);
    }

    /// Remove any attached observer.
    pub async fn clear_observer(&self) {
        *self.observer.write().await = None;
    }

    /// Spawn and register a new subagent.
    pub async fn spawn_subagent(&self, id: &str, name: &str) -> Result<(), String> {
        self.spawn_subagent_with_request(AgentSpawnRequest::new(
            id.to_string(),
            name.to_string(),
            AgentRole::Implementer,
        ))
        .await
    }

    /// Spawn and register a subagent using an explicit specialist configuration.
    pub async fn spawn_subagent_with_request(
        &self,
        request: AgentSpawnRequest,
    ) -> Result<(), String> {
        tracing::info!(agent_id = %request.id, agent_name = %request.name, role = ?request.role, "Spawning subagent");
        self.agent_manager.spawn_agent_with_request(request).await;
        Ok(())
    }

    /// Delegate a task to a subagent.
    ///
    /// - Ensures the target agent exists (spawning a default one if needed)
    /// - Enforces tool permission checks
    /// - Executes the task asynchronously via the unified pipeline
    pub async fn delegate_task(&self, mut task: DelegatedTask) -> Result<String, String> {
        let task_id = task.id.clone();
        let agent_id = task.agent_id.clone();

        if task.run_id.is_none() {
            task.run_id = Some(Uuid::new_v4().to_string());
        }
        if task.role.is_none() {
            task.role = Some(AgentRole::Implementer);
        }
        if task.name.is_none() {
            task.name = Some(format!(
                "{} task",
                task.role.clone().unwrap_or_default().label()
            ));
        }

        if let Some(run_id) = task.run_id.as_deref() {
            let runs = self.supervisor_runs.lock().await;
            if let Some(run) = runs.get(run_id) {
                if task.session_id.is_none() {
                    task.session_id = run.session_id.clone();
                }
                if task.workspace_dir.is_none() {
                    task.workspace_dir = run
                        .workspace_dir
                        .clone()
                        .or_else(|| self.default_workspace_dir.clone());
                }
                if let Some(policy) = run.inherited_policy.as_ref() {
                    policy.apply_to_task(&mut task);
                }
            }
        }
        if task.workspace_dir.is_none() {
            task.workspace_dir = self.default_workspace_dir.clone();
        }

        let session_id = task.session_id.clone();

        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            session_id = ?session_id,
            run_id = ?task.run_id,
            role = ?task.role,
            priority = task.priority,
            "Delegating task to subagent"
        );

        self.ensure_agent_exists(&task).await?;

        // Check local tool permissions only for local/shared execution. Remote
        // tasks rely on the remote peer's authenticated capability contract.
        if task.execution_mode != AgentExecutionMode::Remote {
            for tool in &task.required_tools {
                let check = self
                    .permission_manager
                    .check(tool, "execute", None)
                    .map_err(|e| format!("Permission check error: {}", e))?;
                if !check.allowed {
                    tracing::warn!(tool = %tool, task_id = %task_id, reason = %check.reason, "Tool not permitted for task");
                    return Err(format!("Tool '{}' not permitted: {}", tool, check.reason));
                }
            }
        }

        let environment_record = self.prepare_environment(&task).await?;
        let environment = environment_record.summary();
        task.environment_id = Some(environment_record.id.clone());
        self.ensure_tracking_task(&mut task).await;

        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| "Delegated task is missing run_id".to_string())?;

        let (run_snapshot, task_snapshot, should_start, initial_message) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs.entry(run_id.clone()).or_insert_with(|| SupervisorRun {
                id: run_id.clone(),
                name: task.name.clone(),
                session_id: task.session_id.clone(),
                workspace_dir: task
                    .workspace_dir
                    .clone()
                    .or_else(|| self.default_workspace_dir.clone()),
                lead_agent_id: Some("supervisor".to_string()),
                parent_run: None,
                child_runs: Vec::new(),
                hierarchy_depth: 0,
                max_hierarchy_depth: MAX_CHILD_SUPERVISOR_DEPTH,
                inherited_policy: None,
                status: SupervisorRunStatus::Draft,
                task_summary: SupervisorRunTaskSummary::default(),
                hierarchy_summary: None,
                tasks: Vec::new(),
                messages: Vec::new(),
                shared_cognition: Vec::new(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                completed_at: None,
                metadata: None,
            });

            if run.session_id.is_none() {
                run.session_id = task.session_id.clone();
            }
            if run.workspace_dir.is_none() {
                run.workspace_dir = task
                    .workspace_dir
                    .clone()
                    .or_else(|| self.default_workspace_dir.clone());
            }
            if run.name.is_none() {
                run.name = task.name.clone();
            }
            if task.session_id.is_none() {
                task.session_id = run.session_id.clone();
            }
            if task.workspace_dir.is_none() {
                task.workspace_dir = run
                    .workspace_dir
                    .clone()
                    .or_else(|| self.default_workspace_dir.clone());
            }
            if let Some(policy) = run.inherited_policy.as_ref() {
                policy.apply_to_task(&mut task);
            }
            ensure_shared_memory_tags(&mut task, &run.id);

            let blocked_reasons = unresolved_dependency_reasons(run, &task);
            let approval = if task.approval_required {
                TaskApprovalRecord::pending(
                    &task,
                    ApprovalScope::PreExecution,
                    ApprovalActor::system("orchestrator"),
                    Some("Task submitted. Awaiting explicit pre-execution approval.".to_string()),
                )
            } else {
                TaskApprovalRecord::not_required(&task)
            };
            let mut blocked_reasons = blocked_reasons;
            if matches!(environment.state, EnvironmentState::Failed)
                && let Some(failure) = &environment.failure
            {
                blocked_reasons.push(failure.message.clone());
            }
            let state = if matches!(environment.state, EnvironmentState::Failed) {
                SupervisorTaskState::Blocked
            } else if task.approval_required {
                SupervisorTaskState::PendingApproval
            } else if blocked_reasons.is_empty() {
                SupervisorTaskState::Queued
            } else {
                SupervisorTaskState::Blocked
            };

            let now = Utc::now();
            let mut new_record = SupervisorTaskRecord {
                task: task.clone(),
                state,
                approval,
                environment_id: environment_record.id.clone(),
                environment,
                claimed_by: Some(task.agent_id.clone()),
                attempts: 0,
                blocked_reasons,
                result: None,
                remote_execution: None,
                local_execution: None,
                messages: Vec::new(),
                checkpoint: None,
                created_at: now,
                updated_at: now,
                started_at: None,
                completed_at: None,
            };

            let initial_message = if matches!(state, SupervisorTaskState::PendingApproval) {
                let message =
                    build_gate_request_message(&run.id, &new_record, ApprovalScope::PreExecution);
                new_record.messages.push(message.clone());
                Some(message)
            } else {
                None
            };

            if let Some(existing) = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
            {
                new_record.attempts = existing.attempts;
                new_record.created_at = existing.created_at;
                *existing = new_record.clone();
            } else {
                run.tasks.push(new_record.clone());
            }

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            run.task_summary = summarize_run_tasks(run);
            run.hierarchy_summary = Some(build_hierarchy_summary(run));

            (
                run.clone(),
                new_record.clone(),
                state == SupervisorTaskState::Queued,
                initial_message,
            )
        };

        self.task_run_index
            .lock()
            .await
            .insert(task_id.clone(), run_id.clone());

        record_task_dispatch(&task, &task_snapshot, &run_snapshot);
        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        if let Some(message) = initial_message {
            self.notify_team_message(message).await;
        }

        if should_start {
            self.start_task_execution(task.clone()).await?;
        }

        Ok(task_id)
    }

    /// Create a direct child supervisor run under an existing parent run.
    pub async fn create_child_supervisor_run(
        &self,
        request: ChildSupervisorRunRequest,
    ) -> Result<SupervisorRun, String> {
        let objective = request.objective.trim();
        if objective.is_empty() {
            return Err("Child supervisor objective cannot be empty".to_string());
        }
        if request.lead_agent_id.trim().is_empty() {
            return Err("Child supervisor lead_agent_id cannot be empty".to_string());
        }

        let child_run_id = request
            .run_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("run-child-{}", Uuid::new_v4()));
        if self.get_supervisor_run(&child_run_id).await.is_some() {
            return Err(format!("Supervisor run '{}' already exists", child_run_id));
        }

        let parent_snapshot = self
            .get_supervisor_run(&request.parent_run_id)
            .await
            .ok_or_else(|| {
                format!(
                    "Parent supervisor run '{}' not found",
                    request.parent_run_id
                )
            })?;
        ensure_parent_run_accepts_child(&parent_snapshot)?;

        if self
            .agent_manager
            .get_agent_status(&request.lead_agent_id)
            .await
            .is_none()
        {
            let mut spawn_request = AgentSpawnRequest::new(
                request.lead_agent_id.clone(),
                request
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Child supervisor {}", request.lead_agent_id)),
                AgentRole::Supervisor,
            );
            spawn_request.workspace_dir = request
                .workspace_dir
                .clone()
                .or_else(|| parent_snapshot.workspace_dir.clone());
            spawn_request.execution_mode = request.execution_mode.clone();
            self.spawn_subagent_with_request(spawn_request).await?;
        }

        let now = Utc::now();
        let (child_run, parent_run, parent_message, child_message) = {
            let mut runs = self.supervisor_runs.lock().await;
            let parent = runs.get_mut(&request.parent_run_id).ok_or_else(|| {
                format!(
                    "Parent supervisor run '{}' no longer exists",
                    request.parent_run_id
                )
            })?;
            ensure_parent_run_accepts_child(parent)?;

            let inherited_policy = build_child_inherited_policy(parent, &request);
            let child_run = SupervisorRun {
                id: child_run_id.clone(),
                name: request.name.clone().or_else(|| Some(objective.to_string())),
                session_id: request
                    .session_id
                    .clone()
                    .or_else(|| parent.session_id.clone()),
                workspace_dir: request
                    .workspace_dir
                    .clone()
                    .or_else(|| parent.workspace_dir.clone()),
                lead_agent_id: Some(request.lead_agent_id.clone()),
                parent_run: Some(SupervisorParentRunRef {
                    parent_run_id: parent.id.clone(),
                    parent_task_id: request.parent_task_id.clone(),
                    delegated_by_agent_id: parent.lead_agent_id.clone(),
                    objective: objective.to_string(),
                    created_at: now,
                }),
                child_runs: Vec::new(),
                hierarchy_depth: parent.hierarchy_depth.saturating_add(1),
                max_hierarchy_depth: parent.max_hierarchy_depth,
                inherited_policy: Some(inherited_policy),
                status: SupervisorRunStatus::Draft,
                task_summary: SupervisorRunTaskSummary::default(),
                hierarchy_summary: Some(SupervisorHierarchySummary {
                    depth: parent.hierarchy_depth.saturating_add(1),
                    max_depth: parent.max_hierarchy_depth,
                    child_run_count: 0,
                    descendant_task_count: 0,
                    action_required_child_count: 0,
                    rollup_status: SupervisorRunStatus::Draft,
                    requires_attention: false,
                    blocked_reasons: Vec::new(),
                }),
                tasks: Vec::new(),
                messages: Vec::new(),
                shared_cognition: Vec::new(),
                created_at: now,
                updated_at: now,
                completed_at: None,
                metadata: None,
            };

            let parent_message = TeamMessage::new(
                parent.id.clone(),
                request.parent_task_id.clone(),
                TeamMessageKind::Handoff,
                parent.lead_agent_id.clone(),
                Some(request.lead_agent_id.clone()),
                format!(
                    "Delegated child supervisor run {} for objective: {}",
                    child_run_id, objective
                ),
            );
            let child_message = TeamMessage::new(
                child_run_id.clone(),
                None,
                TeamMessageKind::StatusUpdate,
                parent.lead_agent_id.clone(),
                Some(request.lead_agent_id.clone()),
                format!(
                    "Child supervisor run created under {} with objective: {}",
                    parent.id, objective
                ),
            );

            parent.messages.push(parent_message.clone());
            parent.updated_at = now;
            parent.completed_at = None;
            parent.child_runs.push(build_child_run_summary(&child_run));
            parent.task_summary = summarize_run_tasks(parent);
            parent.status = recalculate_run_status(parent);
            parent.hierarchy_summary = Some(build_hierarchy_summary(parent));
            let parent_run = parent.clone();

            let mut child_run = child_run;
            child_run.messages.push(child_message.clone());
            runs.insert(child_run.id.clone(), child_run.clone());
            (child_run, parent_run, parent_message, child_message)
        };

        self.persist_run_async(&child_run).await?;
        self.persist_run_async(&parent_run).await?;
        self.notify_run_updated(child_run.clone()).await;
        self.notify_run_updated(parent_run).await;
        self.notify_team_message(parent_message).await;
        self.notify_team_message(child_message).await;
        Ok(child_run)
    }

    /// Get the result of a completed task if one is ready.
    pub async fn poll_result(&self) -> Option<TaskResult> {
        let mut rx = self.result_rx.lock().await;
        rx.try_recv().ok()
    }

    /// Get list of currently active tasks.
    pub async fn list_active_tasks(&self) -> Vec<DelegatedTask> {
        let active = self.active_tasks.lock().await;
        active.values().map(|entry| entry.task.clone()).collect()
    }

    /// Get live active-task snapshots enriched with mirrored execution telemetry.
    pub async fn list_active_task_snapshots(&self) -> Vec<ActiveTaskSnapshot> {
        let active = self
            .active_tasks
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if active.is_empty() {
            return Vec::new();
        }

        let runs = self.list_supervisor_runs().await;
        let task_run_index = self.task_run_index.lock().await.clone();

        let mut snapshots = active
            .into_iter()
            .map(|entry| {
                let record = task_run_index
                    .get(&entry.task.id)
                    .and_then(|run_id| runs.iter().find(|run| run.id == *run_id))
                    .and_then(|run| {
                        run.tasks
                            .iter()
                            .find(|record| record.task.id == entry.task.id)
                    });
                active_task_snapshot(entry.task, record)
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            right
                .task
                .priority
                .cmp(&left.task.priority)
                .then_with(|| left.task.id.cmp(&right.task.id))
        });
        snapshots
    }

    /// List live and persisted supervisor runs known to the orchestrator.
    pub async fn list_supervisor_runs(&self) -> Vec<SupervisorRun> {
        let mut runs = self
            .supervisor_runs
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(root) = self.default_workspace_dir.as_deref() {
            for run in load_persisted_runs(root) {
                if !runs.iter().any(|existing| existing.id == run.id) {
                    runs.push(run);
                }
            }
        }
        let checkpoints = load_latest_checkpoints_by_task(checkpoint_roots_for_runs(
            &runs,
            self.default_workspace_dir.as_deref(),
        ));
        attach_checkpoint_summaries(&mut runs, checkpoints);
        synchronize_run_hierarchy_snapshots(&mut runs);
        runs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        runs
    }

    /// List only root supervisor runs (child runs excluded from the top-level list).
    pub async fn list_root_supervisor_runs(&self) -> Vec<SupervisorRun> {
        self.list_supervisor_runs()
            .await
            .into_iter()
            .filter(|run| run.parent_run.is_none())
            .collect()
    }

    /// List child supervisor runs for a specific parent run.
    pub async fn list_child_supervisor_runs(&self, parent_run_id: &str) -> Vec<SupervisorRun> {
        self.list_supervisor_runs()
            .await
            .into_iter()
            .filter(|run| {
                run.parent_run
                    .as_ref()
                    .is_some_and(|parent| parent.parent_run_id == parent_run_id)
            })
            .collect()
    }

    /// Return the ancestor chain for a run, ordered from root to immediate parent.
    pub async fn get_supervisor_run_ancestry(&self, run_id: &str) -> Vec<SupervisorRun> {
        let runs = self.list_supervisor_runs().await;
        let index = runs
            .iter()
            .cloned()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();
        let mut ancestors = Vec::new();
        let mut current_parent = index.get(run_id).and_then(|run| {
            run.parent_run
                .as_ref()
                .map(|parent| parent.parent_run_id.clone())
        });
        while let Some(parent_id) = current_parent {
            let Some(parent) = index.get(&parent_id).cloned() else {
                break;
            };
            current_parent = parent
                .parent_run
                .as_ref()
                .map(|parent| parent.parent_run_id.clone());
            ancestors.push(parent);
        }
        ancestors.reverse();
        ancestors
    }

    /// Return all descendants beneath a run (bounded to one level for now).
    pub async fn get_supervisor_run_descendants(&self, run_id: &str) -> Vec<SupervisorRun> {
        self.list_supervisor_runs()
            .await
            .into_iter()
            .filter(|run| {
                run.parent_run
                    .as_ref()
                    .is_some_and(|parent| parent.parent_run_id == run_id)
            })
            .collect()
    }

    /// Return leaf tasks visible beneath a run, including direct children.
    pub async fn list_supervisor_leaf_tasks(&self, run_id: &str) -> Vec<SupervisorTaskRecord> {
        let runs = self.list_supervisor_runs().await;
        let mut leaf_tasks = Vec::new();
        if let Some(run) = runs.iter().find(|run| run.id == run_id) {
            leaf_tasks.extend(run.tasks.clone());
        }
        for child in runs.iter().filter(|run| {
            run.parent_run
                .as_ref()
                .is_some_and(|parent| parent.parent_run_id == run_id)
        }) {
            leaf_tasks.extend(child.tasks.clone());
        }
        leaf_tasks
    }

    /// Fetch a supervisor run by id.
    pub async fn get_supervisor_run(&self, run_id: &str) -> Option<SupervisorRun> {
        let mut in_memory_runs = self
            .supervisor_runs
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if !in_memory_runs.is_empty() {
            let checkpoints = load_latest_checkpoints_by_task(checkpoint_roots_for_runs(
                &in_memory_runs,
                self.default_workspace_dir.as_deref(),
            ));
            attach_checkpoint_summaries(&mut in_memory_runs, checkpoints);
            synchronize_run_hierarchy_snapshots(&mut in_memory_runs);
            if let Some(run) = in_memory_runs.into_iter().find(|run| run.id == run_id) {
                return Some(run);
            }
        }

        self.default_workspace_dir.as_deref().and_then(|root| {
            let mut runs = load_persisted_runs(root);
            let checkpoints = load_latest_checkpoints_by_task(checkpoint_roots_for_runs(
                &runs,
                self.default_workspace_dir.as_deref(),
            ));
            attach_checkpoint_summaries(&mut runs, checkpoints);
            synchronize_run_hierarchy_snapshots(&mut runs);
            runs.into_iter().find(|run| run.id == run_id)
        })
    }

    /// List team messages for a run.
    pub async fn list_team_messages(&self, run_id: &str) -> Vec<TeamMessage> {
        self.get_supervisor_run(run_id)
            .await
            .map(|run| {
                let mut messages = run.messages;
                for task in run.tasks {
                    messages.extend(task.messages);
                }
                messages.sort_by(|left, right| left.created_at.cmp(&right.created_at));
                messages
            })
            .unwrap_or_default()
    }

    /// List grouped collaboration threads for a run.
    pub async fn list_team_threads(&self, run_id: &str) -> Vec<TeamThread> {
        self.list_team_threads_with_options(run_id, false).await
    }

    /// List grouped collaboration threads for a run with archive controls.
    pub async fn list_team_threads_with_options(
        &self,
        run_id: &str,
        include_archived: bool,
    ) -> Vec<TeamThread> {
        build_team_threads_with_options(&self.list_team_messages(run_id).await, include_archived)
    }

    /// List all running subagents.
    pub async fn list_subagents(&self) -> Vec<AgentInfo> {
        self.agent_manager.list_agents().await
    }

    /// Approve a task before execution or after a review/test gate.
    pub async fn approve_task(
        &self,
        task_id: &str,
        actor: ApprovalActor,
        note: Option<String>,
    ) -> Result<(), String> {
        let run_id = self
            .task_run_index
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        let mut queued_to_start = None;
        let mut completion_environment_id = None;
        let (mut run_snapshot, collaboration_messages, reflection_sync) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let dependency_states = run
                .tasks
                .iter()
                .map(|record| (record.task.id.clone(), record.state))
                .collect::<HashMap<_, _>>();
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;

            let scope = approval_scope_for_state(record.state).ok_or_else(|| {
                format!(
                    "Task '{}' is not waiting for approval or gate completion",
                    task_id
                )
            })?;

            let decision = record.approval.record_decision(
                scope,
                ApprovalDecisionKind::Approved,
                actor,
                note,
            )?;
            let thread_context = resolve_open_gate_request(
                record,
                scope,
                &decision.actor.id,
                CollaborationActionStatus::Resolved,
                decision.note.clone(),
            );
            let mut approval_message = TeamMessage::new(
                run.id.clone(),
                Some(record.task.id.clone()),
                TeamMessageKind::ApprovalDecision,
                Some(decision.actor.id.clone()),
                Some(record.task.agent_id.clone()),
                format_approval_decision_message(&decision),
            );
            if let Some((thread_id, reply_to_message_id)) = thread_context {
                approval_message =
                    approval_message.with_thread(thread_id, Some(reply_to_message_id));
            }
            if let Some(result) = record.result.as_ref() {
                approval_message = approval_message
                    .with_result_reference(TeamResultReference::from_task_result(result));
                approval_message = approval_message.with_artifact_references(
                    result
                        .artifacts
                        .iter()
                        .map(|artifact| {
                            TeamArtifactReference::from_task_artifact(
                                Some(record.task.id.clone()),
                                artifact,
                            )
                        })
                        .collect(),
                );
            }
            let mut collaboration_messages = vec![approval_message.clone()];

            record.updated_at = Utc::now();

            match record.state {
                SupervisorTaskState::PendingApproval => {
                    let blocked = dependency_reasons_from_states(&dependency_states, &record.task);
                    if blocked.is_empty() {
                        record.state = SupervisorTaskState::Queued;
                        record.blocked_reasons.clear();
                        queued_to_start = Some(record.task.clone());
                    } else {
                        record.state = SupervisorTaskState::Blocked;
                        record.blocked_reasons = blocked;
                    }
                }
                SupervisorTaskState::ReviewPending => {
                    if record.task.test_required {
                        record.state = SupervisorTaskState::TestPending;
                        record.approval.request(
                            ApprovalScope::TestValidation,
                            ApprovalActor::system("orchestrator"),
                            Some("Review approved. Awaiting explicit test validation.".to_string()),
                        );
                        let message = build_gate_request_message(
                            &run.id,
                            record,
                            ApprovalScope::TestValidation,
                        );
                        record.messages.push(message.clone());
                        collaboration_messages.push(message);
                    } else {
                        record.state = SupervisorTaskState::Completed;
                        record.completed_at = Some(Utc::now());
                        completion_environment_id = Some(record.environment_id.clone());
                    }
                }
                SupervisorTaskState::TestPending => {
                    record.state = SupervisorTaskState::Completed;
                    record.completed_at = Some(Utc::now());
                    completion_environment_id = Some(record.environment_id.clone());
                }
                _ => {
                    return Err(format!(
                        "Task '{}' is not waiting for approval or gate completion",
                        task_id
                    ));
                }
            }

            let reflection_sync = {
                record.messages.push(approval_message.clone());
                task_reflection_sync_context(&record.task).map(
                    |(workspace_dir, session_id, tracking_task_id)| {
                        (
                            workspace_dir,
                            session_id,
                            tracking_task_id,
                            task_record_outcome_signals(record),
                        )
                    },
                )
            };

            run.messages.push(approval_message.clone());
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (run.clone(), collaboration_messages, reflection_sync)
        };

        if let Some(environment_id) = completion_environment_id
            && let Some(environment) = self
                .finalize_environment_for_task(&environment_id, true, false)
                .await?
        {
            self.update_environment_in_runs(&environment).await?;
            if let Some(run) = self.supervisor_runs.lock().await.get(&run_id).cloned() {
                run_snapshot = run;
            }
        }

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        for message in collaboration_messages {
            self.notify_team_message(message).await;
        }
        if let Some((workspace_dir, session_id, tracking_task_id, signals)) = reflection_sync {
            let _ = sync_task_reflection_outcomes(
                &workspace_dir,
                &session_id,
                &tracking_task_id,
                &signals,
            )
            .await;
        }

        if let Some(task) = queued_to_start {
            self.start_task_execution(task).await?;
        }

        Ok(())
    }

    /// Reject or request revision for a delegated task.
    pub async fn reject_task(
        &self,
        task_id: &str,
        actor: ApprovalActor,
        note: Option<String>,
    ) -> Result<(), String> {
        let run_id = self
            .task_run_index
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        let (mut run_snapshot, approval_message, reflection_sync) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;

            let scope = approval_scope_for_state(record.state).ok_or_else(|| {
                format!(
                    "Task '{}' is not waiting for approval or gate completion",
                    task_id
                )
            })?;
            let decision = record.approval.record_decision(
                scope,
                ApprovalDecisionKind::NeedsRevision,
                actor,
                note,
            )?;
            let thread_context = resolve_open_gate_request(
                record,
                scope,
                &decision.actor.id,
                CollaborationActionStatus::NeedsRevision,
                decision.note.clone(),
            );
            let mut approval_message = TeamMessage::new(
                run.id.clone(),
                Some(record.task.id.clone()),
                TeamMessageKind::ApprovalDecision,
                Some(decision.actor.id.clone()),
                Some(record.task.agent_id.clone()),
                format_approval_decision_message(&decision),
            );
            if let Some((thread_id, reply_to_message_id)) = thread_context {
                approval_message =
                    approval_message.with_thread(thread_id, Some(reply_to_message_id));
            }
            if let Some(result) = record.result.as_ref() {
                approval_message = approval_message
                    .with_result_reference(TeamResultReference::from_task_result(result));
                approval_message = approval_message.with_artifact_references(
                    result
                        .artifacts
                        .iter()
                        .map(|artifact| {
                            TeamArtifactReference::from_task_artifact(
                                Some(record.task.id.clone()),
                                artifact,
                            )
                        })
                        .collect(),
                );
            }
            record.state = SupervisorTaskState::Failed;
            record.updated_at = Utc::now();
            let reflection_sync = {
                record.messages.push(approval_message.clone());
                task_reflection_sync_context(&record.task).map(
                    |(workspace_dir, session_id, tracking_task_id)| {
                        (
                            workspace_dir,
                            session_id,
                            tracking_task_id,
                            task_record_outcome_signals(record),
                        )
                    },
                )
            };
            run.messages.push(approval_message.clone());
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (run.clone(), Some(approval_message), reflection_sync)
        };

        if let Some(environment_id) = run_snapshot
            .tasks
            .iter()
            .find(|record| record.task.id == task_id)
            .map(|record| record.environment_id.clone())
            && let Some(environment) = self
                .finalize_environment_for_task(&environment_id, false, true)
                .await?
        {
            self.update_environment_in_runs(&environment).await?;
            if let Some(run) = self.supervisor_runs.lock().await.get(&run_id).cloned() {
                run_snapshot = run;
            }
        }

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        if let Some(message) = approval_message {
            self.notify_team_message(message).await;
        }
        if let Some((workspace_dir, session_id, tracking_task_id, signals)) = reflection_sync {
            let _ = sync_task_reflection_outcomes(
                &workspace_dir,
                &session_id,
                &tracking_task_id,
                &signals,
            )
            .await;
        }
        Ok(())
    }

    /// Retry a task that previously failed or was blocked.
    pub async fn retry_task(&self, task_id: &str) -> Result<(), String> {
        let run_id = self
            .task_run_index
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        let mut queued_to_start = None;
        let (mut run_snapshot, environment_id, retry_message) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let dependency_states = run
                .tasks
                .iter()
                .map(|record| (record.task.id.clone(), record.state))
                .collect::<HashMap<_, _>>();
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;

            if !matches!(
                record.state,
                SupervisorTaskState::Failed
                    | SupervisorTaskState::Cancelled
                    | SupervisorTaskState::Blocked
            ) {
                return Err(format!(
                    "Task '{}' cannot be retried from its current state",
                    task_id
                ));
            }

            record.attempts += 1;
            record.result = None;
            record.remote_execution = None;
            record.local_execution = None;
            record.completed_at = None;
            record.started_at = None;
            record.updated_at = Utc::now();
            record.blocked_reasons.clear();
            let environment_id = record.environment_id.clone();
            let mut retry_message = None;

            if record.task.approval_required {
                record.state = SupervisorTaskState::PendingApproval;
                record.approval.reset_for_task(&record.task);
                record.approval.request(
                    ApprovalScope::PreExecution,
                    ApprovalActor::system("orchestrator"),
                    Some("Task retried. Awaiting explicit pre-execution approval.".to_string()),
                );
                let message =
                    build_gate_request_message(&run.id, record, ApprovalScope::PreExecution);
                record.messages.push(message.clone());
                retry_message = Some(message);
            } else {
                record.approval.reset_for_task(&record.task);
                let blocked = dependency_reasons_from_states(&dependency_states, &record.task);
                if blocked.is_empty() {
                    record.state = SupervisorTaskState::Queued;
                    record.blocked_reasons.clear();
                    queued_to_start = Some(record.task.clone());
                } else {
                    record.state = SupervisorTaskState::Blocked;
                    record.blocked_reasons = blocked;
                }
            }

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (run.clone(), environment_id, retry_message)
        };

        let environment = self.retry_environment_preparation(&environment_id).await?;
        self.update_environment_in_runs(&environment).await?;
        if matches!(environment.state, EnvironmentState::Failed) {
            let mut runs = self.supervisor_runs.lock().await;
            if let Some(run) = runs.get_mut(&run_id)
                && let Some(record) = run
                    .tasks
                    .iter_mut()
                    .find(|record| record.task.id == task_id)
            {
                record.state = SupervisorTaskState::Blocked;
                record.environment = environment.summary();
                record.blocked_reasons = environment
                    .failure
                    .as_ref()
                    .map(|failure| vec![failure.message.clone()])
                    .unwrap_or_default();
                run.updated_at = Utc::now();
                run.status = recalculate_run_status(run);
                run_snapshot = run.clone();
                queued_to_start = None;
            }
        } else if let Some(run) = self.supervisor_runs.lock().await.get(&run_id).cloned() {
            run_snapshot = run;
        }

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        if let Some(message) = retry_message {
            self.notify_team_message(message).await;
        }

        if let Some(task) = queued_to_start {
            self.start_task_execution(task).await?;
        }

        Ok(())
    }

    /// Resume a blocked workflow task from its latest persisted checkpoint.
    pub async fn resume_task_from_checkpoint(&self, task_id: &str) -> Result<(), String> {
        let run_id = self
            .task_run_index
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;
        let task = self
            .supervisor_task_record(task_id)
            .await
            .map(|record| record.task)
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;
        let checkpoint = self
            .load_delegated_checkpoint_async(&task)
            .await
            .ok_or_else(|| format!("Task '{}' has no delegated checkpoint", task_id))?;

        if checkpoint.result_published {
            return Err(format!(
                "Task '{}' already published a terminal result and cannot be resumed",
                task_id
            ));
        }
        if checkpoint.resume_disposition != DelegatedResumeDisposition::ResumeFromCheckpoint
            || checkpoint.resume_state.is_none()
        {
            return Err(format!(
                "Task '{}' is not currently resumable from checkpoint",
                task_id
            ));
        }
        if self.active_tasks.lock().await.contains_key(task_id) {
            return Err(format!("Task '{}' is already active", task_id));
        }

        let blocked_reason = restart_blocked_reason_for_checkpoint(&checkpoint);
        let (run_snapshot, queued_to_start) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;

            record
                .blocked_reasons
                .retain(|reason| reason != &blocked_reason);
            if !record.blocked_reasons.is_empty() {
                return Err(format!(
                    "Task '{}' still has unresolved blockers: {}",
                    task_id,
                    record.blocked_reasons.join("; ")
                ));
            }

            record.state = SupervisorTaskState::Queued;
            record.result = None;
            record.local_execution = None;
            record.completed_at = None;
            record.started_at = None;
            record.updated_at = Utc::now();
            let queued_to_start = Some(record.task.clone());
            run.updated_at = Utc::now();
            refresh_run_rollups(run);
            (run.clone(), queued_to_start)
        };

        let mut updated_checkpoint = checkpoint.clone();
        updated_checkpoint.stage = DelegatedCheckpointStage::Queued;
        updated_checkpoint.note = Some("manual resume requested from checkpoint".to_string());
        updated_checkpoint.updated_at = Utc::now();
        self.persist_delegated_checkpoint_async(&updated_checkpoint)
            .await?;
        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        if let Some(task) = queued_to_start {
            self.schedule_task_execution(task);
        }
        Ok(())
    }

    /// Restart a workflow task from scratch and discard any saved checkpoint resume state.
    pub async fn restart_task_from_scratch(&self, task_id: &str) -> Result<(), String> {
        if let Some(task) = self
            .supervisor_task_record(task_id)
            .await
            .map(|record| record.task)
            && let Some(mut checkpoint) = self.load_delegated_checkpoint_async(&task).await
        {
            checkpoint.stage = DelegatedCheckpointStage::Queued;
            checkpoint.resume_state = None;
            checkpoint.completed_tool_calls.clear();
            checkpoint.resume_disposition = DelegatedResumeDisposition::RestartFromBoundary;
            checkpoint.safe_boundary_label = "manual restart from scratch".to_string();
            checkpoint.result_published = false;
            checkpoint.note = Some("operator cleared checkpoint resume state".to_string());
            checkpoint.updated_at = Utc::now();
            self.persist_delegated_checkpoint_async(&checkpoint).await?;
        }

        self.retry_task(task_id).await
    }

    /// Record that an operator acknowledged a blocked workflow task without resuming it.
    pub async fn acknowledge_blocked_task(
        &self,
        task_id: &str,
        note: Option<String>,
    ) -> Result<(), String> {
        let run_id = self
            .task_run_index
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;
        let task = self
            .supervisor_task_record(task_id)
            .await
            .map(|record| record.task)
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        let run_snapshot = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;

            if record.state != SupervisorTaskState::Blocked {
                return Err(format!(
                    "Task '{}' is not blocked and does not require acknowledgement",
                    task_id
                ));
            }

            record.updated_at = Utc::now();
            run.updated_at = Utc::now();
            refresh_run_rollups(run);
            run.clone()
        };

        if let Some(mut checkpoint) = self.load_delegated_checkpoint_async(&task).await {
            let message = note
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "blocked state acknowledged by operator".to_string());
            checkpoint.stage = DelegatedCheckpointStage::Blocked;
            checkpoint.note = Some(message);
            checkpoint.updated_at = Utc::now();
            self.persist_delegated_checkpoint_async(&checkpoint).await?;
        }

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        Ok(())
    }

    /// Claim ownership of a queued task for an agent.
    pub async fn claim_task(&self, task_id: &str, agent_id: &str) -> Result<(), String> {
        let run_id = self
            .task_run_index
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        let run_snapshot = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;

            record.claimed_by = Some(agent_id.to_string());
            record.task.agent_id = agent_id.to_string();
            record.updated_at = Utc::now();
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            run.clone()
        };

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        Ok(())
    }

    async fn supervisor_task_record(&self, task_id: &str) -> Option<SupervisorTaskRecord> {
        let run_id = self.task_run_index.lock().await.get(task_id).cloned()?;
        let runs = self.supervisor_runs.lock().await;
        runs.get(&run_id).and_then(|run| {
            run.tasks
                .iter()
                .find(|record| record.task.id == task_id)
                .cloned()
        })
    }

    async fn remote_execution_for_task(&self, task_id: &str) -> Option<RemoteExecutionRecord> {
        self.supervisor_task_record(task_id)
            .await
            .and_then(|record| record.remote_execution)
    }

    async fn sync_remote_task_snapshot(
        &self,
        task: &DelegatedTask,
        remote_target: &RemoteAgentTarget,
        remote_task: &A2ATask,
        manifest: &[ArtifactManifestEntry],
        compatibility: RemoteExecutionCompatibility,
    ) -> Result<(), String> {
        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| format!("Task '{}' missing run_id", task.id))?;
        let run_snapshot = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task.id)
                .ok_or_else(|| format!("Task '{}' not found in run", task.id))?;
            record.remote_execution = Some(RemoteExecutionRecord {
                target: remote_target.clone(),
                remote_task_id: remote_task.id.clone(),
                status: a2a_status_label(remote_task.status),
                status_reason: remote_task.status_reason.clone(),
                lease: remote_task.lease.clone(),
                progress: progress_from_remote(remote_task.progress.as_ref()),
                artifacts: artifacts_from_manifest(manifest),
                provenance: remote_task
                    .provenance
                    .clone()
                    .or_else(|| provenance_from_metadata(&remote_task.metadata)),
                compatibility,
                last_synced_at: Utc::now(),
            });
            if matches!(remote_task.status, A2ATaskStatus::Blocked) {
                record.state = SupervisorTaskState::Blocked;
                record.blocked_reasons = vec![
                    remote_task
                        .status_reason
                        .clone()
                        .unwrap_or_else(|| "Remote execution blocked".to_string()),
                ];
            }
            record.updated_at = Utc::now();
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            run.clone()
        };
        self.persist_run_with_hierarchy_sync(run_snapshot.clone())
            .await?;
        let observer = { self.observer.read().await.clone() };
        if let Some(observer) = observer.as_ref() {
            observer.on_run_updated(run_snapshot).await;
        }
        Ok(())
    }

    async fn sync_local_execution_progress(
        &self,
        task: &DelegatedTask,
        mut progress: LocalExecutionProgress,
    ) -> Result<(), String> {
        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| format!("Task '{}' missing run_id", task.id))?;
        let (run_snapshot, task_snapshot) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let task_index = run
                .tasks
                .iter()
                .position(|record| record.task.id == task.id)
                .ok_or_else(|| format!("Task '{}' not found in run", task.id))?;
            let now = Utc::now();
            {
                let record = &mut run.tasks[task_index];
                progress.environment =
                    Some(environment_snapshot_from_execution(&record.environment));
                let local_execution = record
                    .local_execution
                    .get_or_insert_with(local_execution_record_for_start);
                local_execution.status = local_execution_status_label(record.state).to_string();
                local_execution.status_reason = None;
                local_execution.progress = Some(progress);
                local_execution.last_synced_at = now;
                record.updated_at = now;
            }
            run.updated_at = now;
            run.status = recalculate_run_status(run);
            (run.clone(), run.tasks[task_index].clone())
        };

        record_task_progress(task, &task_snapshot, &run_snapshot);
        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        Ok(())
    }

    /// Record a structured team message.
    pub async fn send_team_message(
        &self,
        run_id: &str,
        task_id: Option<String>,
        kind: TeamMessageKind,
        sender_agent_id: Option<String>,
        recipient_agent_id: Option<String>,
        content: impl Into<String>,
    ) -> Result<TeamMessage, String> {
        self.send_team_message_draft(
            run_id,
            TeamMessageDraft {
                task_id,
                kind,
                sender_agent_id,
                recipient_agent_id,
                content: content.into(),
                thread_id: None,
                reply_to_message_id: None,
                action_request: None,
                escalation: None,
                unread_by_agent_ids: Vec::new(),
            },
        )
        .await
    }

    /// Record a structured collaboration message using the richer draft payload.
    pub async fn send_team_message_draft(
        &self,
        run_id: &str,
        draft: TeamMessageDraft,
    ) -> Result<TeamMessage, String> {
        let message = draft.into_message(run_id.to_string());
        let thread_id = message.effective_thread_id().to_string();

        let run_snapshot = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            insert_team_message(run, message.clone())?;
            apply_collaboration_retention(run);
            run.clone()
        };

        build_team_threads_with_options(&collect_team_messages(&run_snapshot), true)
            .into_iter()
            .find(|thread| thread.id == thread_id)
            .ok_or_else(|| format!("Thread '{}' not found after recording message", thread_id))?;

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        self.capture_shared_cognition_from_message(run_id, &message)
            .await;
        self.notify_team_message(message.clone()).await;
        Ok(message)
    }

    async fn capture_shared_cognition_from_message(&self, run_id: &str, message: &TeamMessage) {
        let run_snapshot = {
            let runs = self.supervisor_runs.lock().await;
            runs.get(run_id).cloned()
        };

        let Some(run_snapshot) = run_snapshot else {
            return;
        };
        let Some(kind) = shared_cognition_kind_for_message(&run_snapshot, message) else {
            return;
        };

        let task_record = message.task_id.as_deref().and_then(|task_id| {
            run_snapshot
                .tasks
                .iter()
                .find(|record| record.task.id == task_id)
        });
        let directive_id = task_record
            .and_then(|record| record.task.directive_id.clone())
            .or_else(|| common_directive_id_for_run(&run_snapshot));
        let mut tags = task_record
            .map(|record| record.task.memory_tags.clone())
            .unwrap_or_default();
        push_unique_tag(&mut tags, SHARED_COGNITION_TAG);
        push_unique_tag(&mut tags, workflow_run_memory_tag(run_id));
        push_unique_tag(&mut tags, shared_cognition_kind_tag(kind));

        let summary = summarize_shared_cognition(&message.content);
        let detail = message.content.trim().to_string();
        let confidence = shared_cognition_confidence(kind);

        let note = SharedCognitionNote {
            id: Uuid::new_v4().to_string(),
            run_id: run_id.to_string(),
            task_id: message.task_id.clone(),
            directive_id: directive_id.clone(),
            kind,
            message_kind: message.kind,
            summary: summary.clone(),
            detail: detail.clone(),
            sender_agent_id: message.sender_agent_id.clone(),
            recipient_agent_id: message.recipient_agent_id.clone(),
            tags: tags.clone(),
            confidence,
            source_message_id: message.id.clone(),
            created_at: message.created_at,
        };

        let memory_path = if let Some(workspace_dir) = run_snapshot.workspace_dir.as_deref() {
            let scope = if note.task_id.is_some() {
                MemoryScope::Task
            } else if note.directive_id.is_some() {
                MemoryScope::Directive
            } else if run_snapshot.session_id.is_some() {
                MemoryScope::Session
            } else {
                MemoryScope::Workspace
            };

            let mut content_lines = vec![
                format!(
                    "Shared cognition ({:?}) for workflow run {}",
                    note.kind, run_snapshot.id
                ),
                format!("Source message kind: {:?}", note.message_kind),
            ];
            if let Some(task_id) = note.task_id.as_deref() {
                content_lines.push(format!("Task: {task_id}"));
            }
            if let Some(directive_id) = note.directive_id.as_deref() {
                content_lines.push(format!("Directive: {directive_id}"));
            }
            if let Some(sender) = note.sender_agent_id.as_deref() {
                content_lines.push(format!("Sender: {sender}"));
            }
            if let Some(recipient) = note.recipient_agent_id.as_deref() {
                content_lines.push(format!("Recipient: {recipient}"));
            }
            content_lines.push(String::new());
            content_lines.push(detail.clone());

            let mut entry = MemoryBankEntry::new(
                run_snapshot
                    .session_id
                    .clone()
                    .unwrap_or_else(|| format!("workflow-run-{}", run_snapshot.id)),
                summary,
                content_lines.join("\n"),
            )
            .with_memory_type(shared_cognition_memory_type(kind))
            .with_scope(scope)
            .with_confidence(confidence)
            .with_provenance(
                note.task_id.clone(),
                note.directive_id.clone(),
                note.sender_agent_id.clone(),
            )
            .with_tags(tags);
            entry.category = Some(SHARED_COGNITION_CATEGORY.to_string());

            match crate::save_to_memory_bank(workspace_dir, &entry).await {
                Ok(path) => Some(path),
                Err(error) => {
                    tracing::warn!(
                        run_id = %run_snapshot.id,
                        message_id = %message.id,
                        error = %error,
                        "Failed to persist shared cognition memory"
                    );
                    None
                }
            }
        } else {
            None
        };

        if let Some(task_record) = task_record
            && let (Some(session_id), Some(tracking_task_id)) = (
                task_record.task.session_id.as_deref(),
                task_record.task.tracking_task_id.as_deref(),
            )
        {
            let phase = match kind {
                SharedCognitionKind::Blocker => crate::tasks::TaskMemoryPhase::Blocked,
                SharedCognitionKind::Handoff => crate::tasks::TaskMemoryPhase::Handoff,
                SharedCognitionKind::Discovery
                | SharedCognitionKind::Hypothesis
                | SharedCognitionKind::Steering
                | SharedCognitionKind::Decision => crate::tasks::TaskMemoryPhase::Promoted,
            };
            let manager = crate::get_global_task_manager();
            let _ = manager.record_memory_event(
                session_id,
                tracking_task_id,
                crate::tasks::TaskMemoryEvent::new(
                    phase,
                    format!("Shared cognition: {}", note.summary),
                    Some(format!("{:?}", kind).to_lowercase()),
                    Some(format!("{:?}", shared_cognition_memory_type(kind)).to_lowercase()),
                    memory_path.as_ref().map(|path| path.display().to_string()),
                ),
            );
        }

        let updated_run = {
            let mut runs = self.supervisor_runs.lock().await;
            let Some(run) = runs.get_mut(run_id) else {
                return;
            };
            if run
                .shared_cognition
                .iter()
                .any(|existing| existing.source_message_id == note.source_message_id)
            {
                return;
            }
            run.shared_cognition.push(note);
            run.updated_at = Utc::now();
            run.clone()
        };

        if let Err(error) = self.persist_run_with_hierarchy_sync(updated_run).await {
            tracing::warn!(
                run_id = %run_id,
                message_id = %message.id,
                error = %error,
                "Failed to persist shared cognition note on supervisor run"
            );
        }
    }

    /// Update the latest actionable request in a collaboration thread.
    pub async fn update_team_thread_action(
        &self,
        run_id: &str,
        thread_id: &str,
        status: CollaborationActionStatus,
        actor_id: Option<String>,
        note: Option<String>,
    ) -> Result<TeamThread, String> {
        let (run_snapshot, thread, action_reply) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;

            let (task_id, reply_to_message_id) = resolve_team_thread_action_request(
                run,
                thread_id,
                status,
                actor_id.clone(),
                note.clone(),
            )?;

            let reply_kind = match status {
                CollaborationActionStatus::NeedsRevision => TeamMessageKind::ReviewFeedback,
                _ => TeamMessageKind::StatusUpdate,
            };
            let reply_content = collaboration_status_message(status, note.clone());
            let action_reply = TeamMessage::new(
                run_id.to_string(),
                task_id,
                reply_kind,
                actor_id.clone(),
                None,
                reply_content,
            )
            .with_thread(thread_id.to_string(), Some(reply_to_message_id));
            insert_team_message(run, action_reply.clone())?;
            apply_collaboration_retention(run);
            let run_snapshot = run.clone();
            let thread =
                build_team_threads_with_options(&collect_team_messages(&run_snapshot), true)
                    .into_iter()
                    .find(|thread| thread.id == thread_id)
                    .ok_or_else(|| format!("Thread '{}' not found", thread_id))?;
            (run_snapshot, thread, action_reply)
        };

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        self.notify_team_message(action_reply).await;
        Ok(thread)
    }

    /// Archive an existing collaboration thread.
    pub async fn archive_team_thread(
        &self,
        run_id: &str,
        thread_id: &str,
        actor_id: Option<String>,
        note: Option<String>,
    ) -> Result<TeamThread, String> {
        let (run_snapshot, thread) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;

            archive_team_thread_messages(run, thread_id, actor_id, note)?;
            apply_collaboration_retention(run);
            let run_snapshot = run.clone();
            let thread =
                build_team_threads_with_options(&collect_team_messages(&run_snapshot), true)
                    .into_iter()
                    .find(|thread| thread.id == thread_id)
                    .ok_or_else(|| format!("Thread '{}' not found", thread_id))?;
            (run_snapshot, thread)
        };

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        self.notify_team_thread_updated(thread.clone()).await;
        Ok(thread)
    }

    /// Cancel a running task.
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), String> {
        let task_control = { self.active_tasks.lock().await.get(task_id).cloned() };

        if let Some(task_control) = task_control {
            let task = task_control.task.clone();
            tracing::info!(task_id = %task_id, agent_id = %task.agent_id, "Cancelling task");
            if matches!(task.execution_mode, AgentExecutionMode::Remote) {
                self.active_tasks.lock().await.remove(task_id);
                if let (Some(remote_target), Some(remote_execution)) = (
                    task.remote_target.clone(),
                    self.remote_execution_for_task(task_id).await,
                ) {
                    let client = match remote_target.auth_token {
                        Some(token) => A2AClient::with_auth(token),
                        None => A2AClient::new(),
                    };
                    if let Err(error) = client
                        .cancel_task(&remote_target.url, &remote_execution.remote_task_id)
                        .await
                    {
                        tracing::warn!(task_id = %task_id, error = %error, "Failed to cancel remote A2A task");
                    }
                }
            } else if let Some(cancel_token) = task_control.local_cancel_token {
                cancel_token.cancel();
                self.agent_manager
                    .send_event(&task.agent_id, format!("cancel:{}", task_id))
                    .await;
            } else {
                return Err(format!(
                    "Task '{}' is active locally but missing a cancellation handle",
                    task_id
                ));
            }

            if let Some(run_id) = self.task_run_index.lock().await.get(task_id).cloned() {
                let mut run_snapshot = {
                    let mut runs = self.supervisor_runs.lock().await;
                    let run = runs
                        .get_mut(&run_id)
                        .ok_or_else(|| format!("Run '{}' not found", run_id))?;
                    if let Some(record) = run
                        .tasks
                        .iter_mut()
                        .find(|record| record.task.id == task_id)
                    {
                        record.state = SupervisorTaskState::Cancelled;
                        record.completed_at = Some(Utc::now());
                        record.updated_at = Utc::now();
                    }
                    run.updated_at = Utc::now();
                    run.status = recalculate_run_status(run);
                    run.clone()
                };
                if let Some(environment_id) = run_snapshot
                    .tasks
                    .iter()
                    .find(|record| record.task.id == task_id)
                    .map(|record| record.environment_id.clone())
                    && let Some(environment) = self
                        .finalize_environment_for_task(&environment_id, false, true)
                        .await?
                {
                    self.update_environment_in_runs(&environment).await?;
                    if let Some(run) = self.supervisor_runs.lock().await.get(&run_id).cloned() {
                        run_snapshot = run;
                    }
                }
                self.persist_run_with_hierarchy_sync(run_snapshot).await?;
            }
            Ok(())
        } else {
            Err(format!("Task '{}' not found", task_id))
        }
    }

    /// Pause a running local delegated task and preserve resumable checkpoint state.
    pub async fn pause_task(&self, task_id: &str) -> Result<(), String> {
        let task_control = self
            .active_tasks
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        if matches!(task_control.task.execution_mode, AgentExecutionMode::Remote) {
            return Err(format!(
                "Task '{}' is running remotely and cannot be paused locally",
                task_id
            ));
        }

        let cancel_token = task_control.local_cancel_token.ok_or_else(|| {
            format!(
                "Task '{}' is active locally but missing a pause control handle",
                task_id
            )
        })?;

        tracing::info!(
            task_id = %task_id,
            agent_id = %task_control.task.agent_id,
            "Pausing local delegated task"
        );
        cancel_token.pause();
        self.agent_manager
            .send_event(&task_control.task.agent_id, format!("pause:{}", task_id))
            .await;
        Ok(())
    }

    /// Shutdown all subagents gracefully.
    pub async fn shutdown_all(&self, grace_secs: u64) {
        tracing::info!(grace_secs = grace_secs, "Shutting down all subagents");
        self.agent_manager.shutdown_all(grace_secs).await;
    }

    async fn ensure_agent_exists(&self, task: &DelegatedTask) -> Result<(), String> {
        if self
            .agent_manager
            .get_agent_status(&task.agent_id)
            .await
            .is_none()
        {
            let role = task.role.clone().unwrap_or_default();
            let mut request = AgentSpawnRequest::new(
                task.agent_id.clone(),
                format!("{}-{}", role.label().replace(' ', "-"), task.agent_id),
                role.clone(),
            );
            request.workspace_dir = task.workspace_dir.clone();
            request.execution_mode = task.execution_mode.clone();
            self.spawn_subagent_with_request(request).await?;
        }
        Ok(())
    }

    async fn ensure_tracking_task(&self, task: &mut DelegatedTask) {
        let Some(session_id) = task.session_id.as_deref() else {
            return;
        };
        if task.tracking_task_id.is_some() {
            return;
        }

        let manager = crate::get_global_task_manager();
        let name = task
            .name
            .clone()
            .unwrap_or_else(|| format!("Delegated: {}", task.agent_id));
        let description = task
            .delegation_brief
            .as_ref()
            .map(|brief| brief.objective.clone())
            .unwrap_or_else(|| task.prompt.clone());
        if let Ok(created) = manager.create_orchestrator_task(
            session_id,
            task.id.clone(),
            task.agent_id.clone(),
            name,
            description,
            task.context.clone(),
        ) {
            task.tracking_task_id = Some(created.id);
        }
    }

    async fn start_task_execution(&self, task: DelegatedTask) -> Result<(), String> {
        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| "Task is missing run_id".to_string())?;
        let environment_id = task
            .environment_id
            .clone()
            .ok_or_else(|| format!("Task '{}' is missing environment_id", task.id))?;
        let leased_environment = self
            .acquire_environment_lease(&environment_id, &task.id, &task.agent_id)
            .await?;

        let local_cancel_token = (!matches!(task.execution_mode, AgentExecutionMode::Remote))
            .then(CancellationToken::new);

        let (run_snapshot, task_snapshot, attempt) = {
            let mut active = self.active_tasks.lock().await;
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let task_snapshot = {
                let record = run
                    .tasks
                    .iter_mut()
                    .find(|record| record.task.id == task.id)
                    .ok_or_else(|| format!("Task '{}' not found in run", task.id))?;

                record.state = SupervisorTaskState::Running;
                record.started_at = Some(Utc::now());
                record.updated_at = Utc::now();
                record.blocked_reasons.clear();
                record.environment = leased_environment.summary();
                record.local_execution =
                    if matches!(task.execution_mode, AgentExecutionMode::Remote) {
                        None
                    } else {
                        Some(local_execution_record_for_start())
                    };
                record.clone()
            };
            let attempt = task_snapshot.attempts;

            active.insert(
                task.id.clone(),
                ActiveTaskControl {
                    task: task.clone(),
                    local_cancel_token: local_cancel_token.clone(),
                    attempt,
                },
            );

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);

            (run.clone(), task_snapshot, attempt)
        };

        record_task_dispatch(&task, &task_snapshot, &run_snapshot);
        self.persist_run_with_hierarchy_sync(run_snapshot.clone())
            .await?;
        if !matches!(task.execution_mode, AgentExecutionMode::Remote) {
            self.persist_delegated_checkpoint_async(&delegated_start_checkpoint(&task))
                .await?;
        }

        let observer_for_start = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_start.as_ref() {
            obs.on_task_started(task.clone()).await;
        }

        let orchestrator = self.clone();
        let execution_span = tracing::info_span!(
            "delegated_task_execution",
            run_id = %run_id,
            task_id = %task.id,
            agent_id = %task.agent_id,
            execution_mode = ?task.execution_mode,
            tracking_task_id = %task.tracking_task_id.as_deref().unwrap_or("n/a"),
        );
        tokio::spawn(
            async move {
                let start = std::time::Instant::now();
                let (task_result, preserve_existing_checkpoint) =
                    if matches!(task.execution_mode, AgentExecutionMode::Remote) {
                        (orchestrator.execute_remote_task(&task, start).await, false)
                    } else {
                        let execution = execute_delegated_task(
                            &orchestrator,
                            &task,
                            local_cancel_token
                                .clone()
                                .expect("local delegated task missing cancellation token"),
                        )
                        .await;
                        let duration_ms = start.elapsed().as_millis() as u64;
                        let preserve_existing_checkpoint = execution.preserve_existing_checkpoint;
                        (
                            execution.into_task_result(&task, duration_ms),
                            preserve_existing_checkpoint,
                        )
                    };

                if let Err(error) = orchestrator
                    .complete_task_execution(
                        task,
                        task_result,
                        preserve_existing_checkpoint,
                        attempt,
                    )
                    .await
                {
                    tracing::error!(error = %error, "Failed to finalize delegated task execution");
                }
            }
            .instrument(execution_span),
        );

        Ok(())
    }

    async fn execute_remote_task(
        &self,
        task: &DelegatedTask,
        start: std::time::Instant,
    ) -> TaskResult {
        let duration_ms = || start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let Some(remote_target) = task.remote_target.clone() else {
            return TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: false,
                run_id: task.run_id.clone(),
                tracking_task_id: task.tracking_task_id.clone(),
                output: "Task is marked remote without a remote target".to_string(),
                summary: task.name.clone(),
                tool_calls: vec![],
                artifacts: vec![],
                terminal_state_hint: Some(TaskTerminalStateHint::Failed),
                duration_ms: duration_ms(),
            };
        };

        let client = match remote_target.auth_token.as_ref() {
            Some(token) => A2AClient::with_auth(token.clone()),
            None => A2AClient::new(),
        };
        let remote_card = match client.discover(&remote_target.url).await {
            Ok(card) => card,
            Err(error) => {
                return TaskResult {
                    task_id: task.id.clone(),
                    agent_id: task.agent_id.clone(),
                    success: false,
                    run_id: task.run_id.clone(),
                    tracking_task_id: task.tracking_task_id.clone(),
                    output: format!(
                        "Failed to discover remote agent at {}: {error}",
                        remote_target.url
                    ),
                    summary: task.name.clone(),
                    tool_calls: vec![],
                    artifacts: vec![],
                    terminal_state_hint: Some(TaskTerminalStateHint::Blocked),
                    duration_ms: duration_ms(),
                };
            }
        };

        let compatibility = compatibility_from_card(&remote_card, &remote_target);
        if remote_card.authentication.is_some() && remote_target.auth_token.is_none() {
            return TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: false,
                run_id: task.run_id.clone(),
                tracking_task_id: task.tracking_task_id.clone(),
                output: format!(
                    "Remote agent at {} requires authentication, but no auth token is configured",
                    remote_target.url
                ),
                summary: task.name.clone(),
                tool_calls: vec![],
                artifacts: vec![],
                terminal_state_hint: Some(TaskTerminalStateHint::Blocked),
                duration_ms: duration_ms(),
            };
        }
        let request = match self.supervisor_task_record(&task.id).await {
            Some(record) => build_remote_task_request(task, &record, &compatibility),
            None => {
                return TaskResult {
                    task_id: task.id.clone(),
                    agent_id: task.agent_id.clone(),
                    success: false,
                    run_id: task.run_id.clone(),
                    tracking_task_id: task.tracking_task_id.clone(),
                    output: format!("Missing supervisor record for remote task {}", task.id),
                    summary: task.name.clone(),
                    tool_calls: vec![],
                    artifacts: vec![],
                    terminal_state_hint: Some(TaskTerminalStateHint::Failed),
                    duration_ms: duration_ms(),
                };
            }
        };

        let remote_task = match client
            .create_task_with_request(&remote_target.url, request)
            .await
        {
            Ok(task_state) => task_state,
            Err(error) => {
                return TaskResult {
                    task_id: task.id.clone(),
                    agent_id: task.agent_id.clone(),
                    success: false,
                    run_id: task.run_id.clone(),
                    tracking_task_id: task.tracking_task_id.clone(),
                    output: format!(
                        "Failed to create remote task on {}: {error}",
                        remote_target.url
                    ),
                    summary: task.name.clone(),
                    tool_calls: vec![],
                    artifacts: vec![],
                    terminal_state_hint: Some(TaskTerminalStateHint::Blocked),
                    duration_ms: duration_ms(),
                };
            }
        };

        let mut manifest = if remote_card
            .supported_rpc_methods
            .iter()
            .any(|method| method == "task/artifacts")
        {
            client
                .list_task_artifacts(&remote_target.url, &remote_task.id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Err(error) = self
            .sync_remote_task_snapshot(
                task,
                &remote_target,
                &remote_task,
                &manifest,
                compatibility.clone(),
            )
            .await
        {
            tracing::warn!(task_id = %task.id, error = %error, "Failed to persist initial remote task snapshot");
        }

        let mut current = remote_task;
        loop {
            match current.status {
                A2ATaskStatus::Completed => {
                    return TaskResult {
                        task_id: task.id.clone(),
                        agent_id: task.agent_id.clone(),
                        success: true,
                        run_id: task.run_id.clone(),
                        tracking_task_id: task.tracking_task_id.clone(),
                        output: summarize_remote_task_output(&current),
                        summary: task.name.clone(),
                        tool_calls: vec![],
                        artifacts: task_artifacts_from_remote_payload(&current.artifacts),
                        terminal_state_hint: task_terminal_hint_for_a2a_status(current.status),
                        duration_ms: duration_ms(),
                    };
                }
                A2ATaskStatus::Failed | A2ATaskStatus::Cancelled | A2ATaskStatus::Blocked => {
                    return TaskResult {
                        task_id: task.id.clone(),
                        agent_id: task.agent_id.clone(),
                        success: false,
                        run_id: task.run_id.clone(),
                        tracking_task_id: task.tracking_task_id.clone(),
                        output: summarize_remote_task_output(&current),
                        summary: task.name.clone(),
                        tool_calls: vec![],
                        artifacts: task_artifacts_from_remote_payload(&current.artifacts),
                        terminal_state_hint: task_terminal_hint_for_a2a_status(current.status),
                        duration_ms: duration_ms(),
                    };
                }
                _ => {}
            }

            sleep(TokioDuration::from_secs(2)).await;
            current = match client
                .get_task_status(&remote_target.url, &current.id)
                .await
            {
                Ok(task_state) => task_state,
                Err(error) => {
                    return TaskResult {
                        task_id: task.id.clone(),
                        agent_id: task.agent_id.clone(),
                        success: false,
                        run_id: task.run_id.clone(),
                        tracking_task_id: task.tracking_task_id.clone(),
                        output: format!("Failed to refresh remote task {}: {error}", current.id),
                        summary: task.name.clone(),
                        tool_calls: vec![],
                        artifacts: vec![],
                        terminal_state_hint: Some(TaskTerminalStateHint::Blocked),
                        duration_ms: duration_ms(),
                    };
                }
            };
            if remote_card
                .supported_rpc_methods
                .iter()
                .any(|method| method == "task/artifacts")
            {
                manifest = client
                    .list_task_artifacts(&remote_target.url, &current.id)
                    .await
                    .unwrap_or_default();
            }
            if let Err(error) = self
                .sync_remote_task_snapshot(
                    task,
                    &remote_target,
                    &current,
                    &manifest,
                    compatibility.clone(),
                )
                .await
            {
                tracing::warn!(task_id = %task.id, error = %error, "Failed to sync remote task snapshot");
            }
        }
    }

    async fn complete_task_execution(
        &self,
        task: DelegatedTask,
        task_result: TaskResult,
        preserve_existing_checkpoint: bool,
        expected_attempt: u32,
    ) -> Result<(), String> {
        let run_id = task_result
            .run_id
            .clone()
            .or_else(|| task.run_id.clone())
            .ok_or_else(|| "Completed task result is missing run_id".to_string())?;

        let (
            mut run_snapshot,
            task_snapshot,
            tasks_to_start,
            environment_id,
            finalized_state,
            gate_message,
        ) = {
            let mut active = self.active_tasks.lock().await;

            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task.id)
                .ok_or_else(|| format!("Task '{}' not found in run", task.id))?;

            if record.attempts != expected_attempt {
                tracing::debug!(
                    task_id = %task.id,
                    run_id = %run_id,
                    expected_attempt,
                    actual_attempt = record.attempts,
                    "Ignoring stale delegated task completion after retry"
                );
                return Ok(());
            }

            if let Some(control) = active.get(&task.id)
                && control.attempt != expected_attempt
            {
                tracing::debug!(
                    task_id = %task.id,
                    run_id = %run_id,
                    expected_attempt,
                    active_attempt = control.attempt,
                    "Ignoring stale delegated task completion for superseded active attempt"
                );
                return Ok(());
            }

            active.remove(&task.id);

            if record.result.is_some()
                && record.completed_at.is_some()
                && matches!(
                    record.state,
                    SupervisorTaskState::Completed
                        | SupervisorTaskState::Failed
                        | SupervisorTaskState::Cancelled
                        | SupervisorTaskState::Blocked
                        | SupervisorTaskState::ReviewPending
                        | SupervisorTaskState::TestPending
                )
            {
                tracing::debug!(
                    task_id = %task.id,
                    run_id = %run_id,
                    state = ?record.state,
                    "Ignoring duplicate terminal task completion"
                );
                return Ok(());
            }

            record.result = Some(task_result.clone());
            record.completed_at = Some(Utc::now());
            record.updated_at = Utc::now();
            let previous_local_execution = record.local_execution.clone();
            let mut gate_message = None;
            let hinted_terminal_state = task_result.terminal_state_hint.map(|hint| match hint {
                TaskTerminalStateHint::Completed => SupervisorTaskState::Completed,
                TaskTerminalStateHint::Failed => SupervisorTaskState::Failed,
                TaskTerminalStateHint::Cancelled => SupervisorTaskState::Cancelled,
                TaskTerminalStateHint::Blocked => SupervisorTaskState::Blocked,
            });

            if task_result.success {
                if record.task.reviewer_required {
                    record.state = SupervisorTaskState::ReviewPending;
                    record.approval.request(
                        ApprovalScope::Review,
                        ApprovalActor::system("orchestrator"),
                        Some("Execution finished. Awaiting explicit review approval.".to_string()),
                    );
                    let message =
                        build_gate_request_message(&run.id, record, ApprovalScope::Review);
                    record.messages.push(message.clone());
                    gate_message = Some(message);
                } else if record.task.test_required {
                    record.state = SupervisorTaskState::TestPending;
                    record.approval.request(
                        ApprovalScope::TestValidation,
                        ApprovalActor::system("orchestrator"),
                        Some("Execution finished. Awaiting explicit test validation.".to_string()),
                    );
                    let message =
                        build_gate_request_message(&run.id, record, ApprovalScope::TestValidation);
                    record.messages.push(message.clone());
                    gate_message = Some(message);
                } else {
                    record.state = SupervisorTaskState::Completed;
                    if matches!(record.approval.state, ApprovalState::Pending) {
                        record.approval.reset_for_task(&record.task);
                        record.approval.note = Some(
                            "Execution completed after clearing a stale pending approval state."
                                .into(),
                        );
                    }
                }
            } else {
                record.state = hinted_terminal_state.unwrap_or(SupervisorTaskState::Failed);
                record.blocked_reasons = if matches!(record.state, SupervisorTaskState::Blocked) {
                    vec![task_result.output.clone()]
                } else {
                    Vec::new()
                };
                if matches!(record.approval.state, ApprovalState::Pending) {
                    record.approval.reset_for_task(&record.task);
                    record.approval.note = Some(
                        "Execution failed after clearing a stale pending approval state.".into(),
                    );
                }
            }

            if matches!(task.execution_mode, AgentExecutionMode::Remote) {
                record.local_execution = None;
            } else {
                record.local_execution = Some(local_execution_record_for_terminal(
                    &task_result,
                    record.state,
                    previous_local_execution.as_ref(),
                ));
            }

            let (task_snapshot, environment_id, finalized_state) =
                (record.clone(), record.environment_id.clone(), record.state);
            let ready_to_start = if preserve_existing_checkpoint
                && matches!(record.state, SupervisorTaskState::Blocked)
            {
                collect_ready_tasks_except(run, Some(&task.id))
            } else {
                collect_ready_tasks(run)
            };
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (
                run.clone(),
                task_snapshot,
                ready_to_start,
                environment_id,
                finalized_state,
                gate_message,
            )
        };
        let memory_file_path = persist_delegated_task_memory(&task_snapshot).await;

        let updated_environment = match finalized_state {
            SupervisorTaskState::Completed => {
                self.finalize_environment_for_task(&environment_id, true, false)
                    .await?
            }
            SupervisorTaskState::Failed | SupervisorTaskState::Cancelled => {
                self.finalize_environment_for_task(&environment_id, false, true)
                    .await?
            }
            _ => self.release_environment_lease(&environment_id).await?,
        };
        if let Some(environment) = updated_environment {
            self.update_environment_in_runs(&environment).await?;
            if let Some(run) = self.supervisor_runs.lock().await.get(&run_id).cloned() {
                run_snapshot = run;
            }
        }

        record_task_completion(
            &task,
            &task_result,
            memory_file_path.as_deref(),
            &task_snapshot,
            &run_snapshot,
        );
        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        if let Some((workspace_dir, session_id, tracking_task_id)) =
            task_reflection_sync_context(&task_snapshot.task)
        {
            let signals = task_record_outcome_signals(&task_snapshot);
            let _ = sync_task_reflection_outcomes(
                &workspace_dir,
                &session_id,
                &tracking_task_id,
                &signals,
            )
            .await;
        }
        self.publish_delegated_task_memory_handoff(
            &run_id,
            &task_snapshot,
            &task_result,
            memory_file_path.as_deref(),
        )
        .await?;
        let should_preserve_blocked_checkpoint =
            preserve_existing_checkpoint && matches!(finalized_state, SupervisorTaskState::Blocked);
        if !matches!(task.execution_mode, AgentExecutionMode::Remote)
            && !should_preserve_blocked_checkpoint
        {
            self.persist_delegated_checkpoint_async(&delegated_terminal_checkpoint(
                &task,
                &task_result,
            ))
            .await?;
        }
        if let Some(message) = gate_message {
            self.notify_team_message(message).await;
        }

        let observer_for_complete = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_complete.as_ref() {
            obs.on_task_completed(task.clone(), task_result.clone())
                .await;
        }

        let _ = self.result_tx.send(task_result).await;

        for next_task in tasks_to_start {
            self.schedule_task_execution(next_task);
        }

        Ok(())
    }

    async fn publish_delegated_task_memory_handoff(
        &self,
        run_id: &str,
        task_record: &SupervisorTaskRecord,
        task_result: &TaskResult,
        memory_file_path: Option<&Path>,
    ) -> Result<(), String> {
        let kind = match task_record.state {
            SupervisorTaskState::Completed
            | SupervisorTaskState::ReviewPending
            | SupervisorTaskState::TestPending => TeamMessageKind::Handoff,
            SupervisorTaskState::Failed
            | SupervisorTaskState::Cancelled
            | SupervisorTaskState::Blocked => TeamMessageKind::Blocker,
            _ => return Ok(()),
        };

        let recipient_agent_id = {
            let runs = self.supervisor_runs.lock().await;
            runs.get(run_id).and_then(|run| run.lead_agent_id.clone())
        };
        let mut message = TeamMessage::new(
            run_id.to_string(),
            Some(task_record.task.id.clone()),
            kind,
            Some(task_record.task.agent_id.clone()),
            recipient_agent_id.clone(),
            build_delegated_task_memory_handoff_content(task_record, task_result, memory_file_path),
        )
        .with_result_reference(TeamResultReference::from_task_result(task_result))
        .with_artifact_references(
            task_result
                .artifacts
                .iter()
                .map(|artifact| {
                    TeamArtifactReference::from_task_artifact(
                        Some(task_record.task.id.clone()),
                        artifact,
                    )
                })
                .collect(),
        );

        let unread_by_agent_ids = recipient_agent_id
            .into_iter()
            .filter(|agent_id| agent_id != &task_record.task.agent_id)
            .collect::<Vec<_>>();
        if !unread_by_agent_ids.is_empty() {
            message = message.with_unread_by_agent_ids(unread_by_agent_ids);
        }

        let run_snapshot = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            insert_team_message(run, message.clone())?;
            apply_collaboration_retention(run);
            run.clone()
        };

        self.persist_run_with_hierarchy_sync(run_snapshot).await?;
        self.capture_shared_cognition_from_message(run_id, &message)
            .await;

        let memory_file_path_display = memory_file_path.map(|path| path.display().to_string());
        tracing::info!(
            run_id = %run_id,
            task_id = %task_record.task.id,
            agent_id = %task_record.task.agent_id,
            message_id = %message.id,
            message_kind = ?kind,
            memory_file_path = %memory_file_path_display.as_deref().unwrap_or("n/a"),
            "Published delegated task memory handoff"
        );

        self.notify_team_message(message).await;
        Ok(())
    }

    fn schedule_task_execution(&self, task: DelegatedTask) {
        let orchestrator = self.clone();
        let run_id = task.run_id.clone().unwrap_or_else(|| "n/a".to_string());
        let task_id = task.id.clone();
        let agent_id = task.agent_id.clone();
        let schedule_span = tracing::info_span!(
            "schedule_ready_task",
            run_id = %run_id,
            task_id = %task_id,
            agent_id = %agent_id,
        );
        tokio::spawn(
            async move {
                if let Err(error) = orchestrator.start_task_execution(task).await {
                    tracing::error!(error = %error, "Failed to start ready task");
                }
            }
            .instrument(schedule_span),
        );
    }

    fn persist_run(&self, run: &SupervisorRun) -> Result<(), String> {
        let Some(root) = run
            .workspace_dir
            .as_deref()
            .or(self.default_workspace_dir.as_deref())
        else {
            return Ok(());
        };

        persist_run_to_disk(root, run)
    }

    async fn persist_run_async(&self, run: &SupervisorRun) -> Result<(), String> {
        let Some(root) = run
            .workspace_dir
            .as_deref()
            .or(self.default_workspace_dir.as_deref())
        else {
            return Ok(());
        };

        persist_run_to_disk_async(root, run).await
    }

    fn persist_delegated_checkpoint(
        &self,
        checkpoint: &DelegatedTaskCheckpoint,
    ) -> Result<(), String> {
        let Some(root) = self
            .default_workspace_dir
            .as_deref()
            .or(checkpoint.workspace_dir.as_deref())
        else {
            return Ok(());
        };

        persist_checkpoint_to_disk(root, checkpoint)
    }

    async fn persist_delegated_checkpoint_async(
        &self,
        checkpoint: &DelegatedTaskCheckpoint,
    ) -> Result<(), String> {
        let Some(root) = self
            .default_workspace_dir
            .as_deref()
            .or(checkpoint.workspace_dir.as_deref())
        else {
            return Ok(());
        };

        persist_checkpoint_to_disk_async(root, checkpoint).await
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn load_delegated_checkpoint(&self, task: &DelegatedTask) -> Option<DelegatedTaskCheckpoint> {
        let mut checkpoints = load_latest_checkpoints_by_task(checkpoint_roots_for_task(
            task,
            self.default_workspace_dir.as_deref(),
        ));
        checkpoints.remove(&task.id)
    }

    async fn load_delegated_checkpoint_async(
        &self,
        task: &DelegatedTask,
    ) -> Option<DelegatedTaskCheckpoint> {
        let roots = checkpoint_roots_for_task(task, self.default_workspace_dir.as_deref());
        let task_id = task.id.clone();
        match tokio::task::spawn_blocking(move || {
            let mut checkpoints = load_latest_checkpoints_by_task(roots);
            checkpoints.remove(&task_id)
        })
        .await
        {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                tracing::warn!(task_id = %task.id, "Failed to join delegated checkpoint load task: {error}");
                None
            }
        }
    }

    async fn load_checkpoint_resume_state_async(
        &self,
        task: &DelegatedTask,
    ) -> Option<PausedExecutionState> {
        self.load_delegated_checkpoint_async(task)
            .await
            .and_then(|checkpoint| checkpoint.resume_state)
    }

    async fn persist_run_with_hierarchy_sync(
        &self,
        run: SupervisorRun,
    ) -> Result<Vec<SupervisorRun>, String> {
        self.persist_run_async(&run).await?;
        let mut updated_runs = vec![run.clone()];
        if let Some(parent_run) = self.sync_parent_run_from_child(&run.id).await? {
            self.persist_run_async(&parent_run).await?;
            updated_runs.push(parent_run);
        }
        for snapshot in &updated_runs {
            self.notify_run_updated(snapshot.clone()).await;
        }
        Ok(updated_runs)
    }

    async fn sync_parent_run_from_child(
        &self,
        child_run_id: &str,
    ) -> Result<Option<SupervisorRun>, String> {
        let mut runs = self.supervisor_runs.lock().await;
        let Some(child_run) = runs.get(child_run_id).cloned() else {
            return Ok(None);
        };
        let Some(parent_ref) = child_run.parent_run.as_ref() else {
            return Ok(None);
        };
        let parent = runs.get_mut(&parent_ref.parent_run_id).ok_or_else(|| {
            format!(
                "Parent supervisor run '{}' not found for child '{}'",
                parent_ref.parent_run_id, child_run_id
            )
        })?;
        upsert_child_run_summary(parent, &child_run);
        parent.updated_at = child_run.updated_at.max(parent.updated_at);
        refresh_run_rollups(parent);
        if matches!(
            parent.status,
            SupervisorRunStatus::Completed | SupervisorRunStatus::Cancelled
        ) {
            parent.completed_at.get_or_insert(Utc::now());
        } else {
            parent.completed_at = None;
        }
        Ok(Some(parent.clone()))
    }

    async fn notify_run_updated(&self, run: SupervisorRun) {
        let observer_for_run = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_run.as_ref() {
            obs.on_run_updated(run).await;
        }
    }

    async fn notify_team_message(&self, message: TeamMessage) {
        let observer_for_message = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_message.as_ref() {
            obs.on_team_message(message.clone()).await;
        }

        if let Some(thread) = self
            .list_team_threads_with_options(&message.run_id, true)
            .await
            .into_iter()
            .find(|thread| thread.id == message.effective_thread_id())
        {
            self.notify_team_thread_updated(thread).await;
        }
    }

    async fn notify_team_thread_updated(&self, thread: TeamThread) {
        let observer_for_thread = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_thread.as_ref() {
            obs.on_team_thread_updated(thread).await;
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TeamMessageLocation {
    Run(usize),
    Task {
        task_index: usize,
        message_index: usize,
    },
}

fn collect_team_messages(run: &SupervisorRun) -> Vec<TeamMessage> {
    let mut messages = Vec::with_capacity(
        run.messages.len()
            + run
                .tasks
                .iter()
                .map(|task| task.messages.len())
                .sum::<usize>(),
    );
    messages.extend(run.messages.clone());
    for task in &run.tasks {
        messages.extend(task.messages.clone());
    }
    messages.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    messages
}

fn insert_team_message(run: &mut SupervisorRun, message: TeamMessage) -> Result<(), String> {
    if let Some(task_id) = message.task_id.as_deref() {
        let record = run
            .tasks
            .iter_mut()
            .find(|record| record.task.id == task_id)
            .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;
        if matches!(message.kind, TeamMessageKind::Blocker)
            && !record
                .blocked_reasons
                .iter()
                .any(|reason| reason == &message.content)
        {
            record.blocked_reasons.push(message.content.clone());
            if !matches!(
                record.state,
                SupervisorTaskState::Completed
                    | SupervisorTaskState::Failed
                    | SupervisorTaskState::Cancelled
            ) {
                record.state = SupervisorTaskState::Blocked;
            }
        }
        record.messages.push(message);
        record.updated_at = Utc::now();
    } else {
        run.messages.push(message);
    }

    run.updated_at = Utc::now();
    run.status = recalculate_run_status(run);
    Ok(())
}

fn find_latest_thread_action_location(
    run: &SupervisorRun,
    thread_id: &str,
) -> Option<(TeamMessageLocation, DateTime<Utc>)> {
    let mut latest = None;
    for (message_index, message) in run.messages.iter().enumerate() {
        if message.effective_thread_id() == thread_id && message.action_request.is_some() {
            latest = Some((TeamMessageLocation::Run(message_index), message.created_at));
        }
    }
    for (task_index, task) in run.tasks.iter().enumerate() {
        for (message_index, message) in task.messages.iter().enumerate() {
            if message.effective_thread_id() == thread_id && message.action_request.is_some() {
                match latest {
                    Some((_, current_time)) if current_time >= message.created_at => {}
                    _ => {
                        latest = Some((
                            TeamMessageLocation::Task {
                                task_index,
                                message_index,
                            },
                            message.created_at,
                        ));
                    }
                }
            }
        }
    }
    latest
}

fn resolve_team_thread_action_request(
    run: &mut SupervisorRun,
    thread_id: &str,
    status: CollaborationActionStatus,
    actor_id: Option<String>,
    note: Option<String>,
) -> Result<(Option<String>, String), String> {
    let (location, _) = find_latest_thread_action_location(run, thread_id)
        .ok_or_else(|| format!("Thread '{}' has no actionable request", thread_id))?;
    let blocker_contents = collect_team_messages(run)
        .into_iter()
        .filter(|message| {
            message.effective_thread_id() == thread_id
                && matches!(message.kind, TeamMessageKind::Blocker)
        })
        .map(|message| message.content)
        .collect::<Vec<_>>();

    let (task_id, message_id) = match location {
        TeamMessageLocation::Run(message_index) => {
            let message = run
                .messages
                .get_mut(message_index)
                .ok_or_else(|| format!("Thread '{}' message not found", thread_id))?;
            let action_request = message
                .action_request
                .as_mut()
                .ok_or_else(|| format!("Thread '{}' has no actionable request", thread_id))?;
            action_request.resolve(status, actor_id.clone(), note.clone());
            message.unread_by_agent_ids.clear();
            (message.task_id.clone(), message.id.clone())
        }
        TeamMessageLocation::Task {
            task_index,
            message_index,
        } => {
            let record = run
                .tasks
                .get_mut(task_index)
                .ok_or_else(|| format!("Thread '{}' task not found", thread_id))?;
            let message = record
                .messages
                .get_mut(message_index)
                .ok_or_else(|| format!("Thread '{}' message not found", thread_id))?;
            let action_request = message
                .action_request
                .as_mut()
                .ok_or_else(|| format!("Thread '{}' has no actionable request", thread_id))?;
            action_request.resolve(status, actor_id.clone(), note.clone());
            message.unread_by_agent_ids.clear();
            record.updated_at = Utc::now();
            (Some(record.task.id.clone()), message.id.clone())
        }
    };

    if matches!(
        status,
        CollaborationActionStatus::Resolved | CollaborationActionStatus::Cancelled
    ) && !blocker_contents.is_empty()
        && let Some(task_id) = task_id.as_deref()
        && let Some(record) = run
            .tasks
            .iter_mut()
            .find(|record| record.task.id == task_id)
    {
        record
            .blocked_reasons
            .retain(|reason| !blocker_contents.iter().any(|content| content == reason));
        if record.blocked_reasons.is_empty() && matches!(record.state, SupervisorTaskState::Blocked)
        {
            record.state = SupervisorTaskState::Queued;
        }
        record.updated_at = Utc::now();
    }

    run.updated_at = Utc::now();
    run.status = recalculate_run_status(run);
    Ok((task_id, message_id))
}

fn archive_team_thread_messages(
    run: &mut SupervisorRun,
    thread_id: &str,
    actor_id: Option<String>,
    note: Option<String>,
) -> Result<(), String> {
    let mut found = false;
    for message in &mut run.messages {
        if message.effective_thread_id() == thread_id {
            message.archive(actor_id.clone(), note.clone());
            found = true;
        }
    }
    for task in &mut run.tasks {
        for message in &mut task.messages {
            if message.effective_thread_id() == thread_id {
                message.archive(actor_id.clone(), note.clone());
                found = true;
            }
        }
        if found {
            task.updated_at = Utc::now();
        }
    }
    if !found {
        return Err(format!("Thread '{}' not found", thread_id));
    }
    run.updated_at = Utc::now();
    run.status = recalculate_run_status(run);
    Ok(())
}

fn apply_collaboration_retention(run: &mut SupervisorRun) {
    let cutoff = Utc::now() - chrono::Duration::days(DEFAULT_RESOLVED_THREAD_RETENTION_DAYS.max(0));
    let thread_ids = build_team_threads_with_options(&collect_team_messages(run), true)
        .into_iter()
        .filter(|thread| {
            !thread.archived
                && matches!(thread.status, CollaborationThreadStatus::Resolved)
                && thread.updated_at <= cutoff
        })
        .map(|thread| thread.id)
        .collect::<Vec<_>>();

    if thread_ids.is_empty() {
        return;
    }

    for message in &mut run.messages {
        if thread_ids
            .iter()
            .any(|candidate| candidate == message.effective_thread_id())
        {
            message.archive(
                Some("orchestrator".to_string()),
                Some("Auto-archived after retention period".to_string()),
            );
        }
    }
    for task in &mut run.tasks {
        let mut touched = false;
        for message in &mut task.messages {
            if thread_ids
                .iter()
                .any(|candidate| candidate == message.effective_thread_id())
            {
                message.archive(
                    Some("orchestrator".to_string()),
                    Some("Auto-archived after retention period".to_string()),
                );
                touched = true;
            }
        }
        if touched {
            task.updated_at = Utc::now();
        }
    }
}

fn collaboration_status_message(status: CollaborationActionStatus, note: Option<String>) -> String {
    note.unwrap_or_else(|| match status {
        CollaborationActionStatus::Open => "Action request reopened.".to_string(),
        CollaborationActionStatus::Acknowledged => "Action request acknowledged.".to_string(),
        CollaborationActionStatus::Resolved => "Action request resolved.".to_string(),
        CollaborationActionStatus::NeedsRevision => {
            "Changes requested before proceeding.".to_string()
        }
        CollaborationActionStatus::Cancelled => "Action request cancelled.".to_string(),
    })
}

/// Execute a delegated task on the specified agent using the unified AgentPipeline.
///
/// Returns the final (text) result (or error string) and a structured list of tool calls.
async fn execute_delegated_task<M: OrchestratorAgentManager>(
    orchestrator: &AgentOrchestrator<M>,
    task: &DelegatedTask,
    cancel_token: CancellationToken,
) -> LocalDelegatedExecutionOutcome {
    let role = task.role.clone().unwrap_or_default();
    let mut prompt_sections = vec![role.prompt_preamble().to_string()];

    if let Some(brief) = &task.delegation_brief {
        prompt_sections.push(format!("Delegation Brief:\n{}", brief.as_prompt_section()));
    }
    if let Some(ctx) = &task.context {
        prompt_sections.push(format!(
            "Context:\n{}",
            serde_json::to_string_pretty(ctx).unwrap_or_default()
        ));
    }
    prompt_sections.push(format!("Task:\n{}", task.prompt));
    if task.planning_only {
        prompt_sections.push(
            "Execution Mode:\nPlanning only. Produce a structured plan and do not claim implementation was performed."
                .to_string(),
        );
    }

    let full_prompt = prompt_sections.join("\n\n");

    tracing::debug!(
        agent_id = %task.agent_id,
        task_id = %task.id,
        prompt_len = full_prompt.len(),
        required_tools = ?task.required_tools,
        "Executing delegated task via AgentPipeline"
    );

    // Update agent activity.
    orchestrator
        .agent_manager
        .update_activity(&task.agent_id)
        .await;

    // Build the agent request with tool filtering.
    let mut request = AgentRequest::new(&full_prompt)
        .with_streaming(true)
        .with_source(RequestSource::Orchestrator)
        .with_allowed_tools(task.required_tools.clone())
        .with_agent(task.agent_id.clone())
        .with_memory_tags(task.memory_tags.clone());

    if !task.prompt.trim().is_empty() {
        request.metadata.hints.insert(
            "requirement_detection_input".to_string(),
            task.prompt.clone(),
        );
    }

    if let Some(session_id) = task.session_id.as_deref() {
        request = request.with_session(session_id.to_string());
    }
    if let Some(directive_id) = task.directive_id.as_deref() {
        request = request.with_directive(directive_id.to_string());
    }
    if let Some(tracking_task_id) = task.tracking_task_id.as_deref() {
        request = request.with_task(tracking_task_id.to_string());
    }
    if let Some(workspace_dir) = task.workspace_dir.as_ref() {
        request = request.with_workspace(workspace_dir.clone());
    }

    if let Some(resume_state) = orchestrator.load_checkpoint_resume_state_async(task).await {
        request = request.with_resume_state(resume_state);
    }

    let pipeline = AgentPipeline::with_provider_optimized_config(orchestrator.config.clone())
        .with_knowledge(
            orchestrator_knowledge_store(),
            orchestrator_knowledge_settings(),
        );
    let (tx, mut rx) = mpsc::channel(256);
    let pipeline_handle =
        tokio::spawn(async move { pipeline.process_streaming(request, tx, cancel_token).await });

    #[derive(Default)]
    struct PendingDelegatedToolCall {
        id: String,
        name: String,
        arguments: String,
    }

    let mut partial_content = String::new();
    let mut partial_thinking = String::new();
    let mut current_iteration = 0u32;
    let mut pending_tool_call: Option<PendingDelegatedToolCall> = None;
    let mut completed_resume_tool_calls: Vec<ToolCallRecord> = Vec::new();
    let mut completed_orchestrator_tool_calls: Vec<OrchestratorToolCall> = Vec::new();
    let mut telemetry_context = LocalExecutionTelemetryContext {
        iteration: current_iteration,
        completed_tool_call_count: 0,
        ..LocalExecutionTelemetryContext::default()
    };

    while let Some(chunk) = rx.recv().await {
        match &chunk {
            StreamChunk::AgentLoopIteration { iteration } => {
                current_iteration = *iteration;
                telemetry_context.iteration = current_iteration;
                if let Some(progress) =
                    local_execution_progress_from_chunk(&chunk, &telemetry_context)
                {
                    let _ = orchestrator
                        .sync_local_execution_progress(task, progress)
                        .await;
                }
            }
            StreamChunk::Text(text) => {
                partial_content.push_str(text);
                telemetry_context.has_partial_content = !partial_content.is_empty();
                telemetry_context.partial_content_chars = partial_content.chars().count();
            }
            StreamChunk::Thinking(thinking) => {
                partial_thinking.push_str(thinking);
                telemetry_context.has_partial_thinking = !partial_thinking.is_empty();
                telemetry_context.partial_thinking_chars = partial_thinking.chars().count();
            }
            StreamChunk::ToolCallStart { id, name } => {
                pending_tool_call = Some(PendingDelegatedToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: String::new(),
                });
                telemetry_context.current_tool_name = Some(name.clone());
                if let Some(progress) =
                    local_execution_progress_from_chunk(&chunk, &telemetry_context)
                {
                    let _ = orchestrator
                        .sync_local_execution_progress(task, progress)
                        .await;
                }
            }
            StreamChunk::ToolCallArgs(args) => {
                if let Some(pending) = pending_tool_call.as_mut() {
                    pending.arguments.push_str(args);
                }
            }
            StreamChunk::ToolCallEnd => {
                if let Some(pending) = pending_tool_call.as_ref() {
                    let pending_call =
                        pending_orchestrator_tool_call(&pending.name, &pending.arguments);
                    let replay_safety = replay_safety_for_tool_call(&pending_call);
                    let checkpoint = delegated_running_checkpoint(
                        task,
                        completed_orchestrator_tool_calls.clone(),
                        build_delegated_resume_state(
                            task,
                            &full_prompt,
                            &partial_content,
                            &partial_thinking,
                            &completed_resume_tool_calls,
                            current_iteration,
                        ),
                        replay_safety,
                        restart_resume_disposition(replay_safety),
                        format!("before tool '{}' execution", pending.name),
                        Some(format!(
                            "Waiting for completion of tool '{}' during delegated execution.",
                            pending.name
                        )),
                    );
                    let _ = orchestrator
                        .persist_delegated_checkpoint_async(&checkpoint)
                        .await;
                }
            }
            StreamChunk::ToolCallResult {
                name,
                success,
                output,
                duration_ms,
            } => {
                let pending = pending_tool_call
                    .take()
                    .unwrap_or(PendingDelegatedToolCall {
                        id: format!("delegated-tool-{}", completed_resume_tool_calls.len() + 1),
                        name: name.clone(),
                        arguments: String::new(),
                    });
                let record = ToolCallRecord {
                    id: pending.id.clone(),
                    name: pending.name.clone(),
                    arguments: pending.arguments.clone(),
                    result: if *success {
                        crate::ToolResult::Success(output.clone())
                    } else {
                        crate::ToolResult::Error(output.clone())
                    },
                    duration_ms: *duration_ms,
                };
                completed_orchestrator_tool_calls.push(orchestrator_tool_call_from_record(&record));
                completed_resume_tool_calls.push(record);
                telemetry_context.current_tool_name = None;
                telemetry_context.last_completed_tool_name = Some(name.clone());
                telemetry_context.last_completed_tool_duration_ms = Some(*duration_ms);
                telemetry_context.completed_tool_call_count = completed_resume_tool_calls.len();
                let checkpoint = delegated_running_checkpoint(
                    task,
                    completed_orchestrator_tool_calls.clone(),
                    build_delegated_resume_state(
                        task,
                        &full_prompt,
                        &partial_content,
                        &partial_thinking,
                        &completed_resume_tool_calls,
                        current_iteration,
                    ),
                    DelegatedReplaySafety::CheckpointResumable,
                    DelegatedResumeDisposition::ResumeFromCheckpoint,
                    format!("after tool '{}' result", pending.name),
                    Some(format!(
                        "Persisted tool result for '{}' and captured resumable state.",
                        pending.name
                    )),
                );
                let _ = orchestrator
                    .persist_delegated_checkpoint_async(&checkpoint)
                    .await;
                if let Some(progress) =
                    local_execution_progress_from_chunk(&chunk, &telemetry_context)
                {
                    let _ = orchestrator
                        .sync_local_execution_progress(task, progress)
                        .await;
                }
            }
            StreamChunk::Paused => {
                telemetry_context.current_tool_name = None;
                if let Some(progress) =
                    local_execution_progress_from_chunk(&chunk, &telemetry_context)
                {
                    let _ = orchestrator
                        .sync_local_execution_progress(task, progress)
                        .await;
                }

                let safe_boundary_label = pending_tool_call
                    .as_ref()
                    .map(|pending| format!("before tool '{}' execution", pending.name))
                    .unwrap_or_else(|| {
                        format!("operator pause during iteration {}", current_iteration)
                    });
                let pause_note = pending_tool_call
                    .as_ref()
                    .map(|pending| {
                        format!(
                            "Paused by operator before tool '{}' execution completed; resumable checkpoint preserved.",
                            pending.name
                        )
                    })
                    .unwrap_or_else(|| {
                        "Paused by operator; resumable checkpoint preserved for local delegated task."
                            .to_string()
                    });
                let checkpoint = DelegatedTaskCheckpoint {
                    id: delegated_checkpoint_id(&task.id),
                    task_id: task.id.clone(),
                    run_id: task.run_id.clone(),
                    session_id: task.session_id.clone(),
                    agent_id: task.agent_id.clone(),
                    environment_id: task.environment_id.clone(),
                    execution_mode: task.execution_mode.clone(),
                    stage: DelegatedCheckpointStage::Blocked,
                    replay_safety: DelegatedReplaySafety::CheckpointResumable,
                    resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
                    safe_boundary_label,
                    workspace_dir: task.workspace_dir.clone(),
                    completed_tool_calls: completed_orchestrator_tool_calls.clone(),
                    result_published: false,
                    note: Some(pause_note.clone()),
                    resume_state: Some(build_delegated_resume_state(
                        task,
                        &full_prompt,
                        &partial_content,
                        &partial_thinking,
                        &completed_resume_tool_calls,
                        current_iteration,
                    )),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let _ = orchestrator
                    .persist_delegated_checkpoint_async(&checkpoint)
                    .await;
                let _ = pipeline_handle.await;

                return LocalDelegatedExecutionOutcome {
                    result: Err(pause_note),
                    tool_calls: completed_orchestrator_tool_calls,
                    terminal_state_hint: TaskTerminalStateHint::Blocked,
                    preserve_existing_checkpoint: true,
                };
            }
            StreamChunk::Cancelled => {
                telemetry_context.current_tool_name = None;
                if let Some(progress) =
                    local_execution_progress_from_chunk(&chunk, &telemetry_context)
                {
                    let _ = orchestrator
                        .sync_local_execution_progress(task, progress)
                        .await;
                }
                let _ = pipeline_handle.await;

                return LocalDelegatedExecutionOutcome {
                    result: Err("Local delegated task cancelled by operator.".to_string()),
                    tool_calls: completed_orchestrator_tool_calls,
                    terminal_state_hint: TaskTerminalStateHint::Cancelled,
                    preserve_existing_checkpoint: false,
                };
            }
            other => {
                if let StreamChunk::TokenUsageUpdate {
                    estimated,
                    limit,
                    percentage,
                    status,
                    estimated_cost,
                } = other
                {
                    telemetry_context.token_usage = Some(LocalExecutionTokenUsageSnapshot {
                        estimated_tokens: Some(*estimated),
                        limit: Some(*limit),
                        percentage: Some(*percentage),
                        status: Some(token_usage_status_label(*status).to_string()),
                        estimated_cost_usd: Some(*estimated_cost),
                        input_tokens: telemetry_context
                            .token_usage
                            .as_ref()
                            .and_then(|usage| usage.input_tokens),
                        output_tokens: telemetry_context
                            .token_usage
                            .as_ref()
                            .and_then(|usage| usage.output_tokens),
                        total_tokens: telemetry_context
                            .token_usage
                            .as_ref()
                            .and_then(|usage| usage.total_tokens),
                        model: telemetry_context
                            .token_usage
                            .as_ref()
                            .and_then(|usage| usage.model.clone()),
                        provider: telemetry_context
                            .token_usage
                            .as_ref()
                            .and_then(|usage| usage.provider.clone()),
                    });
                }
                if let StreamChunk::Done(usage) = other {
                    telemetry_context.token_usage = Some(token_usage_snapshot_from_done(
                        usage.as_ref(),
                        telemetry_context.token_usage.as_ref(),
                    ));
                    telemetry_context.current_tool_name = None;
                }
                if let StreamChunk::ToolConfirmationRequired { tool_name, .. } = other {
                    telemetry_context.current_tool_name = Some(tool_name.clone());
                }
                if let StreamChunk::ToolBlocked { tool_name, .. } = other {
                    telemetry_context.current_tool_name = Some(tool_name.clone());
                }
                if let Some(progress) =
                    local_execution_progress_from_chunk(other, &telemetry_context)
                {
                    let _ = orchestrator
                        .sync_local_execution_progress(task, progress)
                        .await;
                }
            }
        }
    }

    let result = match pipeline_handle.await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(
                agent_id = %task.agent_id,
                task_id = %task.id,
                error = %error,
                "Delegated task pipeline worker panicked"
            );
            return LocalDelegatedExecutionOutcome {
                result: Err(format!("Delegated pipeline task failed: {error}")),
                tool_calls: completed_orchestrator_tool_calls,
                terminal_state_hint: TaskTerminalStateHint::Failed,
                preserve_existing_checkpoint: false,
            };
        }
    };

    match result {
        Ok(response) => {
            let tool_calls = if completed_orchestrator_tool_calls.is_empty() {
                response
                    .tool_calls
                    .iter()
                    .map(orchestrator_tool_call_from_record)
                    .collect()
            } else {
                completed_orchestrator_tool_calls
            };

            tracing::info!(
                agent_id = %task.agent_id,
                task_id = %task.id,
                response_len = response.content.len(),
                tool_calls_count = tool_calls.len(),
                "Task completed via AgentPipeline"
            );

            LocalDelegatedExecutionOutcome {
                result: Ok(response.content),
                tool_calls,
                terminal_state_hint: TaskTerminalStateHint::Completed,
                preserve_existing_checkpoint: false,
            }
        }
        Err(e) => {
            tracing::error!(
                agent_id = %task.agent_id,
                task_id = %task.id,
                error = %e,
                tool_calls_count = completed_orchestrator_tool_calls.len(),
                "Task failed"
            );
            LocalDelegatedExecutionOutcome {
                result: Err(e.to_string()),
                tool_calls: completed_orchestrator_tool_calls,
                terminal_state_hint: TaskTerminalStateHint::Failed,
                preserve_existing_checkpoint: false,
            }
        }
    }
}

async fn persist_delegated_task_memory(record: &SupervisorTaskRecord) -> Option<PathBuf> {
    let task = &record.task;
    let task_result = record.result.as_ref()?;
    let workspace_dir = task.workspace_dir.as_deref()?;
    let session_id = task.session_id.as_deref()?;

    let outcome_signals = task_record_outcome_signals(record);
    let mut tags = task.memory_tags.clone();
    tags.extend(["delegation".to_string(), "subagent".to_string()]);
    tags.extend(
        outcome_signal_labels(&outcome_signals)
            .into_iter()
            .map(|label| format!("outcome:{label}")),
    );
    tags.sort();
    tags.dedup();

    let summary = if task_result.success {
        task.name
            .clone()
            .unwrap_or_else(|| format!("Delegated task completed by {}", task.agent_id))
    } else {
        task.name
            .clone()
            .unwrap_or_else(|| format!("Delegated task blocked on {}", task.agent_id))
    };

    let tool_calls = if task_result.tool_calls.is_empty() {
        "- No tool calls recorded".to_string()
    } else {
        task_result
            .tool_calls
            .iter()
            .map(|call| {
                format!(
                    "- {} (success: {}, {} ms)",
                    call.tool_name, call.success, call.duration_ms
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let outcome_lines = if outcome_signals.is_empty() {
        "- No durable outcome signals recorded yet".to_string()
    } else {
        outcome_signals
            .iter()
            .map(|signal| {
                let detail = signal
                    .summary
                    .as_deref()
                    .map(|summary| format!(": {summary}"))
                    .unwrap_or_default();
                format!("- {}{}", signal.kind.label(), detail)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let content = format!(
        "## Delegated Task\n- Orchestrator Task ID: {}\n- Tracking Task ID: {}\n- Agent ID: {}\n- Directive ID: {}\n\n## Prompt\n{}\n\n## Result\n{}\n\n## Tool Calls\n{}\n\n## Outcome Signals\n{}\n",
        task.id,
        task.tracking_task_id.as_deref().unwrap_or("n/a"),
        task.agent_id,
        task.directive_id.as_deref().unwrap_or("n/a"),
        task.prompt,
        task_result.output,
        tool_calls,
        outcome_lines,
    );

    let entry = MemoryBankEntry::new(session_id.to_string(), summary, content)
        .with_memory_type(if task_result.success {
            MemoryType::Handoff
        } else {
            MemoryType::Blocker
        })
        .with_scope(if task.directive_id.is_some() {
            MemoryScope::Directive
        } else {
            MemoryScope::Session
        })
        .with_category("delegation")
        .with_provenance(
            Some(
                task.tracking_task_id
                    .clone()
                    .unwrap_or_else(|| task.id.clone()),
            ),
            task.directive_id.clone(),
            Some(task.agent_id.clone()),
        )
        .with_outcome_provenance(
            outcome_signal_summary(&outcome_signals),
            outcome_signal_labels(&outcome_signals),
        )
        .with_tags(tags)
        .with_promotion(
            session_id.to_string(),
            "Delegated subagent result promoted for supervisor retrieval",
        )
        .with_confidence(match record.state {
            SupervisorTaskState::Completed => 0.90,
            SupervisorTaskState::ReviewPending | SupervisorTaskState::TestPending => 0.82,
            SupervisorTaskState::Cancelled => 0.55,
            SupervisorTaskState::Failed | SupervisorTaskState::Blocked => 0.65,
            _ => {
                if task_result.success {
                    0.80
                } else {
                    0.65
                }
            }
        });

    match crate::save_to_memory_bank(workspace_dir, &entry).await {
        Ok(path) => {
            tracing::info!(
                task_id = %task.id,
                agent_id = %task.agent_id,
                session_id = %session_id,
                memory_file_path = %path.display(),
                "Persisted delegated task memory"
            );
            Some(path)
        }
        Err(error) => {
            tracing::warn!(
                task_id = %task.id,
                agent_id = %task.agent_id,
                error = %error,
                "Failed to persist delegated task memory"
            );
            None
        }
    }
}

fn task_record_outcome_signals(record: &SupervisorTaskRecord) -> Vec<OutcomeSignal> {
    let mut signals = Vec::new();

    match record.state {
        SupervisorTaskState::ReviewPending => signals.push(
            OutcomeSignal::new(OutcomeSignalKind::ExecutionAwaitingReview)
                .with_summary("Execution finished and is waiting for review approval."),
        ),
        SupervisorTaskState::TestPending => signals.push(
            OutcomeSignal::new(OutcomeSignalKind::ExecutionAwaitingTestValidation)
                .with_summary("Execution finished and is waiting for explicit test validation."),
        ),
        SupervisorTaskState::Completed => {
            signals.push(OutcomeSignal::new(OutcomeSignalKind::TaskCompleted));
        }
        SupervisorTaskState::Failed => {
            let summary = record
                .result
                .as_ref()
                .map(|result| result.output.clone())
                .filter(|value| !value.trim().is_empty());
            signals.push(summary.map_or_else(
                || OutcomeSignal::new(OutcomeSignalKind::TaskFailed),
                |summary| OutcomeSignal::new(OutcomeSignalKind::TaskFailed).with_summary(summary),
            ));
        }
        SupervisorTaskState::Blocked => {
            let summary = record.blocked_reasons.first().cloned();
            signals.push(summary.map_or_else(
                || OutcomeSignal::new(OutcomeSignalKind::TaskBlocked),
                |summary| OutcomeSignal::new(OutcomeSignalKind::TaskBlocked).with_summary(summary),
            ));
        }
        SupervisorTaskState::Cancelled => {
            signals.push(OutcomeSignal::new(OutcomeSignalKind::TaskCancelled));
        }
        SupervisorTaskState::Queued
        | SupervisorTaskState::PendingApproval
        | SupervisorTaskState::Running => {}
    }

    for scope in [
        ApprovalScope::PreExecution,
        ApprovalScope::Review,
        ApprovalScope::TestValidation,
    ] {
        if let Some(decision) = latest_approval_decision_for_scope(&record.approval, scope) {
            let kind = match (scope, decision.decision) {
                (ApprovalScope::PreExecution, ApprovalDecisionKind::Approved) => {
                    OutcomeSignalKind::PreExecutionApproved
                }
                (ApprovalScope::PreExecution, ApprovalDecisionKind::Rejected) => {
                    OutcomeSignalKind::PreExecutionRejected
                }
                (ApprovalScope::PreExecution, ApprovalDecisionKind::NeedsRevision) => {
                    OutcomeSignalKind::PreExecutionNeedsRevision
                }
                (ApprovalScope::Review, ApprovalDecisionKind::Approved) => {
                    OutcomeSignalKind::ReviewApproved
                }
                (ApprovalScope::Review, ApprovalDecisionKind::Rejected) => {
                    OutcomeSignalKind::ReviewRejected
                }
                (ApprovalScope::Review, ApprovalDecisionKind::NeedsRevision) => {
                    OutcomeSignalKind::ReviewNeedsRevision
                }
                (ApprovalScope::TestValidation, ApprovalDecisionKind::Approved) => {
                    OutcomeSignalKind::TestValidationApproved
                }
                (ApprovalScope::TestValidation, ApprovalDecisionKind::Rejected) => {
                    OutcomeSignalKind::TestValidationRejected
                }
                (ApprovalScope::TestValidation, ApprovalDecisionKind::NeedsRevision) => {
                    OutcomeSignalKind::TestValidationNeedsRevision
                }
            };
            signals.push(decision.note.clone().map_or_else(
                || OutcomeSignal::new(kind),
                |note| OutcomeSignal::new(kind).with_summary(note),
            ));
        }
    }

    signals
}

fn latest_approval_decision_for_scope(
    approval: &TaskApprovalRecord,
    scope: ApprovalScope,
) -> Option<&ApprovalDecision> {
    approval
        .decisions
        .iter()
        .rev()
        .find(|decision| decision.scope == scope)
}

fn outcome_signal_summary(signals: &[OutcomeSignal]) -> Option<String> {
    if signals.is_empty() {
        return None;
    }

    Some(
        signals
            .iter()
            .map(|signal| {
                signal
                    .summary
                    .clone()
                    .unwrap_or_else(|| signal.kind.label().to_string())
            })
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn outcome_signal_labels(signals: &[OutcomeSignal]) -> Vec<String> {
    signals
        .iter()
        .map(|signal| signal.durable_label().to_string())
        .collect()
}

fn delegated_checkpoint_id(task_id: &str) -> String {
    format!("checkpoint-{task_id}")
}

fn delegated_start_checkpoint(task: &DelegatedTask) -> DelegatedTaskCheckpoint {
    let now = Utc::now();
    DelegatedTaskCheckpoint {
        id: delegated_checkpoint_id(&task.id),
        task_id: task.id.clone(),
        run_id: task.run_id.clone(),
        session_id: task.session_id.clone(),
        agent_id: task.agent_id.clone(),
        environment_id: task.environment_id.clone(),
        execution_mode: task.execution_mode.clone(),
        stage: DelegatedCheckpointStage::Running,
        replay_safety: DelegatedReplaySafety::CheckpointResumable,
        resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
        safe_boundary_label: "delegated task dispatch boundary".to_string(),
        workspace_dir: task.workspace_dir.clone(),
        completed_tool_calls: Vec::new(),
        result_published: false,
        note: Some("Task dispatched and awaiting local execution progress.".to_string()),
        resume_state: None,
        created_at: now,
        updated_at: now,
    }
}

fn delegated_terminal_checkpoint(
    task: &DelegatedTask,
    task_result: &TaskResult,
) -> DelegatedTaskCheckpoint {
    let now = Utc::now();
    DelegatedTaskCheckpoint {
        id: delegated_checkpoint_id(&task.id),
        task_id: task.id.clone(),
        run_id: task_result.run_id.clone().or_else(|| task.run_id.clone()),
        session_id: task.session_id.clone(),
        agent_id: task.agent_id.clone(),
        environment_id: task.environment_id.clone(),
        execution_mode: task.execution_mode.clone(),
        stage: delegated_terminal_stage(task_result),
        replay_safety: replay_safety_for_task_result(task_result),
        resume_disposition: DelegatedResumeDisposition::NotApplicable,
        safe_boundary_label: format!(
            "result persisted after {} tool call(s)",
            task_result.tool_calls.len()
        ),
        workspace_dir: task.workspace_dir.clone(),
        completed_tool_calls: task_result.tool_calls.clone(),
        result_published: true,
        note: Some(if task_result.success {
            "Task completed and terminal result was published.".to_string()
        } else {
            format!("Task finished unsuccessfully: {}", task_result.output)
        }),
        resume_state: None,
        created_at: now,
        updated_at: now,
    }
}

fn delegated_running_checkpoint(
    task: &DelegatedTask,
    completed_tool_calls: Vec<OrchestratorToolCall>,
    resume_state: PausedExecutionState,
    replay_safety: DelegatedReplaySafety,
    resume_disposition: DelegatedResumeDisposition,
    safe_boundary_label: String,
    note: Option<String>,
) -> DelegatedTaskCheckpoint {
    let now = Utc::now();
    DelegatedTaskCheckpoint {
        id: delegated_checkpoint_id(&task.id),
        task_id: task.id.clone(),
        run_id: task.run_id.clone(),
        session_id: task.session_id.clone(),
        agent_id: task.agent_id.clone(),
        environment_id: task.environment_id.clone(),
        execution_mode: task.execution_mode.clone(),
        stage: DelegatedCheckpointStage::Running,
        replay_safety,
        resume_disposition,
        safe_boundary_label,
        workspace_dir: task.workspace_dir.clone(),
        completed_tool_calls,
        result_published: false,
        note,
        resume_state: Some(resume_state),
        created_at: now,
        updated_at: now,
    }
}

fn delegated_terminal_stage(task_result: &TaskResult) -> DelegatedCheckpointStage {
    if task_result.success {
        return DelegatedCheckpointStage::Completed;
    }

    match task_result.terminal_state_hint {
        Some(TaskTerminalStateHint::Cancelled) => DelegatedCheckpointStage::Cancelled,
        Some(TaskTerminalStateHint::Blocked) => DelegatedCheckpointStage::Blocked,
        _ => DelegatedCheckpointStage::Failed,
    }
}

fn replay_safety_for_task_result(task_result: &TaskResult) -> DelegatedReplaySafety {
    if task_result.tool_calls.is_empty() {
        return DelegatedReplaySafety::CheckpointResumable;
    }

    task_result
        .tool_calls
        .iter()
        .map(replay_safety_for_tool_call)
        .max_by_key(|safety| replay_safety_rank(*safety))
        .unwrap_or(DelegatedReplaySafety::CheckpointResumable)
}

fn replay_safety_for_tool_call(tool_call: &OrchestratorToolCall) -> DelegatedReplaySafety {
    match tool_call.tool_name.as_str() {
        "web" | "web_search" | "code" => DelegatedReplaySafety::PureReadonly,
        "shell" | "screen_record" => DelegatedReplaySafety::NonReplayableSideEffect,
        "a2a" | "mcp" | "permissions" | "task" | "screenshot" => {
            DelegatedReplaySafety::OperatorGated
        }
        "file" => match tool_call
            .input
            .get("operation")
            .and_then(|value| value.as_str())
        {
            Some("read" | "list" | "search" | "tree" | "stat") => {
                DelegatedReplaySafety::PureReadonly
            }
            Some("write" | "append" | "copy" | "move" | "mkdir") => {
                DelegatedReplaySafety::OperatorGated
            }
            Some("delete" | "remove") => DelegatedReplaySafety::NonReplayableSideEffect,
            _ => DelegatedReplaySafety::OperatorGated,
        },
        "git" => match tool_call
            .input
            .get("operation")
            .or_else(|| tool_call.input.get("action"))
            .and_then(|value| value.as_str())
        {
            Some("status" | "diff" | "log" | "show" | "blame" | "branch_list") => {
                DelegatedReplaySafety::PureReadonly
            }
            Some("checkout" | "restore" | "stash_apply" | "worktree_add") => {
                DelegatedReplaySafety::OperatorGated
            }
            Some("commit" | "push" | "merge" | "rebase" | "reset" | "stash") => {
                DelegatedReplaySafety::NonReplayableSideEffect
            }
            _ => DelegatedReplaySafety::OperatorGated,
        },
        _ => DelegatedReplaySafety::OperatorGated,
    }
}

fn replay_safety_rank(safety: DelegatedReplaySafety) -> u8 {
    match safety {
        DelegatedReplaySafety::PureReadonly => 0,
        DelegatedReplaySafety::IdempotentWrite => 1,
        DelegatedReplaySafety::CheckpointResumable => 2,
        DelegatedReplaySafety::OperatorGated => 3,
        DelegatedReplaySafety::NonReplayableSideEffect => 4,
    }
}

fn restart_resume_disposition(safety: DelegatedReplaySafety) -> DelegatedResumeDisposition {
    match safety {
        DelegatedReplaySafety::PureReadonly | DelegatedReplaySafety::IdempotentWrite => {
            DelegatedResumeDisposition::RestartFromBoundary
        }
        DelegatedReplaySafety::CheckpointResumable => {
            DelegatedResumeDisposition::ResumeFromCheckpoint
        }
        DelegatedReplaySafety::OperatorGated | DelegatedReplaySafety::NonReplayableSideEffect => {
            DelegatedResumeDisposition::OperatorInterventionRequired
        }
    }
}

fn checkpoint_for_restart_recovery(
    checkpoint: &DelegatedTaskCheckpoint,
) -> DelegatedTaskCheckpoint {
    let mut updated = checkpoint.clone();
    updated.stage = DelegatedCheckpointStage::Blocked;
    updated.resume_disposition = restart_resume_disposition(updated.replay_safety);
    updated.note = Some("execution interrupted during restart".to_string());
    updated.updated_at = Utc::now();
    updated
}

fn build_delegated_resume_state(
    task: &DelegatedTask,
    original_input: &str,
    partial_content: &str,
    partial_thinking: &str,
    completed_tool_calls: &[ToolCallRecord],
    iteration: u32,
) -> PausedExecutionState {
    PausedExecutionState {
        original_input: original_input.to_string(),
        system_prompt: None,
        history: Vec::new(),
        partial_content: partial_content.to_string(),
        partial_thinking: if partial_thinking.is_empty() {
            None
        } else {
            Some(partial_thinking.to_string())
        },
        completed_tool_calls: completed_tool_calls.to_vec(),
        iteration,
        source: RequestSource::Orchestrator,
        session_id: task.session_id.clone(),
        workspace_dir: task.workspace_dir.clone(),
        model_snapshot: None,
        paused_at: Utc::now(),
    }
}

fn orchestrator_tool_call_from_record(record: &ToolCallRecord) -> OrchestratorToolCall {
    let input = serde_json::from_str(&record.arguments).unwrap_or_else(|_| json!({}));
    let (output, success) = match &record.result {
        crate::ToolResult::Success(value) => (
            serde_json::from_str(value).unwrap_or_else(|_| json!({ "result": value })),
            true,
        ),
        crate::ToolResult::Error(error) => (json!({ "error": error }), false),
        crate::ToolResult::Skipped(reason) => (json!({ "skipped": reason }), false),
    };

    OrchestratorToolCall {
        tool_name: record.name.clone(),
        input,
        output,
        success,
        duration_ms: record.duration_ms,
    }
}

fn pending_orchestrator_tool_call(name: &str, arguments: &str) -> OrchestratorToolCall {
    OrchestratorToolCall {
        tool_name: name.to_string(),
        input: serde_json::from_str(arguments).unwrap_or_else(|_| json!({})),
        output: json!({}),
        success: false,
        duration_ms: 0,
    }
}

fn restart_blocked_reason_for_checkpoint(checkpoint: &DelegatedTaskCheckpoint) -> String {
    match checkpoint.resume_disposition {
        DelegatedResumeDisposition::ResumeFromCheckpoint => format!(
            "execution interrupted during restart; task can resume from checkpoint '{}'",
            checkpoint.safe_boundary_label
        ),
        DelegatedResumeDisposition::RestartFromBoundary => format!(
            "execution interrupted during restart; task can safely restart from boundary '{}'",
            checkpoint.safe_boundary_label
        ),
        DelegatedResumeDisposition::OperatorInterventionRequired => format!(
            "execution interrupted during restart; operator action required because boundary '{}' is {:?}",
            checkpoint.safe_boundary_label, checkpoint.replay_safety
        ),
        DelegatedResumeDisposition::NotApplicable => {
            "execution interrupted during restart".to_string()
        }
    }
}

fn record_task_dispatch(task: &DelegatedTask, record: &SupervisorTaskRecord, run: &SupervisorRun) {
    let Some(session_id) = task.session_id.as_deref() else {
        return;
    };
    let Some(tracking_task_id) = task.tracking_task_id.as_deref() else {
        return;
    };

    let manager = crate::get_global_task_manager();
    let task_status = match record.state {
        SupervisorTaskState::Blocked
        | SupervisorTaskState::PendingApproval
        | SupervisorTaskState::ReviewPending
        | SupervisorTaskState::TestPending => TaskStatus::Blocked,
        SupervisorTaskState::Running => TaskStatus::InProgress,
        SupervisorTaskState::Completed => TaskStatus::Completed,
        SupervisorTaskState::Cancelled | SupervisorTaskState::Failed => TaskStatus::Cancelled,
        _ => TaskStatus::NotStarted,
    };
    let _ = manager.update_task_status(session_id, tracking_task_id, task_status);
    let _ = manager.set_task_background_job(
        session_id,
        tracking_task_id,
        Some(TaskBackgroundJob::new(
            background_status_for_state(record.state),
            Some(task.id.clone()),
            Some(background_message_for_record(record)),
        )),
    );
    let _ = manager.record_memory_event(
        session_id,
        tracking_task_id,
        crate::tasks::TaskMemoryEvent::new(
            crate::tasks::TaskMemoryPhase::Delegated,
            format!("Delegated to agent {} as {:?}", task.agent_id, task.role),
            task.directive_id.as_ref().map(|_| "directive".to_string()),
            Some("handoff".to_string()),
            None,
        ),
    );
    merge_task_metadata(
        manager,
        session_id,
        tracking_task_id,
        json!({
            "delegation": {
                "run_id": task.run_id,
                "orchestrator_task_id": task.id,
                "agent_id": task.agent_id,
                "directive_id": task.directive_id,
                "role": task.role,
                "state": format!("{:?}", record.state).to_lowercase(),
                "approval_state": format!("{:?}", record.approval.state).to_lowercase(),
                "approval": {
                    "scope": record.approval.scope.map(|value| format!("{:?}", value).to_lowercase()),
                    "requested_at": record.approval.requested_at,
                    "decided_at": record.approval.decided_at,
                    "decided_by": record.approval.decided_by,
                    "note": record.approval.note,
                    "active_request": record.approval.active_request.as_ref(),
                    "latest_decision": record.approval.latest_decision(),
                    "policy": {
                        "pre_execution": {
                            "required": record.approval.policy.pre_execution.required,
                            "allowed_deciders": record.approval.policy.pre_execution.allowed_deciders,
                        },
                        "review": {
                            "required": record.approval.policy.review.required,
                            "allowed_deciders": record.approval.policy.review.allowed_deciders,
                        },
                        "test_validation": {
                            "required": record.approval.policy.test_validation.required,
                            "allowed_deciders": record.approval.policy.test_validation.allowed_deciders,
                        }
                    }
                },
                "dependencies": task.depends_on,
                "environment": {
                    "id": record.environment.id,
                    "mode": format!("{:?}", record.environment.execution_mode).to_lowercase(),
                    "root_dir": record.environment.root_dir,
                    "write_access": record.environment.write_access,
                    "state": format!("{:?}", record.environment.state).to_lowercase(),
                    "health": format!("{:?}", record.environment.health).to_lowercase(),
                    "cleanup_policy": format!("{:?}", record.environment.cleanup_policy).to_lowercase(),
                    "recovery_status": format!("{:?}", record.environment.recovery_status).to_lowercase(),
                    "recovery_action": record.environment.recovery_action.map(|value| format!("{:?}", value).to_lowercase()),
                    "failure": record.environment.failure.as_ref(),
                    "branch_name": record.environment.branch_name,
                    "worktree_path": record.environment.worktree_path,
                    "remote_url": record.environment.remote_url,
                },
                "planning_only": task.planning_only,
                "reviewer_required": task.reviewer_required,
                "test_required": task.test_required,
                "memory_tags": task.memory_tags,
                "run_status": format!("{:?}", run.status).to_lowercase(),
            }
        }),
    );
}

fn record_task_progress(task: &DelegatedTask, record: &SupervisorTaskRecord, run: &SupervisorRun) {
    let Some(session_id) = task.session_id.as_deref() else {
        return;
    };
    let Some(tracking_task_id) = task.tracking_task_id.as_deref() else {
        return;
    };

    let manager = crate::get_global_task_manager();
    let task_status = match record.state {
        SupervisorTaskState::Blocked
        | SupervisorTaskState::PendingApproval
        | SupervisorTaskState::ReviewPending
        | SupervisorTaskState::TestPending => TaskStatus::Blocked,
        SupervisorTaskState::Running => TaskStatus::InProgress,
        SupervisorTaskState::Completed => TaskStatus::Completed,
        SupervisorTaskState::Cancelled | SupervisorTaskState::Failed => TaskStatus::Cancelled,
        _ => TaskStatus::NotStarted,
    };
    let _ = manager.update_task_status(session_id, tracking_task_id, task_status);
    let _ = manager.set_task_background_job(
        session_id,
        tracking_task_id,
        Some(TaskBackgroundJob::new(
            background_status_for_state(record.state),
            Some(task.id.clone()),
            Some(background_message_for_record(record)),
        )),
    );
    merge_task_metadata(
        manager,
        session_id,
        tracking_task_id,
        json!({
            "delegation": {
                "state": format!("{:?}", record.state).to_lowercase(),
                "run_status": format!("{:?}", run.status).to_lowercase(),
                "local_execution": record.local_execution.as_ref(),
                "remote_execution": record.remote_execution.as_ref(),
            }
        }),
    );
}

fn record_task_completion(
    task: &DelegatedTask,
    task_result: &TaskResult,
    memory_file_path: Option<&Path>,
    record: &SupervisorTaskRecord,
    run: &SupervisorRun,
) {
    let Some(session_id) = task.session_id.as_deref() else {
        return;
    };
    let Some(tracking_task_id) = task.tracking_task_id.as_deref() else {
        return;
    };

    let manager = crate::get_global_task_manager();
    let task_status = match record.state {
        SupervisorTaskState::Blocked
        | SupervisorTaskState::PendingApproval
        | SupervisorTaskState::ReviewPending
        | SupervisorTaskState::TestPending => TaskStatus::Blocked,
        SupervisorTaskState::Completed => TaskStatus::Completed,
        SupervisorTaskState::Cancelled | SupervisorTaskState::Failed => TaskStatus::Cancelled,
        SupervisorTaskState::Running => TaskStatus::InProgress,
        _ => TaskStatus::NotStarted,
    };
    let _ = manager.update_task_status(session_id, tracking_task_id, task_status);
    let _ = manager.set_task_background_job(
        session_id,
        tracking_task_id,
        Some(TaskBackgroundJob::new(
            background_status_for_state(record.state),
            Some(task.id.clone()),
            Some(background_message_for_record(record)),
        )),
    );
    let _ = manager.record_memory_event(
        session_id,
        tracking_task_id,
        crate::tasks::TaskMemoryEvent::new(
            if matches!(record.state, SupervisorTaskState::Completed) {
                crate::tasks::TaskMemoryPhase::Promoted
            } else {
                crate::tasks::TaskMemoryPhase::Blocked
            },
            if matches!(record.state, SupervisorTaskState::Completed) {
                format!("Delegated work completed by {}", task.agent_id)
            } else {
                format!(
                    "Delegated work waiting on {:?} for {}",
                    record.state, task.agent_id
                )
            },
            Some(
                if task.directive_id.is_some() {
                    "directive"
                } else {
                    "session"
                }
                .to_string(),
            ),
            Some(
                if matches!(record.state, SupervisorTaskState::Completed) {
                    "handoff"
                } else {
                    "blocker"
                }
                .to_string(),
            ),
            memory_file_path.map(|path| path.display().to_string()),
        ),
    );
    merge_task_metadata(
        manager,
        session_id,
        tracking_task_id,
        json!({
            "delegation": {
                "run_id": task.run_id,
                "orchestrator_task_id": task.id,
                "agent_id": task.agent_id,
                "directive_id": task.directive_id,
                "role": task.role,
                "state": format!("{:?}", record.state).to_lowercase(),
                "approval_state": format!("{:?}", record.approval.state).to_lowercase(),
                "approval": {
                    "scope": record.approval.scope.map(|value| format!("{:?}", value).to_lowercase()),
                    "requested_at": record.approval.requested_at,
                    "decided_at": record.approval.decided_at,
                    "decided_by": record.approval.decided_by,
                    "note": record.approval.note,
                    "active_request": record.approval.active_request.as_ref(),
                    "latest_decision": record.approval.latest_decision(),
                    "policy": {
                        "pre_execution": {
                            "required": record.approval.policy.pre_execution.required,
                            "allowed_deciders": record.approval.policy.pre_execution.allowed_deciders,
                        },
                        "review": {
                            "required": record.approval.policy.review.required,
                            "allowed_deciders": record.approval.policy.review.allowed_deciders,
                        },
                        "test_validation": {
                            "required": record.approval.policy.test_validation.required,
                            "allowed_deciders": record.approval.policy.test_validation.allowed_deciders,
                        }
                    }
                },
                "last_output": task_result.output,
                "summary": task_result.summary,
                "attempts": record.attempts,
                "memory_file_path": memory_file_path.map(|path| path.display().to_string()),
                "tool_calls": task_result.tool_calls,
                "artifacts": task_result.artifacts,
                "environment": {
                    "id": record.environment.id,
                    "mode": format!("{:?}", record.environment.execution_mode).to_lowercase(),
                    "state": format!("{:?}", record.environment.state).to_lowercase(),
                    "health": format!("{:?}", record.environment.health).to_lowercase(),
                    "recovery_status": format!("{:?}", record.environment.recovery_status).to_lowercase(),
                    "recovery_action": record.environment.recovery_action.map(|value| format!("{:?}", value).to_lowercase()),
                    "failure": record.environment.failure.as_ref(),
                    "cleanup_result": record.environment.cleanup_result.as_ref(),
                    "branch_name": record.environment.branch_name,
                    "worktree_path": record.environment.worktree_path,
                    "remote_url": record.environment.remote_url,
                },
                "run_status": format!("{:?}", run.status).to_lowercase(),
            }
        }),
    );
}

fn background_status_for_state(state: SupervisorTaskState) -> TaskBackgroundStatus {
    match state {
        SupervisorTaskState::Queued => TaskBackgroundStatus::Queued,
        SupervisorTaskState::Blocked => TaskBackgroundStatus::Blocked,
        SupervisorTaskState::PendingApproval
        | SupervisorTaskState::ReviewPending
        | SupervisorTaskState::TestPending => TaskBackgroundStatus::AwaitingApproval,
        SupervisorTaskState::Running => TaskBackgroundStatus::Running,
        SupervisorTaskState::Completed => TaskBackgroundStatus::Succeeded,
        SupervisorTaskState::Failed => TaskBackgroundStatus::Failed,
        SupervisorTaskState::Cancelled => TaskBackgroundStatus::Cancelled,
    }
}

fn background_message_for_record(record: &SupervisorTaskRecord) -> String {
    if let Some(remote) = remote_background_message(record)
        && matches!(
            record.state,
            SupervisorTaskState::Running | SupervisorTaskState::Blocked
        )
    {
        return remote;
    }
    if let Some(local) = local_background_message(record)
        && matches!(
            record.state,
            SupervisorTaskState::Running | SupervisorTaskState::Blocked
        )
    {
        return local;
    }
    match record.state {
        SupervisorTaskState::Queued => "Queued for execution".to_string(),
        SupervisorTaskState::Blocked => {
            if record.blocked_reasons.is_empty() {
                "Blocked".to_string()
            } else {
                format!("Blocked: {}", record.blocked_reasons.join("; "))
            }
        }
        SupervisorTaskState::PendingApproval => "Awaiting supervisor approval".to_string(),
        SupervisorTaskState::Running => "Running".to_string(),
        SupervisorTaskState::ReviewPending => "Awaiting review approval".to_string(),
        SupervisorTaskState::TestPending => "Awaiting test validation".to_string(),
        SupervisorTaskState::Completed => "Completed".to_string(),
        SupervisorTaskState::Failed => "Failed".to_string(),
        SupervisorTaskState::Cancelled => "Cancelled".to_string(),
    }
}

fn local_background_message(record: &SupervisorTaskRecord) -> Option<String> {
    let local = record.local_execution.as_ref()?;
    let mut parts = vec![format!("Local {}", local.status)];
    if let Some(reason) = local.status_reason.as_deref() {
        parts.push(reason.to_string());
    }
    if let Some(progress) = local.progress.as_ref() {
        parts.push(format!("phase {:?}", progress.phase).to_lowercase());
        if let Some(waiting_reason) = progress.waiting_reason {
            parts.push(format!("waiting {:?}", waiting_reason).to_lowercase());
        }
        if let Some(percent) = progress.percent {
            parts.push(format!("{percent}%"));
        }
        if let Some(stage) = progress.stage.as_deref() {
            parts.push(stage.to_string());
        }
        if let Some(tool) = progress.current_tool_name.as_deref() {
            parts.push(format!("tool {tool}"));
        }
        if progress.completed_tool_call_count > 0 {
            parts.push(format!(
                "{} tool call(s)",
                progress.completed_tool_call_count
            ));
        }
        if let Some(token_usage) = progress.token_usage.as_ref() {
            if let (Some(estimated_tokens), Some(limit), Some(percentage)) = (
                token_usage.estimated_tokens,
                token_usage.limit,
                token_usage.percentage,
            ) {
                parts.push(format!("tokens {estimated_tokens}/{limit} ({percentage}%)"));
            } else if let Some(total_tokens) = token_usage.total_tokens {
                parts.push(format!("tokens {total_tokens}"));
            }
        }
        if let Some(environment) = progress.environment.as_ref() {
            parts.push(format!("env {:?}", environment.state).to_lowercase());
        }
        if let Some(message) = progress.message.as_deref() {
            parts.push(message.to_string());
        }
    }
    Some(parts.join(" • "))
}

fn remote_background_message(record: &SupervisorTaskRecord) -> Option<String> {
    let remote = record.remote_execution.as_ref()?;
    let mut parts = vec![format!("Remote {}", remote.status)];
    if let Some(reason) = remote.status_reason.as_deref() {
        parts.push(reason.to_string());
    }
    if let Some(progress) = remote.progress.as_ref() {
        if let Some(percent) = progress.percent {
            parts.push(format!("{percent}%"));
        }
        if let Some(stage) = progress.stage.as_deref() {
            parts.push(stage.to_string());
        }
    }
    Some(parts.join(" • "))
}

fn a2a_status_label(status: A2ATaskStatus) -> String {
    match status {
        A2ATaskStatus::Pending => "pending",
        A2ATaskStatus::Blocked => "blocked",
        A2ATaskStatus::Running => "running",
        A2ATaskStatus::Completed => "completed",
        A2ATaskStatus::Cancelled => "cancelled",
        A2ATaskStatus::Failed => "failed",
    }
    .to_string()
}

fn task_terminal_hint_for_a2a_status(status: A2ATaskStatus) -> Option<TaskTerminalStateHint> {
    match status {
        A2ATaskStatus::Completed => Some(TaskTerminalStateHint::Completed),
        A2ATaskStatus::Cancelled => Some(TaskTerminalStateHint::Cancelled),
        A2ATaskStatus::Failed => Some(TaskTerminalStateHint::Failed),
        A2ATaskStatus::Blocked => Some(TaskTerminalStateHint::Blocked),
        _ => None,
    }
}

fn progress_from_remote(
    progress: Option<&A2ARemoteTaskProgress>,
) -> Option<RemoteExecutionProgress> {
    progress.map(|progress| RemoteExecutionProgress {
        stage: progress.stage.clone(),
        message: progress.message.clone(),
        percent: progress.percent,
        updated_at: progress.updated_at,
    })
}

#[derive(Debug, Clone, Default)]
struct LocalExecutionTelemetryContext {
    iteration: u32,
    current_tool_name: Option<String>,
    last_completed_tool_name: Option<String>,
    last_completed_tool_duration_ms: Option<u64>,
    completed_tool_call_count: usize,
    partial_content_chars: usize,
    partial_thinking_chars: usize,
    has_partial_content: bool,
    has_partial_thinking: bool,
    token_usage: Option<LocalExecutionTokenUsageSnapshot>,
    environment: Option<LocalExecutionEnvironmentSnapshot>,
}

fn local_execution_record_for_start() -> LocalExecutionRecord {
    let now = Utc::now();
    let context = LocalExecutionTelemetryContext::default();
    LocalExecutionRecord {
        status: "running".to_string(),
        status_reason: None,
        progress: Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("starting".to_string()),
            message: Some("Delegated task dispatched to local agent".to_string()),
            percent: None,
            iteration: context.iteration,
            current_tool_name: None,
            last_completed_tool_name: None,
            last_completed_tool_duration_ms: None,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: None,
            environment: None,
            updated_at: now,
        }),
        last_synced_at: now,
    }
}

fn local_execution_record_for_terminal(
    task_result: &TaskResult,
    state: SupervisorTaskState,
    previous: Option<&LocalExecutionRecord>,
) -> LocalExecutionRecord {
    let now = Utc::now();
    let previous_progress = previous.and_then(|local| local.progress.as_ref());
    LocalExecutionRecord {
        status: local_execution_status_label(state).to_string(),
        status_reason: if task_result.success {
            None
        } else {
            Some(task_result.output.clone())
        },
        progress: Some(LocalExecutionProgress {
            phase: local_execution_phase_for_terminal_state(state),
            waiting_reason: None,
            stage: Some(local_execution_stage_for_terminal_state(state).to_string()),
            message: Some(if task_result.success {
                format!(
                    "Local delegated task completed after {} tool call(s)",
                    task_result.tool_calls.len()
                )
            } else {
                format!(
                    "Local delegated task ended as {}: {}",
                    local_execution_status_label(state),
                    task_result.output
                )
            }),
            percent: Some(if matches!(state, SupervisorTaskState::Completed) {
                100
            } else {
                previous_progress
                    .and_then(|progress| progress.percent)
                    .unwrap_or(100)
            }),
            iteration: previous_progress
                .map(|progress| progress.iteration)
                .unwrap_or_default(),
            current_tool_name: None,
            last_completed_tool_name: previous_progress
                .and_then(|progress| progress.last_completed_tool_name.clone()),
            last_completed_tool_duration_ms: previous_progress
                .and_then(|progress| progress.last_completed_tool_duration_ms),
            completed_tool_call_count: task_result.tool_calls.len(),
            has_partial_content: previous_progress
                .map(|progress| progress.has_partial_content)
                .unwrap_or(false),
            partial_content_chars: previous_progress
                .map(|progress| progress.partial_content_chars)
                .unwrap_or_default(),
            has_partial_thinking: previous_progress
                .map(|progress| progress.has_partial_thinking)
                .unwrap_or(false),
            partial_thinking_chars: previous_progress
                .map(|progress| progress.partial_thinking_chars)
                .unwrap_or_default(),
            token_usage: previous_progress.and_then(|progress| progress.token_usage.clone()),
            environment: previous_progress.and_then(|progress| progress.environment.clone()),
            updated_at: now,
        }),
        last_synced_at: now,
    }
}

fn local_execution_progress_from_chunk(
    chunk: &StreamChunk,
    context: &LocalExecutionTelemetryContext,
) -> Option<LocalExecutionProgress> {
    let now = Utc::now();
    match chunk {
        StreamChunk::Status { message } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("status".to_string()),
            message: Some(message.clone()),
            percent: None,
            iteration: context.iteration,
            current_tool_name: context.current_tool_name.clone(),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::AgentLoopIteration { iteration } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("agent_loop".to_string()),
            message: Some(format!("Agent loop iteration {}", iteration + 1)),
            percent: None,
            iteration: *iteration,
            current_tool_name: None,
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ToolCallStart { name, .. } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("executing_tools".to_string()),
            message: Some(format!("Running tool '{name}'")),
            percent: None,
            iteration: context.iteration,
            current_tool_name: Some(name.clone()),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ToolCallResult {
            name,
            success,
            duration_ms,
            ..
        } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("executing_tools".to_string()),
            message: Some(if *success {
                format!("Tool '{name}' completed successfully")
            } else {
                format!("Tool '{name}' failed")
            }),
            percent: None,
            iteration: context.iteration,
            current_tool_name: None,
            last_completed_tool_name: Some(name.clone()),
            last_completed_tool_duration_ms: Some(*duration_ms),
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ShellLifecycle { state, command, .. } => Some(LocalExecutionProgress {
            phase: if matches!(
                state,
                crate::streaming::ShellProcessState::Started
                    | crate::streaming::ShellProcessState::Paused
                    | crate::streaming::ShellProcessState::Resumed
            ) {
                LocalExecutionPhase::Waiting
            } else {
                LocalExecutionPhase::Running
            },
            waiting_reason: if matches!(
                state,
                crate::streaming::ShellProcessState::Started
                    | crate::streaming::ShellProcessState::Paused
                    | crate::streaming::ShellProcessState::Resumed
            ) {
                Some(LocalExecutionWaitingReason::ShellProcess)
            } else {
                None
            },
            stage: Some("shell".to_string()),
            message: Some(format!("Shell {:?}: {command}", state).to_lowercase()),
            percent: None,
            iteration: context.iteration,
            current_tool_name: Some("shell".to_string()),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::TokenUsageUpdate {
            estimated,
            limit,
            percentage,
            status,
            estimated_cost,
        } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("token_usage".to_string()),
            message: Some(format!(
                "Estimated token usage {estimated}/{limit} ({percentage}%)"
            )),
            percent: Some(*percentage),
            iteration: context.iteration,
            current_tool_name: context.current_tool_name.clone(),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: Some(LocalExecutionTokenUsageSnapshot {
                estimated_tokens: Some(*estimated),
                limit: Some(*limit),
                percentage: Some(*percentage),
                status: Some(token_usage_status_label(*status).to_string()),
                estimated_cost_usd: Some(*estimated_cost),
                input_tokens: context
                    .token_usage
                    .as_ref()
                    .and_then(|usage| usage.input_tokens),
                output_tokens: context
                    .token_usage
                    .as_ref()
                    .and_then(|usage| usage.output_tokens),
                total_tokens: context
                    .token_usage
                    .as_ref()
                    .and_then(|usage| usage.total_tokens),
                model: context
                    .token_usage
                    .as_ref()
                    .and_then(|usage| usage.model.clone()),
                provider: context
                    .token_usage
                    .as_ref()
                    .and_then(|usage| usage.provider.clone()),
            }),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ReflectionStarted { reason } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Waiting,
            waiting_reason: Some(LocalExecutionWaitingReason::Reflection),
            stage: Some("reflection".to_string()),
            message: Some(reason.clone()),
            percent: None,
            iteration: context.iteration,
            current_tool_name: context.current_tool_name.clone(),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ReflectionComplete { summary, .. } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("reflection".to_string()),
            message: Some(summary.clone()),
            percent: None,
            iteration: context.iteration,
            current_tool_name: context.current_tool_name.clone(),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ToolConfirmationRequired { tool_name, .. } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Waiting,
            waiting_reason: Some(LocalExecutionWaitingReason::ToolConfirmation),
            stage: Some("tool_confirmation".to_string()),
            message: Some(format!(
                "Waiting for confirmation before running '{tool_name}'"
            )),
            percent: None,
            iteration: context.iteration,
            current_tool_name: Some(tool_name.clone()),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::ToolBlocked { tool_name, reason } => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Blocked,
            waiting_reason: None,
            stage: Some("tool_blocked".to_string()),
            message: Some(format!("Tool '{tool_name}' blocked: {reason}")),
            percent: None,
            iteration: context.iteration,
            current_tool_name: Some(tool_name.clone()),
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::Done(usage) => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("finishing".to_string()),
            message: Some("Local delegated task finished streaming output".to_string()),
            percent: Some(100),
            iteration: context.iteration,
            current_tool_name: None,
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: Some(token_usage_snapshot_from_done(
                usage.as_ref(),
                context.token_usage.as_ref(),
            )),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::Paused => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Blocked,
            waiting_reason: None,
            stage: Some("paused".to_string()),
            message: Some(
                "Execution paused by operator; resumable checkpoint preserved".to_string(),
            ),
            percent: None,
            iteration: context.iteration,
            current_tool_name: None,
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        StreamChunk::Cancelled => Some(LocalExecutionProgress {
            phase: LocalExecutionPhase::Cancelled,
            waiting_reason: None,
            stage: Some("cancelled".to_string()),
            message: Some("Execution cancelled by operator".to_string()),
            percent: None,
            iteration: context.iteration,
            current_tool_name: None,
            last_completed_tool_name: context.last_completed_tool_name.clone(),
            last_completed_tool_duration_ms: context.last_completed_tool_duration_ms,
            completed_tool_call_count: context.completed_tool_call_count,
            has_partial_content: context.has_partial_content,
            partial_content_chars: context.partial_content_chars,
            has_partial_thinking: context.has_partial_thinking,
            partial_thinking_chars: context.partial_thinking_chars,
            token_usage: context.token_usage.clone(),
            environment: context.environment.clone(),
            updated_at: now,
        }),
        _ => None,
    }
}

fn token_usage_snapshot_from_done(
    usage: Option<&crate::llm_provider::TokenUsage>,
    previous: Option<&LocalExecutionTokenUsageSnapshot>,
) -> LocalExecutionTokenUsageSnapshot {
    LocalExecutionTokenUsageSnapshot {
        estimated_tokens: previous.and_then(|usage| usage.estimated_tokens),
        limit: previous.and_then(|usage| usage.limit),
        percentage: previous.and_then(|usage| usage.percentage),
        status: previous.and_then(|usage| usage.status.clone()),
        estimated_cost_usd: usage
            .and_then(|usage| usage.estimated_cost_usd)
            .or_else(|| previous.and_then(|usage| usage.estimated_cost_usd)),
        input_tokens: usage.map(|usage| usage.input_tokens),
        output_tokens: usage.map(|usage| usage.output_tokens),
        total_tokens: usage.map(|usage| usage.total_tokens),
        model: usage
            .and_then(|usage| usage.model.clone())
            .or_else(|| previous.and_then(|usage| usage.model.clone())),
        provider: usage
            .and_then(|usage| usage.provider.clone())
            .or_else(|| previous.and_then(|usage| usage.provider.clone())),
    }
}

fn token_usage_status_label(status: crate::streaming::TokenUsageStatus) -> &'static str {
    match status {
        crate::streaming::TokenUsageStatus::Green => "green",
        crate::streaming::TokenUsageStatus::Yellow => "yellow",
        crate::streaming::TokenUsageStatus::Red => "red",
    }
}

fn environment_snapshot_from_execution(
    environment: &ExecutionEnvironment,
) -> LocalExecutionEnvironmentSnapshot {
    LocalExecutionEnvironmentSnapshot {
        state: environment.state,
        health: environment.health,
        recovery_status: environment.recovery_status,
        updated_at: Utc::now(),
    }
}

fn local_execution_status_label(state: SupervisorTaskState) -> &'static str {
    match state {
        SupervisorTaskState::Queued => "queued",
        SupervisorTaskState::PendingApproval => "pending_approval",
        SupervisorTaskState::Running => "running",
        SupervisorTaskState::ReviewPending => "review_pending",
        SupervisorTaskState::TestPending => "test_pending",
        SupervisorTaskState::Completed => "completed",
        SupervisorTaskState::Failed => "failed",
        SupervisorTaskState::Blocked => "blocked",
        SupervisorTaskState::Cancelled => "cancelled",
    }
}

fn local_execution_phase_for_terminal_state(state: SupervisorTaskState) -> LocalExecutionPhase {
    match state {
        SupervisorTaskState::Completed => LocalExecutionPhase::Completed,
        SupervisorTaskState::Cancelled => LocalExecutionPhase::Cancelled,
        SupervisorTaskState::Blocked => LocalExecutionPhase::Blocked,
        SupervisorTaskState::Failed => LocalExecutionPhase::Failed,
        SupervisorTaskState::Queued
        | SupervisorTaskState::PendingApproval
        | SupervisorTaskState::Running
        | SupervisorTaskState::ReviewPending
        | SupervisorTaskState::TestPending => LocalExecutionPhase::Running,
    }
}

fn local_execution_stage_for_terminal_state(state: SupervisorTaskState) -> &'static str {
    match state {
        SupervisorTaskState::Completed => "completed",
        SupervisorTaskState::Cancelled => "cancelled",
        SupervisorTaskState::Blocked => "blocked",
        SupervisorTaskState::Failed => "failed",
        _ => "running",
    }
}

fn artifacts_from_manifest(manifest: &[ArtifactManifestEntry]) -> Vec<RemoteExecutionArtifact> {
    manifest
        .iter()
        .map(|artifact| RemoteExecutionArtifact {
            name: artifact.name.clone(),
            part_count: artifact.part_count,
            metadata: artifact.metadata.clone(),
        })
        .collect()
}

fn summarize_remote_task_output(task: &A2ATask) -> String {
    task.messages
        .iter()
        .rev()
        .flat_map(|message| message.parts.iter().rev())
        .find_map(|part| match part {
            MessagePart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .or_else(|| task.status_reason.clone())
        .unwrap_or_else(|| {
            format!(
                "Remote task {} finished with status {}",
                task.id,
                a2a_status_label(task.status)
            )
        })
}

fn task_artifacts_from_remote_payload(artifacts: &[RemoteArtifact]) -> Vec<TaskArtifactRecord> {
    artifacts
        .iter()
        .map(|artifact| TaskArtifactRecord {
            name: artifact.name.clone(),
            kind: "a2a_artifact".to_string(),
            uri: Some(format!("a2a://artifact/{}", artifact.name)),
            summary: Some(
                artifact
                    .parts
                    .iter()
                    .find_map(|part| match part {
                        MessagePart::Text { text } => {
                            Some(text.chars().take(160).collect::<String>())
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| "Remote artifact".to_string()),
            ),
        })
        .collect()
}

fn compatibility_from_card(
    card: &gestura_core_a2a::AgentCard,
    remote_target: &RemoteAgentTarget,
) -> RemoteExecutionCompatibility {
    let mut warnings = Vec::new();
    if card.authentication.is_some() && remote_target.auth_token.is_none() {
        warnings.push(
            "Remote peer advertises authentication, but no auth token is configured".to_string(),
        );
    }
    if !card
        .supported_task_features
        .iter()
        .any(|feature| feature == "authenticated-mutations")
    {
        warnings
            .push("Remote peer does not advertise authenticated mutation enforcement".to_string());
    }
    if !card
        .supported_task_features
        .iter()
        .any(|feature| feature == "provenance")
    {
        warnings.push("Remote peer does not advertise provenance support".to_string());
    }
    if !card
        .supported_task_features
        .iter()
        .any(|feature| feature == "leases")
    {
        warnings.push(
            "Remote peer does not advertise lease support; heartbeat-based tracking will be disabled"
                .to_string(),
        );
    }
    if !card
        .supported_task_features
        .iter()
        .any(|feature| feature == "idempotency")
    {
        warnings.push(
            "Remote peer does not advertise idempotency support; retries may duplicate work"
                .to_string(),
        );
    }
    if !card
        .supported_rpc_methods
        .iter()
        .any(|method| method == "task/artifacts")
    {
        warnings.push("Remote peer does not advertise artifact manifest support".to_string());
    }
    RemoteExecutionCompatibility {
        supported_features: card.supported_task_features.clone(),
        warnings,
        protocol_version: Some(card.protocol_version.clone()),
    }
}

fn build_remote_task_request(
    task: &DelegatedTask,
    record: &SupervisorTaskRecord,
    compatibility: &RemoteExecutionCompatibility,
) -> CreateTaskRequest {
    let brief = task
        .delegation_brief
        .clone()
        .unwrap_or_else(|| DelegationBrief {
            objective: task.name.clone().unwrap_or_else(|| task.prompt.clone()),
            acceptance_criteria: Vec::new(),
            constraints: Vec::new(),
            deliverables: vec!["Provide a concise text result".to_string()],
            context_summary: None,
        });
    let mut metadata = HashMap::new();
    metadata.insert("gesturaRunId".to_string(), json!(task.run_id));
    metadata.insert("gesturaTaskId".to_string(), json!(task.id));
    metadata.insert("executionMode".to_string(), json!("remote"));
    metadata.insert("attempt".to_string(), json!(record.attempts));
    metadata.insert(
        "approvalRequired".to_string(),
        json!(task.approval_required),
    );
    metadata.insert(
        "reviewerRequired".to_string(),
        json!(task.reviewer_required),
    );
    metadata.insert("testRequired".to_string(), json!(task.test_required));
    if let Some(workspace_dir) = task.workspace_dir.as_ref() {
        metadata.insert(
            "workspaceDir".to_string(),
            json!(workspace_dir.to_string_lossy().to_string()),
        );
    }
    CreateTaskRequest {
        message: A2AMessage {
            role: "user".to_string(),
            parts: vec![MessagePart::Text {
                text: task.prompt.clone(),
            }],
        },
        run_id: task.run_id.clone(),
        parent_task_id: task.parent_task_id.clone(),
        role: task
            .role
            .clone()
            .map(|role| format!("{role:?}").to_lowercase()),
        requested_capabilities: task.required_tools.clone(),
        contract: Some(RemoteTaskContract {
            objective: brief.objective,
            acceptance_criteria: brief.acceptance_criteria,
            constraints: brief.constraints,
            deliverables: brief.deliverables,
            output_format: Some("text".to_string()),
        }),
        idempotency_key: compatibility
            .supported_features
            .iter()
            .any(|feature| feature == "idempotency")
            .then(|| {
                format!(
                    "gestura:{}:{}:{}",
                    task.run_id
                        .clone()
                        .unwrap_or_else(|| "standalone".to_string()),
                    task.id,
                    record.attempts
                )
            }),
        lease_request: compatibility
            .supported_features
            .iter()
            .any(|feature| feature == "leases")
            .then_some(RemoteTaskLeaseRequest {
                ttl_secs: 120,
                heartbeat_interval_secs: 15,
            }),
        metadata,
    }
}

fn provenance_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<TaskProvenance> {
    let caller_agent_id = metadata
        .get("caller_agent_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let caller_name = metadata
        .get("caller_name")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let caller_version = metadata
        .get("caller_version")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let caller_capabilities = metadata
        .get("caller_capabilities")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let authenticated = metadata
        .get("caller_authenticated")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let auth_scheme = metadata
        .get("caller_auth_scheme")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    (caller_agent_id.is_some()
        || caller_name.is_some()
        || caller_version.is_some()
        || !caller_capabilities.is_empty()
        || authenticated
        || auth_scheme.is_some())
    .then_some(TaskProvenance {
        caller_agent_id,
        caller_name,
        caller_version,
        caller_capabilities,
        authenticated,
        auth_scheme,
    })
}

fn checkpoint_roots_for_runs(runs: &[SupervisorRun], default_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = HashSet::new();
    if let Some(root) = default_root {
        roots.insert(root.to_path_buf());
    }
    for run in runs {
        if let Some(workspace_dir) = run.workspace_dir.as_ref() {
            roots.insert(workspace_dir.clone());
        }
        for record in &run.tasks {
            if let Some(workspace_dir) = record.task.workspace_dir.as_ref() {
                roots.insert(workspace_dir.clone());
            }
        }
    }
    roots.into_iter().collect()
}

fn checkpoint_roots_for_task(task: &DelegatedTask, default_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = default_root {
        roots.push(root.to_path_buf());
    }
    if let Some(workspace_dir) = task.workspace_dir.as_ref()
        && !roots.iter().any(|root| root == workspace_dir)
    {
        roots.push(workspace_dir.clone());
    }
    roots
}

fn load_latest_checkpoints_by_task<I>(roots: I) -> HashMap<String, DelegatedTaskCheckpoint>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut checkpoints: HashMap<String, DelegatedTaskCheckpoint> = HashMap::new();
    for root in roots {
        for checkpoint in load_persisted_checkpoints(&root) {
            match checkpoints.get(&checkpoint.task_id) {
                Some(existing) if existing.updated_at >= checkpoint.updated_at => {}
                _ => {
                    checkpoints.insert(checkpoint.task_id.clone(), checkpoint);
                }
            }
        }
    }
    checkpoints
}

fn attach_checkpoint_summaries(
    runs: &mut [SupervisorRun],
    checkpoints: HashMap<String, DelegatedTaskCheckpoint>,
) {
    for run in runs {
        for record in &mut run.tasks {
            record.checkpoint = checkpoints
                .get(&record.task.id)
                .map(|checkpoint| checkpoint_summary_for_record(checkpoint, record.state));
        }
    }
}

fn checkpoint_summary_for_record(
    checkpoint: &DelegatedTaskCheckpoint,
    task_state: SupervisorTaskState,
) -> DelegatedCheckpointSummary {
    DelegatedCheckpointSummary {
        stage: checkpoint.stage,
        replay_safety: checkpoint.replay_safety,
        resume_disposition: checkpoint.resume_disposition,
        safe_boundary_label: checkpoint.safe_boundary_label.clone(),
        available_actions: checkpoint_available_actions(checkpoint, task_state),
        note: checkpoint.note.clone(),
        completed_tool_call_count: checkpoint.completed_tool_calls.len(),
        has_resume_state: checkpoint.resume_state.is_some(),
        result_published: checkpoint.result_published,
        updated_at: checkpoint.updated_at,
    }
}

fn active_task_snapshot(
    task: DelegatedTask,
    record: Option<&SupervisorTaskRecord>,
) -> ActiveTaskSnapshot {
    if let Some(record) = record {
        ActiveTaskSnapshot {
            task,
            state: record.state,
            remote_execution: record.remote_execution.clone(),
            local_execution: record.local_execution.clone(),
            blocked_reasons: record.blocked_reasons.clone(),
            checkpoint: record.checkpoint.clone(),
        }
    } else {
        ActiveTaskSnapshot {
            task,
            state: SupervisorTaskState::Running,
            remote_execution: None,
            local_execution: None,
            blocked_reasons: Vec::new(),
            checkpoint: None,
        }
    }
}

fn checkpoint_available_actions(
    checkpoint: &DelegatedTaskCheckpoint,
    task_state: SupervisorTaskState,
) -> Vec<DelegatedCheckpointAction> {
    if task_state != SupervisorTaskState::Blocked || checkpoint.result_published {
        return Vec::new();
    }

    let mut actions = Vec::new();
    if checkpoint.resume_disposition == DelegatedResumeDisposition::ResumeFromCheckpoint
        && checkpoint.resume_state.is_some()
    {
        actions.push(DelegatedCheckpointAction::ResumeFromCheckpoint);
    }
    actions.push(DelegatedCheckpointAction::RestartFromScratch);
    actions.push(DelegatedCheckpointAction::AcknowledgeBlocked);
    actions
}

fn unresolved_dependency_reasons(run: &SupervisorRun, task: &DelegatedTask) -> Vec<String> {
    let dependency_states = run
        .tasks
        .iter()
        .map(|record| (record.task.id.clone(), record.state))
        .collect::<HashMap<_, _>>();
    dependency_reasons_from_states(&dependency_states, task)
}

fn dependency_reasons_from_states(
    dependency_states: &HashMap<String, SupervisorTaskState>,
    task: &DelegatedTask,
) -> Vec<String> {
    task.depends_on
        .iter()
        .filter_map(|dependency_id| match dependency_states.get(dependency_id) {
            Some(SupervisorTaskState::Completed) => None,
            Some(state) => Some(format!("Waiting on task '{}' ({:?})", dependency_id, state)),
            None => Some(format!("Waiting on unknown dependency '{}'", dependency_id)),
        })
        .collect()
}

fn recalculate_run_status(run: &SupervisorRun) -> SupervisorRunStatus {
    if run.tasks.is_empty() && run.child_runs.is_empty() {
        return SupervisorRunStatus::Draft;
    }
    let own_tasks = !run.tasks.is_empty();
    let own_cancelled = own_tasks
        && run
            .tasks
            .iter()
            .all(|record| matches!(record.state, SupervisorTaskState::Cancelled));
    let child_cancelled = !run.child_runs.is_empty()
        && run
            .child_runs
            .iter()
            .all(|child| matches!(child.status, SupervisorRunStatus::Cancelled));
    if (own_tasks || !run.child_runs.is_empty())
        && (if own_tasks { own_cancelled } else { true })
        && (if !run.child_runs.is_empty() {
            child_cancelled
        } else {
            true
        })
    {
        return SupervisorRunStatus::Cancelled;
    }
    if run.tasks.iter().any(|record| {
        matches!(
            record.state,
            SupervisorTaskState::Running | SupervisorTaskState::Queued
        )
    }) || run
        .child_runs
        .iter()
        .any(|child| matches!(child.status, SupervisorRunStatus::Running))
    {
        return SupervisorRunStatus::Running;
    }
    if run.tasks.iter().any(|record| {
        matches!(
            record.state,
            SupervisorTaskState::Blocked
                | SupervisorTaskState::PendingApproval
                | SupervisorTaskState::ReviewPending
                | SupervisorTaskState::TestPending
        )
    }) || run.child_runs.iter().any(|child| {
        child.requires_attention || matches!(child.status, SupervisorRunStatus::Waiting)
    }) {
        return SupervisorRunStatus::Waiting;
    }
    if run
        .tasks
        .iter()
        .any(|record| matches!(record.state, SupervisorTaskState::Failed))
        || run
            .child_runs
            .iter()
            .any(|child| matches!(child.status, SupervisorRunStatus::Failed))
    {
        return SupervisorRunStatus::Failed;
    }
    let own_completed = own_tasks
        && run
            .tasks
            .iter()
            .all(|record| matches!(record.state, SupervisorTaskState::Completed));
    let child_completed = !run.child_runs.is_empty()
        && run
            .child_runs
            .iter()
            .all(|child| matches!(child.status, SupervisorRunStatus::Completed));
    if (own_tasks || !run.child_runs.is_empty())
        && (if own_tasks { own_completed } else { true })
        && (if !run.child_runs.is_empty() {
            child_completed
        } else {
            true
        })
    {
        return SupervisorRunStatus::Completed;
    }
    if run.tasks.iter().all(|record| {
        matches!(
            record.state,
            SupervisorTaskState::Completed | SupervisorTaskState::Cancelled
        )
    }) && run.child_runs.iter().all(|child| {
        matches!(
            child.status,
            SupervisorRunStatus::Completed | SupervisorRunStatus::Cancelled
        )
    }) {
        return SupervisorRunStatus::Completed;
    }
    SupervisorRunStatus::Draft
}

fn summarize_run_tasks(run: &SupervisorRun) -> SupervisorRunTaskSummary {
    let mut summary = SupervisorRunTaskSummary {
        total: run.tasks.len(),
        ..SupervisorRunTaskSummary::default()
    };
    for record in &run.tasks {
        match record.state {
            SupervisorTaskState::Queued => summary.queued += 1,
            SupervisorTaskState::Blocked => summary.blocked += 1,
            SupervisorTaskState::PendingApproval => summary.pending_approval += 1,
            SupervisorTaskState::Running => summary.running += 1,
            SupervisorTaskState::ReviewPending => summary.review_pending += 1,
            SupervisorTaskState::TestPending => summary.test_pending += 1,
            SupervisorTaskState::Completed => summary.completed += 1,
            SupervisorTaskState::Failed => summary.failed += 1,
            SupervisorTaskState::Cancelled => summary.cancelled += 1,
        }
    }
    summary
}

fn run_requires_attention(run: &SupervisorRun) -> bool {
    run.tasks.iter().any(|record| {
        matches!(
            record.state,
            SupervisorTaskState::Blocked
                | SupervisorTaskState::PendingApproval
                | SupervisorTaskState::ReviewPending
                | SupervisorTaskState::TestPending
                | SupervisorTaskState::Failed
        )
    }) || run.child_runs.iter().any(|child| child.requires_attention)
}

fn build_child_inherited_policy(
    parent: &SupervisorRun,
    request: &ChildSupervisorRunRequest,
) -> SupervisorInheritancePolicy {
    let mut policy = parent.inherited_policy.clone().unwrap_or_default();
    policy.approval_required |= request.approval_required;
    policy.reviewer_required |= request.reviewer_required;
    policy.test_required |= request.test_required;
    policy.execution_mode = Some(request.execution_mode.clone());
    policy.workspace_dir = request
        .workspace_dir
        .clone()
        .or_else(|| parent.workspace_dir.clone());
    for tag in &request.memory_tags {
        if !policy.memory_tags.contains(tag) {
            policy.memory_tags.push(tag.clone());
        }
    }
    for note in &request.constraint_notes {
        if !policy.constraint_notes.contains(note) {
            policy.constraint_notes.push(note.clone());
        }
    }
    policy
}

fn build_child_run_summary(run: &SupervisorRun) -> ChildSupervisorRunSummary {
    ChildSupervisorRunSummary {
        run_id: run.id.clone(),
        name: run.name.clone(),
        objective: run
            .parent_run
            .as_ref()
            .map(|parent| parent.objective.clone())
            .unwrap_or_else(|| run.name.clone().unwrap_or_else(|| run.id.clone())),
        lead_agent_id: run.lead_agent_id.clone(),
        status: recalculate_run_status(run),
        task_summary: summarize_run_tasks(run),
        requires_attention: run_requires_attention(run),
        blocked_reasons: run
            .tasks
            .iter()
            .flat_map(|record| record.blocked_reasons.clone())
            .collect(),
        created_at: run.created_at,
        updated_at: run.updated_at,
        completed_at: run.completed_at,
    }
}

fn upsert_child_run_summary(parent: &mut SupervisorRun, child: &SupervisorRun) {
    let summary = build_child_run_summary(child);
    if let Some(existing) = parent
        .child_runs
        .iter_mut()
        .find(|entry| entry.run_id == child.id)
    {
        *existing = summary;
    } else {
        parent.child_runs.push(summary);
    }
}

fn build_hierarchy_summary(run: &SupervisorRun) -> SupervisorHierarchySummary {
    let descendant_task_count = run
        .child_runs
        .iter()
        .map(|child| child.task_summary.total)
        .sum();
    let action_required_child_count = run
        .child_runs
        .iter()
        .filter(|child| child.requires_attention)
        .count();
    let blocked_reasons = run
        .child_runs
        .iter()
        .flat_map(|child| child.blocked_reasons.clone())
        .collect();
    SupervisorHierarchySummary {
        depth: run.hierarchy_depth,
        max_depth: run.max_hierarchy_depth,
        child_run_count: run.child_runs.len(),
        descendant_task_count,
        action_required_child_count,
        rollup_status: recalculate_run_status(run),
        requires_attention: action_required_child_count > 0,
        blocked_reasons,
    }
}

fn refresh_run_rollups(run: &mut SupervisorRun) {
    if run.name.is_none() {
        run.name = run
            .tasks
            .first()
            .and_then(|record| record.task.name.clone());
    }
    if run.max_hierarchy_depth == 0 {
        run.max_hierarchy_depth = MAX_CHILD_SUPERVISOR_DEPTH;
    }
    run.task_summary = summarize_run_tasks(run);
    run.status = recalculate_run_status(run);
    run.hierarchy_summary = Some(build_hierarchy_summary(run));
}

fn synchronize_run_hierarchy_snapshots(runs: &mut [SupervisorRun]) {
    for run in runs.iter_mut() {
        run.task_summary = summarize_run_tasks(run);
    }

    let mut child_map = std::collections::HashMap::<String, Vec<ChildSupervisorRunSummary>>::new();
    for run in runs.iter() {
        if let Some(parent) = run.parent_run.as_ref() {
            child_map
                .entry(parent.parent_run_id.clone())
                .or_default()
                .push(build_child_run_summary(run));
        }
    }

    for run in runs.iter_mut() {
        run.child_runs = child_map.remove(&run.id).unwrap_or_default();
        refresh_run_rollups(run);
    }
}

fn ensure_parent_run_accepts_child(parent: &SupervisorRun) -> Result<(), String> {
    if parent.hierarchy_depth >= parent.max_hierarchy_depth {
        return Err(format!(
            "Supervisor run '{}' is already at the maximum hierarchy depth of {}",
            parent.id, parent.max_hierarchy_depth
        ));
    }
    if parent.parent_run.is_some() {
        return Err(format!(
            "Supervisor run '{}' is already a child run and cannot delegate another child supervisor",
            parent.id
        ));
    }
    if matches!(
        parent.status,
        SupervisorRunStatus::Completed | SupervisorRunStatus::Cancelled
    ) {
        return Err(format!(
            "Supervisor run '{}' is terminal and cannot accept a child supervisor",
            parent.id
        ));
    }
    Ok(())
}

fn collect_ready_tasks(run: &mut SupervisorRun) -> Vec<DelegatedTask> {
    collect_ready_tasks_except(run, None)
}

fn collect_ready_tasks_except(
    run: &mut SupervisorRun,
    skip_task_id: Option<&str>,
) -> Vec<DelegatedTask> {
    let completed_ids = run
        .tasks
        .iter()
        .filter(|record| matches!(record.state, SupervisorTaskState::Completed))
        .map(|record| record.task.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let now = Utc::now();

    let mut ready = Vec::new();
    for record in &mut run.tasks {
        if skip_task_id.is_some_and(|task_id| record.task.id == task_id) {
            continue;
        }
        if !matches!(record.state, SupervisorTaskState::Blocked) {
            continue;
        }
        if matches!(record.approval.state, ApprovalState::Pending) {
            continue;
        }
        if record
            .task
            .depends_on
            .iter()
            .all(|dependency_id| completed_ids.contains(dependency_id))
        {
            record.state = SupervisorTaskState::Queued;
            record.blocked_reasons.clear();
            record.updated_at = now;
            ready.push(record.task.clone());
        }
    }

    ready
}

fn merge_task_metadata(
    manager: &TaskManager,
    session_id: &str,
    task_id: &str,
    patch: serde_json::Value,
) {
    let existing = manager
        .get_task(session_id, task_id)
        .ok()
        .flatten()
        .and_then(|task| task.metadata)
        .unwrap_or_else(|| json!({}));

    let Some(mut existing_map) = existing.as_object().cloned() else {
        return;
    };
    let Some(patch_map) = patch.as_object() else {
        return;
    };

    for (key, value) in patch_map {
        existing_map.insert(key.clone(), value.clone());
    }

    let _ =
        manager.update_task_metadata(session_id, task_id, serde_json::Value::Object(existing_map));
}

fn task_reflection_sync_context(task: &DelegatedTask) -> Option<(PathBuf, String, String)> {
    Some((
        task.workspace_dir.clone()?,
        task.session_id.clone()?,
        task.tracking_task_id.clone()?,
    ))
}

fn approval_scope_for_state(state: SupervisorTaskState) -> Option<ApprovalScope> {
    match state {
        SupervisorTaskState::PendingApproval => Some(ApprovalScope::PreExecution),
        SupervisorTaskState::ReviewPending => Some(ApprovalScope::Review),
        SupervisorTaskState::TestPending => Some(ApprovalScope::TestValidation),
        _ => None,
    }
}

fn collaboration_request_kind_for_scope(scope: ApprovalScope) -> CollaborationRequestKind {
    match scope {
        ApprovalScope::PreExecution => CollaborationRequestKind::ApprovalRequest,
        ApprovalScope::Review => CollaborationRequestKind::ReviewRequest,
        ApprovalScope::TestValidation => CollaborationRequestKind::TestValidationRequest,
    }
}

fn team_message_kind_for_scope(scope: ApprovalScope) -> TeamMessageKind {
    match scope {
        ApprovalScope::PreExecution => TeamMessageKind::ApprovalRequest,
        ApprovalScope::Review => TeamMessageKind::ReviewRequest,
        ApprovalScope::TestValidation => TeamMessageKind::TestValidationRequest,
    }
}

fn gate_request_message_content(record: &SupervisorTaskRecord, scope: ApprovalScope) -> String {
    let task_summary = record
        .task
        .name
        .as_deref()
        .unwrap_or(record.task.prompt.as_str());
    match scope {
        ApprovalScope::PreExecution => {
            format!("Pre-execution approval requested for: {task_summary}")
        }
        ApprovalScope::Review => format!("Review requested for: {task_summary}"),
        ApprovalScope::TestValidation => format!("Test validation requested for: {task_summary}"),
    }
}

fn build_gate_request_message(
    run_id: &str,
    record: &SupervisorTaskRecord,
    scope: ApprovalScope,
) -> TeamMessage {
    let note = record.approval.note.clone();
    let mut action_request = TeamActionRequest::new(
        collaboration_request_kind_for_scope(scope),
        Some("orchestrator".to_string()),
        note.clone(),
    );
    action_request.approval_scope = Some(scope);
    action_request.requested_for_actor_kinds = record.approval.allowed_actor_kinds(scope).to_vec();

    let mut message = TeamMessage::new(
        run_id.to_string(),
        Some(record.task.id.clone()),
        team_message_kind_for_scope(scope),
        Some("orchestrator".to_string()),
        None,
        gate_request_message_content(record, scope),
    )
    .with_action_request(action_request);

    if let Some(result) = record.result.as_ref() {
        message = message.with_result_reference(TeamResultReference::from_task_result(result));
        message = message.with_artifact_references(
            result
                .artifacts
                .iter()
                .map(|artifact| {
                    TeamArtifactReference::from_task_artifact(
                        Some(record.task.id.clone()),
                        artifact,
                    )
                })
                .collect(),
        );
    }

    message
}

fn build_delegated_task_memory_handoff_content(
    record: &SupervisorTaskRecord,
    task_result: &TaskResult,
    memory_file_path: Option<&Path>,
) -> String {
    let task_name = record
        .task
        .name
        .as_deref()
        .or(task_result.summary.as_deref())
        .unwrap_or(record.task.id.as_str());
    let outcome = format!("{:?}", record.state).to_ascii_lowercase();
    let mut lines = vec![format!("Delegated task handoff for {task_name}")];
    lines.push(format!("Outcome: {outcome}"));
    lines.push(format!("Duration: {} ms", task_result.duration_ms));

    if let Some(summary) = task_result
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
    {
        lines.push(format!(
            "Summary: {}",
            truncate_delegated_task_message(summary, 240)
        ));
    }

    if !task_result.output.trim().is_empty() {
        lines.push(format!(
            "Result: {}",
            truncate_delegated_task_message(&task_result.output, 400)
        ));
    }

    if !task_result.artifacts.is_empty() {
        let artifact_names = task_result
            .artifacts
            .iter()
            .map(|artifact| artifact.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("Artifacts: {artifact_names}"));
    }

    if let Some(path) = memory_file_path {
        lines.push(format!("Memory: {}", path.display()));
    }

    lines.join("\n")
}

fn truncate_delegated_task_message(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let truncated = trimmed.chars().take(max_chars).collect::<String>();
    if trimmed.chars().count() > max_chars {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn resolve_open_gate_request(
    record: &mut SupervisorTaskRecord,
    scope: ApprovalScope,
    actor_id: &str,
    status: CollaborationActionStatus,
    note: Option<String>,
) -> Option<(String, String)> {
    let message = record.messages.iter_mut().rev().find(|message| {
        message.action_request.as_ref().is_some_and(|request| {
            request.approval_scope == Some(scope) && request.requires_attention()
        })
    })?;

    if let Some(request) = message.action_request.as_mut() {
        request.resolve(status, Some(actor_id.to_string()), note);
    }
    message.unread_by_agent_ids.clear();
    Some((
        message.effective_thread_id().to_string(),
        message.id.clone(),
    ))
}

fn format_approval_decision_message(decision: &ApprovalDecision) -> String {
    format!(
        "{:?} gate {:?} by {} ({:?}){}",
        decision.scope,
        decision.decision,
        decision.actor.id,
        decision.actor.kind,
        decision
            .note
            .as_ref()
            .map(|note| format!(": {note}"))
            .unwrap_or_default()
    )
}

#[async_trait::async_trait]
impl OrchestratorAgentManager for crate::agents::AgentManager {
    async fn get_agent_status(&self, id: &str) -> Option<AgentInfo> {
        crate::agents::AgentManager::get_agent_status(self, id).await
    }

    async fn list_agents(&self) -> Vec<AgentInfo> {
        crate::agents::AgentManager::list_agents(self).await
    }

    async fn update_activity(&self, id: &str) {
        crate::agents::AgentManager::update_activity(self, id).await;
    }
}

// ── Knowledge store (G6) ────────────────────────────────────────────────────
// Module-level singletons so subagent pipelines are always wired with the
// built-in knowledge base, mirroring the pattern in `gestura-gui/src/api.rs`.

/// Global knowledge store for orchestrator pipelines.
static ORCHESTRATOR_KNOWLEDGE_STORE: OnceLock<crate::KnowledgeStore> = OnceLock::new();

/// Global knowledge settings manager for orchestrator pipelines.
static ORCHESTRATOR_KNOWLEDGE_SETTINGS: OnceLock<crate::KnowledgeSettingsManager> = OnceLock::new();

fn orchestrator_knowledge_store() -> &'static crate::KnowledgeStore {
    ORCHESTRATOR_KNOWLEDGE_STORE.get_or_init(|| {
        let store = crate::KnowledgeStore::with_default_dir();
        crate::register_builtin_knowledge(&store);
        if let Err(e) = store.load_user_items() {
            tracing::warn!(error = %e, "Failed to load persisted user knowledge (continuing)");
        }
        store
    })
}

fn orchestrator_knowledge_settings() -> &'static crate::KnowledgeSettingsManager {
    ORCHESTRATOR_KNOWLEDGE_SETTINGS.get_or_init(|| {
        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        crate::KnowledgeSettingsManager::new(base_dir)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_gestura_agent_card;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_environment(id: &str, root_dir: PathBuf) -> ExecutionEnvironment {
        ExecutionEnvironment {
            id: id.to_string(),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            root_dir,
            write_access: true,
            branch_name: None,
            worktree_path: None,
            remote_url: None,
            state: EnvironmentState::Ready,
            health: EnvironmentHealth::Clean,
            cleanup_policy: CleanupPolicy::KeepAlways,
            recovery_status: RecoveryStatus::NotRequired,
            recovery_action: None,
            failure: None,
            cleanup_result: None,
        }
    }

    fn empty_supervisor_run(id: &str, workspace_dir: PathBuf) -> SupervisorRun {
        SupervisorRun {
            id: id.to_string(),
            name: Some(id.to_string()),
            session_id: None,
            workspace_dir: Some(workspace_dir),
            lead_agent_id: Some("supervisor-root".to_string()),
            parent_run: None,
            child_runs: Vec::new(),
            hierarchy_depth: 0,
            max_hierarchy_depth: MAX_CHILD_SUPERVISOR_DEPTH,
            inherited_policy: None,
            status: SupervisorRunStatus::Draft,
            task_summary: SupervisorRunTaskSummary::default(),
            hierarchy_summary: None,
            tasks: Vec::new(),
            messages: Vec::new(),
            shared_cognition: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
            metadata: None,
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        runs: tokio::sync::Mutex<Vec<SupervisorRun>>,
    }

    #[async_trait::async_trait]
    impl OrchestratorObserver for RecordingObserver {
        async fn on_task_started(&self, _task: DelegatedTask) {}

        async fn on_task_completed(&self, _task: DelegatedTask, _result: TaskResult) {}

        async fn on_run_updated(&self, run: SupervisorRun) {
            self.runs.lock().await.push(run);
        }
    }

    async fn seed_review_pending_task(
        orchestrator: &AgentOrchestrator<crate::agents::AgentManager>,
        workspace_dir: PathBuf,
        test_required: bool,
    ) {
        let task = DelegatedTask {
            id: "task-approval".into(),
            agent_id: "agent-reviewer".into(),
            prompt: "Review the patch".into(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-approval".into()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Reviewer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: true,
            test_required,
            workspace_dir: Some(workspace_dir.clone()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: Some("env-approval".into()),
            remote_target: None,
            memory_tags: vec![],
            name: Some("Review the patch".into()),
        };

        let record = SupervisorTaskRecord {
            task: task.clone(),
            state: SupervisorTaskState::ReviewPending,
            approval: TaskApprovalRecord::pending(
                &task,
                ApprovalScope::Review,
                ApprovalActor::system("orchestrator"),
                Some("Execution finished. Awaiting explicit review approval.".into()),
            ),
            environment_id: "env-approval".into(),
            environment: test_environment("env-approval", workspace_dir.clone()),
            claimed_by: Some(task.agent_id.clone()),
            attempts: 0,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            local_execution: None,
            messages: vec![],
            checkpoint: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };

        let run = SupervisorRun {
            id: "run-approval".into(),
            name: Some("approval-run".into()),
            session_id: None,
            workspace_dir: Some(workspace_dir),
            lead_agent_id: None,
            parent_run: None,
            child_runs: Vec::new(),
            hierarchy_depth: 0,
            max_hierarchy_depth: MAX_CHILD_SUPERVISOR_DEPTH,
            inherited_policy: None,
            task_summary: SupervisorRunTaskSummary::default(),
            hierarchy_summary: None,
            metadata: None,
            status: SupervisorRunStatus::Waiting,
            tasks: vec![record],
            messages: vec![],
            shared_cognition: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(run.id.clone(), run);
        orchestrator
            .task_run_index
            .lock()
            .await
            .insert(task.id.clone(), "run-approval".into());
    }

    fn sample_task_result(success: bool, output: &str) -> TaskResult {
        TaskResult {
            task_id: "task-approval".into(),
            agent_id: "agent-reviewer".into(),
            success,
            run_id: Some("run-approval".into()),
            tracking_task_id: Some("tracking-approval".into()),
            output: output.into(),
            summary: None,
            tool_calls: vec![],
            artifacts: vec![],
            terminal_state_hint: None,
            duration_ms: 125,
        }
    }

    fn sample_task_record(
        workspace_dir: PathBuf,
        state: SupervisorTaskState,
        reviewer_required: bool,
        test_required: bool,
    ) -> SupervisorTaskRecord {
        let task = DelegatedTask {
            id: "task-approval".into(),
            agent_id: "agent-reviewer".into(),
            prompt: "Review the patch".into(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-approval".into()),
            directive_id: Some("directive-approval".into()),
            tracking_task_id: Some("tracking-approval".into()),
            run_id: Some("run-approval".into()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Reviewer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required,
            test_required,
            workspace_dir: Some(workspace_dir.clone()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: Some("env-approval".into()),
            remote_target: None,
            memory_tags: vec!["quality".into()],
            name: Some("Review the patch".into()),
        };

        SupervisorTaskRecord {
            task: task.clone(),
            state,
            approval: TaskApprovalRecord::not_required(&task),
            environment_id: "env-approval".into(),
            environment: test_environment("env-approval", workspace_dir),
            claimed_by: Some(task.agent_id.clone()),
            attempts: 1,
            blocked_reasons: vec![],
            result: Some(sample_task_result(true, "Patch applied and checks passed.")),
            remote_execution: None,
            local_execution: None,
            messages: vec![],
            checkpoint: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: Some(Utc::now()),
            completed_at: Some(Utc::now()),
        }
    }

    #[tokio::test]
    async fn test_orchestrator_creation_and_spawn() {
        let manager = crate::agents::AgentManager::new(PathBuf::from("/tmp/test.db"));
        let config = AppConfig::default();

        let orchestrator = AgentOrchestrator::new(manager, config);
        assert!(orchestrator.list_subagents().await.is_empty());

        orchestrator
            .spawn_subagent("test-1", "Test Agent")
            .await
            .unwrap();

        let agents = orchestrator.list_subagents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "test-1");
    }

    #[tokio::test]
    async fn test_delegate_task_submission() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("test.db"));
        let config = AppConfig::default();

        let orchestrator = AgentOrchestrator::new(manager, config);

        let task = DelegatedTask {
            id: "task-1".into(),
            agent_id: "agent-1".into(),
            prompt: "Hello".into(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-test".into()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: None,
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec![],
            name: None,
        };

        // Verify task can be submitted
        let id = orchestrator.delegate_task(task).await.unwrap();
        assert_eq!(id, "task-1");
    }

    #[tokio::test]
    async fn test_review_gate_rejects_unauthorized_actor() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("approval.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        seed_review_pending_task(&orchestrator, tmp.path().to_path_buf(), false).await;

        let error = orchestrator
            .approve_task(
                "task-approval",
                ApprovalActor::new(ApprovalActorKind::Tester, "tester-1"),
                Some("Looks good".into()),
            )
            .await
            .unwrap_err();

        assert!(error.contains("not authorized"));
    }

    #[tokio::test]
    async fn test_review_gate_records_authorized_decision() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("approval-success.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        seed_review_pending_task(&orchestrator, tmp.path().to_path_buf(), false).await;

        orchestrator
            .approve_task(
                "task-approval",
                ApprovalActor::new(ApprovalActorKind::Reviewer, "reviewer-1"),
                Some("Approved after review".into()),
            )
            .await
            .unwrap();

        let run = orchestrator
            .supervisor_runs
            .lock()
            .await
            .get("run-approval")
            .cloned()
            .unwrap();
        let record = run
            .tasks
            .iter()
            .find(|record| record.task.id == "task-approval")
            .unwrap();

        assert_eq!(record.state, SupervisorTaskState::Completed);
        assert_eq!(record.approval.state, ApprovalState::Approved);
        assert_eq!(record.approval.decisions.len(), 1);
        assert_eq!(
            record.approval.decisions[0].actor.kind,
            ApprovalActorKind::Reviewer
        );
        assert_eq!(
            run.messages.last().map(|message| message.kind),
            Some(TeamMessageKind::ApprovalDecision)
        );
    }

    #[tokio::test]
    async fn test_review_then_test_gates_require_separate_authorized_decisions() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("approval-multistep.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        seed_review_pending_task(&orchestrator, tmp.path().to_path_buf(), true).await;

        orchestrator
            .approve_task(
                "task-approval",
                ApprovalActor::new(ApprovalActorKind::Reviewer, "reviewer-1"),
                Some("Review passed".into()),
            )
            .await
            .unwrap();

        {
            let run = orchestrator
                .supervisor_runs
                .lock()
                .await
                .get("run-approval")
                .cloned()
                .unwrap();
            let record = run
                .tasks
                .iter()
                .find(|record| record.task.id == "task-approval")
                .unwrap();
            assert_eq!(record.state, SupervisorTaskState::TestPending);
            assert_eq!(record.approval.state, ApprovalState::Pending);
            assert_eq!(record.approval.scope, Some(ApprovalScope::TestValidation));
            assert_eq!(record.approval.requests.len(), 2);
            assert_eq!(record.approval.decisions.len(), 1);
        }

        let error = orchestrator
            .approve_task(
                "task-approval",
                ApprovalActor::new(ApprovalActorKind::Reviewer, "reviewer-1"),
                Some("Trying to approve test gate".into()),
            )
            .await
            .unwrap_err();
        assert!(error.contains("not authorized"));

        orchestrator
            .approve_task(
                "task-approval",
                ApprovalActor::new(ApprovalActorKind::Tester, "tester-1"),
                Some("Tests passed".into()),
            )
            .await
            .unwrap();

        let run = orchestrator
            .supervisor_runs
            .lock()
            .await
            .get("run-approval")
            .cloned()
            .unwrap();
        let record = run
            .tasks
            .iter()
            .find(|record| record.task.id == "task-approval")
            .unwrap();

        assert_eq!(record.state, SupervisorTaskState::Completed);
        assert_eq!(record.approval.state, ApprovalState::Approved);
        assert_eq!(record.approval.decisions.len(), 2);
        assert_eq!(
            record.approval.decisions[1].scope,
            ApprovalScope::TestValidation
        );
        assert_eq!(
            record.approval.decisions[1].actor.kind,
            ApprovalActorKind::Tester
        );
    }

    #[test]
    fn task_record_outcome_signals_include_gate_and_terminal_decisions() {
        let workspace = tempdir().unwrap();
        let mut record = sample_task_record(
            workspace.path().to_path_buf(),
            SupervisorTaskState::Completed,
            true,
            true,
        );
        record.approval.decisions.push(ApprovalDecision {
            id: "decision-review".into(),
            request_id: "request-review".into(),
            scope: ApprovalScope::Review,
            actor: ApprovalActor::new(ApprovalActorKind::Reviewer, "reviewer-1"),
            decision: ApprovalDecisionKind::Approved,
            decided_at: Utc::now(),
            note: Some("Review sign-off recorded.".into()),
        });
        record.approval.decisions.push(ApprovalDecision {
            id: "decision-test".into(),
            request_id: "request-test".into(),
            scope: ApprovalScope::TestValidation,
            actor: ApprovalActor::new(ApprovalActorKind::Tester, "tester-1"),
            decision: ApprovalDecisionKind::Approved,
            decided_at: Utc::now(),
            note: Some("Targeted tests passed.".into()),
        });

        let signals = task_record_outcome_signals(&record);
        let labels = outcome_signal_labels(&signals);
        let summary = outcome_signal_summary(&signals).unwrap();

        assert!(labels.contains(&"task_completed".to_string()));
        assert!(labels.contains(&"review_approved".to_string()));
        assert!(labels.contains(&"test_validation_approved".to_string()));
        assert!(summary.contains("Review sign-off recorded."));
        assert!(summary.contains("Targeted tests passed."));
    }

    #[tokio::test]
    async fn delegated_task_memory_persists_outcome_provenance() {
        let workspace = tempdir().unwrap();
        let mut record = sample_task_record(
            workspace.path().to_path_buf(),
            SupervisorTaskState::Completed,
            true,
            true,
        );
        record.approval.decisions.push(ApprovalDecision {
            id: "decision-review".into(),
            request_id: "request-review".into(),
            scope: ApprovalScope::Review,
            actor: ApprovalActor::new(ApprovalActorKind::Reviewer, "reviewer-1"),
            decision: ApprovalDecisionKind::Approved,
            decided_at: Utc::now(),
            note: Some("Review sign-off recorded.".into()),
        });
        record.approval.decisions.push(ApprovalDecision {
            id: "decision-test".into(),
            request_id: "request-test".into(),
            scope: ApprovalScope::TestValidation,
            actor: ApprovalActor::new(ApprovalActorKind::Tester, "tester-1"),
            decision: ApprovalDecisionKind::Approved,
            decided_at: Utc::now(),
            note: Some("Targeted tests passed.".into()),
        });

        let path = persist_delegated_task_memory(&record).await.unwrap();
        let entry = crate::memory_bank::load_from_memory_bank(&path)
            .await
            .unwrap();

        assert_eq!(
            entry.outcome_labels,
            vec![
                "task_completed",
                "review_approved",
                "test_validation_approved"
            ]
        );
        assert!(
            entry
                .outcome_summary
                .unwrap_or_default()
                .contains("Review sign-off recorded.")
        );
        assert!(entry.content.contains("## Outcome Signals"));
    }

    #[tokio::test]
    async fn test_delegate_task_creates_pre_execution_collaboration_request() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("collaboration-request.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-collab".into(),
                agent_id: "agent-impl".into(),
                prompt: "Implement the feature".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-collab".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: true,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(tmp.path().to_path_buf()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: Some("env-collab".into()),
                remote_target: None,
                memory_tags: vec![],
                name: Some("Implement the feature".into()),
            })
            .await
            .unwrap();

        let run = orchestrator.get_supervisor_run("run-collab").await.unwrap();
        let record = run.tasks.first().unwrap();
        let message = record.messages.first().unwrap();

        assert_eq!(record.state, SupervisorTaskState::PendingApproval);
        assert_eq!(message.kind, TeamMessageKind::ApprovalRequest);
        assert_eq!(
            message
                .action_request
                .as_ref()
                .and_then(|request| request.approval_scope),
            Some(ApprovalScope::PreExecution)
        );
        assert_eq!(
            message
                .action_request
                .as_ref()
                .map(|request| request.status),
            Some(CollaborationActionStatus::Open)
        );

        let threads = orchestrator.list_team_threads("run-collab").await;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].status, CollaborationThreadStatus::ActionRequired);
        assert!(threads[0].requires_attention);
    }

    #[tokio::test]
    async fn test_thread_actions_can_be_acknowledged_resolved_and_archived() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("thread-actions.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-thread-actions".into(),
                agent_id: "agent-impl".into(),
                prompt: "Implement the feature".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-thread-actions".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: true,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(tmp.path().to_path_buf()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: Some("env-thread-actions".into()),
                remote_target: None,
                memory_tags: vec![],
                name: Some("Implement the feature".into()),
            })
            .await
            .unwrap();

        let thread_id = orchestrator
            .list_team_threads("run-thread-actions")
            .await
            .first()
            .unwrap()
            .id
            .clone();

        let acknowledged = orchestrator
            .update_team_thread_action(
                "run-thread-actions",
                &thread_id,
                CollaborationActionStatus::Acknowledged,
                Some("reviewer-1".to_string()),
                Some("Looking now".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(
            acknowledged.status,
            CollaborationThreadStatus::ActionRequired
        );
        assert_eq!(
            acknowledged
                .latest_action_request
                .as_ref()
                .map(|request| request.status),
            Some(CollaborationActionStatus::Acknowledged)
        );

        let resolved = orchestrator
            .update_team_thread_action(
                "run-thread-actions",
                &thread_id,
                CollaborationActionStatus::Resolved,
                Some("reviewer-1".to_string()),
                Some("Approved after review".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(resolved.status, CollaborationThreadStatus::Resolved);
        assert!(!resolved.requires_attention);

        let archived = orchestrator
            .archive_team_thread(
                "run-thread-actions",
                &thread_id,
                Some("reviewer-1".to_string()),
                Some("Archive resolved thread".to_string()),
            )
            .await
            .unwrap();
        assert!(archived.archived);
        assert!(
            orchestrator
                .list_team_threads("run-thread-actions")
                .await
                .is_empty()
        );
        assert_eq!(
            orchestrator
                .list_team_threads_with_options("run-thread-actions", true)
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn test_blocker_collaboration_marks_task_blocked_until_resolved() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("thread-blocker.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-blocker".into(),
                agent_id: "agent-impl".into(),
                prompt: "Implement the feature".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-blocker".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: true,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(tmp.path().to_path_buf()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: Some("env-blocker".into()),
                remote_target: None,
                memory_tags: vec![],
                name: Some("Implement the feature".into()),
            })
            .await
            .unwrap();

        let blocker = orchestrator
            .send_team_message_draft(
                "run-blocker",
                TeamMessageDraft {
                    task_id: Some("task-blocker".to_string()),
                    kind: TeamMessageKind::Blocker,
                    sender_agent_id: Some("agent-impl".to_string()),
                    recipient_agent_id: None,
                    content: "Waiting on credentials".to_string(),
                    thread_id: None,
                    reply_to_message_id: None,
                    action_request: Some(TeamActionRequestDraft {
                        kind: CollaborationRequestKind::BlockerEscalation,
                        requested_for_agent_ids: Vec::new(),
                        requested_for_roles: vec![AgentRole::Supervisor],
                        requested_for_actor_kinds: Vec::new(),
                        approval_scope: None,
                        note: Some("Need credentials".to_string()),
                    }),
                    escalation: Some(TeamEscalationDraft {
                        level: CollaborationEscalationLevel::Warning,
                        escalated_by_agent_id: Some("agent-impl".to_string()),
                        target_role: Some(AgentRole::Supervisor),
                        note: Some("Credentials missing".to_string()),
                    }),
                    unread_by_agent_ids: Vec::new(),
                },
            )
            .await
            .unwrap();

        let run = orchestrator
            .get_supervisor_run("run-blocker")
            .await
            .unwrap();
        let record = run.tasks.first().unwrap();
        assert_eq!(record.state, SupervisorTaskState::Blocked);
        assert!(
            record
                .blocked_reasons
                .iter()
                .any(|reason| reason == "Waiting on credentials")
        );

        orchestrator
            .update_team_thread_action(
                "run-blocker",
                blocker.effective_thread_id(),
                CollaborationActionStatus::Resolved,
                Some("supervisor-1".to_string()),
                Some("Credentials delivered".to_string()),
            )
            .await
            .unwrap();

        let resolved_run = orchestrator
            .get_supervisor_run("run-blocker")
            .await
            .unwrap();
        let resolved_record = resolved_run.tasks.first().unwrap();
        assert!(resolved_record.blocked_reasons.is_empty());
        assert_eq!(resolved_record.state, SupervisorTaskState::Queued);
    }

    #[tokio::test]
    async fn test_team_message_publishes_shared_cognition_to_run_and_memory_bank() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("shared-cognition.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-shared".into(),
                agent_id: "agent-impl".into(),
                prompt: "Implement the feature".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: Some("session-shared".into()),
                directive_id: Some("directive-shared".into()),
                tracking_task_id: None,
                run_id: Some("run-shared".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: true,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(tmp.path().to_path_buf()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec!["frontend".to_string()],
                name: Some("Implement the feature".into()),
            })
            .await
            .unwrap();

        orchestrator
            .send_team_message(
                "run-shared",
                Some("task-shared".to_string()),
                TeamMessageKind::StatusUpdate,
                Some("supervisor".to_string()),
                Some("agent-impl".to_string()),
                "Use ripgrep first and keep the worktree clean.",
            )
            .await
            .unwrap();

        let run = orchestrator.get_supervisor_run("run-shared").await.unwrap();
        assert_eq!(run.shared_cognition.len(), 1);
        let note = &run.shared_cognition[0];
        assert_eq!(note.kind, SharedCognitionKind::Steering);
        assert_eq!(note.task_id.as_deref(), Some("task-shared"));
        assert_eq!(note.directive_id.as_deref(), Some("directive-shared"));
        assert!(note.tags.contains(&SHARED_COGNITION_TAG.to_string()));
        assert!(note.tags.contains(&workflow_run_memory_tag("run-shared")));

        let query = crate::memory_bank::MemoryBankQuery::default()
            .with_category(SHARED_COGNITION_CATEGORY)
            .with_task("task-shared")
            .with_directive("directive-shared")
            .with_tags(vec![workflow_run_memory_tag("run-shared")])
            .with_limit(5);
        let results = crate::memory_bank::search_memory_bank_with_query(tmp.path(), &query)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].entry.category.as_deref(),
            Some(SHARED_COGNITION_CATEGORY)
        );
        assert!(
            results[0]
                .entry
                .tags
                .contains(&workflow_run_memory_tag("run-shared"))
        );
    }

    #[tokio::test]
    async fn test_team_message_publishes_partial_discovery_and_blocker_shared_cognition() {
        let tmp = tempdir().unwrap();
        let manager =
            crate::agents::AgentManager::new(tmp.path().join("shared-cognition-partial.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-partial".into(),
                agent_id: "agent-impl".into(),
                prompt: "Investigate the flaky test".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: Some("session-partial".into()),
                directive_id: Some("directive-partial".into()),
                tracking_task_id: None,
                run_id: Some("run-partial".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: true,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(tmp.path().to_path_buf()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec!["flaky-test".to_string()],
                name: Some("Investigate flaky test".into()),
            })
            .await
            .unwrap();

        orchestrator
            .send_team_message(
                "run-partial",
                Some("task-partial".to_string()),
                TeamMessageKind::StatusUpdate,
                Some("agent-impl".to_string()),
                Some("supervisor".to_string()),
                "Partial finding: the failure appears after the fixture cache is cleared.",
            )
            .await
            .unwrap();
        orchestrator
            .send_team_message(
                "run-partial",
                Some("task-partial".to_string()),
                TeamMessageKind::Blocker,
                Some("agent-impl".to_string()),
                Some("supervisor".to_string()),
                "Blocked on reproducing the cleanup timing issue without the CI fixture bundle.",
            )
            .await
            .unwrap();

        let run = orchestrator
            .get_supervisor_run("run-partial")
            .await
            .unwrap();
        assert_eq!(run.shared_cognition.len(), 2);
        assert_eq!(run.shared_cognition[0].kind, SharedCognitionKind::Discovery);
        assert_eq!(run.shared_cognition[1].kind, SharedCognitionKind::Blocker);
        assert_eq!(
            run.shared_cognition[0].sender_agent_id.as_deref(),
            Some("agent-impl")
        );
        assert_eq!(
            run.shared_cognition[1].sender_agent_id.as_deref(),
            Some("agent-impl")
        );
        assert!(run.shared_cognition[0].confidence < run.shared_cognition[1].confidence);

        assert!(run.shared_cognition[0].summary.contains("Partial finding"));
        assert!(
            run.shared_cognition[1]
                .summary
                .contains("Blocked on reproducing")
        );
    }

    #[tokio::test]
    async fn test_team_message_publishes_conflicting_hypotheses_from_multiple_agents() {
        let tmp = tempdir().unwrap();
        let manager =
            crate::agents::AgentManager::new(tmp.path().join("shared-cognition-hypotheses.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        let make_task = |id: &str, agent_id: &str| DelegatedTask {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            prompt: "Compare ownership approaches".into(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-hypothesis".into()),
            directive_id: Some("directive-hypothesis".into()),
            tracking_task_id: None,
            run_id: Some("run-hypothesis".into()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: true,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec!["ownership".to_string()],
            name: Some(format!("Ownership check {agent_id}")),
        };

        orchestrator
            .delegate_task(make_task("task-h1", "agent-a"))
            .await
            .unwrap();
        orchestrator
            .delegate_task(make_task("task-h2", "agent-b"))
            .await
            .unwrap();
        {
            let mut runs = orchestrator.supervisor_runs.lock().await;
            runs.get_mut("run-hypothesis").unwrap().lead_agent_id = Some("supervisor".to_string());
        }

        orchestrator
            .send_team_message(
                "run-hypothesis",
                Some("task-h1".to_string()),
                TeamMessageKind::Clarification,
                Some("agent-a".to_string()),
                Some("supervisor".to_string()),
                "Hypothesis A: the bug is in ownership normalization before task routing.",
            )
            .await
            .unwrap();
        orchestrator
            .send_team_message(
                "run-hypothesis",
                Some("task-h2".to_string()),
                TeamMessageKind::Clarification,
                Some("agent-b".to_string()),
                Some("supervisor".to_string()),
                "Hypothesis B: the bug is downstream in the assignment cache after routing succeeds.",
            )
            .await
            .unwrap();

        let run = orchestrator
            .get_supervisor_run("run-hypothesis")
            .await
            .unwrap();
        let hypothesis_notes = run
            .shared_cognition
            .iter()
            .filter(|note| note.kind == SharedCognitionKind::Hypothesis)
            .collect::<Vec<_>>();
        assert_eq!(hypothesis_notes.len(), 2);
        assert!(
            hypothesis_notes
                .iter()
                .any(|note| note.sender_agent_id.as_deref() == Some("agent-a"))
        );
        assert!(
            hypothesis_notes
                .iter()
                .any(|note| note.sender_agent_id.as_deref() == Some("agent-b"))
        );

        assert!(
            hypothesis_notes
                .iter()
                .any(|note| note.summary.contains("Hypothesis A"))
        );
        assert!(
            hypothesis_notes
                .iter()
                .any(|note| note.summary.contains("Hypothesis B"))
        );
    }

    #[tokio::test]
    #[cfg(not(target_os = "windows"))]
    async fn test_child_supervisor_run_inherits_policy_and_task_defaults() {
        use std::process::Command;

        let tmp = tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-b", "main"])
                .current_dir(tmp.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(tmp.path().join("README.md"), "workspace root\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "."])
                .current_dir(tmp.path())
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Gestura Tests",
                    "-c",
                    "user.email=tests@example.com",
                    "commit",
                    "-m",
                    "Initial commit",
                ])
                .current_dir(tmp.path())
                .status()
                .unwrap()
                .success()
        );

        let manager = crate::agents::AgentManager::new(tmp.path().join("child-hierarchy.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        let mut parent_run = empty_supervisor_run("run-parent", tmp.path().to_path_buf());
        parent_run.session_id = Some("session-child-hierarchy".to_string());
        parent_run.inherited_policy = Some(SupervisorInheritancePolicy {
            approval_required: true,
            reviewer_required: false,
            test_required: false,
            execution_mode: Some(AgentExecutionMode::GitWorktree),
            workspace_dir: Some(tmp.path().to_path_buf()),
            memory_tags: vec!["root-tag".to_string()],
            constraint_notes: vec!["Stay within app scope".to_string()],
        });
        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(parent_run.id.clone(), parent_run);

        let child_run = orchestrator
            .create_child_supervisor_run(ChildSupervisorRunRequest {
                parent_run_id: "run-parent".to_string(),
                run_id: Some("run-child".to_string()),
                lead_agent_id: "supervisor-frontend".to_string(),
                objective: "Own frontend delivery".to_string(),
                name: Some("Frontend pod".to_string()),
                parent_task_id: None,
                session_id: None,
                workspace_dir: None,
                approval_required: false,
                reviewer_required: true,
                test_required: false,
                execution_mode: AgentExecutionMode::GitWorktree,
                memory_tags: vec!["child-tag".to_string()],
                constraint_notes: vec!["Escalate API changes".to_string()],
            })
            .await
            .unwrap();

        assert_eq!(child_run.hierarchy_depth, 1);
        assert_eq!(
            child_run.parent_run.as_ref().unwrap().parent_run_id,
            "run-parent"
        );
        let inherited = child_run.inherited_policy.as_ref().unwrap();
        assert!(inherited.approval_required);
        assert!(inherited.reviewer_required);
        assert!(inherited.memory_tags.contains(&"root-tag".to_string()));
        assert!(inherited.memory_tags.contains(&"child-tag".to_string()));

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-child".into(),
                agent_id: "implementer-1".into(),
                prompt: "Implement frontend flow".into(),
                context: None,
                required_tools: vec![],
                priority: 2,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-child".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: None,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec![],
                name: Some("Frontend implementation".into()),
            })
            .await
            .unwrap();

        let child_run = orchestrator.get_supervisor_run("run-child").await.unwrap();
        let record = child_run.tasks.first().unwrap();
        assert_eq!(record.state, SupervisorTaskState::PendingApproval);
        assert_eq!(record.task.session_id, child_run.session_id);
        assert_eq!(record.task.workspace_dir, child_run.workspace_dir);
        assert!(record.task.tracking_task_id.is_some());
        assert_eq!(record.task.execution_mode, AgentExecutionMode::GitWorktree);
        assert!(record.task.memory_tags.contains(&"root-tag".to_string()));
        assert!(record.task.memory_tags.contains(&"child-tag".to_string()));
        assert!(
            record
                .task
                .memory_tags
                .contains(&SHARED_COGNITION_TAG.to_string())
        );
        assert!(
            record
                .task
                .memory_tags
                .contains(&workflow_run_memory_tag("run-child"))
        );
    }

    #[test]
    fn local_execution_progress_from_chunk_tracks_iterations_and_tool_results() {
        let mut context = LocalExecutionTelemetryContext::default();
        let iteration_progress = local_execution_progress_from_chunk(
            &StreamChunk::AgentLoopIteration { iteration: 2 },
            &context,
        )
        .expect("iteration progress should be captured");
        assert_eq!(iteration_progress.stage.as_deref(), Some("agent_loop"));
        assert_eq!(iteration_progress.iteration, 2);

        context.iteration = 2;
        let tool_start = local_execution_progress_from_chunk(
            &StreamChunk::ToolCallStart {
                id: "tool-1".to_string(),
                name: "file".to_string(),
            },
            &context,
        )
        .expect("tool start progress should be captured");
        assert_eq!(tool_start.stage.as_deref(), Some("executing_tools"));
        assert_eq!(tool_start.current_tool_name.as_deref(), Some("file"));

        context.completed_tool_call_count = 1;
        context.last_completed_tool_duration_ms = Some(14);
        let tool_result = local_execution_progress_from_chunk(
            &StreamChunk::ToolCallResult {
                name: "file".to_string(),
                success: true,
                output: "{\"ok\":true}".to_string(),
                duration_ms: 14,
            },
            &context,
        )
        .expect("tool result progress should be captured");
        assert_eq!(tool_result.completed_tool_call_count, 1);
        assert_eq!(tool_result.last_completed_tool_duration_ms, Some(14));
        assert_eq!(
            tool_result.message.as_deref(),
            Some("Tool 'file' completed successfully")
        );
    }

    #[test]
    fn local_execution_progress_from_chunk_captures_waiting_and_token_usage() {
        let mut context = LocalExecutionTelemetryContext {
            iteration: 3,
            current_tool_name: Some("shell".to_string()),
            has_partial_content: true,
            partial_content_chars: 18,
            ..LocalExecutionTelemetryContext::default()
        };

        let shell_progress = local_execution_progress_from_chunk(
            &StreamChunk::ShellLifecycle {
                process_id: "cmd-1".to_string(),
                shell_session_id: None,
                duration_ms: None,
                command: "cargo test".to_string(),
                state: crate::streaming::ShellProcessState::Started,
                exit_code: None,
                cwd: None,
            },
            &context,
        )
        .expect("shell lifecycle progress should be captured");
        assert_eq!(shell_progress.phase, LocalExecutionPhase::Waiting);
        assert_eq!(
            shell_progress.waiting_reason,
            Some(LocalExecutionWaitingReason::ShellProcess)
        );
        assert!(shell_progress.has_partial_content);

        context.token_usage = Some(LocalExecutionTokenUsageSnapshot {
            estimated_tokens: Some(256),
            limit: Some(4096),
            percentage: Some(6),
            status: Some("green".to_string()),
            estimated_cost_usd: Some(0.0001),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            model: None,
            provider: None,
        });
        let token_progress = local_execution_progress_from_chunk(
            &StreamChunk::TokenUsageUpdate {
                estimated: 512,
                limit: 4096,
                percentage: 12,
                status: crate::streaming::TokenUsageStatus::Green,
                estimated_cost: 0.0002,
            },
            &context,
        )
        .expect("token usage progress should be captured");
        let token_usage = token_progress
            .token_usage
            .as_ref()
            .expect("token usage snapshot should be stored");
        assert_eq!(token_usage.estimated_tokens, Some(512));
        assert_eq!(token_usage.limit, Some(4096));
        assert_eq!(token_usage.percentage, Some(12));
    }

    #[test]
    fn background_message_prefers_local_execution_progress_when_available() {
        let now = Utc::now();
        let record = SupervisorTaskRecord {
            task: DelegatedTask {
                id: "task-local-progress".to_string(),
                agent_id: "agent-local".to_string(),
                prompt: "Implement local telemetry".to_string(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-local-progress".to_string()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(std::env::temp_dir()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: Some("env-local".to_string()),
                remote_target: None,
                memory_tags: vec![],
                name: Some("Local telemetry".to_string()),
            },
            state: SupervisorTaskState::Running,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-local".to_string(),
            environment: test_environment("env-local", std::env::temp_dir()),
            claimed_by: Some("agent-local".to_string()),
            attempts: 1,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            local_execution: Some(LocalExecutionRecord {
                status: "running".to_string(),
                status_reason: None,
                progress: Some(LocalExecutionProgress {
                    phase: LocalExecutionPhase::Running,
                    waiting_reason: Some(LocalExecutionWaitingReason::ShellProcess),
                    stage: Some("executing_tools".to_string()),
                    message: Some("Running tool 'file'".to_string()),
                    percent: Some(40),
                    iteration: 2,
                    current_tool_name: Some("file".to_string()),
                    last_completed_tool_name: Some("read".to_string()),
                    last_completed_tool_duration_ms: Some(9),
                    completed_tool_call_count: 1,
                    has_partial_content: true,
                    partial_content_chars: 24,
                    has_partial_thinking: false,
                    partial_thinking_chars: 0,
                    token_usage: Some(LocalExecutionTokenUsageSnapshot {
                        estimated_tokens: Some(256),
                        limit: Some(4096),
                        percentage: Some(6),
                        status: Some("green".to_string()),
                        estimated_cost_usd: Some(0.0001),
                        input_tokens: None,
                        output_tokens: None,
                        total_tokens: None,
                        model: None,
                        provider: None,
                    }),
                    environment: Some(environment_snapshot_from_execution(&test_environment(
                        "env-local",
                        std::env::temp_dir(),
                    ))),
                    updated_at: now,
                }),
                last_synced_at: now,
            }),
            messages: vec![],
            checkpoint: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: None,
        };

        let message = background_message_for_record(&record);
        assert!(message.contains("Local running"));
        assert!(message.contains("phase running"));
        assert!(message.contains("waiting shellprocess"));
        assert!(message.contains("executing_tools"));
        assert!(message.contains("tool file"));
        assert!(message.contains("1 tool call(s)"));
        assert!(message.contains("tokens 256/4096 (6%)"));
        assert!(message.contains("env ready"));
    }

    #[test]
    fn background_message_surfaces_remote_progress_and_reason() {
        let now = Utc::now();
        let record = SupervisorTaskRecord {
            task: DelegatedTask {
                id: "task-remote-progress".to_string(),
                agent_id: "agent-remote".to_string(),
                prompt: "Track remote telemetry".to_string(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-remote-progress".to_string()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: None,
                execution_mode: AgentExecutionMode::Remote,
                environment_id: None,
                remote_target: Some(RemoteAgentTarget {
                    url: "http://localhost:32145/a2a".to_string(),
                    name: Some("remote-peer".to_string()),
                    auth_token: None,
                    capabilities: vec!["shell".to_string()],
                }),
                memory_tags: vec![],
                name: Some("Remote telemetry".to_string()),
            },
            state: SupervisorTaskState::Blocked,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-remote".to_string(),
            environment: test_environment("env-remote", std::env::temp_dir()),
            claimed_by: Some("agent-remote".to_string()),
            attempts: 1,
            blocked_reasons: vec!["Awaiting remote shell completion".to_string()],
            result: None,
            remote_execution: Some(RemoteExecutionRecord {
                target: RemoteAgentTarget {
                    url: "http://localhost:32145/a2a".to_string(),
                    name: Some("remote-peer".to_string()),
                    auth_token: None,
                    capabilities: vec!["shell".to_string()],
                },
                remote_task_id: "remote-task-1".to_string(),
                status: "blocked".to_string(),
                status_reason: Some("Awaiting remote shell completion".to_string()),
                lease: None,
                progress: Some(RemoteExecutionProgress {
                    stage: Some("shell_running".to_string()),
                    message: Some("Remote shell still streaming".to_string()),
                    percent: Some(60),
                    updated_at: now,
                }),
                artifacts: vec![RemoteExecutionArtifact {
                    name: "result.txt".to_string(),
                    part_count: 1,
                    metadata: HashMap::new(),
                }],
                provenance: None,
                compatibility: RemoteExecutionCompatibility {
                    supported_features: vec!["artifacts".to_string()],
                    warnings: vec![],
                    protocol_version: Some("2025-11-25".to_string()),
                },
                last_synced_at: now,
            }),
            local_execution: None,
            messages: vec![],
            checkpoint: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: None,
        };

        let message = background_message_for_record(&record);
        assert!(message.contains("Remote blocked"));
        assert!(message.contains("Awaiting remote shell completion"));
        assert!(message.contains("60%"));
        assert!(message.contains("shell_running"));
    }

    #[tokio::test]
    async fn sync_local_execution_progress_persists_and_notifies_observer() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("local-progress.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());
        let observer = Arc::new(RecordingObserver::default());
        orchestrator.set_observer(observer.clone()).await;

        let task = DelegatedTask {
            id: "task-local-sync".to_string(),
            agent_id: "agent-local".to_string(),
            prompt: "Implement telemetry".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-local-sync".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: Some("env-local-sync".to_string()),
            remote_target: None,
            memory_tags: vec![],
            name: Some("Local sync".to_string()),
        };
        let environment = test_environment("env-local-sync", tmp.path().to_path_buf());
        orchestrator.supervisor_runs.lock().await.insert(
            "run-local-sync".to_string(),
            SupervisorRun {
                status: SupervisorRunStatus::Running,
                task_summary: SupervisorRunTaskSummary {
                    total: 1,
                    running: 1,
                    ..SupervisorRunTaskSummary::default()
                },
                tasks: vec![SupervisorTaskRecord {
                    task: task.clone(),
                    state: SupervisorTaskState::Running,
                    approval: TaskApprovalRecord::default(),
                    environment_id: environment.id.clone(),
                    environment,
                    claimed_by: Some("agent-local".to_string()),
                    attempts: 1,
                    blocked_reasons: vec![],
                    result: None,
                    remote_execution: None,
                    local_execution: None,
                    messages: vec![],
                    checkpoint: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    started_at: Some(Utc::now()),
                    completed_at: None,
                }],
                ..empty_supervisor_run("run-local-sync", tmp.path().to_path_buf())
            },
        );

        let progress = LocalExecutionProgress {
            phase: LocalExecutionPhase::Running,
            waiting_reason: None,
            stage: Some("executing_tools".to_string()),
            message: Some("Running tool 'file'".to_string()),
            percent: Some(35),
            iteration: 1,
            current_tool_name: Some("file".to_string()),
            last_completed_tool_name: Some("read".to_string()),
            last_completed_tool_duration_ms: Some(11),
            completed_tool_call_count: 1,
            has_partial_content: true,
            partial_content_chars: 42,
            has_partial_thinking: true,
            partial_thinking_chars: 17,
            token_usage: Some(LocalExecutionTokenUsageSnapshot {
                estimated_tokens: Some(512),
                limit: Some(4096),
                percentage: Some(12),
                status: Some("green".to_string()),
                estimated_cost_usd: Some(0.0002),
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                model: None,
                provider: None,
            }),
            environment: None,
            updated_at: Utc::now(),
        };

        orchestrator
            .sync_local_execution_progress(&task, progress.clone())
            .await
            .expect("local progress sync should succeed");

        let run = orchestrator
            .get_supervisor_run("run-local-sync")
            .await
            .expect("run should persist");
        let record = run.tasks.first().expect("task record should persist");
        let local_execution = record
            .local_execution
            .as_ref()
            .expect("local execution should be stored on the task");
        assert_eq!(local_execution.status, "running");
        assert_eq!(
            local_execution
                .progress
                .as_ref()
                .and_then(|progress| progress.current_tool_name.as_deref()),
            Some("file")
        );

        let observed = observer.runs.lock().await;
        assert!(observed.iter().any(|run| {
            run.id == "run-local-sync"
                && run
                    .tasks
                    .first()
                    .and_then(|record| record.local_execution.as_ref())
                    .and_then(|local| local.progress.as_ref())
                    .and_then(|progress| progress.stage.as_deref())
                    == Some("executing_tools")
        }));
    }

    #[tokio::test]
    async fn test_child_supervisor_run_updates_parent_rollup_status() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("child-rollup.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator.supervisor_runs.lock().await.insert(
            "run-parent".to_string(),
            empty_supervisor_run("run-parent", tmp.path().to_path_buf()),
        );

        orchestrator
            .create_child_supervisor_run(ChildSupervisorRunRequest {
                parent_run_id: "run-parent".to_string(),
                run_id: Some("run-child".to_string()),
                lead_agent_id: "supervisor-child".to_string(),
                objective: "Handle the frontend pod".to_string(),
                name: Some("Frontend pod".to_string()),
                parent_task_id: None,
                session_id: None,
                workspace_dir: None,
                approval_required: true,
                reviewer_required: false,
                test_required: false,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                memory_tags: vec![],
                constraint_notes: vec![],
            })
            .await
            .unwrap();

        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-child-rollup".into(),
                agent_id: "implementer-rollup".into(),
                prompt: "Build the assigned scope".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-child".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: None,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec![],
                name: Some("Build child scope".into()),
            })
            .await
            .unwrap();

        let parent_waiting = orchestrator.get_supervisor_run("run-parent").await.unwrap();
        assert_eq!(parent_waiting.status, SupervisorRunStatus::Waiting);
        assert_eq!(parent_waiting.child_runs.len(), 1);
        assert_eq!(
            parent_waiting.child_runs[0].status,
            SupervisorRunStatus::Waiting
        );

        orchestrator
            .approve_task(
                "task-child-rollup",
                ApprovalActor::new(ApprovalActorKind::Supervisor, "supervisor-root"),
                Some("Proceed".into()),
            )
            .await
            .unwrap();

        let child_task = orchestrator
            .get_supervisor_run("run-child")
            .await
            .unwrap()
            .tasks
            .first()
            .unwrap()
            .task
            .clone();
        orchestrator
            .complete_task_execution(
                child_task,
                TaskResult {
                    task_id: "task-child-rollup".to_string(),
                    agent_id: "implementer-rollup".to_string(),
                    run_id: Some("run-child".to_string()),
                    tracking_task_id: None,
                    success: true,
                    output: "Completed child work".to_string(),
                    summary: Some("Completed child work".to_string()),
                    tool_calls: Vec::new(),
                    artifacts: Vec::new(),
                    terminal_state_hint: Some(TaskTerminalStateHint::Completed),
                    duration_ms: 10,
                },
                false,
                0,
            )
            .await
            .unwrap();

        let parent_completed = orchestrator.get_supervisor_run("run-parent").await.unwrap();
        assert_eq!(parent_completed.status, SupervisorRunStatus::Completed);
        assert_eq!(
            parent_completed.child_runs[0].status,
            SupervisorRunStatus::Completed
        );
    }

    #[tokio::test]
    async fn test_child_supervisor_run_cannot_create_grandchild() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("grandchild-block.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator.supervisor_runs.lock().await.insert(
            "run-parent".to_string(),
            empty_supervisor_run("run-parent", tmp.path().to_path_buf()),
        );

        orchestrator
            .create_child_supervisor_run(ChildSupervisorRunRequest {
                parent_run_id: "run-parent".to_string(),
                run_id: Some("run-child".to_string()),
                lead_agent_id: "supervisor-child".to_string(),
                objective: "Manage sub-scope".to_string(),
                name: None,
                parent_task_id: None,
                session_id: None,
                workspace_dir: None,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                memory_tags: vec![],
                constraint_notes: vec![],
            })
            .await
            .unwrap();

        let error = orchestrator
            .create_child_supervisor_run(ChildSupervisorRunRequest {
                parent_run_id: "run-child".to_string(),
                run_id: Some("run-grandchild".to_string()),
                lead_agent_id: "supervisor-grandchild".to_string(),
                objective: "Forbidden depth".to_string(),
                name: None,
                parent_task_id: None,
                session_id: None,
                workspace_dir: None,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                memory_tags: vec![],
                constraint_notes: vec![],
            })
            .await
            .unwrap_err();

        assert!(error.contains("maximum hierarchy depth") || error.contains("child run"));
    }

    #[tokio::test]
    async fn test_hierarchy_query_apis_return_ancestry_descendants_and_leaf_tasks() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("hierarchy-queries.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        orchestrator.supervisor_runs.lock().await.insert(
            "run-parent".to_string(),
            empty_supervisor_run("run-parent", tmp.path().to_path_buf()),
        );
        orchestrator
            .create_child_supervisor_run(ChildSupervisorRunRequest {
                parent_run_id: "run-parent".to_string(),
                run_id: Some("run-child".to_string()),
                lead_agent_id: "supervisor-child".to_string(),
                objective: "Own sub-scope".to_string(),
                name: None,
                parent_task_id: None,
                session_id: None,
                workspace_dir: None,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                memory_tags: vec![],
                constraint_notes: vec![],
            })
            .await
            .unwrap();
        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-parent".into(),
                agent_id: "agent-parent".into(),
                prompt: "Root task".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-parent".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: None,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec![],
                name: Some("Root task".into()),
            })
            .await
            .unwrap();
        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-child-query".into(),
                agent_id: "agent-child".into(),
                prompt: "Child task".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-child".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: None,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec![],
                name: Some("Child task".into()),
            })
            .await
            .unwrap();

        let ancestry = orchestrator.get_supervisor_run_ancestry("run-child").await;
        assert_eq!(ancestry.len(), 1);
        assert_eq!(ancestry[0].id, "run-parent");

        let descendants = orchestrator
            .get_supervisor_run_descendants("run-parent")
            .await;
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].id, "run-child");

        let leaf_tasks = orchestrator.list_supervisor_leaf_tasks("run-parent").await;
        let leaf_ids = leaf_tasks
            .into_iter()
            .map(|record| record.task.id)
            .collect::<Vec<_>>();
        assert!(leaf_ids.contains(&"task-parent".to_string()));
        assert!(leaf_ids.contains(&"task-child-query".to_string()));
    }

    #[tokio::test]
    async fn test_child_run_cancellation_and_retry_roll_up_to_parent() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("child-cancel-retry.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        orchestrator.supervisor_runs.lock().await.insert(
            "run-parent".to_string(),
            empty_supervisor_run("run-parent", tmp.path().to_path_buf()),
        );
        orchestrator
            .create_child_supervisor_run(ChildSupervisorRunRequest {
                parent_run_id: "run-parent".to_string(),
                run_id: Some("run-child".to_string()),
                lead_agent_id: "supervisor-child".to_string(),
                objective: "Handle retryable sub-scope".to_string(),
                name: None,
                parent_task_id: None,
                session_id: None,
                workspace_dir: Some(tmp.path().to_path_buf()),
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                execution_mode: AgentExecutionMode::SharedWorkspace,
                memory_tags: vec![],
                constraint_notes: vec![],
            })
            .await
            .unwrap();
        orchestrator
            .delegate_task(DelegatedTask {
                id: "task-child-retry".into(),
                agent_id: "agent-child-retry".into(),
                prompt: "Retryable child task".into(),
                context: None,
                required_tools: vec![],
                priority: 1,
                session_id: None,
                directive_id: None,
                tracking_task_id: None,
                run_id: Some("run-child".into()),
                parent_task_id: None,
                depends_on: vec![],
                role: Some(AgentRole::Implementer),
                delegation_brief: None,
                planning_only: false,
                approval_required: false,
                reviewer_required: false,
                test_required: false,
                workspace_dir: Some(tmp.path().to_path_buf()),
                execution_mode: AgentExecutionMode::SharedWorkspace,
                environment_id: None,
                remote_target: None,
                memory_tags: vec![],
                name: Some("Retryable child task".into()),
            })
            .await
            .unwrap();

        orchestrator.cancel_task("task-child-retry").await.unwrap();
        let parent_cancelled = orchestrator.get_supervisor_run("run-parent").await.unwrap();
        assert_eq!(parent_cancelled.status, SupervisorRunStatus::Cancelled);

        orchestrator.retry_task("task-child-retry").await.unwrap();
        let parent_retried = orchestrator.get_supervisor_run("run-parent").await.unwrap();
        let child_retried = orchestrator.get_supervisor_run("run-child").await.unwrap();
        let child_record = child_retried
            .tasks
            .iter()
            .find(|record| record.task.id == "task-child-retry")
            .expect("retried child task should still exist");
        assert_eq!(child_record.attempts, 1);
        assert_ne!(child_record.state, SupervisorTaskState::Cancelled);
        assert_ne!(child_retried.status, SupervisorRunStatus::Cancelled);
        assert_ne!(parent_retried.status, SupervisorRunStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_supervisor_run_queries_attach_checkpoint_summary() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("checkpoint-summary.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let task = DelegatedTask {
            id: "task-checkpoint-summary".to_string(),
            agent_id: "agent-checkpoint".to_string(),
            prompt: "Inspect resumability".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-checkpoint".to_string()),
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-checkpoint-summary".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec![],
            name: Some("Checkpoint task".to_string()),
        };
        let now = Utc::now();
        let record = SupervisorTaskRecord {
            task: task.clone(),
            state: SupervisorTaskState::Blocked,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-checkpoint".to_string(),
            environment: test_environment("env-checkpoint", tmp.path().to_path_buf()),
            claimed_by: None,
            attempts: 1,
            blocked_reasons: vec!["Task was running when orchestrator restarted; resumable checkpoint available (before tool 'file').".to_string()],
            result: None,
            remote_execution: None,
            local_execution: None,
            messages: vec![],
            checkpoint: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        };
        let run = SupervisorRun {
            id: "run-checkpoint-summary".to_string(),
            session_id: Some("session-checkpoint".to_string()),
            workspace_dir: Some(tmp.path().to_path_buf()),
            lead_agent_id: Some("supervisor".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: MAX_CHILD_SUPERVISOR_DEPTH,
            inherited_policy: None,
            status: SupervisorRunStatus::Waiting,
            task_summary: SupervisorRunTaskSummary {
                total: 1,
                queued: 0,
                blocked: 1,
                pending_approval: 0,
                running: 0,
                review_pending: 0,
                test_pending: 0,
                completed: 0,
                failed: 0,
                cancelled: 0,
            },
            hierarchy_summary: None,
            tasks: vec![record],
            messages: vec![],
            shared_cognition: vec![],
            created_at: now,
            updated_at: now,
            completed_at: None,
            name: None,
            metadata: None,
        };
        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(run.id.clone(), run.clone());
        orchestrator
            .task_run_index
            .lock()
            .await
            .insert(task.id.clone(), run.id.clone());

        let completed_tool_call_records = vec![ToolCallRecord {
            id: "tool-file-1".to_string(),
            name: "file".to_string(),
            arguments: "{\"path\":\"README.md\"}".to_string(),
            result: crate::ToolResult::Success("{\"ok\":true}".to_string()),
            duration_ms: 12,
        }];
        let completed_tool_calls = vec![OrchestratorToolCall {
            tool_name: "file".to_string(),
            input: json!({ "path": "README.md" }),
            output: json!({ "ok": true }),
            success: true,
            duration_ms: 12,
        }];
        let checkpoint = DelegatedTaskCheckpoint {
            id: delegated_checkpoint_id(&task.id),
            task_id: task.id.clone(),
            run_id: task.run_id.clone(),
            session_id: task.session_id.clone(),
            agent_id: task.agent_id.clone(),
            environment_id: task.environment_id.clone(),
            execution_mode: task.execution_mode.clone(),
            stage: DelegatedCheckpointStage::Blocked,
            replay_safety: DelegatedReplaySafety::CheckpointResumable,
            resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
            safe_boundary_label: "after tool 'file' result".to_string(),
            workspace_dir: task.workspace_dir.clone(),
            completed_tool_calls: completed_tool_calls.clone(),
            result_published: false,
            note: Some("resume available after restart".to_string()),
            resume_state: Some(build_delegated_resume_state(
                &task,
                "prompt",
                "partial",
                "",
                &completed_tool_call_records,
                1,
            )),
            created_at: now,
            updated_at: now,
        };
        persist_checkpoint_to_disk(tmp.path(), &checkpoint).unwrap();

        let listed_run = orchestrator
            .get_supervisor_run("run-checkpoint-summary")
            .await
            .unwrap();
        let checkpoint_summary = listed_run.tasks[0].checkpoint.as_ref().unwrap();
        assert_eq!(checkpoint_summary.stage, DelegatedCheckpointStage::Blocked);
        assert_eq!(
            checkpoint_summary.resume_disposition,
            DelegatedResumeDisposition::ResumeFromCheckpoint
        );
        assert!(checkpoint_summary.has_resume_state);
        assert_eq!(checkpoint_summary.completed_tool_call_count, 1);
        assert!(
            checkpoint_summary
                .available_actions
                .contains(&DelegatedCheckpointAction::ResumeFromCheckpoint)
        );
    }

    #[tokio::test]
    async fn list_active_task_snapshots_attach_local_execution_telemetry() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("active-snapshots.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let now = Utc::now();
        let task = DelegatedTask {
            id: "task-active-snapshot".to_string(),
            agent_id: "agent-local".to_string(),
            prompt: "Surface live local telemetry".to_string(),
            context: None,
            required_tools: vec!["file".to_string()],
            priority: 2,
            session_id: Some("session-active-snapshot".to_string()),
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-active-snapshot".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: Some("env-active-snapshot".to_string()),
            remote_target: None,
            memory_tags: vec![],
            name: Some("Active telemetry task".to_string()),
        };
        let mut run = empty_supervisor_run("run-active-snapshot", tmp.path().to_path_buf());
        run.session_id = task.session_id.clone();
        run.status = SupervisorRunStatus::Running;
        run.task_summary = SupervisorRunTaskSummary {
            total: 1,
            running: 1,
            ..SupervisorRunTaskSummary::default()
        };
        run.tasks.push(SupervisorTaskRecord {
            task: task.clone(),
            state: SupervisorTaskState::Running,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-active-snapshot".to_string(),
            environment: test_environment("env-active-snapshot", tmp.path().to_path_buf()),
            claimed_by: Some(task.agent_id.clone()),
            attempts: 1,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            local_execution: Some(LocalExecutionRecord {
                status: "running".to_string(),
                status_reason: None,
                progress: Some(LocalExecutionProgress {
                    phase: LocalExecutionPhase::Waiting,
                    waiting_reason: Some(LocalExecutionWaitingReason::ShellProcess),
                    stage: Some("shell_running".to_string()),
                    message: Some("Streaming shell output".to_string()),
                    percent: Some(45),
                    iteration: 2,
                    current_tool_name: Some("shell".to_string()),
                    last_completed_tool_name: Some("file".to_string()),
                    last_completed_tool_duration_ms: Some(12),
                    completed_tool_call_count: 1,
                    has_partial_content: true,
                    partial_content_chars: 48,
                    has_partial_thinking: false,
                    partial_thinking_chars: 0,
                    token_usage: None,
                    environment: None,
                    updated_at: now,
                }),
                last_synced_at: now,
            }),
            messages: vec![],
            checkpoint: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: None,
        });
        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(run.id.clone(), run);
        orchestrator
            .task_run_index
            .lock()
            .await
            .insert(task.id.clone(), "run-active-snapshot".to_string());
        orchestrator.active_tasks.lock().await.insert(
            task.id.clone(),
            ActiveTaskControl {
                task: task.clone(),
                local_cancel_token: Some(CancellationToken::new()),
                attempt: 1,
            },
        );
        persist_checkpoint_to_disk(
            tmp.path(),
            &DelegatedTaskCheckpoint {
                id: delegated_checkpoint_id(&task.id),
                task_id: task.id.clone(),
                run_id: task.run_id.clone(),
                session_id: task.session_id.clone(),
                agent_id: task.agent_id.clone(),
                environment_id: task.environment_id.clone(),
                execution_mode: task.execution_mode.clone(),
                stage: DelegatedCheckpointStage::Running,
                replay_safety: DelegatedReplaySafety::CheckpointResumable,
                resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
                safe_boundary_label: "after file".to_string(),
                workspace_dir: task.workspace_dir.clone(),
                completed_tool_calls: vec![],
                result_published: false,
                note: Some("live telemetry checkpoint".to_string()),
                resume_state: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();

        let snapshots = orchestrator.list_active_task_snapshots().await;
        let snapshot = snapshots
            .iter()
            .find(|snapshot| snapshot.task.id == task.id)
            .unwrap();

        assert_eq!(snapshot.state, SupervisorTaskState::Running);
        assert_eq!(
            snapshot
                .local_execution
                .as_ref()
                .and_then(|local| local.progress.as_ref())
                .and_then(|progress| progress.current_tool_name.as_deref()),
            Some("shell")
        );
        assert_eq!(
            snapshot
                .local_execution
                .as_ref()
                .and_then(|local| local.progress.as_ref())
                .and_then(|progress| progress.waiting_reason),
            Some(LocalExecutionWaitingReason::ShellProcess)
        );
        assert_eq!(
            snapshot
                .checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.stage),
            Some(DelegatedCheckpointStage::Running)
        );
    }

    #[tokio::test]
    async fn test_acknowledge_blocked_task_updates_checkpoint_note() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("checkpoint-ack.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let task = DelegatedTask {
            id: "task-checkpoint-ack".to_string(),
            agent_id: "agent-checkpoint".to_string(),
            prompt: "Need operator acknowledgement".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-checkpoint".to_string()),
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-checkpoint-ack".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec![],
            name: Some("Blocked checkpoint task".to_string()),
        };
        let now = Utc::now();
        let run = SupervisorRun {
            id: "run-checkpoint-ack".to_string(),
            session_id: Some("session-checkpoint".to_string()),
            workspace_dir: Some(tmp.path().to_path_buf()),
            lead_agent_id: Some("supervisor".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: MAX_CHILD_SUPERVISOR_DEPTH,
            inherited_policy: None,
            status: SupervisorRunStatus::Waiting,
            task_summary: SupervisorRunTaskSummary {
                total: 1,
                queued: 0,
                blocked: 1,
                pending_approval: 0,
                running: 0,
                review_pending: 0,
                test_pending: 0,
                completed: 0,
                failed: 0,
                cancelled: 0,
            },
            hierarchy_summary: None,
            tasks: vec![SupervisorTaskRecord {
                task: task.clone(),
                state: SupervisorTaskState::Blocked,
                approval: TaskApprovalRecord::default(),
                environment_id: "env-checkpoint".to_string(),
                environment: test_environment("env-checkpoint", tmp.path().to_path_buf()),
                claimed_by: None,
                attempts: 1,
                blocked_reasons: vec!["manual review required".to_string()],
                result: None,
                remote_execution: None,
                local_execution: None,
                messages: vec![],
                checkpoint: None,
                created_at: now,
                updated_at: now,
                started_at: None,
                completed_at: None,
            }],
            messages: vec![],
            shared_cognition: vec![],
            created_at: now,
            updated_at: now,
            completed_at: None,
            name: None,
            metadata: None,
        };
        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(run.id.clone(), run.clone());
        orchestrator
            .task_run_index
            .lock()
            .await
            .insert(task.id.clone(), run.id.clone());

        persist_checkpoint_to_disk(
            tmp.path(),
            &DelegatedTaskCheckpoint {
                id: delegated_checkpoint_id(&task.id),
                task_id: task.id.clone(),
                run_id: task.run_id.clone(),
                session_id: task.session_id.clone(),
                agent_id: task.agent_id.clone(),
                environment_id: task.environment_id.clone(),
                execution_mode: task.execution_mode.clone(),
                stage: DelegatedCheckpointStage::Blocked,
                replay_safety: DelegatedReplaySafety::OperatorGated,
                resume_disposition: DelegatedResumeDisposition::OperatorInterventionRequired,
                safe_boundary_label: "before tool 'shell' execution".to_string(),
                workspace_dir: task.workspace_dir.clone(),
                completed_tool_calls: vec![],
                result_published: false,
                note: Some("waiting for operator".to_string()),
                resume_state: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();

        orchestrator
            .acknowledge_blocked_task(&task.id, Some("reviewed by operator".to_string()))
            .await
            .unwrap();

        let listed = orchestrator
            .get_supervisor_run("run-checkpoint-ack")
            .await
            .unwrap();
        assert_eq!(listed.tasks[0].state, SupervisorTaskState::Blocked);
        assert_eq!(
            listed.tasks[0]
                .checkpoint
                .as_ref()
                .and_then(|summary| summary.note.as_deref()),
            Some("reviewed by operator")
        );
    }

    #[tokio::test]
    async fn pause_task_requests_local_pause_intent() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("pause-local.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let task = DelegatedTask {
            id: "task-local-pause".to_string(),
            agent_id: "agent-local".to_string(),
            prompt: "Pause this local task".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-local-pause".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec![],
            name: Some("Local pause test".to_string()),
        };
        let token = CancellationToken::new();
        orchestrator.active_tasks.lock().await.insert(
            task.id.clone(),
            ActiveTaskControl {
                task: task.clone(),
                local_cancel_token: Some(token.clone()),
                attempt: 1,
            },
        );

        orchestrator.pause_task(&task.id).await.unwrap();

        assert!(token.is_cancelled());
        assert!(token.is_pause_requested());
        assert!(
            orchestrator
                .active_tasks
                .lock()
                .await
                .contains_key(&task.id)
        );
    }

    #[tokio::test]
    async fn pause_task_rejects_remote_execution() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("pause-remote.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let task = DelegatedTask {
            id: "task-remote-pause".to_string(),
            agent_id: "agent-remote".to_string(),
            prompt: "Attempt remote pause".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-remote-pause".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: None,
            execution_mode: AgentExecutionMode::Remote,
            environment_id: None,
            remote_target: Some(RemoteAgentTarget {
                url: "http://localhost:32145/a2a".to_string(),
                name: Some("remote-peer".to_string()),
                auth_token: Some("token".to_string()),
                capabilities: vec!["shell".to_string()],
            }),
            memory_tags: vec![],
            name: Some("Remote pause test".to_string()),
        };
        orchestrator.active_tasks.lock().await.insert(
            task.id.clone(),
            ActiveTaskControl {
                task: task.clone(),
                local_cancel_token: None,
                attempt: 1,
            },
        );

        let error = orchestrator.pause_task(&task.id).await.unwrap_err();
        assert!(error.contains("cannot be paused locally"));
        assert!(
            orchestrator
                .active_tasks
                .lock()
                .await
                .contains_key(&task.id)
        );
    }

    #[tokio::test]
    async fn complete_task_execution_preserves_paused_checkpoint_resume_state() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("pause-checkpoint.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let task = DelegatedTask {
            id: "task-paused-checkpoint".to_string(),
            agent_id: "agent-local".to_string(),
            prompt: "Resume me later".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-paused".to_string()),
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-paused-checkpoint".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec![],
            name: Some("Paused checkpoint task".to_string()),
        };
        let now = Utc::now();
        let mut run = empty_supervisor_run("run-paused-checkpoint", tmp.path().to_path_buf());
        run.session_id = task.session_id.clone();
        run.status = SupervisorRunStatus::Running;
        run.task_summary = SupervisorRunTaskSummary {
            total: 1,
            queued: 0,
            blocked: 0,
            pending_approval: 0,
            running: 1,
            review_pending: 0,
            test_pending: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
        };
        run.tasks.push(SupervisorTaskRecord {
            task: task.clone(),
            state: SupervisorTaskState::Running,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-paused-checkpoint".to_string(),
            environment: test_environment("env-paused-checkpoint", tmp.path().to_path_buf()),
            claimed_by: Some(task.agent_id.clone()),
            attempts: 1,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            local_execution: Some(local_execution_record_for_start()),
            messages: vec![],
            checkpoint: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: None,
        });
        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(run.id.clone(), run);
        orchestrator
            .task_run_index
            .lock()
            .await
            .insert(task.id.clone(), "run-paused-checkpoint".to_string());

        let completed_tool_call_records = vec![ToolCallRecord {
            id: "tool-file-1".to_string(),
            name: "file".to_string(),
            arguments: "{\"path\":\"README.md\"}".to_string(),
            result: crate::ToolResult::Success("{\"ok\":true}".to_string()),
            duration_ms: 12,
        }];
        orchestrator
            .persist_delegated_checkpoint(&DelegatedTaskCheckpoint {
                id: delegated_checkpoint_id(&task.id),
                task_id: task.id.clone(),
                run_id: task.run_id.clone(),
                session_id: task.session_id.clone(),
                agent_id: task.agent_id.clone(),
                environment_id: task.environment_id.clone(),
                execution_mode: task.execution_mode.clone(),
                stage: DelegatedCheckpointStage::Blocked,
                replay_safety: DelegatedReplaySafety::CheckpointResumable,
                resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
                safe_boundary_label: "operator pause during iteration 2".to_string(),
                workspace_dir: task.workspace_dir.clone(),
                completed_tool_calls: vec![OrchestratorToolCall {
                    tool_name: "file".to_string(),
                    input: serde_json::json!({ "path": "README.md" }),
                    output: serde_json::json!({ "ok": true }),
                    success: true,
                    duration_ms: 12,
                }],
                result_published: false,
                note: Some("Paused by operator; resumable checkpoint preserved.".to_string()),
                resume_state: Some(build_delegated_resume_state(
                    &task,
                    "prompt",
                    "partial",
                    "thinking",
                    &completed_tool_call_records,
                    2,
                )),
                created_at: now,
                updated_at: now,
            })
            .unwrap();

        orchestrator
            .complete_task_execution(
                task.clone(),
                TaskResult {
                    task_id: task.id.clone(),
                    agent_id: task.agent_id.clone(),
                    success: false,
                    run_id: task.run_id.clone(),
                    tracking_task_id: None,
                    output: "Paused by operator; resumable checkpoint preserved.".to_string(),
                    summary: task.name.clone(),
                    tool_calls: vec![],
                    artifacts: vec![],
                    terminal_state_hint: Some(TaskTerminalStateHint::Blocked),
                    duration_ms: 10,
                },
                true,
                1,
            )
            .await
            .unwrap();

        let listed = orchestrator
            .supervisor_runs
            .lock()
            .await
            .get("run-paused-checkpoint")
            .cloned()
            .unwrap();
        let record = &listed.tasks[0];
        assert_eq!(record.state, SupervisorTaskState::Blocked);
        assert!(
            record
                .blocked_reasons
                .iter()
                .any(|reason| reason.contains("Paused by operator"))
        );

        let persisted_checkpoint = orchestrator.load_delegated_checkpoint(&task).unwrap();
        assert_eq!(
            persisted_checkpoint.stage,
            DelegatedCheckpointStage::Blocked
        );
        assert!(!persisted_checkpoint.result_published);
        assert!(persisted_checkpoint.resume_state.is_some());
        assert!(
            checkpoint_available_actions(&persisted_checkpoint, SupervisorTaskState::Blocked,)
                .contains(&DelegatedCheckpointAction::ResumeFromCheckpoint)
        );
    }

    #[tokio::test]
    async fn complete_task_execution_publishes_memory_handoff_to_supervisor() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("memory-handoff.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            manager,
            AppConfig::default(),
            Some(tmp.path().to_path_buf()),
        );

        let task = DelegatedTask {
            id: "task-memory-handoff".to_string(),
            agent_id: "agent-impl".to_string(),
            prompt: "Implement the requested fix".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-memory-handoff".to_string()),
            directive_id: Some("directive-memory-handoff".to_string()),
            tracking_task_id: Some("tracking-memory-handoff".to_string()),
            run_id: Some("run-memory-handoff".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(tmp.path().to_path_buf()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: None,
            remote_target: None,
            memory_tags: vec!["delegation".to_string()],
            name: Some("Implement delegated fix".to_string()),
        };
        let now = Utc::now();
        let mut run = empty_supervisor_run("run-memory-handoff", tmp.path().to_path_buf());
        run.session_id = task.session_id.clone();
        run.lead_agent_id = Some("supervisor-root".to_string());
        run.status = SupervisorRunStatus::Running;
        run.task_summary = SupervisorRunTaskSummary {
            total: 1,
            queued: 0,
            blocked: 0,
            pending_approval: 0,
            running: 1,
            review_pending: 0,
            test_pending: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
        };
        run.tasks.push(SupervisorTaskRecord {
            task: task.clone(),
            state: SupervisorTaskState::Running,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-memory-handoff".to_string(),
            environment: test_environment("env-memory-handoff", tmp.path().to_path_buf()),
            claimed_by: Some(task.agent_id.clone()),
            attempts: 1,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            local_execution: Some(local_execution_record_for_start()),
            messages: vec![],
            checkpoint: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: None,
        });
        orchestrator
            .supervisor_runs
            .lock()
            .await
            .insert(run.id.clone(), run);

        orchestrator
            .complete_task_execution(
                task.clone(),
                TaskResult {
                    task_id: task.id.clone(),
                    agent_id: task.agent_id.clone(),
                    success: true,
                    run_id: task.run_id.clone(),
                    tracking_task_id: task.tracking_task_id.clone(),
                    output: "Applied the fix and verified the targeted path.".to_string(),
                    summary: Some("Delegated fix applied".to_string()),
                    tool_calls: vec![],
                    artifacts: vec![TaskArtifactRecord {
                        name: "summary.md".to_string(),
                        kind: "report".to_string(),
                        uri: Some("memory://summary".to_string()),
                        summary: Some("Delegated completion summary".to_string()),
                    }],
                    terminal_state_hint: Some(TaskTerminalStateHint::Completed),
                    duration_ms: 25,
                },
                false,
                1,
            )
            .await
            .unwrap();

        let run = orchestrator
            .supervisor_runs
            .lock()
            .await
            .get("run-memory-handoff")
            .cloned()
            .unwrap();
        let record = &run.tasks[0];
        assert!(record.messages.iter().any(|message| {
            message.kind == TeamMessageKind::Handoff
                && message.sender_agent_id.as_deref() == Some("agent-impl")
                && message
                    .result_reference
                    .as_ref()
                    .is_some_and(|result| result.success)
        }));
        assert!(
            record
                .messages
                .iter()
                .any(|message| message.content.contains("Memory:"))
        );
        assert!(run.shared_cognition.iter().any(|note| {
            note.kind == SharedCognitionKind::Handoff
                && note.sender_agent_id.as_deref() == Some("agent-impl")
        }));

        let shared_query = crate::memory_bank::MemoryBankQuery::default()
            .with_category(SHARED_COGNITION_CATEGORY)
            .with_task("task-memory-handoff")
            .with_tags(vec![workflow_run_memory_tag("run-memory-handoff")])
            .with_limit(5);
        let shared_results =
            crate::memory_bank::search_memory_bank_with_query(tmp.path(), &shared_query)
                .await
                .unwrap();
        assert_eq!(shared_results.len(), 1);
    }

    #[test]
    fn test_approval_record_deserializes_from_legacy_shape() {
        let legacy = serde_json::json!({
            "state": "pending",
            "requested_at": "2026-03-10T00:00:00Z",
            "note": "Legacy approval"
        });

        let record: TaskApprovalRecord = serde_json::from_value(legacy).unwrap();

        assert_eq!(record.state, ApprovalState::Pending);
        assert_eq!(record.scope, None);
        assert!(record.requests.is_empty());
        assert!(record.decisions.is_empty());
        assert!(record.active_request.is_none());
    }

    #[test]
    fn test_supervisor_run_deserializes_legacy_shape_without_shared_cognition() {
        let mut value = serde_json::to_value(empty_supervisor_run(
            "run-legacy",
            PathBuf::from("/tmp/gestura-legacy-run"),
        ))
        .unwrap();
        value.as_object_mut().unwrap().remove("shared_cognition");

        let run: SupervisorRun = serde_json::from_value(value).unwrap();

        assert!(run.shared_cognition.is_empty());
        assert_eq!(run.id, "run-legacy");
    }

    #[test]
    fn test_compatibility_from_card_warns_for_missing_auth_and_remote_features() {
        let mut card = create_gestura_agent_card("http://localhost:32145");
        card.authentication = Some(crate::AuthenticationInfo {
            schemes: vec!["bearer".to_string()],
            oauth2: None,
        });
        card.supported_task_features = vec!["artifacts".to_string()];
        card.supported_rpc_methods
            .retain(|method| method != "task/artifacts");
        let remote_target = RemoteAgentTarget {
            url: card.url.clone(),
            name: Some("legacy-peer".to_string()),
            auth_token: None,
            capabilities: vec!["shell".to_string()],
        };

        let compatibility = compatibility_from_card(&card, &remote_target);

        assert!(
            compatibility
                .warnings
                .iter()
                .any(|warning| warning.contains("no auth token"))
        );
        assert!(
            compatibility
                .warnings
                .iter()
                .any(|warning| warning.contains("authenticated mutation enforcement"))
        );
        assert!(
            compatibility
                .warnings
                .iter()
                .any(|warning| warning.contains("provenance support"))
        );
        assert!(
            compatibility
                .warnings
                .iter()
                .any(|warning| warning.contains("lease support"))
        );
        assert!(
            compatibility
                .warnings
                .iter()
                .any(|warning| warning.contains("idempotency support"))
        );
        assert!(
            compatibility
                .warnings
                .iter()
                .any(|warning| warning.contains("artifact manifest support"))
        );
    }

    #[test]
    fn test_build_remote_task_request_omits_unsupported_lease_and_idempotency() {
        let task = DelegatedTask {
            id: "task-remote-compat".to_string(),
            agent_id: "agent-remote".to_string(),
            prompt: "Inspect the codebase".to_string(),
            context: None,
            required_tools: vec!["shell".to_string()],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-remote-compat".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: true,
            reviewer_required: true,
            test_required: false,
            workspace_dir: None,
            execution_mode: AgentExecutionMode::Remote,
            environment_id: None,
            remote_target: Some(RemoteAgentTarget {
                url: "http://localhost:32145/a2a".to_string(),
                name: Some("legacy-peer".to_string()),
                auth_token: Some("token".to_string()),
                capabilities: vec!["shell".to_string()],
            }),
            memory_tags: vec!["integration".to_string()],
            name: Some("Compatibility task".to_string()),
        };
        let record = SupervisorTaskRecord {
            task: task.clone(),
            state: SupervisorTaskState::Queued,
            approval: TaskApprovalRecord::default(),
            environment_id: "env-compat".to_string(),
            environment: test_environment("env-compat", PathBuf::from("/tmp")),
            claimed_by: None,
            attempts: 2,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            local_execution: None,
            messages: vec![],
            checkpoint: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        let compatibility = RemoteExecutionCompatibility {
            protocol_version: Some("0.2.0".to_string()),
            supported_features: vec!["artifacts".to_string(), "provenance".to_string()],
            warnings: vec![],
        };

        let request = build_remote_task_request(&task, &record, &compatibility);

        assert!(request.idempotency_key.is_none());
        assert!(request.lease_request.is_none());
        assert_eq!(
            request
                .metadata
                .get("approvalRequired")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            request
                .metadata
                .get("reviewerRequired")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_provenance_from_metadata_restores_authenticated_caller_context() {
        let provenance = provenance_from_metadata(&HashMap::from([
            (
                "caller_agent_id".to_string(),
                serde_json::json!("remote-caller"),
            ),
            (
                "caller_name".to_string(),
                serde_json::json!("Remote Caller"),
            ),
            ("caller_version".to_string(), serde_json::json!("1.2.3")),
            (
                "caller_capabilities".to_string(),
                serde_json::json!(["shell", "file"]),
            ),
            ("caller_authenticated".to_string(), serde_json::json!(true)),
            (
                "caller_auth_scheme".to_string(),
                serde_json::json!("bearer"),
            ),
        ]))
        .expect("metadata should restore provenance");

        assert_eq!(provenance.caller_agent_id.as_deref(), Some("remote-caller"));
        assert_eq!(provenance.caller_name.as_deref(), Some("Remote Caller"));
        assert_eq!(provenance.caller_version.as_deref(), Some("1.2.3"));
        assert_eq!(provenance.caller_capabilities, vec!["shell", "file"]);
        assert!(provenance.authenticated);
        assert_eq!(provenance.auth_scheme.as_deref(), Some("bearer"));
    }
}
