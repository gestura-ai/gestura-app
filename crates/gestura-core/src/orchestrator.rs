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
use crate::{MemoryBankEntry, MemoryScope, MemoryType};
use crate::{TaskManager, TaskStatus};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
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

        record_task_dispatch(&task);

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

            let task_result = TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: true,
                output: response.content.clone(),
                tool_calls: tool_calls.clone(),
                duration_ms: 0,
            };
            let memory_file_path = persist_delegated_task_memory(task, &task_result).await;
            record_task_completion(task, &task_result, memory_file_path.as_deref());

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
            let task_result = TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: false,
                output: e.to_string(),
                tool_calls: Vec::new(),
                duration_ms: 0,
            };
            let memory_file_path = persist_delegated_task_memory(task, &task_result).await;
            record_task_completion(task, &task_result, memory_file_path.as_deref());

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

fn record_task_dispatch(task: &DelegatedTask) {
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
    let _ = manager.update_task_status(session_id, tracking_task_id, TaskStatus::InProgress);
    let _ = manager.record_memory_event(
        session_id,
        tracking_task_id,
        crate::tasks::TaskMemoryEvent::new(
            crate::tasks::TaskMemoryPhase::Delegated,
            format!("Delegated to agent {}", task.agent_id),
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
                "orchestrator_task_id": task.id,
                "agent_id": task.agent_id,
                "directive_id": task.directive_id,
                "state": "delegated",
                "memory_tags": task.memory_tags,
            }
        }),
    );
}

fn record_task_completion(
    task: &DelegatedTask,
    task_result: &TaskResult,
    memory_file_path: Option<&Path>,
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
    if task_result.success {
        let _ = manager.update_task_status(session_id, tracking_task_id, TaskStatus::Completed);
    }
    let _ = manager.record_memory_event(
        session_id,
        tracking_task_id,
        crate::tasks::TaskMemoryEvent::new(
            if task_result.success {
                crate::tasks::TaskMemoryPhase::Promoted
            } else {
                crate::tasks::TaskMemoryPhase::Blocked
            },
            if task_result.success {
                format!("Delegated work completed by {}", task.agent_id)
            } else {
                format!("Delegated work blocked on {}", task.agent_id)
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
                if task_result.success {
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
                "orchestrator_task_id": task.id,
                "agent_id": task.agent_id,
                "directive_id": task.directive_id,
                "state": if task_result.success { "completed" } else { "blocked" },
                "last_output": task_result.output,
                "memory_file_path": memory_file_path.map(|path| path.display().to_string()),
                "tool_calls": task_result.tool_calls,
            }
        }),
    );
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
            workspace_dir: None,
            memory_tags: vec![],
            name: None,
        };

        // Verify task can be submitted
        let id = orchestrator.delegate_task(task).await.unwrap();
        assert_eq!(id, "task-1");
    }
}
