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
mod environment;
mod persistence;
mod recovery;

use crate::tasks::{TaskBackgroundJob, TaskBackgroundStatus};
use crate::tools::PermissionManager;
use crate::{AgentPipeline, AgentRequest, AppConfig, RequestSource, SessionWorkspace};
use crate::{MemoryBankEntry, MemoryScope, MemoryType};
use crate::{TaskManager, TaskStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, RwLock, mpsc};
use uuid::Uuid;

use self::persistence::{
    load_persisted_environment_by_id, load_persisted_environments, load_persisted_run_by_id,
    load_persisted_runs, persist_environment_to_disk, persist_run_to_disk,
};

// Re-export shared task types for convenience and adapter compatibility.
pub use self::approval::{
    ApprovalActor, ApprovalActorKind, ApprovalDecision, ApprovalDecisionKind, ApprovalPolicy,
    ApprovalRequest, ApprovalRequirement, ApprovalScope, ApprovalState, TaskApprovalRecord,
    actor_kind_for_agent_role, default_actor_kind_for_scope,
};
pub use crate::agents::{
    AgentExecutionMode, AgentInfo, AgentRole, AgentSpawnRequest, AgentSpawner, DelegatedTask,
    DelegationBrief, OrchestratorToolCall, RemoteAgentTarget, TaskArtifactRecord, TaskResult,
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
        Self {
            id: Uuid::new_v4().to_string(),
            run_id: run_id.into(),
            task_id,
            kind,
            sender_agent_id,
            recipient_agent_id,
            content: content.into(),
            created_at: Utc::now(),
        }
    }
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
    /// Optional session association.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Workspace root used for persistence/environment prep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Lead agent coordinating the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_agent_id: Option<String>,
    /// Aggregate run status.
    pub status: SupervisorRunStatus,
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

        // Check tool permissions.
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

        let environment_record = self.prepare_environment(&task).await?;
        let environment = environment_record.summary();
        task.environment_id = Some(environment_record.id.clone());
        self.ensure_tracking_task(&mut task).await;

        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| "Delegated task is missing run_id".to_string())?;

        let (run_snapshot, task_snapshot, should_start) = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs.entry(run_id.clone()).or_insert_with(|| SupervisorRun {
                id: run_id.clone(),
                session_id: task.session_id.clone(),
                workspace_dir: task
                    .workspace_dir
                    .clone()
                    .or_else(|| self.default_workspace_dir.clone()),
                lead_agent_id: Some("supervisor".to_string()),
                status: SupervisorRunStatus::Draft,
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
                messages: Vec::new(),
                created_at: now,
                updated_at: now,
                started_at: None,
                completed_at: None,
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

            (
                run.clone(),
                new_record.clone(),
                state == SupervisorTaskState::Queued,
            )
        };

        self.task_run_index
            .lock()
            .await
            .insert(task_id.clone(), run_id.clone());

        record_task_dispatch(&task, &task_snapshot, &run_snapshot);
        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;

        if should_start {
            self.start_task_execution(task.clone()).await?;
        }

        Ok(task_id)
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
        runs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        runs
    }

    /// Fetch a supervisor run by id.
    pub async fn get_supervisor_run(&self, run_id: &str) -> Option<SupervisorRun> {
        if let Some(run) = self.supervisor_runs.lock().await.get(run_id).cloned() {
            return Some(run);
        }

        self.default_workspace_dir
            .as_deref()
            .and_then(|root| load_persisted_run_by_id(root, run_id))
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
        let (mut run_snapshot, approval_message) = {
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
            let approval_message = TeamMessage::new(
                run.id.clone(),
                Some(record.task.id.clone()),
                TeamMessageKind::ApprovalDecision,
                Some(decision.actor.id.clone()),
                Some(record.task.agent_id.clone()),
                format_approval_decision_message(&decision),
            );

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
            (run.clone(), Some(approval_message))
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

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;
        if let Some(message) = approval_message {
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
            let approval_message = TeamMessage::new(
                run.id.clone(),
                Some(record.task.id.clone()),
                TeamMessageKind::ApprovalDecision,
                Some(decision.actor.id.clone()),
                Some(record.task.agent_id.clone()),
                format_approval_decision_message(&decision),
            );
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

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;
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
        let (mut run_snapshot, environment_id) = {
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
            record.completed_at = None;
            record.started_at = None;
            record.updated_at = Utc::now();
            record.blocked_reasons.clear();
            let environment_id = record.environment_id.clone();

            if record.task.approval_required {
                record.state = SupervisorTaskState::PendingApproval;
                record.approval.reset_for_task(&record.task);
                record.approval.request(
                    ApprovalScope::PreExecution,
                    ApprovalActor::system("orchestrator"),
                    Some("Task retried. Awaiting explicit pre-execution approval.".to_string()),
                );
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
            (run.clone(), environment_id)
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

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;

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

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;
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
        let message = TeamMessage::new(
            run_id.to_string(),
            task_id.clone(),
            kind,
            sender_agent_id,
            recipient_agent_id,
            content,
        );

        let run_snapshot = {
            let mut runs = self.supervisor_runs.lock().await;
            let run = runs
                .get_mut(run_id)
                .ok_or_else(|| format!("Run '{}' not found", run_id))?;

            if let Some(task_id) = task_id.as_deref() {
                let record = run
                    .tasks
                    .iter_mut()
                    .find(|record| record.task.id == task_id)
                    .ok_or_else(|| format!("Task '{}' not found in run", task_id))?;
                record.messages.push(message.clone());
                record.updated_at = Utc::now();
            } else {
                run.messages.push(message.clone());
            }

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            run.clone()
        };

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;
        self.notify_team_message(message.clone()).await;
        Ok(message)
    }

    /// Cancel a running task.
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), String> {
        let task_opt = {
            let mut active = self.active_tasks.lock().await;
            active.remove(task_id)
        };

        if let Some(task) = task_opt {
            tracing::info!(task_id = %task_id, agent_id = %task.agent_id, "Cancelling task");
            // Send cancellation event to the agent.
            self.agent_manager
                .send_event(&task.agent_id, format!("cancel:{}", task_id))
                .await;

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
                self.persist_run(&run_snapshot)?;
                self.notify_run_updated(run_snapshot).await;
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
        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot.clone()).await;

        let observer_for_start = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_start.as_ref() {
            obs.on_task_started(task.clone()).await;
        }

        let orchestrator = self.clone();
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let (result, tool_calls) =
                execute_delegated_task(&orchestrator.agent_manager, &orchestrator.config, &task)
                    .await;
            let duration_ms = start.elapsed().as_millis() as u64;

            let task_result = TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: result.is_ok(),
                run_id: task.run_id.clone(),
                tracking_task_id: task.tracking_task_id.clone(),
                output: result.clone().unwrap_or_else(|error| error),
                summary: task.name.clone(),
                tool_calls,
                artifacts: Vec::new(),
                duration_ms,
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
        let (mut run_snapshot, task_snapshot, tasks_to_start, environment_id, finalized_state) = {
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

            if task_result.success {
                if record.task.reviewer_required {
                    record.state = SupervisorTaskState::ReviewPending;
                    record.approval.request(
                        ApprovalScope::Review,
                        ApprovalActor::system("orchestrator"),
                        Some("Execution finished. Awaiting explicit review approval.".to_string()),
                    );
                } else if record.task.test_required {
                    record.state = SupervisorTaskState::TestPending;
                    record.approval.request(
                        ApprovalScope::TestValidation,
                        ApprovalActor::system("orchestrator"),
                        Some("Execution finished. Awaiting explicit test validation.".to_string()),
                    );
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
                record.state = SupervisorTaskState::Failed;
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
        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;

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

    async fn notify_run_updated(&self, run: SupervisorRun) {
        let observer_for_run = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_run.as_ref() {
            obs.on_run_updated(run).await;
        }
    }

    async fn notify_team_message(&self, message: TeamMessage) {
        let observer_for_message = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_message.as_ref() {
            obs.on_team_message(message).await;
        }
    }
}

/// Execute a delegated task on the specified agent using the unified AgentPipeline.
///
/// Returns the final (text) result (or error string) and a structured list of tool calls.
async fn execute_delegated_task<M: OrchestratorAgentManager>(
    agent_manager: &M,
    config: &AppConfig,
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
    agent_manager.update_activity(&task.agent_id).await;

    // Build the agent request with tool filtering.
    let mut request = AgentRequest::new(&full_prompt)
        .with_streaming(false)
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

    // Execute via unified pipeline.
    let pipeline = AgentPipeline::with_provider_optimized_config(config.clone()).with_knowledge(
        orchestrator_knowledge_store(),
        orchestrator_knowledge_settings(),
    );
    let result = pipeline.process_blocking(request).await;

    match result {
        Ok(response) => {
            // Convert pipeline tool calls to orchestrator format.
            let tool_calls: Vec<OrchestratorToolCall> = response
                .tool_calls
                .into_iter()
                .map(|tc| {
                    // Parse arguments as JSON for input.
                    let input =
                        serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));

                    // Extract output and success from ToolResult.
                    let (output, success) = match &tc.result {
                        crate::ToolResult::Success(s) => (
                            serde_json::from_str(s).unwrap_or(serde_json::json!({"result": s})),
                            true,
                        ),
                        crate::ToolResult::Error(e) => (serde_json::json!({"error": e}), false),
                        crate::ToolResult::Skipped(reason) => {
                            (serde_json::json!({"skipped": reason}), false)
                        }
                    };

                    OrchestratorToolCall {
                        tool_name: tc.name,
                        input,
                        output,
                        success,
                        duration_ms: tc.duration_ms,
                    }
                })
                .collect();

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
                "Task failed"
            );
            (Err(e.to_string()), Vec::new())
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
    if run.tasks.is_empty() {
        return SupervisorRunStatus::Draft;
    }
    if run
        .tasks
        .iter()
        .all(|record| matches!(record.state, SupervisorTaskState::Cancelled))
    {
        return SupervisorRunStatus::Cancelled;
    }
    if run.tasks.iter().any(|record| {
        matches!(
            record.state,
            SupervisorTaskState::Running | SupervisorTaskState::Queued
        )
    }) {
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
    }) {
        return SupervisorRunStatus::Waiting;
    }
    if run
        .tasks
        .iter()
        .all(|record| matches!(record.state, SupervisorTaskState::Completed))
    {
        return SupervisorRunStatus::Completed;
    }
    if run
        .tasks
        .iter()
        .any(|record| matches!(record.state, SupervisorTaskState::Failed))
    {
        return SupervisorRunStatus::Failed;
    }
    SupervisorRunStatus::Draft
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
            messages: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };

        let run = SupervisorRun {
            id: "run-approval".into(),
            session_id: None,
            workspace_dir: Some(workspace_dir),
            lead_agent_id: None,
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
}
