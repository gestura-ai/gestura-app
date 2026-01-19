//! Subagent orchestration module
//!
//! Wires AgentManager/AgentSpawner into the main workflow with tool calling,
//! MCP integration, delegated tasks, and observability.

use crate::AppConfig;
use crate::agents::AgentManager;
use crate::mcp_server::McpServer;
use gestura_core::tools::PermissionManager;
use gestura_core::{AgentPipeline, AgentRequest, RequestSource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

/// Task that can be delegated to a subagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedTask {
    /// Unique task identifier
    pub id: String,
    /// Agent ID to delegate to
    pub agent_id: String,
    /// Task description/prompt
    pub prompt: String,
    /// Optional context from parent
    pub context: Option<serde_json::Value>,
    /// Required tools for the task
    pub required_tools: Vec<String>,
    /// Priority (lower = higher priority)
    pub priority: u8,
}

/// Result from a delegated task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub agent_id: String,
    pub success: bool,
    pub output: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub duration_ms: u64,
}

/// Record of a tool call during task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub success: bool,
    pub duration_ms: u64,
}

/// Orchestrator for coordinating subagents, tool calls, and MCP integration
pub struct AgentOrchestrator {
    agent_manager: AgentManager,
    mcp_server: Option<Arc<McpServer>>,
    permission_manager: PermissionManager,
    #[allow(dead_code)]
    task_queue: Arc<Mutex<Vec<DelegatedTask>>>,
    active_tasks: Arc<Mutex<HashMap<String, DelegatedTask>>>,
    result_tx: mpsc::Sender<TaskResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<TaskResult>>>,
    config: AppConfig,
}

impl AgentOrchestrator {
    /// Create a new orchestrator with the given agent manager
    pub fn new(agent_manager: AgentManager, config: AppConfig) -> Self {
        let (result_tx, result_rx) = mpsc::channel(100);
        Self {
            agent_manager,
            mcp_server: None,
            permission_manager: PermissionManager::new(),
            task_queue: Arc::new(Mutex::new(Vec::new())),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            result_tx,
            result_rx: Arc::new(Mutex::new(result_rx)),
            config,
        }
    }

    /// Attach an MCP server for tool execution
    pub fn attach_mcp_server(&mut self, mcp_server: Arc<McpServer>) {
        self.mcp_server = Some(mcp_server);
    }

    /// Spawn and register a new subagent
    pub async fn spawn_subagent(&self, id: &str, name: &str) -> Result<(), String> {
        tracing::info!(agent_id = %id, agent_name = %name, "Spawning subagent");
        self.agent_manager
            .spawn_agent(id.to_string(), name.to_string())
            .await;
        Ok(())
    }

    /// Delegate a task to a subagent
    pub async fn delegate_task(&self, task: DelegatedTask) -> Result<String, String> {
        let task_id = task.id.clone();
        let agent_id = task.agent_id.clone();

        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            priority = task.priority,
            "Delegating task to subagent"
        );

        // Check if agent exists, spawn if needed
        if self
            .agent_manager
            .get_agent_status(&agent_id)
            .await
            .is_none()
        {
            self.spawn_subagent(&agent_id, &format!("Subagent-{}", agent_id))
                .await?;
        }

        // Check tool permissions
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

        // Add to active tasks
        {
            let mut active = self.active_tasks.lock().await;
            active.insert(task_id.clone(), task.clone());
        }

        // Execute task in background
        let agent_manager = self.agent_manager.clone();
        let config = self.config.clone();
        let result_tx = self.result_tx.clone();
        let active_tasks = Arc::clone(&self.active_tasks);

        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let (result, tool_calls) = execute_delegated_task(&agent_manager, &config, &task).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            let task_result = TaskResult {
                task_id: task.id.clone(),
                agent_id: task.agent_id.clone(),
                success: result.is_ok(),
                output: result.unwrap_or_else(|e| e),
                tool_calls,
                duration_ms,
            };

            // Remove from active, send result
            active_tasks.lock().await.remove(&task.id);
            let _ = result_tx.send(task_result).await;
        });

        Ok(task_id)
    }

    /// Get the result of a completed task
    pub async fn poll_result(&self) -> Option<TaskResult> {
        let mut rx = self.result_rx.lock().await;
        rx.try_recv().ok()
    }

    /// Get list of active tasks
    pub async fn list_active_tasks(&self) -> Vec<DelegatedTask> {
        let active = self.active_tasks.lock().await;
        active.values().cloned().collect()
    }

    /// List all running subagents
    pub async fn list_subagents(&self) -> Vec<crate::agents::AgentInfo> {
        self.agent_manager.list_agents().await
    }

    /// Cancel a running task
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), String> {
        let task_opt = {
            let mut active = self.active_tasks.lock().await;
            active.remove(task_id)
        };

        if let Some(task) = task_opt {
            tracing::info!(task_id = %task_id, agent_id = %task.agent_id, "Cancelling task");
            // Send cancellation event to the agent
            self.agent_manager
                .send_event(&task.agent_id, format!("cancel:{}", task_id))
                .await;
            Ok(())
        } else {
            Err(format!("Task '{}' not found", task_id))
        }
    }

    /// Shutdown all subagents gracefully
    pub async fn shutdown_all(&self, grace_secs: u64) {
        tracing::info!(grace_secs = grace_secs, "Shutting down all subagents");
        self.agent_manager.shutdown_all(grace_secs).await;
    }
}

/// Execute a delegated task on the specified agent using the unified AgentPipeline
/// Returns (result, tool_calls) tuple for tracking tool usage
async fn execute_delegated_task(
    agent_manager: &AgentManager,
    config: &AppConfig,
    task: &DelegatedTask,
) -> (Result<String, String>, Vec<ToolCallRecord>) {
    // Build prompt with context
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

    // Update agent activity
    agent_manager.update_activity(&task.agent_id).await;

    // Build the agent request with tool filtering
    let request = AgentRequest::new(&full_prompt)
        .with_streaming(false)
        .with_source(RequestSource::Orchestrator)
        .with_allowed_tools(task.required_tools.clone());

    // Execute via unified pipeline
    let pipeline = AgentPipeline::new(config.clone());
    let result = pipeline.process_blocking(request).await;

    match result {
        Ok(response) => {
            // Convert pipeline tool calls to orchestrator format
            let tool_calls: Vec<ToolCallRecord> = response
                .tool_calls
                .into_iter()
                .map(|tc| {
                    // Parse arguments as JSON for input
                    let input =
                        serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));

                    // Extract output and success from ToolResult
                    let (output, success) = match &tc.result {
                        gestura_core::ToolResult::Success(s) => (
                            serde_json::from_str(s).unwrap_or(serde_json::json!({"result": s})),
                            true,
                        ),
                        gestura_core::ToolResult::Error(e) => {
                            (serde_json::json!({"error": e}), false)
                        }
                        gestura_core::ToolResult::Skipped(reason) => {
                            (serde_json::json!({"skipped": reason}), false)
                        }
                    };

                    ToolCallRecord {
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

// Note: Tool execution is now handled by AgentPipeline internally.
// The old parse_tool_call and execute_tool functions have been removed
// in favor of the unified pipeline approach.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentManager;

    #[tokio::test]
    async fn test_orchestrator_creation() {
        let manager = AgentManager::new(std::path::PathBuf::from("/tmp/test.db"));
        let config = AppConfig::default();
        let orchestrator = AgentOrchestrator::new(manager, config);

        assert!(orchestrator.list_subagents().await.is_empty());
        assert!(orchestrator.list_active_tasks().await.is_empty());
    }

    #[tokio::test]
    async fn test_spawn_subagent() {
        let manager = AgentManager::new(std::path::PathBuf::from("/tmp/test.db"));
        let config = AppConfig::default();
        let orchestrator = AgentOrchestrator::new(manager, config);

        orchestrator
            .spawn_subagent("test-1", "Test Agent")
            .await
            .unwrap();

        let agents = orchestrator.list_subagents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "test-1");
    }

    #[test]
    fn test_delegated_task_serialization() {
        let task = DelegatedTask {
            id: "task-1".into(),
            agent_id: "agent-1".into(),
            prompt: "Do something".into(),
            context: Some(serde_json::json!({"key": "value"})),
            required_tools: vec!["shell".into()],
            priority: 1,
        };

        let json = serde_json::to_string(&task).unwrap();
        let parsed: DelegatedTask = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "task-1");
        assert_eq!(parsed.required_tools.len(), 1);
    }
}
