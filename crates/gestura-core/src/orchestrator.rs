//! Subagent orchestration (core-owned, tauri-free).
//!
//! This module coordinates delegated tasks across subagents and executes them via the
//! unified [`crate::pipeline::AgentPipeline`].
//!
//! ## Layering
//! - **gestura-core** owns orchestration policy (permissions, task tracking, execution).
//! - Adapters (GUI/CLI) may attach observers to emit UI events, but must not re-implement
//!   orchestration logic.

use crate::tasks::{TaskBackgroundJob, TaskBackgroundStatus};
use crate::tools::PermissionManager;
use crate::{AgentPipeline, AgentRequest, AppConfig, RequestSource};
use crate::{MemoryBankEntry, MemoryScope, MemoryType};
use crate::{TaskManager, TaskStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, RwLock, mpsc};
use uuid::Uuid;

// Re-export shared task types for convenience and adapter compatibility.
pub use crate::agents::{
    AgentExecutionMode, AgentInfo, AgentRole, AgentSpawnRequest, AgentSpawner, DelegatedTask,
    DelegationBrief, OrchestratorToolCall, RemoteAgentTarget, TaskArtifactRecord, TaskResult,
};

/// Approval state tracked by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// No explicit approval step is required.
    NotRequired,
    /// Waiting for a human or supervisor decision.
    Pending,
    /// Approved to proceed or complete.
    Approved,
    /// Rejected and should not proceed.
    Rejected,
    /// Revision requested before retrying.
    NeedsRevision,
}

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
}

/// Approval details tracked per task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskApprovalRecord {
    /// Current approval state.
    pub state: ApprovalState,
    /// When approval was requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<DateTime<Utc>>,
    /// When a decision was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
    /// Actor that made the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    /// Optional explanatory note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TaskApprovalRecord {
    fn not_required() -> Self {
        Self {
            state: ApprovalState::NotRequired,
            requested_at: None,
            decided_at: None,
            decided_by: None,
            note: None,
        }
    }

    fn pending() -> Self {
        Self {
            state: ApprovalState::Pending,
            requested_at: Some(Utc::now()),
            decided_at: None,
            decided_by: None,
            note: None,
        }
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
        let (result_tx, result_rx) = mpsc::channel(100);
        Self {
            agent_manager,
            permission_manager: Arc::new(PermissionManager::new()),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            supervisor_runs: Arc::new(Mutex::new(HashMap::new())),
            task_run_index: Arc::new(Mutex::new(HashMap::new())),
            result_tx,
            result_rx: Arc::new(Mutex::new(result_rx)),
            config,
            observer: Arc::new(RwLock::new(None)),
            default_workspace_dir: std::env::current_dir().ok(),
        }
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

        let environment = self.prepare_environment(&task)?;
        task.environment_id = Some(environment.id.clone());
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
                TaskApprovalRecord::pending()
            } else {
                TaskApprovalRecord::not_required()
            };
            let state = if task.approval_required {
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
        decided_by: Option<String>,
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
        let run_snapshot = {
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

            record.approval.state = ApprovalState::Approved;
            record.approval.decided_at = Some(Utc::now());
            record.approval.decided_by = decided_by;
            record.approval.note = note;
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
                        record.approval = TaskApprovalRecord::pending();
                        record.approval.note =
                            Some("Review approved. Awaiting explicit test validation.".to_string());
                    } else {
                        record.state = SupervisorTaskState::Completed;
                        record.completed_at = Some(Utc::now());
                    }
                }
                SupervisorTaskState::TestPending => {
                    record.state = SupervisorTaskState::Completed;
                    record.completed_at = Some(Utc::now());
                }
                _ => {
                    return Err(format!(
                        "Task '{}' is not waiting for approval or gate completion",
                        task_id
                    ));
                }
            }

            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            run.clone()
        };

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;

        if let Some(task) = queued_to_start {
            self.start_task_execution(task).await?;
        }

        Ok(())
    }

    /// Reject or request revision for a delegated task.
    pub async fn reject_task(
        &self,
        task_id: &str,
        decided_by: Option<String>,
        note: Option<String>,
    ) -> Result<(), String> {
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

            record.approval.state = ApprovalState::NeedsRevision;
            record.approval.decided_at = Some(Utc::now());
            record.approval.decided_by = decided_by;
            record.approval.note = note;
            record.state = SupervisorTaskState::Failed;
            record.updated_at = Utc::now();
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            run.clone()
        };

        self.persist_run(&run_snapshot)?;
        self.notify_run_updated(run_snapshot).await;
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
        let run_snapshot = {
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

            if record.task.approval_required {
                record.state = SupervisorTaskState::PendingApproval;
                record.approval = TaskApprovalRecord::pending();
            } else {
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
            run.clone()
        };

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
                let run_snapshot = {
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

    fn prepare_environment(&self, task: &DelegatedTask) -> Result<ExecutionEnvironment, String> {
        let root = task
            .workspace_dir
            .clone()
            .or_else(|| self.default_workspace_dir.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let run_id = task.run_id.as_deref().unwrap_or("global-run").to_string();
        let environment_id = task
            .environment_id
            .clone()
            .unwrap_or_else(|| format!("env-{}-{}", run_id, task.agent_id));

        let prepared_root = match task.execution_mode {
            AgentExecutionMode::SharedWorkspace => root.clone(),
            AgentExecutionMode::IsolatedWorkspace | AgentExecutionMode::GitWorktree => {
                let prepared = root
                    .join(".gestura")
                    .join("environments")
                    .join(&run_id)
                    .join(&task.agent_id);
                fs::create_dir_all(&prepared)
                    .map_err(|error| format!("Failed to prepare environment: {error}"))?;
                prepared
            }
            AgentExecutionMode::Remote => root.clone(),
        };

        Ok(ExecutionEnvironment {
            id: environment_id,
            execution_mode: task.execution_mode.clone(),
            root_dir: prepared_root.clone(),
            write_access: !task.planning_only,
            branch_name: matches!(task.execution_mode, AgentExecutionMode::GitWorktree)
                .then(|| format!("gestura/{run_id}/{}", task.agent_id)),
            worktree_path: matches!(task.execution_mode, AgentExecutionMode::GitWorktree)
                .then_some(prepared_root),
            remote_url: task.remote_target.as_ref().map(|target| target.url.clone()),
        })
    }

    async fn start_task_execution(&self, task: DelegatedTask) -> Result<(), String> {
        let run_id = task
            .run_id
            .clone()
            .ok_or_else(|| "Task is missing run_id".to_string())?;

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
        let (run_snapshot, task_snapshot, tasks_to_start) = {
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
                    record.approval = TaskApprovalRecord::pending();
                    record.approval.note =
                        Some("Execution finished. Awaiting explicit review approval.".to_string());
                } else if record.task.test_required {
                    record.state = SupervisorTaskState::TestPending;
                    record.approval = TaskApprovalRecord::pending();
                    record.approval.note =
                        Some("Execution finished. Awaiting explicit test validation.".to_string());
                } else {
                    record.state = SupervisorTaskState::Completed;
                    if matches!(record.approval.state, ApprovalState::Pending) {
                        record.approval.state = ApprovalState::Approved;
                        record.approval.decided_at = Some(Utc::now());
                    }
                }
            } else {
                record.state = SupervisorTaskState::Failed;
                if matches!(record.approval.state, ApprovalState::Pending) {
                    record.approval.state = ApprovalState::NeedsRevision;
                    record.approval.note = Some("Execution failed and requires revision".into());
                }
            }

            let task_snapshot = record.clone();
            let ready_to_start = collect_ready_tasks(run);
            run.updated_at = Utc::now();
            run.status = recalculate_run_status(run);
            (run.clone(), task_snapshot, ready_to_start)
        };

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
                "dependencies": task.depends_on,
                "environment": {
                    "id": record.environment.id,
                    "mode": format!("{:?}", record.environment.execution_mode).to_lowercase(),
                    "root_dir": record.environment.root_dir,
                    "write_access": record.environment.write_access,
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
                "last_output": task_result.output,
                "summary": task_result.summary,
                "attempts": record.attempts,
                "memory_file_path": memory_file_path.map(|path| path.display().to_string()),
                "tool_calls": task_result.tool_calls,
                "artifacts": task_result.artifacts,
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

fn persist_run_to_disk(root: &Path, run: &SupervisorRun) -> Result<(), String> {
    let session_key = run.session_id.as_deref().unwrap_or("global");
    let dir = root.join(".gestura").join("orchestrator").join(session_key);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create orchestrator persistence dir: {error}"))?;
    let path = dir.join(format!("{}.json", run.id));
    let content = serde_json::to_vec_pretty(run)
        .map_err(|error| format!("Failed to serialize supervisor run: {error}"))?;
    fs::write(path, content).map_err(|error| format!("Failed to persist supervisor run: {error}"))
}

fn load_persisted_runs(root: &Path) -> Vec<SupervisorRun> {
    let base = root.join(".gestura").join("orchestrator");
    let Ok(session_dirs) = fs::read_dir(base) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for session_dir in session_dirs.flatten() {
        let path = session_dir.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = fs::read(&path) else {
                continue;
            };
            let Ok(run) = serde_json::from_slice::<SupervisorRun>(&content) else {
                continue;
            };
            runs.push(run);
        }
    }
    runs
}

fn load_persisted_run_by_id(root: &Path, run_id: &str) -> Option<SupervisorRun> {
    load_persisted_runs(root)
        .into_iter()
        .find(|run| run.id == run_id)
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
        let manager = crate::agents::AgentManager::new(PathBuf::from("/tmp/test.db"));
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
}
