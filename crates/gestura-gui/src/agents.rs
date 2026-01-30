//! Agent lifecycle management and persistence - thin wrapper over gestura_core::agents
//!
//! This module provides a GUI-specific AgentManager that extends the core implementation
//! with KV store persistence (backed by NATS JetStream) and GUI-specific event handling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time;

use crate::kv::KvStore;
use crate::llm_provider::{AgentContext, select_provider};
use crate::mcp::{MdhResource, mdh_translate};

// Re-export core types for backwards compatibility
pub use gestura_core::agents::{
    AgentCommand, AgentEnvelope, AgentInfo, AgentSpawner, AgentStatus, DelegatedTask,
    OrchestratorToolCall, TaskResult,
};

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
        let (tx, mut rx) = mpsc::channel::<AgentCommand>(32);
        self.persist_state(&id, &name, AgentStatus::Running);

        // GUI-specific agent task with MDH and LLM integration
        let handle = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    AgentCommand::Shutdown => break,
                    AgentCommand::Event(_payload) => {
                        // Handle data-query events with MDH and LLM
                        if let Some(path) = _payload.strip_prefix("data-query:") {
                            let ld_path = PathBuf::from(path.trim());
                            let mdh: Option<MdhResource> = mdh_translate(ld_path).ok();
                            let mut prompt = String::from("Handle event with context.");
                            if let Some(res) = mdh {
                                prompt.push_str(&format!("\nMDH: {}", res.uri));
                            }
                            let provider = select_provider(
                                &crate::AppConfig::load(),
                                &AgentContext {
                                    agent_id: String::from("default"),
                                },
                            );
                            let _ = provider.call(&prompt).await;
                        }
                    }
                }
            }
        });

        let now = chrono::Utc::now();
        let rec = AgentRecord {
            name: name.clone(),
            tx,
            _handle: handle,
            created_at: now,
            last_activity: now,
        };
        self.inner.lock().await.agents.insert(id, rec);
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
