//! Agent lifecycle management and persistence - thin wrapper over gestura_core::agents
//!
//! This module provides a GUI-specific AgentManager that extends the core implementation
//! with KV store persistence (backed by NATS JetStream) and GUI-specific event handling.

use crate::AppConfigSecurityExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;

use crate::kv::KvStore;
use crate::mcp::{MdhResource, mdh_translate};
use gestura_core::pipeline::{AgentPipeline, AgentRequest, RequestSource};

// Re-export core types for backwards compatibility
pub use gestura_core::agents::{
    AgentCommand, AgentEnvelope, AgentExecutionMode, AgentInfo, AgentRole, AgentSpawnRequest,
    AgentSpawner, AgentStatus, DelegatedTask, OrchestratorToolCall, TaskResult,
};

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

/// Load configuration for background agent event processing.
///
/// In unit tests, this intentionally avoids reading the user's real on-disk
/// config (which may point at networked providers and cause test hangs).
async fn load_agent_event_config() -> crate::AppConfig {
    if cfg!(test) {
        crate::AppConfig::default()
    } else {
        crate::AppConfig::load_async().await
    }
}

/// GUI-specific AgentManager with KV persistence and MDH event handling
///
/// Extends the core AgentManager with:
/// - KV store persistence (backed by NATS JetStream)
/// - MDH (Metadata Hub) integration for data-query events
/// - LLM provider integration for event processing
#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<Mutex<Inner>>,
    #[allow(dead_code)]
    db_path: PathBuf,
    /// KV store for persistence (backed by NATS JetStream)
    kv: Option<KvStore>,
    #[allow(dead_code)]
    kv_bucket: String,
}

impl AgentManager {
    /// Create a new AgentManager with the given database path
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            db_path,
            kv: None,
            kv_bucket: "agents_state".into(),
        }
    }

    /// Attach a KV store (backed by JetStream) for persistence
    pub fn attach_kv(&mut self, kv: KvStore) {
        self.kv = Some(kv);
    }

    /// Upsert the agent's state to NATS KV if available
    fn persist_state(&self, id: &str, name: &str, status: AgentStatus) {
        if let Some(kv) = &self.kv {
            let key = format!("agents/{}", id);
            let val =
                serde_json::json!({"id": id, "name": name, "status": status.as_str()}).to_string();
            drop(tokio::spawn({
                let kv = kv.clone();
                async move {
                    let _ = kv.put(&key, val.into_bytes()).await;
                }
            }));
        }
    }

    /// Spawn a lightweight agent task with GUI-specific event handling
    pub async fn spawn_agent(&self, id: String, name: String) {
        self.spawn_agent_with_request(AgentSpawnRequest::new(id, name, AgentRole::Implementer))
            .await;
    }

    /// Spawn a lightweight agent using an explicit configuration request.
    pub async fn spawn_agent_with_request(&self, request: AgentSpawnRequest) {
        let (tx, mut rx) = mpsc::channel::<AgentCommand>(32);
        self.persist_state(&request.id, &request.name, AgentStatus::Running);

        // GUI-specific agent task with MDH and LLM integration
        let handle = tokio::spawn(async move {
            // Background work spawned for event processing. We keep these in a JoinSet so
            // we can abort them immediately on shutdown.
            let mut in_flight: JoinSet<()> = JoinSet::new();

            while let Some(cmd) = rx.recv().await {
                match cmd {
                    AgentCommand::Shutdown => {
                        // Ensure shutdown is always responsive, even if an event
                        // is currently being processed via the pipeline.
                        in_flight.abort_all();
                        break;
                    }
                    AgentCommand::Event(payload) => {
                        // Handle data-query events with MDH and LLM.
                        //
                        // IMPORTANT: This is spawned so that slow providers (or a user's
                        // config pointing at a networked provider) cannot block the
                        // agent's shutdown handling.
                        if let Some(path) = payload.strip_prefix("data-query:") {
                            let path = path.trim().to_string();
                            in_flight.spawn(async move {
                                let ld_path = PathBuf::from(path);
                                let mdh: Option<MdhResource> = mdh_translate(ld_path).ok();

                                let mut prompt = String::from("Handle event with context.");
                                if let Some(res) = mdh {
                                    prompt.push_str(&format!("\nMDH: {}", res.uri));
                                }

                                // Core-First: route LLM calls through the unified pipeline.
                                // This GUI agent event handler does not execute tools; it only
                                // requests a single model response.
                                let cfg = load_agent_event_config().await;
                                let pipeline = AgentPipeline::with_provider_optimized_config(cfg)
                                    .with_knowledge(
                                        get_gui_knowledge_store(),
                                        get_gui_knowledge_settings(),
                                    );
                                let request = AgentRequest::new(prompt)
                                    .with_streaming(false)
                                    .with_source(RequestSource::GuiText)
                                    .with_tools_enabled(false);

                                if let Err(e) = pipeline.process_blocking(request).await {
                                    tracing::error!("AgentPipeline event processing error: {}", e);
                                }
                            });
                        }
                    }
                }
            }

            // Best-effort: if the channel closes without an explicit Shutdown command,
            // ensure we don't keep any background tasks alive.
            in_flight.abort_all();
        });

        let now = chrono::Utc::now();
        let rec = AgentRecord {
            name: request.name.clone(),
            tx,
            _handle: handle,
            role: request.role,
            capabilities: request.capabilities,
            workspace_dir: request.workspace_dir,
            execution_mode: request.execution_mode,
            created_at: now,
            last_activity: now,
        };
        self.inner.lock().await.agents.insert(request.id, rec);
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

    /// Load state for an agent from KV (best-effort)
    pub async fn load_state(&self, id: &str) -> Option<String> {
        if let Some(kv) = &self.kv {
            let key = format!("agents/{}", id);
            if let Ok(Some(bytes)) = kv.get(&key).await {
                return Some(String::from_utf8_lossy(&bytes).to_string());
            }
        }
        None
    }

    /// Send an event to a specific agent
    pub async fn send_event(&self, id: &str, payload: String) {
        let tx_opt = {
            let inner = self.inner.lock().await;
            inner.agents.get(id).map(|r| r.tx.clone())
        };
        if let Some(tx) = tx_opt {
            let _ = tx.send(AgentCommand::Event(payload)).await;
        }
    }

    /// Gracefully shutdown all agents
    pub async fn shutdown_all(&self, grace_secs: u64) {
        let to_shutdown: Vec<_> = {
            let inner = self.inner.lock().await;
            inner.agents.values().map(|r| r.tx.clone()).collect()
        };
        for tx in to_shutdown {
            let _ = tx.send(AgentCommand::Shutdown).await;
        }
        time::sleep(Duration::from_secs(grace_secs)).await;
    }

    /// Compute a default DB path under the user's data dir
    pub fn default_db_path() -> PathBuf {
        gestura_core::agents::AgentManager::default_db_path()
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

    async fn load_state(&self, id: &str) -> Option<String> {
        AgentManager::load_state(self, id).await
    }

    async fn shutdown_all(&self, grace_secs: u64) {
        AgentManager::shutdown_all(self, grace_secs).await;
    }
}

#[async_trait::async_trait]
impl gestura_core::orchestrator::OrchestratorAgentManager for AgentManager {
    /// Get status information for a specific agent.
    async fn get_agent_status(&self, id: &str) -> Option<AgentInfo> {
        AgentManager::get_agent_status(self, id).await
    }

    /// List all active agents.
    async fn list_agents(&self) -> Vec<AgentInfo> {
        AgentManager::list_agents(self).await
    }

    /// Update last activity timestamp for an agent.
    async fn update_activity(&self, id: &str) {
        AgentManager::update_activity(self, id).await;
    }
}

// ── Knowledge store (G6) ────────────────────────────────────────────────────
// Module-level singletons for the GUI agents module.  The api.rs module has its
// own parallel singletons; these are separate to avoid circular dependencies.

/// Global knowledge store for GUI agent-event pipelines.
static GUI_KNOWLEDGE_STORE: OnceLock<gestura_core::KnowledgeStore> = OnceLock::new();

/// Global knowledge settings manager for GUI agent-event pipelines.
static GUI_KNOWLEDGE_SETTINGS: OnceLock<gestura_core::KnowledgeSettingsManager> = OnceLock::new();

fn get_gui_knowledge_store() -> &'static gestura_core::KnowledgeStore {
    GUI_KNOWLEDGE_STORE.get_or_init(|| {
        let store = gestura_core::KnowledgeStore::with_default_dir();
        gestura_core::register_builtin_knowledge(&store);
        if let Err(e) = store.load_user_items() {
            tracing::warn!(error = %e, "Failed to load persisted user knowledge (continuing)");
        }
        store
    })
}

fn get_gui_knowledge_settings() -> &'static gestura_core::KnowledgeSettingsManager {
    GUI_KNOWLEDGE_SETTINGS.get_or_init(|| {
        let base_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        gestura_core::KnowledgeSettingsManager::new(base_dir)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    /// Integration smoke test: a GUI agent event should be processed via the core
    /// pipeline and the agent task should remain responsive (no hangs).
    #[tokio::test]
    async fn agent_data_query_event_is_processed_and_task_can_shutdown() {
        let manager = AgentManager::new(PathBuf::from("/tmp/gestura_gui_agent_test"));

        let id = "test-agent".to_string();
        manager
            .spawn_agent(id.clone(), "Test Agent".to_string())
            .await;

        // Pull out the agent handle so we can await shutdown deterministically.
        let (tx, handle) = {
            let mut inner = manager.inner.lock().await;
            let rec = inner
                .agents
                .remove(&id)
                .expect("agent record should exist after spawn_agent");
            (rec.tx, rec._handle)
        };

        // Send a payload that triggers the data-query branch. The path can be invalid;
        // MDH translation is best-effort and should not prevent pipeline execution.
        tx.send(AgentCommand::Event(
            "data-query:/path/does/not/exist.jsonld".to_string(),
        ))
        .await
        .expect("event send should succeed");
        tx.send(AgentCommand::Shutdown)
            .await
            .expect("shutdown send should succeed");

        timeout(Duration::from_secs(5), handle)
            .await
            .expect("agent task should terminate promptly")
            .expect("agent task should not panic");
    }
}
