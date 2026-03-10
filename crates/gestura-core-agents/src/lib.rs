//! Agent lifecycle management and orchestration
//!
//! Provides core types for agent spawning, task delegation, and result tracking.
//! GUI-specific orchestration (with Tauri AppHandle) remains in the GUI crate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

/// IPC envelope for events exchanged with agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEnvelope {
    /// Agent identifier
    pub agent_id: String,
    /// Subject/topic of the event
    pub subject: String,
    /// JSON payload
    pub payload: serde_json::Value,
}

/// Commands that can be sent to an agent task
#[derive(Debug, Clone)]
pub enum AgentCommand {
    /// Instruct the agent to shutdown
    Shutdown,
    /// Deliver a generic event from MQ or system
    Event(String),
}

/// Status value for an agent
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    /// Agent is running
    Running,
    /// Agent has stopped
    Stopped,
}

impl AgentStatus {
    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Stopped => "stopped",
        }
    }
}

/// Public agent info for status queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Unique agent identifier
    pub id: String,
    /// Human-readable agent name
    pub name: String,
    /// Current status string
    pub status: String,
    /// Last activity timestamp
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

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
    /// Session ID for UI integration
    #[serde(default)]
    pub session_id: Option<String>,
    /// Shared directive identifier for cross-agent coordination.
    #[serde(default)]
    pub directive_id: Option<String>,
    /// Optional task identifier in the session task list used for lifecycle tracking.
    #[serde(default)]
    pub tracking_task_id: Option<String>,
    /// Workspace root for sandboxing and durable memory persistence.
    #[serde(default)]
    pub workspace_dir: Option<PathBuf>,
    /// Tags used for targeted memory retrieval and promotion.
    #[serde(default)]
    pub memory_tags: Vec<String>,
    /// Human-readable task name for UI display
    #[serde(default)]
    pub name: Option<String>,
}

/// Result from a delegated task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Task identifier
    pub task_id: String,
    /// Agent that executed the task
    pub agent_id: String,
    /// Whether task completed successfully
    pub success: bool,
    /// Output or error message
    pub output: String,
    /// Tool calls made during execution
    pub tool_calls: Vec<OrchestratorToolCall>,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

/// Record of a tool call during orchestrated task execution
///
/// This is separate from `ToolCallRecord` in the pipeline module, which tracks
/// raw tool calls. This struct is for orchestrator-level task tracking with
/// structured input/output JSON values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorToolCall {
    /// Tool name
    pub tool_name: String,
    /// Input parameters as JSON
    pub input: serde_json::Value,
    /// Output value as JSON
    pub output: serde_json::Value,
    /// Whether the call succeeded
    pub success: bool,
    /// Call duration in milliseconds
    pub duration_ms: u64,
}

/// Trait for spawning and managing isolated agents
#[async_trait::async_trait]
pub trait AgentSpawner: Send + Sync {
    /// Spawn an agent and return its id
    async fn spawn_agent(&self, id: String, name: String);
    /// Send an event envelope to a running agent
    async fn send_event(&self, id: &str, payload: String);
    /// Attempt to restore state for an agent
    async fn load_state(&self, id: &str) -> Option<String>;
    /// Shutdown all agents with a grace period
    async fn shutdown_all(&self, grace_secs: u64);
}

/// Record kept for each agent in memory
struct AgentRecord {
    name: String,
    tx: mpsc::Sender<AgentCommand>,
    _handle: JoinHandle<()>,
    #[allow(dead_code)]
    created_at: chrono::DateTime<chrono::Utc>,
    last_activity: chrono::DateTime<chrono::Utc>,
}

#[derive(Default)]
struct Inner {
    agents: HashMap<String, AgentRecord>,
}

/// Core agent manager implementation
///
/// Manages agent lifecycles without GUI dependencies.
/// GUI/Tauri-specific features are in the GUI crate's wrapper.
#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<Mutex<Inner>>,
    #[allow(dead_code)]
    db_path: PathBuf,
}

impl AgentManager {
    /// Create a new AgentManager with the given database path
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            db_path,
        }
    }

    /// Spawn a lightweight agent task that listens for commands
    ///
    /// This is a basic implementation. GUI can override spawn behavior
    /// by wrapping this manager.
    pub async fn spawn_agent(&self, id: String, name: String) {
        let (tx, mut rx) = mpsc::channel::<AgentCommand>(32);

        // Basic agent task body
        let handle = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    AgentCommand::Shutdown => break,
                    AgentCommand::Event(_payload) => {
                        // Basic event handling - GUI can override with richer behavior
                        tracing::debug!(payload = %_payload, "Agent received event");
                    }
                }
            }
        });

        let now = chrono::Utc::now();
        let rec = AgentRecord {
            name,
            tx,
            _handle: handle,
            created_at: now,
            last_activity: now,
        };
        let mut inner = self.inner.lock().await;
        inner.agents.insert(id, rec);
    }

    /// Get status information for a specific agent
    pub async fn get_agent_status(&self, id: &str) -> Option<AgentInfo> {
        let inner = self.inner.lock().await;
        inner.agents.get(id).map(|rec| AgentInfo {
            id: id.to_string(),
            name: rec.name.clone(),
            status: "running".to_string(),
            last_activity: rec.last_activity,
        })
    }

    /// List all active agents
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        let inner = self.inner.lock().await;
        inner
            .agents
            .iter()
            .map(|(id, rec)| AgentInfo {
                id: id.clone(),
                name: rec.name.clone(),
                status: "running".to_string(),
                last_activity: rec.last_activity,
            })
            .collect()
    }

    /// Update last activity timestamp for an agent
    pub async fn update_activity(&self, id: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(rec) = inner.agents.get_mut(id) {
            rec.last_activity = chrono::Utc::now();
        }
    }

    /// Publish an event to a specific agent if present
    pub async fn send_event(&self, id: &str, payload: String) {
        let tx_opt = {
            let inner = self.inner.lock().await;
            inner.agents.get(id).map(|r| r.tx.clone())
        };
        if let Some(tx) = tx_opt {
            let _ = tx.send(AgentCommand::Event(payload)).await;
        }
    }

    /// Gracefully shutdown all agents, waiting up to `grace_secs` for completion
    pub async fn shutdown_all(&self, grace_secs: u64) {
        let mut to_shutdown: Vec<mpsc::Sender<AgentCommand>> = Vec::new();
        {
            let inner = self.inner.lock().await;
            for (_id, rec) in inner.agents.iter() {
                to_shutdown.push(rec.tx.clone());
            }
        }
        for tx in to_shutdown {
            let _ = tx.send(AgentCommand::Shutdown).await;
        }
        tokio::time::sleep(Duration::from_secs(grace_secs)).await;
    }

    /// Compute a default DB path under the user's data dir
    pub fn default_db_path() -> PathBuf {
        let mut dir = dirs::data_dir().unwrap_or_default();
        dir.push("Gestura");
        std::fs::create_dir_all(&dir).ok();
        dir.push("gestura.db");
        dir
    }
}

#[async_trait::async_trait]
impl AgentSpawner for AgentManager {
    async fn spawn_agent(&self, id: String, name: String) {
        AgentManager::spawn_agent(self, id, name).await;
    }

    async fn send_event(&self, id: &str, payload: String) {
        AgentManager::send_event(self, id, payload).await;
    }

    async fn load_state(&self, _id: &str) -> Option<String> {
        // Core manager doesn't have KV store - GUI wrapper provides this
        None
    }

    async fn shutdown_all(&self, grace_secs: u64) {
        AgentManager::shutdown_all(self, grace_secs).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_manager_new() {
        let manager = AgentManager::new(PathBuf::from("/tmp/test.db"));
        assert!(manager.list_agents().await.is_empty());
    }

    #[tokio::test]
    async fn test_spawn_and_list_agents() {
        let manager = AgentManager::new(PathBuf::from("/tmp/test.db"));
        manager
            .spawn_agent("agent-1".into(), "Test Agent".into())
            .await;

        let agents = manager.list_agents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "agent-1");
        assert_eq!(agents[0].name, "Test Agent");
    }

    #[tokio::test]
    async fn test_get_agent_status() {
        let manager = AgentManager::new(PathBuf::from("/tmp/test.db"));
        manager
            .spawn_agent("agent-1".into(), "Test Agent".into())
            .await;

        let status = manager.get_agent_status("agent-1").await;
        assert!(status.is_some());
        assert_eq!(status.unwrap().status, "running");

        let missing = manager.get_agent_status("nonexistent").await;
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_send_event() {
        let manager = AgentManager::new(PathBuf::from("/tmp/test.db"));
        manager
            .spawn_agent("agent-1".into(), "Test Agent".into())
            .await;

        // Should not panic
        manager.send_event("agent-1", "test-event".into()).await;
        manager.send_event("nonexistent", "test-event".into()).await;
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
            session_id: Some("session-123".into()),
            directive_id: Some("directive-1".into()),
            tracking_task_id: Some("task-track-1".into()),
            workspace_dir: Some(PathBuf::from("/tmp/workspace")),
            memory_tags: vec!["memory".into(), "delegation".into()],
            name: Some("Test Task".into()),
        };

        let json = serde_json::to_string(&task).unwrap();
        let parsed: DelegatedTask = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "task-1");
        assert_eq!(parsed.session_id, Some("session-123".into()));
        assert_eq!(parsed.directive_id, Some("directive-1".into()));
        assert_eq!(parsed.tracking_task_id, Some("task-track-1".into()));
        assert_eq!(parsed.memory_tags, vec!["memory", "delegation"]);
    }

    #[test]
    fn test_task_result_serialization() {
        let result = TaskResult {
            task_id: "task-1".into(),
            agent_id: "agent-1".into(),
            success: true,
            output: "Done".into(),
            tool_calls: vec![],
            duration_ms: 100,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: TaskResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.task_id, "task-1");
        assert!(parsed.success);
    }
}
