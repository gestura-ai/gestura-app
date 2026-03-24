//! Task management system for tracking agent workflows
//!
//! This module provides a task management system that integrates with the agent loop
//! to track complex workflows, subtasks, and progress throughout conversation sessions.
//!
//! # Architecture
//!
//! ```text
//! .gestura/tasks/
//! ├── {session_id_1}.json    # Tasks for session 1
//! ├── {session_id_2}.json    # Tasks for session 2
//! └── ...
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use gestura_core::tasks::{TaskManager, Task, TaskStatus};
//!
//! let manager = TaskManager::new("/path/to/workspace");
//! let task = manager.create_task("session-123", "Implement feature", "Add new API endpoint", None)?;
//! manager.update_task_status("session-123", &task.id, TaskStatus::InProgress)?;
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Status of a task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task has not been started
    NotStarted,
    /// Task is blocked on dependencies, approvals, or other coordination gates
    Blocked,
    /// Task is currently in progress
    InProgress,
    /// Task has been completed
    Completed,
    /// Task has been cancelled
    Cancelled,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "not_started"),
            Self::Blocked => write!(f, "blocked"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    /// Parse a task status (case-insensitive, accepts common aliases).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        match norm.as_str() {
            "not_started" | "todo" | "new" => Ok(Self::NotStarted),
            "blocked" | "waiting" | "pending" => Ok(Self::Blocked),
            "in_progress" | "doing" | "wip" => Ok(Self::InProgress),
            "completed" | "done" => Ok(Self::Completed),
            "cancelled" | "canceled" | "dropped" => Ok(Self::Cancelled),
            _ => Err(format!(
                "Unknown task status: '{}'. Expected: not_started, blocked, in_progress, completed, cancelled",
                s
            )),
        }
    }
}

/// Source of a task (who created it)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TaskSource {
    /// Created manually by user via the task panel UI
    #[default]
    User,
    /// Created automatically by the agent during processing
    Agent,
    /// Created by the orchestrator for workflow delegation
    Orchestrator,
}

/// Background execution status for tasks that represent long-running work.
///
/// This is primarily intended for UI dashboards to reflect delegated work
/// (e.g. orchestrator / agent tasks) that may run asynchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskBackgroundStatus {
    /// The background job has been queued but has not started.
    Queued,
    /// The background job is blocked on another dependency or review gate.
    Blocked,
    /// The background job is waiting on explicit approval.
    AwaitingApproval,
    /// The background job is currently running.
    Running,
    /// The background job completed successfully.
    Succeeded,
    /// The background job failed.
    Failed,
    /// The background job was cancelled.
    Cancelled,
}

/// Background job metadata attached to a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBackgroundJob {
    /// Current background job status.
    pub status: TaskBackgroundStatus,
    /// Optional identifier for the job in an external system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Optional human-readable status message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Phase in the task/memory lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskMemoryPhase {
    /// Task was delegated to a subagent.
    Delegated,
    /// Task produced a handoff summary.
    Handoff,
    /// Task promoted durable/shared memory.
    Promoted,
    /// Task hit a blocker relevant to memory tracking.
    Blocked,
}

/// Structured event recorded in task metadata for memory lifecycle tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMemoryEvent {
    /// Lifecycle phase represented by this event.
    pub phase: TaskMemoryPhase,
    /// Human-readable summary.
    pub summary: String,
    /// Optional scope for the related memory record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Optional memory type for the related memory record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    /// Optional durable memory file path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_file_path: Option<String>,
    /// Timestamp when this event was recorded.
    pub recorded_at: DateTime<Utc>,
}

impl TaskMemoryEvent {
    /// Create a new task memory event.
    pub fn new(
        phase: TaskMemoryPhase,
        summary: impl Into<String>,
        scope: Option<String>,
        memory_type: Option<String>,
        memory_file_path: Option<String>,
    ) -> Self {
        Self {
            phase,
            summary: summary.into(),
            scope,
            memory_type,
            memory_file_path,
            recorded_at: Utc::now(),
        }
    }
}

/// Structured task-local memory lifecycle information persisted in metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskMemoryLifecycle {
    /// Recorded lifecycle events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<TaskMemoryEvent>,
    /// Most recent durable memory file path, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_memory_file_path: Option<String>,
}

/// Runtime-owned execution kind inferred or assigned for a tracked task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecutionKind {
    /// Investigation, analysis, or planning work.
    Planning,
    /// Concrete implementation or file mutation work.
    Implementation,
    /// Build, test, lint, or other verification work.
    Verification,
    /// Fallback when no stronger runtime classification is available.
    #[default]
    General,
}

/// Runtime-authored verification requirements for a tracked task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TaskVerificationProfile {
    /// Primary execution kind for the task.
    #[serde(default)]
    pub execution_kind: TaskExecutionKind,
    /// Whether the task requires a successful source mutation.
    #[serde(default)]
    pub requires_mutation: bool,
    /// Whether the task requires a successful build/check command.
    #[serde(default)]
    pub requires_build: bool,
    /// Whether the task requires a successful test command.
    #[serde(default)]
    pub requires_test: bool,
    /// Whether the runtime considers the task safe to run in parallel with other
    /// ready tasks.
    #[serde(default)]
    pub parallel_safe: bool,
}

/// Structured runtime evidence recorded for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecutionEvidenceKind {
    /// Some successful tool work occurred for the task.
    ToolActivity,
    /// A source mutation completed successfully.
    Mutation,
    /// A build or compile verification command succeeded.
    Build,
    /// A test command succeeded.
    Test,
    /// An artifact was produced or discovered.
    Artifact,
}

/// A single execution evidence record stored in task metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskExecutionEvidence {
    /// Kind of runtime evidence observed.
    pub kind: TaskExecutionEvidenceKind,
    /// Human-readable summary of what happened.
    pub summary: String,
    /// Tool responsible for the evidence, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Command associated with the evidence, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Whether the evidence corresponds to a successful outcome.
    #[serde(default = "default_true")]
    pub success: bool,
    /// Timestamp when the evidence was recorded.
    pub recorded_at: DateTime<Utc>,
}

/// Runtime-owned execution state persisted in task metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TaskExecutionState {
    /// Verification requirements currently assigned to the task.
    #[serde(default)]
    pub verification_profile: TaskVerificationProfile,
    /// Whether any successful tool activity has been observed for the task.
    #[serde(default)]
    pub saw_tool_activity: bool,
    /// Whether successful source mutation evidence has been observed.
    #[serde(default)]
    pub saw_mutation: bool,
    /// Whether successful build/check evidence has been observed.
    #[serde(default)]
    pub build_succeeded: bool,
    /// Whether successful test evidence has been observed.
    #[serde(default)]
    pub test_succeeded: bool,
    /// Optional runtime note for UI/status surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_runtime_note: Option<String>,
    /// Structured evidence history, capped to a small rolling window.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<TaskExecutionEvidence>,
}

fn default_true() -> bool {
    true
}

impl TaskExecutionEvidence {
    /// Create a new task execution evidence record.
    pub fn new(
        kind: TaskExecutionEvidenceKind,
        summary: impl Into<String>,
        tool_name: Option<String>,
        command: Option<String>,
    ) -> Self {
        Self {
            kind,
            summary: summary.into(),
            tool_name,
            command,
            success: true,
            recorded_at: Utc::now(),
        }
    }
}

impl TaskExecutionState {
    /// Merge a runtime-authored verification profile into the current state.
    pub fn merge_profile(&mut self, profile: TaskVerificationProfile) {
        self.verification_profile = profile;
    }

    /// Record evidence and update the derived boolean summary.
    pub fn record_evidence(&mut self, evidence: TaskExecutionEvidence) -> bool {
        let duplicate = self.evidence.iter().any(|existing| {
            existing.kind == evidence.kind
                && existing.summary == evidence.summary
                && existing.tool_name == evidence.tool_name
                && existing.command == evidence.command
                && existing.success == evidence.success
        });
        if duplicate {
            return false;
        }

        match evidence.kind {
            TaskExecutionEvidenceKind::ToolActivity => self.saw_tool_activity = true,
            TaskExecutionEvidenceKind::Mutation => {
                self.saw_tool_activity = true;
                self.saw_mutation = true;
            }
            TaskExecutionEvidenceKind::Build => {
                self.saw_tool_activity = true;
                self.build_succeeded = true;
            }
            TaskExecutionEvidenceKind::Test => {
                self.saw_tool_activity = true;
                self.test_succeeded = true;
            }
            TaskExecutionEvidenceKind::Artifact => self.saw_tool_activity = true,
        }

        self.evidence.push(evidence);
        if self.evidence.len() > 32 {
            let excess = self.evidence.len() - 32;
            self.evidence.drain(0..excess);
        }
        true
    }

    /// Return `true` when the observed runtime evidence satisfies the task's
    /// assigned verification profile.
    pub fn satisfies_profile(&self) -> bool {
        let requires_progress = !self.verification_profile.requires_mutation
            && !self.verification_profile.requires_build
            && !self.verification_profile.requires_test;

        (!self.verification_profile.requires_mutation || self.saw_mutation)
            && (!self.verification_profile.requires_build || self.build_succeeded)
            && (!self.verification_profile.requires_test || self.test_succeeded)
            && (!requires_progress || self.saw_tool_activity)
    }
}

impl TaskBackgroundJob {
    /// Create a new background job record.
    pub fn new(
        status: TaskBackgroundStatus,
        job_id: Option<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            status,
            job_id,
            message,
        }
    }
}

/// A task represents a unit of work to be tracked
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier for this task
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Detailed description
    pub description: String,
    /// Current status
    pub status: TaskStatus,
    /// Parent task ID (for subtasks)
    pub parent_id: Option<String>,

    /// IDs of tasks that block this task from being started/completed.
    ///
    /// This is modeled as a dependency list ("blocked by") so we can derive
    /// the inverse relationship ("blocks") on demand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,

    /// Optional background job state for tasks that run asynchronously.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_job: Option<TaskBackgroundJob>,

    /// Optional ordering hint for UI dashboards (lower first).
    #[serde(default)]
    pub sort_order: i32,

    /// Optional phase/group label for UI dashboards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task was last updated
    pub updated_at: DateTime<Utc>,
    /// Session ID this task belongs to
    pub session_id: String,
    /// Source of the task (user, agent, or orchestrator)
    #[serde(default)]
    pub source: TaskSource,
    /// ID linking to an orchestrator DelegatedTask (for bidirectional sync)
    #[serde(default)]
    pub orchestrator_task_id: Option<String>,
    /// ID of the agent that created/owns this task
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Additional metadata (tool calls, output, context, etc.)
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

impl Task {
    /// Create a new task (defaults to User source)
    pub fn new(
        session_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        parent_id: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            status: TaskStatus::NotStarted,
            parent_id,
            blocked_by: Vec::new(),
            background_job: None,
            sort_order: 0,
            phase: None,
            created_at: now,
            updated_at: now,
            session_id: session_id.into(),
            source: TaskSource::User,
            orchestrator_task_id: None,
            agent_id: None,
            metadata: None,
        }
    }

    /// Create a new task with a specific source
    pub fn new_with_source(
        session_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        parent_id: Option<String>,
        source: TaskSource,
        agent_id: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            status: TaskStatus::NotStarted,
            parent_id,
            blocked_by: Vec::new(),
            background_job: None,
            sort_order: 0,
            phase: None,
            created_at: now,
            updated_at: now,
            session_id: session_id.into(),
            source,
            orchestrator_task_id: None,
            agent_id,
            metadata: None,
        }
    }

    /// Create a task from an orchestrator delegated task
    pub fn from_orchestrator_task(
        session_id: impl Into<String>,
        orchestrator_task_id: impl Into<String>,
        agent_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        context: Option<serde_json::Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            status: TaskStatus::NotStarted,
            parent_id: None,
            blocked_by: Vec::new(),
            background_job: Some(TaskBackgroundJob::new(
                TaskBackgroundStatus::Queued,
                None,
                Some("Created by orchestrator".to_string()),
            )),
            sort_order: 0,
            phase: None,
            created_at: now,
            updated_at: now,
            session_id: session_id.into(),
            source: TaskSource::Orchestrator,
            orchestrator_task_id: Some(orchestrator_task_id.into()),
            agent_id: Some(agent_id.into()),
            metadata: context,
        }
    }

    /// Set metadata (e.g., tool calls, output)
    pub fn set_metadata(&mut self, metadata: serde_json::Value) {
        self.metadata = Some(metadata);
        self.updated_at = Utc::now();
    }

    /// Update the task status
    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// Update the task name
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.updated_at = Utc::now();
    }

    /// Update the task description
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
        self.updated_at = Utc::now();
    }

    /// Returns `true` when the task is in a terminal (non-active) status.
    pub fn is_terminal(&self) -> bool {
        matches!(self.status, TaskStatus::Completed | TaskStatus::Cancelled)
    }

    /// Replace the background job state and update timestamps.
    pub fn set_background_job(&mut self, job: Option<TaskBackgroundJob>) {
        self.background_job = job;
        self.updated_at = Utc::now();
    }

    /// Add a dependency ("blocked by") to this task.
    ///
    /// This does not validate existence; callers should validate using the session's `TaskList`.
    pub fn add_blocked_by(&mut self, task_id: impl Into<String>) {
        let id = task_id.into();
        if !self.blocked_by.iter().any(|x| x == &id) {
            self.blocked_by.push(id);
            self.updated_at = Utc::now();
        }
    }

    /// Set the phase/group label for dashboard grouping.
    pub fn set_phase(&mut self, phase: Option<String>) {
        self.phase = phase;
        self.updated_at = Utc::now();
    }
}

/// A list of tasks for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    /// Session ID
    pub session_id: String,
    /// All tasks in this session
    pub tasks: Vec<Task>,
    /// Optional "current task" pointer for UI focus and checkpoint/rewind.
    ///
    /// This is persisted alongside the task list so the UI can restore the
    /// user's current focus when resuming a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
}

/// A hierarchical node for rendering task trees in dashboards.
#[derive(Debug, Clone, Serialize)]
pub struct TaskTreeNode {
    /// The task for this node.
    pub task: Task,
    /// Child tasks.
    pub children: Vec<TaskTreeNode>,
}

impl TaskList {
    /// Create a new task list for a session
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            tasks: Vec::new(),
            current_task_id: None,
        }
    }

    /// Get the currently focused task id.
    pub fn current_task_id(&self) -> Option<&str> {
        self.current_task_id.as_deref()
    }

    /// Set or clear the current task pointer.
    ///
    /// If `task_id` is `Some`, the id must exist in this task list.
    pub fn set_current_task_id(&mut self, task_id: Option<String>) -> Result<(), TaskError> {
        if let Some(ref id) = task_id
            && self.find_task(id.as_str()).is_none()
        {
            return Err(TaskError::InvalidInput(format!(
                "current_task_id '{id}' does not exist in task list"
            )));
        }

        self.current_task_id = task_id;
        Ok(())
    }

    /// Add a task to the list
    pub fn add_task(&mut self, task: Task) {
        self.tasks.push(task);
    }

    /// Find a task by ID
    pub fn find_task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == task_id)
    }

    /// Find a task by ID (mutable)
    pub fn find_task_mut(&mut self, task_id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == task_id)
    }

    /// Remove a task by ID
    pub fn remove_task(&mut self, task_id: &str) -> Option<Task> {
        if let Some(pos) = self.tasks.iter().position(|t| t.id == task_id) {
            if self.current_task_id.as_deref() == Some(task_id) {
                self.current_task_id = None;
            }
            Some(self.tasks.remove(pos))
        } else {
            None
        }
    }

    /// Get all root tasks (tasks without a parent)
    pub fn root_tasks(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.parent_id.is_none())
            .collect()
    }

    /// Get all subtasks of a given task
    pub fn subtasks(&self, parent_id: &str) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    fn sort_task_refs_by_priority(tasks: &mut [&Task]) {
        tasks.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
    }

    /// Get all descendant tasks of a given task in depth-first order.
    pub fn descendants(&self, task_id: &str) -> Vec<&Task> {
        let mut descendants = Vec::new();
        let mut root_children = self.subtasks(task_id);
        Self::sort_task_refs_by_priority(&mut root_children);
        let mut pending_ids = root_children
            .into_iter()
            .rev()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();

        while let Some(current_id) = pending_ids.pop() {
            if !seen.insert(current_id.clone()) {
                continue;
            }

            let Some(task) = self.find_task(&current_id) else {
                continue;
            };
            descendants.push(task);

            let mut children = self.subtasks(&current_id);
            Self::sort_task_refs_by_priority(&mut children);
            for child in children.into_iter().rev() {
                pending_ids.push(child.id.clone());
            }
        }

        descendants
    }

    /// Return `true` when the task is blocked by any dependency that is not terminal.
    pub fn is_task_blocked(&self, task_id: &str) -> Result<bool, TaskError> {
        let task = self
            .find_task(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        for dep_id in &task.blocked_by {
            // Missing dependency is treated as blocking; it's a data integrity issue.
            let Some(dep) = self.find_task(dep_id) else {
                return Ok(true);
            };
            if !dep.is_terminal() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Validate that a task can transition to the requested status.
    fn validate_status_transition(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), TaskError> {
        if status != TaskStatus::Completed {
            return Ok(());
        }

        let task = self
            .find_task(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        if self.is_task_blocked(task_id)? {
            return Err(TaskError::InvalidInput(format!(
                "task '{}' cannot be completed while dependencies remain open",
                task.name
            )));
        }

        let open_subtasks: Vec<&Task> = self
            .descendants(task_id)
            .into_iter()
            .filter(|subtask| !subtask.is_terminal())
            .collect();
        if !open_subtasks.is_empty() {
            let names = open_subtasks
                .iter()
                .map(|subtask| format!("'{}'", subtask.name))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(TaskError::InvalidInput(format!(
                "task '{}' cannot be completed while subtasks remain open: {}",
                task.name, names
            )));
        }

        if let Some(job) = task.background_job.as_ref()
            && matches!(
                job.status,
                TaskBackgroundStatus::Queued
                    | TaskBackgroundStatus::Blocked
                    | TaskBackgroundStatus::AwaitingApproval
                    | TaskBackgroundStatus::Running
            )
        {
            return Err(TaskError::InvalidInput(format!(
                "task '{}' cannot be completed while its background job is still {:?}",
                task.name, job.status
            )));
        }

        Ok(())
    }

    /// Add a dependency relationship: `task_id` is blocked by `blocked_by_id`.
    ///
    /// This enforces:
    /// - both tasks must exist
    /// - no self-dependencies
    /// - no cycles across `blocked_by` relationships
    pub fn add_dependency(&mut self, task_id: &str, blocked_by_id: &str) -> Result<(), TaskError> {
        if task_id == blocked_by_id {
            return Err(TaskError::InvalidInput(
                "task cannot be blocked by itself".to_string(),
            ));
        }

        // Ensure both exist.
        if self.find_task(task_id).is_none() {
            return Err(TaskError::NotFound(task_id.to_string()));
        }
        if self.find_task(blocked_by_id).is_none() {
            return Err(TaskError::NotFound(blocked_by_id.to_string()));
        }

        // Reject cycles: if `blocked_by_id` depends on `task_id` already, we'd create a cycle.
        if self.depends_on_transitively(blocked_by_id, task_id) {
            return Err(TaskError::InvalidInput(
                "dependency would create a cycle".to_string(),
            ));
        }

        let task = self
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.add_blocked_by(blocked_by_id.to_string());
        Ok(())
    }

    /// Build a recursive task tree.
    ///
    /// Nodes are ordered by `sort_order` then creation time.
    ///
    /// Tasks referencing a missing parent are treated as roots.
    pub fn build_tree(&self) -> Vec<TaskTreeNode> {
        let ids: HashSet<String> = self.tasks.iter().map(|t| t.id.clone()).collect();
        self.build_tree_for_parent(None, &ids)
    }

    /// Internal helper for building a task tree for a given parent.
    ///
    /// - `parent_id=None` means "roots".
    /// - Any task whose parent is missing from `ids` is treated as a root.
    fn build_tree_for_parent(
        &self,
        parent_id: Option<&str>,
        ids: &HashSet<String>,
    ) -> Vec<TaskTreeNode> {
        let mut children: Vec<Task> = self
            .tasks
            .iter()
            .filter(|t| match parent_id {
                Some(pid) => t.parent_id.as_deref() == Some(pid),
                None => {
                    t.parent_id.is_none()
                        || t.parent_id.as_deref().is_some_and(|p| !ids.contains(p))
                }
            })
            .cloned()
            .collect();

        children.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });

        children
            .into_iter()
            .map(|task| TaskTreeNode {
                children: self.build_tree_for_parent(Some(task.id.as_str()), ids),
                task,
            })
            .collect()
    }

    /// Returns `true` if `start` depends on `target` directly or transitively.
    fn depends_on_transitively(&self, start: &str, target: &str) -> bool {
        let mut visited: HashSet<String> = HashSet::new();
        self.depends_on_transitively_inner(start, target, &mut visited)
    }

    /// DFS helper for `depends_on_transitively`.
    fn depends_on_transitively_inner(
        &self,
        start: &str,
        target: &str,
        visited: &mut HashSet<String>,
    ) -> bool {
        if start == target {
            return true;
        }

        if visited.contains(start) {
            return false;
        }
        visited.insert(start.to_string());

        let Some(task) = self.find_task(start) else {
            return false;
        };

        for dep in &task.blocked_by {
            if self.depends_on_transitively_inner(dep, target, visited) {
                return true;
            }
        }

        false
    }
}

/// Error type for task operations
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    /// Task not found
    #[error("Task not found: {0}")]
    NotFound(String),
    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Task manager for persisting and managing tasks
pub struct TaskManager {
    /// Base directory for task files
    base_dir: PathBuf,
    /// In-memory cache of task lists by session ID
    cache: std::sync::RwLock<HashMap<String, TaskList>>,
}

impl TaskManager {
    /// Create a new task manager with the given base directory
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into().join(".gestura").join("tasks");
        Self {
            base_dir,
            cache: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Get the path to a session's task file
    fn task_file_path(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("{}.json", session_id))
    }

    /// Load tasks for a session from disk
    fn load_from_disk(&self, session_id: &str) -> Result<TaskList, TaskError> {
        let path = self.task_file_path(session_id);
        if !path.exists() {
            return Ok(TaskList::new(session_id));
        }

        let content = fs::read_to_string(&path)?;
        let task_list: TaskList = serde_json::from_str(&content)?;
        Ok(task_list)
    }

    /// Save tasks for a session to disk
    fn save_to_disk(&self, task_list: &TaskList) -> Result<(), TaskError> {
        // Ensure directory exists
        fs::create_dir_all(&self.base_dir)?;

        let path = self.task_file_path(&task_list.session_id);
        let content = serde_json::to_string_pretty(task_list)?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// Get or load task list for a session
    fn get_or_load(&self, session_id: &str) -> Result<TaskList, TaskError> {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(task_list) = cache.get(session_id) {
                return Ok(task_list.clone());
            }
        }

        // Load from disk
        let task_list = self.load_from_disk(session_id)?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(session_id.to_string(), task_list.clone());
        }

        Ok(task_list)
    }

    /// Update cache and save to disk
    fn update_and_save(&self, task_list: TaskList) -> Result<(), TaskError> {
        // Save to disk first
        self.save_to_disk(&task_list)?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(task_list.session_id.clone(), task_list);
        }

        Ok(())
    }

    /// Load the full task list for a session.
    ///
    /// This returns the persisted state (or an empty list if none exists yet).
    /// It is used by checkpoint/rewind to snapshot and restore task state.
    pub fn load_task_list(&self, session_id: &str) -> Result<TaskList, TaskError> {
        self.get_or_load(session_id)
    }

    /// Replace the persisted task list for a session.
    ///
    /// This is primarily used by checkpoint/rewind to restore a previous task
    /// state.
    pub fn replace_task_list(&self, task_list: TaskList) -> Result<(), TaskError> {
        self.update_and_save(task_list)
    }

    /// Set or clear the current task pointer for a session.
    ///
    /// If `task_id` is `Some`, it must exist in the session's task list.
    pub fn set_current_task_id(
        &self,
        session_id: &str,
        task_id: Option<String>,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        task_list.set_current_task_id(task_id)?;
        self.update_and_save(task_list)
    }

    /// Get the current task pointer for a session.
    pub fn get_current_task_id(&self, session_id: &str) -> Result<Option<String>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.current_task_id.clone())
    }

    /// Create a new task
    pub fn create_task(
        &self,
        session_id: &str,
        name: impl Into<String>,
        description: impl Into<String>,
        parent_id: Option<String>,
    ) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = Task::new(session_id, name, description, parent_id);
        task_list.add_task(task.clone());
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// Update a task's status
    pub fn update_task_status(
        &self,
        session_id: &str,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        task_list.validate_status_transition(task_id, status)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.set_status(status);
        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Update a task's name and description
    pub fn update_task(
        &self,
        session_id: &str,
        task_id: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        if let Some(name) = name {
            task.set_name(name);
        }
        if let Some(description) = description {
            task.set_description(description);
        }

        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Delete a task
    pub fn delete_task(&self, session_id: &str, task_id: &str) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .remove_task(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// List all tasks for a session
    pub fn list_tasks(&self, session_id: &str) -> Result<Vec<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.tasks.clone())
    }

    /// Get task hierarchy for a session
    pub fn get_hierarchy(&self, session_id: &str) -> Result<Vec<(Task, Vec<Task>)>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        let mut hierarchy = Vec::new();

        for root in task_list.root_tasks() {
            let subtasks = task_list.subtasks(&root.id).into_iter().cloned().collect();
            hierarchy.push((root.clone(), subtasks));
        }

        Ok(hierarchy)
    }

    /// List all descendant tasks for the given task.
    pub fn list_descendants(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<Vec<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        if task_list.find_task(task_id).is_none() {
            return Err(TaskError::NotFound(task_id.to_string()));
        }

        Ok(task_list
            .descendants(task_id)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Get a full recursive task tree for a session.
    pub fn get_task_tree(&self, session_id: &str) -> Result<Vec<TaskTreeNode>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.build_tree())
    }

    /// Add a dependency relationship to a task.
    pub fn add_task_dependency(
        &self,
        session_id: &str,
        task_id: &str,
        blocked_by_id: &str,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        task_list.add_dependency(task_id, blocked_by_id)?;
        self.update_and_save(task_list)
    }

    /// Update background job state for a task.
    pub fn set_task_background_job(
        &self,
        session_id: &str,
        task_id: &str,
        job: Option<TaskBackgroundJob>,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.set_background_job(job);
        self.update_and_save(task_list)
    }

    /// Create a task from an agent (during LLM processing)
    pub fn create_agent_task(
        &self,
        session_id: &str,
        name: impl Into<String>,
        description: impl Into<String>,
        agent_id: Option<String>,
        parent_id: Option<String>,
    ) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = Task::new_with_source(
            session_id,
            name,
            description,
            parent_id,
            TaskSource::Agent,
            agent_id,
        );
        task_list.add_task(task.clone());
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// Create a task from an orchestrator delegated task
    pub fn create_orchestrator_task(
        &self,
        session_id: &str,
        orchestrator_task_id: impl Into<String>,
        agent_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        context: Option<serde_json::Value>,
    ) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = Task::from_orchestrator_task(
            session_id,
            orchestrator_task_id,
            agent_id,
            name,
            description,
            context,
        );
        task_list.add_task(task.clone());
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// Find a task by its orchestrator_task_id
    pub fn find_by_orchestrator_id(
        &self,
        session_id: &str,
        orchestrator_task_id: &str,
    ) -> Result<Option<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list
            .tasks
            .iter()
            .find(|t| t.orchestrator_task_id.as_deref() == Some(orchestrator_task_id))
            .cloned())
    }

    /// Update a task's metadata
    pub fn update_task_metadata(
        &self,
        session_id: &str,
        task_id: &str,
        metadata: serde_json::Value,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.set_metadata(metadata);
        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Record a structured memory lifecycle event in task metadata.
    pub fn record_memory_event(
        &self,
        session_id: &str,
        task_id: &str,
        event: TaskMemoryEvent,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        let metadata = task.metadata.get_or_insert_with(|| serde_json::json!({}));
        if !metadata.is_object() {
            *metadata = serde_json::json!({});
        }

        let Some(metadata_map) = metadata.as_object_mut() else {
            return Err(TaskError::InvalidInput(
                "task metadata could not be represented as an object".to_string(),
            ));
        };

        let mut lifecycle: TaskMemoryLifecycle = metadata_map
            .get("memory_lifecycle")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();

        if let Some(memory_file_path) = event.memory_file_path.clone() {
            lifecycle.last_memory_file_path = Some(memory_file_path);
        }
        lifecycle.events.push(event);
        if lifecycle.events.len() > 32 {
            let excess = lifecycle.events.len() - 32;
            lifecycle.events.drain(0..excess);
        }

        metadata_map.insert(
            "memory_lifecycle".to_string(),
            serde_json::to_value(&lifecycle)?,
        );
        task.updated_at = Utc::now();

        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Read structured memory lifecycle data from task metadata.
    pub fn get_memory_lifecycle(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<Option<TaskMemoryLifecycle>, TaskError> {
        let task = self.get_task(session_id, task_id)?;
        let Some(task) = task else {
            return Ok(None);
        };
        let Some(metadata) = task.metadata else {
            return Ok(None);
        };
        let Some(value) = metadata.get("memory_lifecycle") else {
            return Ok(None);
        };

        Ok(Some(serde_json::from_value(value.clone())?))
    }

    /// Update the runtime execution state stored in task metadata.
    pub fn update_execution_state<F>(
        &self,
        session_id: &str,
        task_id: &str,
        update: F,
    ) -> Result<TaskExecutionState, TaskError>
    where
        F: FnOnce(&mut TaskExecutionState),
    {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        let metadata = task.metadata.get_or_insert_with(|| serde_json::json!({}));
        if !metadata.is_object() {
            *metadata = serde_json::json!({});
        }

        let Some(metadata_map) = metadata.as_object_mut() else {
            return Err(TaskError::InvalidInput(
                "task metadata could not be represented as an object".to_string(),
            ));
        };

        let mut state: TaskExecutionState = metadata_map
            .get("execution_state")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();

        update(&mut state);

        metadata_map.insert("execution_state".to_string(), serde_json::to_value(&state)?);
        task.updated_at = Utc::now();
        self.update_and_save(task_list)?;
        Ok(state)
    }

    /// Read runtime execution state from task metadata.
    pub fn get_execution_state(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<Option<TaskExecutionState>, TaskError> {
        let task = self.get_task(session_id, task_id)?;
        let Some(task) = task else {
            return Ok(None);
        };
        let Some(metadata) = task.metadata else {
            return Ok(None);
        };
        let Some(value) = metadata.get("execution_state") else {
            return Ok(None);
        };

        Ok(Some(serde_json::from_value(value.clone())?))
    }

    /// Get a specific task by ID
    pub fn get_task(&self, session_id: &str, task_id: &str) -> Result<Option<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.find_task(task_id).cloned())
    }
}

/// Process-wide global [`TaskManager`] instance.
///
/// All subsystems — Tauri commands, orchestrator observer, and the agent tool
/// pipeline — **must** use this single instance so they share one in-memory
/// cache and avoid stale-read bugs caused by independent `OnceLock` statics.
static GLOBAL_TASK_MANAGER: std::sync::OnceLock<TaskManager> = std::sync::OnceLock::new();

/// Returns the process-wide shared [`TaskManager`].
///
/// Initializes with `~/.gestura/tasks/` on first call; subsequent calls return
/// the same instance.  Callers must **not** create their own `OnceLock<TaskManager>`
/// statics — doing so produces independent in-memory caches that cause stale
/// reads when a different subsystem writes tasks to disk.
pub fn get_global_task_manager() -> &'static TaskManager {
    GLOBAL_TASK_MANAGER.get_or_init(|| {
        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        TaskManager::new(base_dir)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_task_creation() {
        let task = Task::new("session-123", "Test Task", "Test description", None);
        assert_eq!(task.session_id, "session-123");
        assert_eq!(task.name, "Test Task");
        assert_eq!(task.description, "Test description");
        assert_eq!(task.status, TaskStatus::NotStarted);
        assert!(task.parent_id.is_none());
        assert!(task.blocked_by.is_empty());
        assert!(task.background_job.is_none());
        assert!(!task.id.is_empty());
    }

    #[test]
    fn test_task_status_update() {
        let mut task = Task::new("session-123", "Test Task", "Test description", None);
        let original_updated_at = task.updated_at;

        // Small delay to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        task.set_status(TaskStatus::InProgress);
        assert_eq!(task.status, TaskStatus::InProgress);
        assert!(task.updated_at > original_updated_at);
    }

    #[test]
    fn test_task_list_operations() {
        let mut list = TaskList::new("session-123");
        assert_eq!(list.tasks.len(), 0);
        assert!(list.current_task_id().is_none());

        let task1 = Task::new("session-123", "Task 1", "Description 1", None);
        let task2 = Task::new(
            "session-123",
            "Task 2",
            "Description 2",
            Some(task1.id.clone()),
        );

        list.add_task(task1.clone());
        list.add_task(task2.clone());
        assert_eq!(list.tasks.len(), 2);

        // Setting current task requires it to exist
        assert!(list.set_current_task_id(Some(task1.id.clone())).is_ok());
        assert_eq!(list.current_task_id(), Some(task1.id.as_str()));
        assert!(matches!(
            list.set_current_task_id(Some("does-not-exist".to_string())),
            Err(TaskError::InvalidInput(_))
        ));

        // Test find
        let found = list.find_task(&task1.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Task 1");

        // Test root tasks
        let roots = list.root_tasks();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, task1.id);

        // Test subtasks
        let subtasks = list.subtasks(&task1.id);
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].id, task2.id);

        // Test remove
        let removed = list.remove_task(&task1.id);
        assert!(removed.is_some());
        assert_eq!(list.tasks.len(), 1);
        // removing the current task clears the pointer
        assert!(list.current_task_id().is_none());
    }

    #[test]
    fn test_task_current_pointer_roundtrip() {
        let temp_dir = TempDir::new().unwrap();

        let session_id = "session-123";
        let task_id = {
            let manager = TaskManager::new(temp_dir.path());
            let task = manager
                .create_task(session_id, "Test Task", "Test description", None)
                .unwrap();
            manager
                .set_current_task_id(session_id, Some(task.id.clone()))
                .unwrap();
            task.id
        };

        let manager = TaskManager::new(temp_dir.path());
        let loaded = manager.get_current_task_id(session_id).unwrap();
        assert_eq!(loaded, Some(task_id));
    }

    #[test]
    fn test_task_replace_task_list() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let session_id = "session-123";
        let task = Task::new(session_id, "Task 1", "Description 1", None);

        let mut list = TaskList::new(session_id);
        list.add_task(task.clone());
        list.set_current_task_id(Some(task.id.clone())).unwrap();

        manager.replace_task_list(list).unwrap();

        let loaded = manager.load_task_list(session_id).unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.current_task_id(), Some(task.id.as_str()));
    }

    #[test]
    fn test_task_manager_create_and_list() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        assert_eq!(task.name, "Test Task");

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task.id);
    }

    #[test]
    fn test_task_manager_persistence() {
        let temp_dir = TempDir::new().unwrap();

        // Create task with first manager instance
        {
            let manager = TaskManager::new(temp_dir.path());
            manager
                .create_task("session-123", "Test Task", "Test description", None)
                .unwrap();
        }

        // Load with second manager instance
        {
            let manager = TaskManager::new(temp_dir.path());
            let tasks = manager.list_tasks("session-123").unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].name, "Test Task");
        }
    }

    #[test]
    fn test_task_manager_update_status() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        manager
            .update_task_status("session-123", &task.id, TaskStatus::InProgress)
            .unwrap();

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks[0].status, TaskStatus::InProgress);
    }

    #[test]
    fn test_task_manager_update_task() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        manager
            .update_task(
                "session-123",
                &task.id,
                Some("Updated Task".to_string()),
                Some("Updated description".to_string()),
            )
            .unwrap();

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks[0].name, "Updated Task");
        assert_eq!(tasks[0].description, "Updated description");
    }

    #[test]
    fn test_task_manager_delete() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        let deleted = manager.delete_task("session-123", &task.id).unwrap();
        assert_eq!(deleted.id, task.id);

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    fn test_task_manager_hierarchy() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let root = manager
            .create_task("session-123", "Root Task", "Root description", None)
            .unwrap();

        manager
            .create_task(
                "session-123",
                "Subtask 1",
                "Subtask description",
                Some(root.id.clone()),
            )
            .unwrap();

        manager
            .create_task(
                "session-123",
                "Subtask 2",
                "Subtask description",
                Some(root.id.clone()),
            )
            .unwrap();

        let hierarchy = manager.get_hierarchy("session-123").unwrap();
        assert_eq!(hierarchy.len(), 1);
        assert_eq!(hierarchy[0].0.id, root.id);
        assert_eq!(hierarchy[0].1.len(), 2);
    }

    #[test]
    fn test_task_dependencies_blocking() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let session_id = "session-123";
        let dep = manager
            .create_task(session_id, "Dep", "Dependency", None)
            .unwrap();
        let task = manager
            .create_task(session_id, "Task", "Work", None)
            .unwrap();

        manager
            .add_task_dependency(session_id, &task.id, &dep.id)
            .unwrap();

        let list = manager.load_task_list(session_id).unwrap();
        assert!(list.is_task_blocked(&task.id).unwrap());

        manager
            .update_task_status(session_id, &dep.id, TaskStatus::Completed)
            .unwrap();

        let list = manager.load_task_list(session_id).unwrap();
        assert!(!list.is_task_blocked(&task.id).unwrap());
    }

    #[test]
    fn test_task_manager_rejects_completion_with_open_subtasks() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-123";

        let parent = manager
            .create_task(session_id, "Build hello world", "Create the app", None)
            .unwrap();
        let child = manager
            .create_task(
                session_id,
                "Implement UI",
                "Render hello world in a Tauri window",
                Some(parent.id.clone()),
            )
            .unwrap();

        let err = manager
            .update_task_status(session_id, &parent.id, TaskStatus::Completed)
            .unwrap_err();

        assert!(matches!(err, TaskError::InvalidInput(_)));
        assert!(err.to_string().contains("Implement UI"));

        manager
            .update_task_status(session_id, &child.id, TaskStatus::Completed)
            .unwrap();
        manager
            .update_task_status(session_id, &parent.id, TaskStatus::Completed)
            .unwrap();
    }

    #[test]
    fn test_task_list_descendants_include_nested_children() {
        let mut list = TaskList::new("session-123");
        let root = Task::new("session-123", "Root", "Root", None);
        let child = Task::new("session-123", "Child", "Child", Some(root.id.clone()));
        let grandchild = Task::new(
            "session-123",
            "Grandchild",
            "Grandchild",
            Some(child.id.clone()),
        );

        let root_id = root.id.clone();
        let child_id = child.id.clone();
        let grandchild_id = grandchild.id.clone();

        list.add_task(root);
        list.add_task(child);
        list.add_task(grandchild);

        let descendants = list.descendants(&root_id);
        assert_eq!(descendants.len(), 2);
        assert!(descendants.iter().any(|task| task.id == child_id));
        assert!(descendants.iter().any(|task| task.id == grandchild_id));
    }

    #[test]
    fn test_task_list_descendants_preserve_priority_order_depth_first() {
        let mut list = TaskList::new("session-priority-123");
        let root = Task::new("session-priority-123", "Root", "Root", None);
        let root_id = root.id.clone();

        let mut first = Task::new(
            "session-priority-123",
            "First",
            "First",
            Some(root.id.clone()),
        );
        first.sort_order = 0;

        let mut second = Task::new(
            "session-priority-123",
            "Second",
            "Second",
            Some(root.id.clone()),
        );
        second.sort_order = 10;

        let mut nested = Task::new(
            "session-priority-123",
            "Nested",
            "Nested",
            Some(first.id.clone()),
        );
        nested.sort_order = 0;

        let expected = vec![first.id.clone(), nested.id.clone(), second.id.clone()];

        list.add_task(root);
        list.add_task(first);
        list.add_task(second);
        list.add_task(nested);

        let ordered = list
            .descendants(&root_id)
            .into_iter()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();

        assert_eq!(ordered, expected);
    }

    #[test]
    fn test_task_manager_rejects_completion_with_open_nested_descendants() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-nested-123";

        let mut root = Task::new(session_id, "Root", "Root", None);
        let mut child = Task::new(session_id, "Child", "Child", Some(root.id.clone()));
        let grandchild = Task::new(
            session_id,
            "Grandchild",
            "Grandchild",
            Some(child.id.clone()),
        );
        child.set_status(TaskStatus::Completed);
        root.set_status(TaskStatus::InProgress);

        let mut task_list = TaskList::new(session_id);
        task_list.add_task(root.clone());
        task_list.add_task(child);
        task_list.add_task(grandchild);
        manager.replace_task_list(task_list).unwrap();

        let err = manager
            .update_task_status(session_id, &root.id, TaskStatus::Completed)
            .unwrap_err();

        assert!(matches!(err, TaskError::InvalidInput(_)));
        assert!(err.to_string().contains("Grandchild"));
    }

    #[test]
    fn test_task_manager_rejects_completion_while_blocked() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-123";

        let plan = manager
            .create_task(session_id, "Plan work", "Break down the task", None)
            .unwrap();
        let implementation = manager
            .create_task(session_id, "Implement app", "Build the Tauri app", None)
            .unwrap();

        manager
            .add_task_dependency(session_id, &implementation.id, &plan.id)
            .unwrap();

        let err = manager
            .update_task_status(session_id, &implementation.id, TaskStatus::Completed)
            .unwrap_err();

        assert!(matches!(err, TaskError::InvalidInput(_)));
        assert!(err.to_string().contains("dependencies remain open"));

        manager
            .update_task_status(session_id, &plan.id, TaskStatus::Completed)
            .unwrap();
        manager
            .update_task_status(session_id, &implementation.id, TaskStatus::Completed)
            .unwrap();
    }

    #[test]
    fn test_task_dependency_cycle_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-123";

        let a = manager.create_task(session_id, "A", "A", None).unwrap();
        let b = manager.create_task(session_id, "B", "B", None).unwrap();

        manager
            .add_task_dependency(session_id, &a.id, &b.id)
            .unwrap();

        let err = manager
            .add_task_dependency(session_id, &b.id, &a.id)
            .unwrap_err();

        assert!(matches!(err, TaskError::InvalidInput(_)));
    }

    #[test]
    fn test_task_tree_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-123";

        let root = manager
            .create_task(session_id, "Root", "Root", None)
            .unwrap();
        let child = manager
            .create_task(session_id, "Child", "Child", Some(root.id.clone()))
            .unwrap();
        manager
            .create_task(session_id, "Grand", "Grand", Some(child.id.clone()))
            .unwrap();

        let tree = manager.get_task_tree(session_id).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].task.id, root.id);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].task.id, child.id);
        assert_eq!(tree[0].children[0].children.len(), 1);
    }

    #[test]
    fn test_record_memory_event() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-123";

        let task = manager
            .create_task(session_id, "Memory Task", "Track delegation", None)
            .unwrap();

        manager
            .record_memory_event(
                session_id,
                &task.id,
                TaskMemoryEvent::new(
                    TaskMemoryPhase::Delegated,
                    "Delegated to subagent",
                    Some("directive".to_string()),
                    Some("handoff".to_string()),
                    None,
                ),
            )
            .unwrap();

        let lifecycle = manager
            .get_memory_lifecycle(session_id, &task.id)
            .unwrap()
            .unwrap();

        assert_eq!(lifecycle.events.len(), 1);
        assert_eq!(lifecycle.events[0].phase, TaskMemoryPhase::Delegated);
        assert_eq!(lifecycle.events[0].summary, "Delegated to subagent");
    }

    #[test]
    fn test_update_execution_state_records_profile_and_evidence() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());
        let session_id = "session-execution-state";

        let task = manager
            .create_task(session_id, "Implement feature", "Edit and verify", None)
            .unwrap();

        manager
            .update_execution_state(session_id, &task.id, |state| {
                state.merge_profile(TaskVerificationProfile {
                    execution_kind: TaskExecutionKind::Implementation,
                    requires_mutation: true,
                    ..TaskVerificationProfile::default()
                });
                state.record_evidence(TaskExecutionEvidence::new(
                    TaskExecutionEvidenceKind::Mutation,
                    "Edited src/main.rs",
                    Some("file".to_string()),
                    None,
                ));
            })
            .unwrap();

        let state = manager
            .get_execution_state(session_id, &task.id)
            .unwrap()
            .unwrap();

        assert_eq!(
            state.verification_profile.execution_kind,
            TaskExecutionKind::Implementation
        );
        assert!(state.verification_profile.requires_mutation);
        assert!(state.saw_mutation);
        assert!(state.satisfies_profile());
        assert_eq!(state.evidence.len(), 1);
    }

    #[test]
    fn generic_verification_profile_requires_observed_progress() {
        let mut state = TaskExecutionState::default();
        state.merge_profile(TaskVerificationProfile {
            execution_kind: TaskExecutionKind::Verification,
            ..TaskVerificationProfile::default()
        });

        assert!(!state.satisfies_profile());

        state.record_evidence(TaskExecutionEvidence::new(
            TaskExecutionEvidenceKind::ToolActivity,
            "Reviewed the generated artifact and cross-checked the facts",
            Some("web_search".to_string()),
            None,
        ));

        assert!(state.satisfies_profile());
    }
}
