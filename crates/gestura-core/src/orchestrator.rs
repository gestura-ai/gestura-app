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
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::{Duration as TokioDuration, sleep};
use uuid::Uuid;

use self::persistence::{
    load_persisted_checkpoints, load_persisted_environment_by_id, load_persisted_environments,
    load_persisted_runs, persist_checkpoint_to_disk, persist_environment_to_disk,
    persist_run_to_disk,
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

fn default_max_child_supervisor_depth() -> u8 {
    MAX_CHILD_SUPERVISOR_DEPTH
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
    /// Task-scoped coordination messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<TeamMessage>,
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

/// Orchestrator for coordinating subagents and delegated task execution.
///
/// The orchestrator is core-owned and does not depend on Tauri. GUI/CLI layers can
/// attach an [`OrchestratorObserver`] to receive lifecycle events.
#[derive(Clone)]
pub struct AgentOrchestrator<M: OrchestratorAgentManager> {
    agent_manager: M,
    permission_manager: Arc<PermissionManager>,
    active_tasks: Arc<Mutex<HashMap<String, DelegatedTask>>>,
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
        let session_id = task.session_id.clone();

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
            if let Some(policy) = run.inherited_policy.as_ref() {
                policy.apply_to_task(&mut task);
            }

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
                messages: Vec::new(),
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

        self.persist_run(&child_run)?;
        self.persist_run(&parent_run)?;
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
        active.values().cloned().collect()
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
            synchronize_run_hierarchy_snapshots(&mut in_memory_runs);
            if let Some(run) = in_memory_runs.into_iter().find(|run| run.id == run_id) {
                return Some(run);
            }
        }

        self.default_workspace_dir.as_deref().and_then(|root| {
            let mut runs = load_persisted_runs(root);
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
        let (mut run_snapshot, collaboration_messages) = {
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

            record.messages.push(approval_message.clone());
            run.messages.push(approval_message.clone());

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (run.clone(), collaboration_messages)
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

        let (mut run_snapshot, approval_message) = {
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
            record.messages.push(approval_message.clone());
            run.messages.push(approval_message.clone());
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (run.clone(), Some(approval_message))
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
        self.notify_team_message(message.clone()).await;
        Ok(message)
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
        let task_opt = {
            let mut active = self.active_tasks.lock().await;
            active.remove(task_id)
        };

        if let Some(task) = task_opt {
            tracing::info!(task_id = %task_id, agent_id = %task.agent_id, "Cancelling task");
            if matches!(task.execution_mode, AgentExecutionMode::Remote) {
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
            } else {
                self.agent_manager
                    .send_event(&task.agent_id, format!("cancel:{}", task_id))
                    .await;
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
        let (Some(workspace_dir), Some(session_id)) =
            (task.workspace_dir.as_deref(), task.session_id.as_deref())
        else {
            return;
        };
        if task.tracking_task_id.is_some() {
            return;
        }

        let manager = TaskManager::new(workspace_dir);
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

        let (run_snapshot, task_snapshot) = {
            let mut active = self.active_tasks.lock().await;
            active.insert(task.id.clone(), task.clone());
            drop(active);

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
                record.clone()
            };

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);

            (run.clone(), task_snapshot)
        };

        record_task_dispatch(&task, &task_snapshot, &run_snapshot);
        self.persist_run_with_hierarchy_sync(run_snapshot.clone())
            .await?;
        if !matches!(task.execution_mode, AgentExecutionMode::Remote) {
            self.persist_delegated_checkpoint(&delegated_start_checkpoint(&task))?;
        }

        let observer_for_start = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_start.as_ref() {
            obs.on_task_started(task.clone()).await;
        }

        let orchestrator = self.clone();
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let task_result = if matches!(task.execution_mode, AgentExecutionMode::Remote) {
                orchestrator.execute_remote_task(&task, start).await
            } else {
                let (result, tool_calls) = execute_delegated_task(&orchestrator, &task).await;
                let duration_ms = start.elapsed().as_millis() as u64;
                TaskResult {
                    task_id: task.id.clone(),
                    agent_id: task.agent_id.clone(),
                    success: result.is_ok(),
                    run_id: task.run_id.clone(),
                    tracking_task_id: task.tracking_task_id.clone(),
                    output: result.clone().unwrap_or_else(|error| error),
                    summary: task.name.clone(),
                    tool_calls,
                    artifacts: Vec::new(),
                    terminal_state_hint: Some(if result.is_ok() {
                        TaskTerminalStateHint::Completed
                    } else {
                        TaskTerminalStateHint::Failed
                    }),
                    duration_ms,
                }
            };

            if let Err(error) = orchestrator
                .complete_task_execution(task, task_result)
                .await
            {
                tracing::error!(error = %error, "Failed to finalize delegated task execution");
            }
        });

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
    ) -> Result<(), String> {
        let run_id = task_result
            .run_id
            .clone()
            .or_else(|| task.run_id.clone())
            .ok_or_else(|| "Completed task result is missing run_id".to_string())?;

        let memory_file_path = persist_delegated_task_memory(&task, &task_result).await;
        let (
            mut run_snapshot,
            task_snapshot,
            tasks_to_start,
            environment_id,
            finalized_state,
            gate_message,
        ) = {
            self.active_tasks.lock().await.remove(&task.id);

            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(&run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;
            let record = run
                .tasks
                .iter_mut()
                .find(|record| record.task.id == task.id)
                .ok_or_else(|| format!("Task '{}' not found in run", task.id))?;

            record.result = Some(task_result.clone());
            record.completed_at = Some(Utc::now());
            record.updated_at = Utc::now();
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

            let (task_snapshot, environment_id, finalized_state) =
                (record.clone(), record.environment_id.clone(), record.state);
            let ready_to_start = collect_ready_tasks(run);
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
        if !matches!(task.execution_mode, AgentExecutionMode::Remote) {
            self.persist_delegated_checkpoint(&delegated_terminal_checkpoint(&task, &task_result))?;
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

    fn schedule_task_execution(&self, task: DelegatedTask) {
        let orchestrator = self.clone();
        tokio::spawn(async move {
            if let Err(error) = orchestrator.start_task_execution(task).await {
                tracing::error!(error = %error, "Failed to start ready task");
            }
        });
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

    fn load_checkpoint_resume_state(&self, task: &DelegatedTask) -> Option<PausedExecutionState> {
        let root = self
            .default_workspace_dir
            .as_deref()
            .or(task.workspace_dir.as_deref())?;
        load_persisted_checkpoints(root)
            .into_iter()
            .find(|checkpoint| checkpoint.task_id == task.id)
            .and_then(|checkpoint| checkpoint.resume_state)
    }

    async fn persist_run_with_hierarchy_sync(
        &self,
        run: SupervisorRun,
    ) -> Result<Vec<SupervisorRun>, String> {
        self.persist_run(&run)?;
        let mut updated_runs = vec![run.clone()];
        if let Some(parent_run) = self.sync_parent_run_from_child(&run.id).await? {
            self.persist_run(&parent_run)?;
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
) -> (Result<String, String>, Vec<OrchestratorToolCall>) {
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

    if let Some(resume_state) = orchestrator.load_checkpoint_resume_state(task) {
        request = request.with_resume_state(resume_state);
    }

    let pipeline = AgentPipeline::with_provider_optimized_config(orchestrator.config.clone())
        .with_knowledge(
            orchestrator_knowledge_store(),
            orchestrator_knowledge_settings(),
        );
    let (tx, mut rx) = mpsc::channel(256);
    let cancel_token = CancellationToken::new();
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

    while let Some(chunk) = rx.recv().await {
        match chunk {
            StreamChunk::AgentLoopIteration { iteration } => {
                current_iteration = iteration;
            }
            StreamChunk::Text(text) => partial_content.push_str(&text),
            StreamChunk::Thinking(thinking) => partial_thinking.push_str(&thinking),
            StreamChunk::ToolCallStart { id, name } => {
                pending_tool_call = Some(PendingDelegatedToolCall {
                    id,
                    name,
                    arguments: String::new(),
                });
            }
            StreamChunk::ToolCallArgs(args) => {
                if let Some(pending) = pending_tool_call.as_mut() {
                    pending.arguments.push_str(&args);
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
                    let _ = orchestrator.persist_delegated_checkpoint(&checkpoint);
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
                        name,
                        arguments: String::new(),
                    });
                let record = ToolCallRecord {
                    id: pending.id.clone(),
                    name: pending.name.clone(),
                    arguments: pending.arguments.clone(),
                    result: if success {
                        crate::ToolResult::Success(output.clone())
                    } else {
                        crate::ToolResult::Error(output.clone())
                    },
                    duration_ms,
                };
                completed_orchestrator_tool_calls.push(orchestrator_tool_call_from_record(&record));
                completed_resume_tool_calls.push(record);
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
                let _ = orchestrator.persist_delegated_checkpoint(&checkpoint);
            }
            _ => {}
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
            return (
                Err(format!("Delegated pipeline task failed: {error}")),
                completed_orchestrator_tool_calls,
            );
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

            (Ok(response.content), tool_calls)
        }
        Err(e) => {
            tracing::error!(
                agent_id = %task.agent_id,
                task_id = %task.id,
                error = %e,
                tool_calls_count = completed_orchestrator_tool_calls.len(),
                "Task failed"
            );
            (Err(e.to_string()), completed_orchestrator_tool_calls)
        }
    }
}

async fn persist_delegated_task_memory(
    task: &DelegatedTask,
    task_result: &TaskResult,
) -> Option<PathBuf> {
    let workspace_dir = task.workspace_dir.as_deref()?;
    let session_id = task.session_id.as_deref()?;

    let mut tags = task.memory_tags.clone();
    tags.extend(["delegation".to_string(), "subagent".to_string()]);
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

    let content = format!(
        "## Delegated Task\n- Orchestrator Task ID: {}\n- Tracking Task ID: {}\n- Agent ID: {}\n- Directive ID: {}\n\n## Prompt\n{}\n\n## Result\n{}\n\n## Tool Calls\n{}\n",
        task.id,
        task.tracking_task_id.as_deref().unwrap_or("n/a"),
        task.agent_id,
        task.directive_id.as_deref().unwrap_or("n/a"),
        task.prompt,
        task_result.output,
        tool_calls,
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
        .with_tags(tags)
        .with_promotion(
            session_id.to_string(),
            "Delegated subagent result promoted for supervisor retrieval",
        )
        .with_confidence(if task_result.success { 0.88 } else { 0.65 });

    match crate::save_to_memory_bank(workspace_dir, &entry).await {
        Ok(path) => Some(path),
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
    let Some(workspace_dir) = task.workspace_dir.as_deref() else {
        return;
    };
    let Some(session_id) = task.session_id.as_deref() else {
        return;
    };
    let Some(tracking_task_id) = task.tracking_task_id.as_deref() else {
        return;
    };

    let manager = TaskManager::new(workspace_dir);
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
        &manager,
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

fn record_task_completion(
    task: &DelegatedTask,
    task_result: &TaskResult,
    memory_file_path: Option<&Path>,
    record: &SupervisorTaskRecord,
    run: &SupervisorRun,
) {
    let Some(workspace_dir) = task.workspace_dir.as_deref() else {
        return;
    };
    let Some(session_id) = task.session_id.as_deref() else {
        return;
    };
    let Some(tracking_task_id) = task.tracking_task_id.as_deref() else {
        return;
    };

    let manager = TaskManager::new(workspace_dir);
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
        &manager,
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
    let completed_ids = run
        .tasks
        .iter()
        .filter(|record| matches!(record.state, SupervisorTaskState::Completed))
        .map(|record| record.task.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let now = Utc::now();

    let mut ready = Vec::new();
    for record in &mut run.tasks {
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
            metadata: None,
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
            messages: vec![],
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
                approval_required: false,
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
    async fn test_child_supervisor_run_inherits_policy_and_task_defaults() {
        let tmp = tempdir().unwrap();
        let manager = crate::agents::AgentManager::new(tmp.path().join("child-hierarchy.db"));
        let orchestrator = AgentOrchestrator::new(manager, AppConfig::default());

        let mut parent_run = empty_supervisor_run("run-parent", tmp.path().to_path_buf());
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
        assert_eq!(record.task.execution_mode, AgentExecutionMode::GitWorktree);
        assert!(record.task.memory_tags.contains(&"root-tag".to_string()));
        assert!(record.task.memory_tags.contains(&"child-tag".to_string()));
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
                objective: "Handle retryable sub-scope".to_string(),
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
                workspace_dir: None,
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
        assert_eq!(parent_retried.status, SupervisorRunStatus::Running);
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
            messages: vec![],
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
