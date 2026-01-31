//! Subagent orchestration (core-owned, tauri-free).
//!
//! This module coordinates delegated tasks across subagents and executes them via the
//! unified [`crate::pipeline::AgentPipeline`].
//!
//! ## Layering
//! - **gestura-core** owns orchestration policy (permissions, task tracking, execution).
//! - Adapters (GUI/CLI) may attach observers to emit UI events, but must not re-implement
//!   orchestration logic.

use crate::tools::PermissionManager;
use crate::{AgentPipeline, AgentRequest, AppConfig, RequestSource};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, mpsc};

// Re-export shared task types for convenience and adapter compatibility.
pub use crate::agents::{AgentInfo, AgentSpawner, DelegatedTask, OrchestratorToolCall, TaskResult};

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
}

/// Orchestrator for coordinating subagents and delegated task execution.
///
/// The orchestrator is core-owned and does not depend on Tauri. GUI/CLI layers can
/// attach an [`OrchestratorObserver`] to receive lifecycle events.
pub struct AgentOrchestrator<M: OrchestratorAgentManager> {
    agent_manager: M,
    permission_manager: PermissionManager,
    active_tasks: Arc<Mutex<HashMap<String, DelegatedTask>>>,
    result_tx: mpsc::Sender<TaskResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<TaskResult>>>,
    config: AppConfig,
    observer: Arc<RwLock<Option<Arc<dyn OrchestratorObserver>>>>,
}

impl<M: OrchestratorAgentManager> AgentOrchestrator<M> {
    /// Create a new orchestrator with the given agent manager and application config.
    pub fn new(agent_manager: M, config: AppConfig) -> Self {
        let (result_tx, result_rx) = mpsc::channel(100);
        Self {
            agent_manager,
            permission_manager: PermissionManager::new(),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            result_tx,
            result_rx: Arc::new(Mutex::new(result_rx)),
            config,
            observer: Arc::new(RwLock::new(None)),
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
        tracing::info!(agent_id = %id, agent_name = %name, "Spawning subagent");
        self.agent_manager
            .spawn_agent(id.to_string(), name.to_string())
            .await;
        Ok(())
    }

    /// Delegate a task to a subagent.
    ///
    /// - Ensures the target agent exists (spawning a default one if needed)
    /// - Enforces tool permission checks
    /// - Executes the task asynchronously via the unified pipeline
    pub async fn delegate_task(&self, task: DelegatedTask) -> Result<String, String> {
        let task_id = task.id.clone();
        let agent_id = task.agent_id.clone();
        let session_id = task.session_id.clone();

        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            session_id = ?session_id,
            priority = task.priority,
            "Delegating task to subagent"
        );

        // Check if agent exists, spawn if needed.
        if self
            .agent_manager
            .get_agent_status(&agent_id)
            .await
            .is_none()
        {
            self.spawn_subagent(&agent_id, &format!("Subagent-{}", agent_id))
                .await?;
        }

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

        // Store in active tasks.
        {
            let mut active = self.active_tasks.lock().await;
            active.insert(task_id.clone(), task.clone());
        }

        // Notify observer (best-effort).
        let observer_for_start = { self.observer.read().await.clone() };
        if let Some(obs) = observer_for_start.as_ref() {
            obs.on_task_started(task.clone()).await;
        }

        // Execute task in background.
        let agent_manager = self.agent_manager.clone();
        let config = self.config.clone();
        let result_tx = self.result_tx.clone();
        let active_tasks = Arc::clone(&self.active_tasks);
        let observer = Arc::clone(&self.observer);

        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let (result, tool_calls) = execute_delegated_task(&agent_manager, &config, &task).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            let task_result = TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: result.is_ok(),
                output: result.clone().unwrap_or_else(|e| e),
                tool_calls: tool_calls.clone(),
                duration_ms,
            };

            // Remove from active.
            active_tasks.lock().await.remove(&task.id);

            // Observer notification (best-effort).
            let observer_for_complete = { observer.read().await.clone() };
            if let Some(obs) = observer_for_complete.as_ref() {
                obs.on_task_completed(task.clone(), task_result.clone())
                    .await;
            }

            // Send result to consumers (best-effort).
            let _ = result_tx.send(task_result).await;
        });

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

    /// List all running subagents.
    pub async fn list_subagents(&self) -> Vec<AgentInfo> {
        self.agent_manager.list_agents().await
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
}

/// Execute a delegated task on the specified agent using the unified AgentPipeline.
///
/// Returns the final (text) result (or error string) and a structured list of tool calls.
async fn execute_delegated_task<M: OrchestratorAgentManager>(
    agent_manager: &M,
    config: &AppConfig,
    task: &DelegatedTask,
) -> (Result<String, String>, Vec<OrchestratorToolCall>) {
    // Build prompt with context.
    let full_prompt = if let Some(ctx) = &task.context {
        format!(
            "Context:\n{}\n\nTask:\n{}",
            serde_json::to_string_pretty(ctx).unwrap_or_default(),
            task.prompt
        )
    } else {
        task.prompt.clone()
    };

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
    let request = AgentRequest::new(&full_prompt)
        .with_streaming(false)
        .with_source(RequestSource::Orchestrator)
        .with_allowed_tools(task.required_tools.clone());

    // Execute via unified pipeline.
    let pipeline = AgentPipeline::with_provider_optimized_config(config.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_orchestrator_creation_and_spawn() {
        let manager = crate::agents::AgentManager::new(PathBuf::from("/tmp/test.db"));
        let mut config = AppConfig::default();
        config.llm.primary = "echo".into();

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
    async fn test_delegate_task_uses_pipeline_echo_provider() {
        let manager = crate::agents::AgentManager::new(PathBuf::from("/tmp/test.db"));
        let mut config = AppConfig::default();
        config.llm.primary = "echo".into();

        let orchestrator = AgentOrchestrator::new(manager, config);

        let task = DelegatedTask {
            id: "task-1".into(),
            agent_id: "agent-1".into(),
            prompt: "Hello".into(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: None,
            name: None,
        };

        let id = orchestrator.delegate_task(task).await.unwrap();
        assert_eq!(id, "task-1");

        // Wait for the async task to complete and send a result.
        for _ in 0..50 {
            if let Some(res) = orchestrator.poll_result().await {
                assert_eq!(res.task_id, "task-1");
                assert!(res.success);
                assert!(res.output.starts_with("ECHO: "));
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("timed out waiting for orchestrator result");
    }
}
