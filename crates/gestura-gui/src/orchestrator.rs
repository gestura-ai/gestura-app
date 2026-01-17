//! Subagent orchestration module
//!
//! Wires AgentManager/AgentSpawner into the main workflow with tool calling,
//! MCP integration, delegated tasks, and observability.

use crate::agents::AgentManager;
use crate::llm_provider::{AgentContext, select_provider};
use crate::mcp_server::McpServer;
use crate::AppConfig;
use gestura_core::tools::PermissionManager;
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
        if self.agent_manager.get_agent_status(&agent_id).await.is_none() {
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

/// Execute a delegated task on the specified agent
/// Returns (result, tool_calls) tuple for tracking tool usage
async fn execute_delegated_task(
    agent_manager: &AgentManager,
    config: &AppConfig,
    task: &DelegatedTask,
) -> (Result<String, String>, Vec<ToolCallRecord>) {
    let mut tool_calls = Vec::new();

    let provider = select_provider(
        config,
        &AgentContext {
            agent_id: task.agent_id.clone(),
        },
    );

    // Build prompt with context and available tools
    let tools_info = if !task.required_tools.is_empty() {
        format!(
            "\n\nAvailable tools: {}\nTo use a tool, respond with: TOOL_CALL: <tool_name> <json_args>",
            task.required_tools.join(", ")
        )
    } else {
        String::new()
    };

    let full_prompt = if let Some(ctx) = &task.context {
        format!(
            "Context:\n{}\n\nTask:\n{}{}",
            serde_json::to_string_pretty(ctx).unwrap_or_default(),
            task.prompt,
            tools_info
        )
    } else {
        format!("{}{}", task.prompt, tools_info)
    };

    tracing::debug!(
        agent_id = %task.agent_id,
        task_id = %task.id,
        prompt_len = full_prompt.len(),
        required_tools = ?task.required_tools,
        "Executing delegated task"
    );

    // Update agent activity
    agent_manager.update_activity(&task.agent_id).await;

    // Agentic loop: call LLM, check for tool calls, execute tools, repeat
    let mut current_prompt = full_prompt;
    let mut final_response = String::new();
    let max_iterations = 10; // Prevent infinite loops

    for iteration in 0..max_iterations {
        // Call LLM
        let response = match provider.call(&current_prompt).await {
            Ok(r) => r,
            Err(e) => return (Err(format!("LLM error: {}", e)), tool_calls),
        };

        // Check for tool call pattern in response
        if let Some(tool_call) = parse_tool_call(&response) {
            // Verify tool is in required_tools list
            if !task.required_tools.contains(&tool_call.tool_name) {
                tracing::warn!(
                    tool = %tool_call.tool_name,
                    "Agent attempted to use unauthorized tool"
                );
                current_prompt = format!(
                    "{}\n\nError: Tool '{}' is not available. Available tools: {}",
                    response,
                    tool_call.tool_name,
                    task.required_tools.join(", ")
                );
                continue;
            }

            // Execute the tool
            let tool_start = std::time::Instant::now();
            let tool_result = execute_tool(&tool_call.tool_name, &tool_call.input).await;
            let tool_duration = tool_start.elapsed().as_millis() as u64;

            let (output, success) = match &tool_result {
                Ok(out) => (out.clone(), true),
                Err(e) => (serde_json::json!({"error": e}), false),
            };

            // Record the tool call
            tool_calls.push(ToolCallRecord {
                tool_name: tool_call.tool_name.clone(),
                input: tool_call.input.clone(),
                output: output.clone(),
                success,
                duration_ms: tool_duration,
            });

            tracing::info!(
                tool = %tool_call.tool_name,
                success = success,
                duration_ms = tool_duration,
                iteration = iteration,
                "Tool call executed"
            );

            // Continue conversation with tool result
            current_prompt = format!(
                "{}\n\nTool result for {}:\n{}",
                response,
                tool_call.tool_name,
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        } else {
            // No tool call, this is the final response
            final_response = response;
            break;
        }
    }

    tracing::info!(
        agent_id = %task.agent_id,
        task_id = %task.id,
        response_len = final_response.len(),
        tool_calls_count = tool_calls.len(),
        "Task completed"
    );

    (Ok(final_response), tool_calls)
}

/// Parsed tool call from LLM response
struct ParsedToolCall {
    tool_name: String,
    input: serde_json::Value,
}

/// Parse a tool call from LLM response
/// Expected format: TOOL_CALL: <tool_name> <json_args>
fn parse_tool_call(response: &str) -> Option<ParsedToolCall> {
    let marker = "TOOL_CALL:";
    let idx = response.find(marker)?;
    let rest = response[idx + marker.len()..].trim();

    // Split into tool name and args
    let parts: Vec<&str> = rest.splitn(2, ' ').collect();
    if parts.is_empty() {
        return None;
    }

    let tool_name = parts[0].trim().to_string();
    let input = if parts.len() > 1 {
        serde_json::from_str(parts[1].trim()).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    Some(ParsedToolCall { tool_name, input })
}

/// Execute a tool by name with given input
async fn execute_tool(tool_name: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    use gestura_core::tools::{shell::ShellTools, file::FileTools};

    match tool_name {
        "shell" => {
            let command = input.get("command")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'command' argument")?;
            let timeout = input.get("timeout_secs")
                .and_then(|v| v.as_u64());

            let shell = ShellTools::new();
            let result = shell.run(command, timeout)
                .map_err(|e| format!("Shell error: {}", e))?;

            Ok(serde_json::json!({
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exit_code": result.exit_code,
                "success": result.success,
                "duration_ms": result.duration_ms
            }))
        }
        "file" => {
            let operation = input.get("operation")
                .and_then(|v| v.as_str())
                .unwrap_or("read");
            let path = input.get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'path' argument")?;

            let file_tools = FileTools::new();

            match operation {
                "read" => {
                    // read(path, start_line, end_line) - None for full file
                    let content = file_tools.read(std::path::Path::new(path), None, None)
                        .map_err(|e| format!("File read error: {}", e))?;
                    Ok(serde_json::json!({"content": content}))
                }
                "write" => {
                    let content = input.get("content")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'content' for write operation")?;
                    file_tools.write(std::path::Path::new(path), content)
                        .map_err(|e| format!("File write error: {}", e))?;
                    Ok(serde_json::json!({"success": true, "path": path}))
                }
                "list" => {
                    let show_hidden = input.get("show_hidden")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let entries = file_tools.list(std::path::Path::new(path), show_hidden)
                        .map_err(|e| format!("File list error: {}", e))?;
                    Ok(serde_json::json!({"entries": entries}))
                }
                _ => Err(format!("Unknown file operation: {}", operation))
            }
        }
        _ => Err(format!("Unknown tool: {}", tool_name))
    }
}

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

        orchestrator.spawn_subagent("test-1", "Test Agent").await.unwrap();

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

