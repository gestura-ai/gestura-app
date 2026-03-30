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

use gestura_core::{
    Task, TaskSource, TaskStatus, tasks::TrackedTaskFinalization as CoreTrackedTaskFinalization,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
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

fn merge_task_metadata(
    existing: Option<serde_json::Value>,
    patch: serde_json::Value,
) -> serde_json::Value {
    let mut merged = existing
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    if let Some(patch_map) = patch.as_object() {
        for (key, value) in patch_map {
            merged.insert(key.clone(), value.clone());
        }
    }

    serde_json::Value::Object(merged)
}

/// Outcome of reconciling a tracked agent task after a run completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackedTaskFinalization {
    /// The tracked task tree is terminal, so the root task was completed.
    Completed,
    /// The run ended while planned subtasks still remained open.
    StillInProgress { open_subtasks: Vec<String> },
}

fn finalize_tracked_task_after_agent_run_with_manager(
    manager: &gestura_core::TaskManager,
    session_id: &str,
    task_id: &str,
) -> Result<TrackedTaskFinalization, String> {
    let tracked_task = manager
        .get_task(session_id, task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Tracked task '{task_id}' was not found"))?;

    let finalization = manager
        .finalize_tracked_task_after_agent_run(session_id, task_id)
        .map_err(|e| e.to_string())?;

    if let CoreTrackedTaskFinalization::StillInProgress { open_subtask_ids } = &finalization {
        let open_subtasks = open_subtask_ids
            .iter()
            .filter_map(|open_task_id| {
                manager
                    .get_task(session_id, open_task_id)
                    .ok()
                    .flatten()
                    .map(|task| task.name)
            })
            .collect::<Vec<_>>();

        let metadata = merge_task_metadata(
            tracked_task.metadata,
            json!({
                "agent_run": {
                    "completion_state": "pending_reconciliation",
                    "last_finished_at": chrono::Utc::now().to_rfc3339(),
                    "open_subtasks": open_subtasks,
                }
            }),
        );
        manager
            .update_task_metadata(session_id, task_id, metadata)
            .map_err(|e| e.to_string())?;

        return Ok(TrackedTaskFinalization::StillInProgress { open_subtasks });
    }

    let metadata = merge_task_metadata(
        tracked_task.metadata,
        json!({
            "agent_run": {
                "completion_state": "completed",
                "last_finished_at": chrono::Utc::now().to_rfc3339(),
                "open_subtasks": serde_json::Value::Array(Vec::new()),
            }
        }),
    );
    manager
        .update_task_metadata(session_id, task_id, metadata)
        .map_err(|e| e.to_string())?;

    match finalization {
        CoreTrackedTaskFinalization::Completed => Ok(TrackedTaskFinalization::Completed),
        CoreTrackedTaskFinalization::StillInProgress { .. } => unreachable!(),
    }
}

fn cancel_open_descendants_with_manager(
    manager: &gestura_core::TaskManager,
    session_id: &str,
    task_id: &str,
) -> Result<Vec<String>, String> {
    let descendants = manager
        .list_descendants(session_id, task_id)
        .map_err(|e| e.to_string())?;
    let name_by_id = descendants
        .into_iter()
        .map(|task| (task.id, task.name))
        .collect::<std::collections::HashMap<_, _>>();

    let cancelled_ids = manager
        .cancel_open_descendants(session_id, task_id)
        .map_err(|e| e.to_string())?;

    Ok(cancelled_ids
        .into_iter()
        .filter_map(|task_id| name_by_id.get(&task_id).cloned())
        .collect())
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
    let manager = get_task_manager();
    let cancelled_descendants = cancel_open_descendants_with_manager(manager, session_id, task_id)?;
    manager
        .update_task_status(session_id, task_id, TaskStatus::Cancelled)
        .map_err(|e| e.to_string())?;

    if manager
        .get_current_task_id(session_id)
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some(task_id)
    {
        manager
            .set_current_task_id(session_id, None)
            .map_err(|e| e.to_string())?;
    }

    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": format!("{:?}", TaskStatus::Cancelled),
            "cancelled_open_subtasks": cancelled_descendants,
        }),
    );

    Ok(())
}

/// Finalize a tracked task after an agent run by reconciling its subtask tree.
pub fn finalize_tracked_task_after_agent_run(
    app: &AppHandle,
    session_id: &str,
    task_id: &str,
) -> Result<TrackedTaskFinalization, String> {
    let outcome = finalize_tracked_task_after_agent_run_with_manager(
        get_task_manager(),
        session_id,
        task_id,
    )?;

    let payload = match &outcome {
        TrackedTaskFinalization::Completed => json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": format!("{:?}", TaskStatus::Completed),
            "finalized": true,
        }),
        TrackedTaskFinalization::StillInProgress { open_subtasks } => json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": format!("{:?}", TaskStatus::InProgress),
            "finalized": false,
            "open_subtasks": open_subtasks,
        }),
    };
    let _ = app.emit("task-updated", payload);

    Ok(outcome)
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
    let status = update_task_with_result_with_manager(
        get_task_manager(),
        session_id,
        task_id,
        success,
        output,
        tool_calls,
        duration_ms,
    )?;

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
        status = ?status,
        "Task result recorded"
    );

    Ok(())
}

fn update_task_with_result_with_manager(
    manager: &gestura_core::TaskManager,
    session_id: &str,
    task_id: &str,
    success: bool,
    output: &str,
    tool_calls: Option<serde_json::Value>,
    duration_ms: Option<u64>,
) -> Result<TaskStatus, String> {
    let tool_call_count = tool_calls
        .as_ref()
        .and_then(|value| value.as_array())
        .map(|calls| calls.len() as i32)
        .unwrap_or_default();
    let duration_ms_i32 = duration_ms
        .map(|value| i32::try_from(value).unwrap_or(i32::MAX))
        .unwrap_or_default();

    let existing_task = manager
        .get_task(session_id, task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Task '{task_id}' not found"))?;

    let mut metadata_patch = serde_json::json!({
        "success": success,
        "output": output,
        "tool_calls": tool_call_count,
        "duration_ms": duration_ms_i32,
        "completed_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Some(tool_calls) = tool_calls {
        metadata_patch["tool_calls"] = tool_calls;
    }

    let metadata = merge_task_metadata(existing_task.metadata, metadata_patch);

    manager
        .update_task_metadata(session_id, task_id, metadata)
        .map_err(|e| e.to_string())?;

    if success {
        return match finalize_tracked_task_after_agent_run_with_manager(
            manager, session_id, task_id,
        )? {
            TrackedTaskFinalization::Completed => Ok(TaskStatus::Completed),
            TrackedTaskFinalization::StillInProgress { .. } => Ok(TaskStatus::InProgress),
        };
    }

    let _ = cancel_open_descendants_with_manager(manager, session_id, task_id)?;
    manager
        .update_task_status(session_id, task_id, TaskStatus::Cancelled)
        .map_err(|e| e.to_string())?;

    Ok(TaskStatus::Cancelled)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_root_completes_when_all_direct_subtasks_are_terminal() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("task-finalize-complete-{}", uuid::Uuid::new_v4());

        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        let child = manager
            .create_task(&session_id, "Run build", "desc", Some(root.id.clone()))
            .expect("child task");
        manager
            .update_task_status(&session_id, &child.id, TaskStatus::Completed)
            .expect("complete child");
        manager
            .set_current_task_id(&session_id, Some(root.id.clone()))
            .expect("set current task");

        let outcome =
            finalize_tracked_task_after_agent_run_with_manager(&manager, &session_id, &root.id)
                .expect("finalize tracked root");

        assert_eq!(outcome, TrackedTaskFinalization::Completed);
        let stored_root = manager
            .get_task(&session_id, &root.id)
            .expect("get root")
            .expect("stored root");
        assert_eq!(stored_root.status, TaskStatus::Completed);
        assert_eq!(manager.get_current_task_id(&session_id).unwrap(), None);
    }

    #[test]
    fn tracked_root_stays_in_progress_when_open_subtasks_remain() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("task-finalize-open-{}", uuid::Uuid::new_v4());

        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        let child = manager
            .create_task(&session_id, "Run build", "desc", Some(root.id.clone()))
            .expect("child task");

        let outcome =
            finalize_tracked_task_after_agent_run_with_manager(&manager, &session_id, &root.id)
                .expect("reconcile tracked root");

        assert_eq!(
            outcome,
            TrackedTaskFinalization::StillInProgress {
                open_subtasks: vec![child.name.clone()]
            }
        );
        let stored_root = manager
            .get_task(&session_id, &root.id)
            .expect("get root")
            .expect("stored root");
        let stored_child = manager
            .get_task(&session_id, &child.id)
            .expect("get child")
            .expect("stored child");
        assert_eq!(stored_root.status, TaskStatus::InProgress);
        assert_eq!(stored_child.status, TaskStatus::NotStarted);
        assert_eq!(
            stored_root
                .metadata
                .as_ref()
                .and_then(|value| value.get("agent_run"))
                .and_then(|value| value.get("completion_state"))
                .and_then(|value| value.as_str()),
            Some("pending_reconciliation")
        );
    }

    #[test]
    fn tracked_root_stays_in_progress_when_nested_subtasks_remain_open() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("task-finalize-nested-open-{}", uuid::Uuid::new_v4());

        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        let child = manager
            .create_task(&session_id, "Implement UI", "desc", Some(root.id.clone()))
            .expect("child task");
        let grandchild = manager
            .create_task(&session_id, "Run build", "desc", Some(child.id.clone()))
            .expect("grandchild task");
        manager
            .update_task_status(&session_id, &grandchild.id, TaskStatus::Completed)
            .expect("complete grandchild");

        let mut task_list = manager.load_task_list(&session_id).expect("load task list");
        let stored_child = task_list
            .find_task_mut(&child.id)
            .expect("stored child task");
        stored_child.set_status(TaskStatus::Completed);
        task_list
            .find_task_mut(&grandchild.id)
            .expect("stored grandchild task")
            .set_status(TaskStatus::NotStarted);
        manager
            .replace_task_list(task_list)
            .expect("replace task list");

        let outcome =
            finalize_tracked_task_after_agent_run_with_manager(&manager, &session_id, &root.id)
                .expect("reconcile tracked root");

        assert_eq!(
            outcome,
            TrackedTaskFinalization::StillInProgress {
                open_subtasks: vec![grandchild.name.clone()]
            }
        );
        let stored_grandchild = manager
            .get_task(&session_id, &grandchild.id)
            .expect("get grandchild")
            .expect("stored grandchild");
        assert_eq!(stored_grandchild.status, TaskStatus::NotStarted);
    }

    #[test]
    fn successful_task_result_keeps_parent_in_progress_when_subtasks_remain_open() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("task-result-open-subtasks-{}", uuid::Uuid::new_v4());

        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        let child = manager
            .create_task(&session_id, "Run build", "desc", Some(root.id.clone()))
            .expect("child task");

        let status = update_task_with_result_with_manager(
            &manager,
            &session_id,
            &root.id,
            true,
            "run succeeded but follow-up work remains",
            None,
            Some(42),
        )
        .expect("record task result");

        let stored_root = manager
            .get_task(&session_id, &root.id)
            .expect("get root")
            .expect("stored root");
        let stored_child = manager
            .get_task(&session_id, &child.id)
            .expect("get child")
            .expect("stored child");

        assert_eq!(status, TaskStatus::InProgress);
        assert_eq!(stored_root.status, TaskStatus::InProgress);
        assert_eq!(stored_child.status, TaskStatus::NotStarted);
        assert_eq!(
            stored_root
                .metadata
                .as_ref()
                .and_then(|value| value.get("success"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }
}
