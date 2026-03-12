//! GUI-side adapters for the core subagent orchestrator.
//!
//! This module is intentionally **thin**.
//! - All orchestration policy/execution lives in `gestura_core::orchestrator`.
//! - The GUI only provides a Tauri-backed observer to mirror task lifecycle events
//!   into the UI task panel.

use std::collections::HashMap;
use tauri::AppHandle;
use tauri::Emitter;
use tokio::sync::Mutex;

pub use gestura_core::agents::{
    AgentExecutionMode, AgentRole, AgentSpawnRequest, DelegatedTask, OrchestratorToolCall,
    TaskResult,
};
pub use gestura_core::orchestrator::{
    ActiveTaskSnapshot, AgentOrchestrator, ApprovalActor, ApprovalActorKind, ApprovalDecision,
    ApprovalDecisionKind, ApprovalPolicy, ApprovalRequest, ApprovalRequirement, ApprovalScope,
    ApprovalState, CleanupResult, CollaborationActionStatus, CollaborationEscalationLevel,
    CollaborationRequestKind, CollaborationThreadStatus, EnvironmentHealth, EnvironmentRecord,
    EnvironmentState, ExecutionEnvironment, OrchestratorAgentManager, OrchestratorObserver,
    RecoveryAction, RecoveryStatus, SupervisorRun, SupervisorRunStatus, SupervisorTaskRecord,
    SupervisorTaskState, TaskApprovalRecord, TeamActionRequest, TeamActionRequestDraft,
    TeamArtifactReference, TeamEscalation, TeamEscalationDraft, TeamMessage, TeamMessageDraft,
    TeamMessageKind, TeamResultReference, TeamThread,
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

        if let Some(existing_task_id) = task.tracking_task_id.clone() {
            let _ = crate::task_integration::mark_task_in_progress(
                &self.app,
                session_id,
                &existing_task_id,
            );
            self.ui_task_mapping
                .lock()
                .await
                .insert(task.id.clone(), existing_task_id);
            return;
        }

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

    async fn on_run_updated(&self, run: SupervisorRun) {
        let _ = self.app.emit("orchestrator-run-updated", &run);
    }

    async fn on_team_message(&self, message: TeamMessage) {
        let _ = self.app.emit("orchestrator-team-message", &message);
    }

    async fn on_team_thread_updated(&self, thread: TeamThread) {
        let _ = self.app.emit("orchestrator-team-thread-updated", &thread);
    }

    async fn on_environment_updated(&self, environment: EnvironmentRecord) {
        let _ = self
            .app
            .emit("orchestrator-environment-updated", &environment);
    }

    async fn on_environment_recovery(
        &self,
        environment_id: String,
        action: RecoveryAction,
        summary: String,
    ) {
        let _ = self.app.emit(
            "orchestrator-environment-recovery",
            &serde_json::json!({
                "environment_id": environment_id,
                "action": action,
                "summary": summary,
            }),
        );
    }

    async fn on_environment_cleanup(&self, environment_id: String, result: CleanupResult) {
        let _ = self.app.emit(
            "orchestrator-environment-cleanup",
            &serde_json::json!({
                "environment_id": environment_id,
                "result": result,
            }),
        );
    }
}
