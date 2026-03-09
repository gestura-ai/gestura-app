//! Task Integration Module
//!
//! Bridges the AgentOrchestrator's workflow tasks with the TaskManager that powers
//! the task panel UI, enabling bidirectional synchronization.
//!
//! ## Features
//! - Syncs orchestrator tasks to UI task panel
//! - Syncs user task changes back to agent workflow context
//! - Handles task completion/failure events
//! - Emits Tauri events for real-time UI updates

use gestura_core::{Task, TaskSource, TaskStatus};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Returns the process-wide shared [`gestura_core::TaskManager`].
///
/// Delegates to the canonical singleton in `gestura-core-tasks` so that the
/// orchestrator observer, the agent tool pipeline, and the Tauri command
/// handlers all share one in-memory cache — preventing stale reads in the UI.
pub fn get_task_manager() -> &'static gestura_core::TaskManager {
    gestura_core::get_global_task_manager()
}

/// Event payload for task events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEventPayload {
    pub session_id: String,
    pub task: Task,
    pub event_type: TaskEventType,
}

/// Type of task event
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TaskEventType {
    Created,
    Updated,
    StatusChanged,
    Completed,
    Cancelled,
    Deleted,
}

/// Create an agent task and emit events
///
/// Called when the agent starts working on something that should be visible in the UI
pub fn create_agent_task(
    app: &AppHandle,
    session_id: &str,
    name: &str,
    description: &str,
    agent_id: Option<String>,
    parent_id: Option<String>,
) -> Result<Task, String> {
    let manager = get_task_manager();
    let task = manager
        .create_agent_task(session_id, name, description, agent_id, parent_id)
        .map_err(|e| e.to_string())?;

    // Emit task-created event
    let _ = app.emit(
        "task-created",
        serde_json::json!({
            "session_id": session_id,
            "task": &task,
            "source": "agent"
        }),
    );

    tracing::info!(
        session_id = %session_id,
        task_id = %task.id,
        task_name = %task.name,
        "Agent task created and synced to UI"
    );

    Ok(task)
}

/// Create a task from an orchestrator delegated task
///
/// Called when the orchestrator delegates a task to a subagent
pub fn create_orchestrator_task(
    app: &AppHandle,
    session_id: &str,
    orchestrator_task_id: &str,
    agent_id: &str,
    name: &str,
    description: &str,
    context: Option<serde_json::Value>,
) -> Result<Task, String> {
    let manager = get_task_manager();
    let task = manager
        .create_orchestrator_task(
            session_id,
            orchestrator_task_id,
            agent_id,
            name,
            description,
            context,
        )
        .map_err(|e| e.to_string())?;

    // Emit task-created event
    let _ = app.emit(
        "task-created",
        serde_json::json!({
            "session_id": session_id,
            "task": &task,
            "source": "orchestrator"
        }),
    );

    tracing::info!(
        session_id = %session_id,
        task_id = %task.id,
        orchestrator_task_id = %orchestrator_task_id,
        "Orchestrator task created and synced to UI"
    );

    Ok(task)
}

/// Update task status with event emission
pub fn update_task_status(
    app: &AppHandle,
    session_id: &str,
    task_id: &str,
    status: TaskStatus,
) -> Result<(), String> {
    let manager = get_task_manager();
    manager
        .update_task_status(session_id, task_id, status)
        .map_err(|e| e.to_string())?;

    // Emit task-updated event
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": format!("{:?}", status)
        }),
    );

    Ok(())
}

/// Mark a task as in progress
pub fn mark_task_in_progress(
    app: &AppHandle,
    session_id: &str,
    task_id: &str,
) -> Result<(), String> {
    update_task_status(app, session_id, task_id, TaskStatus::InProgress)
}

/// Mark a task as completed
pub fn mark_task_completed(app: &AppHandle, session_id: &str, task_id: &str) -> Result<(), String> {
    update_task_status(app, session_id, task_id, TaskStatus::Completed)
}

/// Mark a task as cancelled
pub fn mark_task_cancelled(app: &AppHandle, session_id: &str, task_id: &str) -> Result<(), String> {
    update_task_status(app, session_id, task_id, TaskStatus::Cancelled)
}

/// Update task metadata with execution results
pub fn update_task_with_result(
    app: &AppHandle,
    session_id: &str,
    task_id: &str,
    success: bool,
    output: &str,
    tool_calls: Option<serde_json::Value>,
    duration_ms: Option<u64>,
) -> Result<(), String> {
    let manager = get_task_manager();

    // Update status
    let status = if success {
        TaskStatus::Completed
    } else {
        TaskStatus::Cancelled
    };

    manager
        .update_task_status(session_id, task_id, status)
        .map_err(|e| e.to_string())?;

    // Update metadata with result info
    let metadata = serde_json::json!({
        "success": success,
        "output": output,
        "tool_calls": tool_calls,
        "duration_ms": duration_ms,
        "completed_at": chrono::Utc::now().to_rfc3339()
    });

    manager
        .update_task_metadata(session_id, task_id, metadata)
        .map_err(|e| e.to_string())?;

    // Emit task-updated event
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": format!("{:?}", status),
            "success": success
        }),
    );

    tracing::info!(
        session_id = %session_id,
        task_id = %task_id,
        success = success,
        duration_ms = ?duration_ms,
        "Task result recorded"
    );

    Ok(())
}

/// Find a UI task by its orchestrator task ID
pub fn find_task_by_orchestrator_id(
    session_id: &str,
    orchestrator_task_id: &str,
) -> Result<Option<Task>, String> {
    let manager = get_task_manager();
    manager
        .find_by_orchestrator_id(session_id, orchestrator_task_id)
        .map_err(|e| e.to_string())
}

/// Check if a task is agent/orchestrator originated
pub fn is_agent_task(task: &Task) -> bool {
    matches!(task.source, TaskSource::Agent | TaskSource::Orchestrator)
}

/// Get all agent/orchestrator tasks for a session
pub fn get_agent_tasks(session_id: &str) -> Result<Vec<Task>, String> {
    let manager = get_task_manager();
    let all_tasks = manager.list_tasks(session_id).map_err(|e| e.to_string())?;
    Ok(all_tasks
        .into_iter()
        .filter(|t| matches!(t.source, TaskSource::Agent | TaskSource::Orchestrator))
        .collect())
}
