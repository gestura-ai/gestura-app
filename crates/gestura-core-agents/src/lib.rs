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

/// Specialist role assigned to a managed agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Supervisor/lead coordinator for a team run.
    Supervisor,
    /// Research-focused subagent.
    Researcher,
    /// Default implementation-oriented subagent.
    #[default]
    Implementer,
    /// Review and critique specialist.
    Reviewer,
    /// Test authoring and validation specialist.
    Tester,
    /// Security-focused reviewer.
    SecurityReviewer,
    /// Remote execution worker.
    RemoteWorker,
    /// Custom role label.
    Custom(String),
}

impl AgentRole {
    /// Human-readable label for the role.
    pub fn label(&self) -> String {
        match self {
            Self::Supervisor => "Supervisor".to_string(),
            Self::Researcher => "Researcher".to_string(),
            Self::Implementer => "Implementer".to_string(),
            Self::Reviewer => "Reviewer".to_string(),
            Self::Tester => "Tester".to_string(),
            Self::SecurityReviewer => "Security Reviewer".to_string(),
            Self::RemoteWorker => "Remote Worker".to_string(),
            Self::Custom(value) => value.clone(),
        }
    }

    /// Default capability tags advertised for this role.
    pub fn default_capabilities(&self) -> Vec<String> {
        match self {
            Self::Supervisor => vec!["planning", "delegation", "synthesis"],
            Self::Researcher => vec!["research", "analysis", "summarization"],
            Self::Implementer => vec!["implementation", "editing", "refactoring"],
            Self::Reviewer => vec!["review", "critique", "quality"],
            Self::Tester => vec!["testing", "validation", "regression"],
            Self::SecurityReviewer => vec!["security", "threat-modeling", "review"],
            Self::RemoteWorker => vec!["remote_execution", "handoff", "artifacts"],
            Self::Custom(_) => vec!["custom"],
        }
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// Prompt preamble that reinforces specialist behavior.
    pub fn prompt_preamble(&self) -> &'static str {
        match self {
            Self::Supervisor => {
                "You are the team supervisor. Plan carefully, coordinate subtasks, and synthesize outcomes for the user."
            }
            Self::Researcher => {
                "You are the research specialist. Focus on collecting evidence, clarifying unknowns, and producing concise findings."
            }
            Self::Implementer => {
                "You are the implementation specialist. Make precise code changes, respect existing patterns, and explain what changed."
            }
            Self::Reviewer => {
                "You are the reviewer specialist. Critique plans and changes, identify risks, and recommend concrete fixes."
            }
            Self::Tester => {
                "You are the testing specialist. Design coverage, verify behavior, and surface regressions clearly."
            }
            Self::SecurityReviewer => {
                "You are the security reviewer. Prioritize threat modeling, permissions, trust boundaries, and misuse risks."
            }
            Self::RemoteWorker => {
                "You are a remote worker. Operate with explicit contracts, produce durable artifacts, and report provenance."
            }
            Self::Custom(_) => {
                "You are a specialist subagent. Operate within the delegated role, constraints, and deliverables."
            }
        }
    }
}

/// Execution mode used by a subagent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionMode {
    /// Shared workspace with read/write access controlled elsewhere.
    #[default]
    SharedWorkspace,
    /// Dedicated isolated workspace path.
    IsolatedWorkspace,
    /// Git worktree-backed isolated execution.
    GitWorktree,
    /// Remote execution target.
    Remote,
}

/// Structured brief attached to delegated work.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DelegationBrief {
    /// High-level objective for the delegated work.
    pub objective: String,
    /// Acceptance criteria for completion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,
    /// Constraints the subagent must respect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    /// Expected deliverables/artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliverables: Vec<String>,
    /// Condensed context summary for the child agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_summary: Option<String>,
}

impl DelegationBrief {
    /// Render the brief into prompt text.
    pub fn as_prompt_section(&self) -> String {
        let acceptance = if self.acceptance_criteria.is_empty() {
            "- No explicit acceptance criteria provided".to_string()
        } else {
            self.acceptance_criteria
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let constraints = if self.constraints.is_empty() {
            "- No additional constraints provided".to_string()
        } else {
            self.constraints
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let deliverables = if self.deliverables.is_empty() {
            "- Report results in plain text".to_string()
        } else {
            self.deliverables
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            "Objective:\n{}\n\nAcceptance Criteria:\n{}\n\nConstraints:\n{}\n\nDeliverables:\n{}{}",
            self.objective,
            acceptance,
            constraints,
            deliverables,
            self.context_summary
                .as_ref()
                .map(|summary| format!("\n\nContext Summary:\n{summary}"))
                .unwrap_or_default()
        )
    }
}

/// Remote target details for delegated work that may execute via A2A.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteAgentTarget {
    /// Remote agent base URL.
    pub url: String,
    /// Optional remote agent display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Capability tags requested from the remote agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// Spawn configuration for a managed agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnRequest {
    /// Unique agent identifier.
    pub id: String,
    /// Human-readable agent name.
    pub name: String,
    /// Specialist role.
    #[serde(default)]
    pub role: AgentRole,
    /// Workspace path assigned to the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Execution mode for the agent.
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    /// Capability tags this agent should advertise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl AgentSpawnRequest {
    /// Create a new spawn request using role defaults.
    pub fn new(id: impl Into<String>, name: impl Into<String>, role: AgentRole) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            capabilities: role.default_capabilities(),
            role,
            workspace_dir: None,
            execution_mode: AgentExecutionMode::SharedWorkspace,
        }
    }
}

/// Artifact produced by delegated task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArtifactRecord {
    /// Artifact display name.
    pub name: String,
    /// Artifact kind/category.
    pub kind: String,
    /// Optional URI/path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

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
    /// Specialist role assigned to the agent.
    #[serde(default)]
    pub role: AgentRole,
    /// Capability tags advertised by the agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Optional workspace path bound to the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Execution mode for the agent.
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
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
    /// Supervisor run identifier.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Parent delegated task identifier for hierarchical delegation.
    #[serde(default)]
    pub parent_task_id: Option<String>,
    /// Dependencies that must complete before execution can start.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Specialist role requested for the assignee.
    #[serde(default)]
    pub role: Option<AgentRole>,
    /// Structured brief generated by the supervisor.
    #[serde(default)]
    pub delegation_brief: Option<DelegationBrief>,
    /// Whether the task should stop after planning.
    #[serde(default)]
    pub planning_only: bool,
    /// Whether execution requires explicit approval before running.
    #[serde(default)]
    pub approval_required: bool,
    /// Whether review must occur before the task is considered complete.
    #[serde(default)]
    pub reviewer_required: bool,
    /// Whether test validation must occur before the task is considered complete.
    #[serde(default)]
    pub test_required: bool,
    /// Workspace root for sandboxing and durable memory persistence.
    #[serde(default)]
    pub workspace_dir: Option<PathBuf>,
    /// Assigned execution mode.
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    /// Optional environment identifier managed by the supervisor.
    #[serde(default)]
    pub environment_id: Option<String>,
    /// Optional remote execution target.
    #[serde(default)]
    pub remote_target: Option<RemoteAgentTarget>,
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
    /// Run identifier for grouped orchestration.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Session task tracking identifier.
    #[serde(default)]
    pub tracking_task_id: Option<String>,
    /// Output or error message
    pub output: String,
    /// Optional concise summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Tool calls made during execution
    pub tool_calls: Vec<OrchestratorToolCall>,
    /// Produced artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<TaskArtifactRecord>,
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
    /// Spawn an agent with an explicit request payload.
    async fn spawn_agent_with_request(&self, request: AgentSpawnRequest) {
        self.spawn_agent(request.id, request.name).await;
    }
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
    role: AgentRole,
    capabilities: Vec<String>,
    workspace_dir: Option<PathBuf>,
    execution_mode: AgentExecutionMode,
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
        self.spawn_agent_with_request(AgentSpawnRequest::new(id, name, AgentRole::Implementer))
            .await;
    }

    /// Spawn a lightweight agent using an explicit configuration request.
    pub async fn spawn_agent_with_request(&self, request: AgentSpawnRequest) {
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
            name: request.name,
            tx,
            _handle: handle,
            role: request.role,
            capabilities: request.capabilities,
            workspace_dir: request.workspace_dir,
            execution_mode: request.execution_mode,
            created_at: now,
            last_activity: now,
        };
        let mut inner = self.inner.lock().await;
        inner.agents.insert(request.id, rec);
    }

    /// Get status information for a specific agent
    pub async fn get_agent_status(&self, id: &str) -> Option<AgentInfo> {
        let inner = self.inner.lock().await;
        inner.agents.get(id).map(|rec| AgentInfo {
            id: id.to_string(),
            name: rec.name.clone(),
            status: "running".to_string(),
            last_activity: rec.last_activity,
            role: rec.role.clone(),
            capabilities: rec.capabilities.clone(),
            workspace_dir: rec.workspace_dir.clone(),
            execution_mode: rec.execution_mode.clone(),
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
                role: rec.role.clone(),
                capabilities: rec.capabilities.clone(),
                workspace_dir: rec.workspace_dir.clone(),
                execution_mode: rec.execution_mode.clone(),
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

    async fn spawn_agent_with_request(&self, request: AgentSpawnRequest) {
        AgentManager::spawn_agent_with_request(self, request).await;
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
        assert_eq!(agents[0].role, AgentRole::Implementer);
    }

    #[tokio::test]
    async fn test_spawn_with_role_and_workspace() {
        let manager = AgentManager::new(PathBuf::from("/tmp/test.db"));
        manager
            .spawn_agent_with_request(AgentSpawnRequest {
                id: "reviewer-1".into(),
                name: "Reviewer".into(),
                role: AgentRole::Reviewer,
                workspace_dir: Some(PathBuf::from("/tmp/worktree/reviewer-1")),
                execution_mode: AgentExecutionMode::GitWorktree,
                capabilities: vec!["review".into(), "quality".into()],
            })
            .await;

        let status = manager.get_agent_status("reviewer-1").await.unwrap();
        assert_eq!(status.role, AgentRole::Reviewer);
        assert_eq!(status.execution_mode, AgentExecutionMode::GitWorktree);
        assert_eq!(
            status.workspace_dir,
            Some(PathBuf::from("/tmp/worktree/reviewer-1"))
        );
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
            run_id: Some("run-1".into()),
            parent_task_id: Some("task-parent".into()),
            depends_on: vec!["task-prep".into()],
            role: Some(AgentRole::Reviewer),
            delegation_brief: Some(DelegationBrief {
                objective: "Review the patch".into(),
                acceptance_criteria: vec!["List risks".into()],
                constraints: vec!["Do not modify files".into()],
                deliverables: vec!["Risk summary".into()],
                context_summary: Some("User requested a review".into()),
            }),
            planning_only: true,
            approval_required: true,
            reviewer_required: true,
            test_required: false,
            workspace_dir: Some(PathBuf::from("/tmp/workspace")),
            execution_mode: AgentExecutionMode::GitWorktree,
            environment_id: Some("env-1".into()),
            remote_target: Some(RemoteAgentTarget {
                url: "https://remote.example".into(),
                name: Some("Remote Reviewer".into()),
                capabilities: vec!["review".into()],
            }),
            memory_tags: vec!["memory".into(), "delegation".into()],
            name: Some("Test Task".into()),
        };

        let json = serde_json::to_string(&task).unwrap();
        let parsed: DelegatedTask = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "task-1");
        assert_eq!(parsed.session_id, Some("session-123".into()));
        assert_eq!(parsed.directive_id, Some("directive-1".into()));
        assert_eq!(parsed.tracking_task_id, Some("task-track-1".into()));
        assert_eq!(parsed.run_id, Some("run-1".into()));
        assert_eq!(parsed.role, Some(AgentRole::Reviewer));
        assert_eq!(parsed.memory_tags, vec!["memory", "delegation"]);
    }

    #[test]
    fn test_task_result_serialization() {
        let result = TaskResult {
            task_id: "task-1".into(),
            agent_id: "agent-1".into(),
            success: true,
            run_id: Some("run-1".into()),
            tracking_task_id: Some("tracking-1".into()),
            output: "Done".into(),
            summary: Some("Task succeeded".into()),
            tool_calls: vec![],
            artifacts: vec![TaskArtifactRecord {
                name: "summary.md".into(),
                kind: "report".into(),
                uri: Some("memory://summary".into()),
                summary: Some("Delegation summary".into()),
            }],
            duration_ms: 100,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: TaskResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.task_id, "task-1");
        assert!(parsed.success);
    }
}
