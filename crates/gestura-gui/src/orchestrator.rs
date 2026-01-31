//! GUI-side adapters for the core subagent orchestrator.
//!
//! This module is intentionally **thin**.
//! - All orchestration policy/execution lives in `gestura_core::orchestrator`.
//! - The GUI only provides a Tauri-backed observer to mirror task lifecycle events
//!   into the UI task panel.

use std::collections::HashMap;
use tauri::AppHandle;
use tokio::sync::Mutex;

pub use gestura_core::agents::{DelegatedTask, OrchestratorToolCall, TaskResult};
pub use gestura_core::orchestrator::{
    AgentOrchestrator, OrchestratorAgentManager, OrchestratorObserver,
};

/// A Tauri-backed observer that mirrors orchestrator task lifecycle events into the
/// GUI task panel via `crate::task_integration`.
pub struct TauriTaskObserver {
    app: AppHandle,
    ui_task_mapping: Mutex<HashMap<String, String>>,
}

impl TauriTaskObserver {
    /// Create a new task observer that emits `task-created` / `task-updated` events.
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            ui_task_mapping: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl OrchestratorObserver for TauriTaskObserver {
    async fn on_task_started(&self, task: DelegatedTask) {
        let Some(session_id) = task.session_id.as_deref() else {
            return;
        };

        let task_name = task.name.clone().unwrap_or_else(|| {
            // Generate a name from the prompt (first 50 chars)
            let prompt_preview = task.prompt.chars().take(50).collect::<String>();
            if task.prompt.len() > 50 {
                format!("{}...", prompt_preview)
            } else {
                prompt_preview
            }
        });

        match crate::task_integration::create_orchestrator_task(
            &self.app,
            session_id,
            &task.id,
            &task.agent_id,
            &task_name,
            &task.prompt,
            task.context.clone(),
        ) {
            Ok(ui_task) => {
                let _ = crate::task_integration::mark_task_in_progress(
                    &self.app,
                    session_id,
                    &ui_task.id,
                );

                self.ui_task_mapping
                    .lock()
                    .await
                    .insert(task.id.clone(), ui_task.id);
            }
            Err(e) => {
                tracing::warn!(task_id = %task.id, error = %e, "Failed to create UI task for orchestrator task");
            }
        }
    }

    async fn on_task_completed(&self, task: DelegatedTask, result: TaskResult) {
        let Some(session_id) = task.session_id.as_deref() else {
            return;
        };

        let ui_task_id = { self.ui_task_mapping.lock().await.get(&task.id).cloned() };
        let Some(ui_task_id) = ui_task_id else {
            return;
        };

        let tool_calls_json = serde_json::to_value(&result.tool_calls).ok();
        let _ = crate::task_integration::update_task_with_result(
            &self.app,
            session_id,
            &ui_task_id,
            result.success,
            &result.output,
            tool_calls_json,
            Some(result.duration_ms),
        );

        self.ui_task_mapping.lock().await.remove(&task.id);
    }
}
