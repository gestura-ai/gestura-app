//! Agent lifecycle management and persistence (Stage 3)
//! AgentSpawner trait and IPC envelopes are provided here to allow isolated agent processes later.

//! Provides an AgentManager that spawns lightweight agent tasks, handles
//! graceful shutdown with a configurable grace period, and persists state
//! to a local SQLite database.

use std::sync::Arc;
use std::{collections::HashMap, path::PathBuf, time::Duration};

use crate::llm_provider::{AgentContext, select_provider};
use crate::mcp::{MdhResource, mdh_translate};

use crate::kv::KvStore;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time,
};

/// Commands that can be sent to an agent task.
/// IPC envelope for events exchanged with agents.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentEnvelope {
    pub agent_id: String,
    pub subject: String,
    pub payload: serde_json::Value,
}

/// Trait for spawning and managing isolated agents (process/task abstraction).
#[async_trait::async_trait]
pub trait AgentSpawner: Send + Sync {
    /// Spawn an agent and return its id.
    async fn spawn_agent(&self, id: String, name: String);
    /// Send an event envelope to a running agent.
    async fn send_event(&self, id: &str, payload: String);
    /// Attempt to restore state for an agent.
    async fn load_state(&self, id: &str) -> Option<String>;
    /// Shutdown all agents with a grace period.
    async fn shutdown_all(&self, grace_secs: u64);
}

#[async_trait::async_trait]
impl AgentSpawner for AgentManager {
    async fn spawn_agent(&self, id: String, name: String) {
        self.spawn_agent(id, name).await;
    }

    async fn send_event(&self, id: &str, payload: String) {
        self.send_event(id, payload).await;
    }

    async fn load_state(&self, id: &str) -> Option<String> {
        self.load_state(id).await
    }

    async fn shutdown_all(&self, grace_secs: u64) {
        self.shutdown_all(grace_secs).await;
    }
}

// JetStream integration will be added later when nats-rs API is stabilized in our stack.

pub enum AgentCommand {
    /// Instruct the agent to shutdown.
    Shutdown,
    /// Deliver a generic event from MQ or system.
    Event(String),
}

/// Status value persisted for an agent.
#[derive(Debug, Clone)]
pub enum AgentStatus {
    Running,
    Stopped,
}

impl AgentStatus {
    fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Stopped => "stopped",
        }
    }
}

/// Public agent info for status queries
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

/// Record kept for each agent in memory.
#[derive(Debug)]
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

/// Manages agent lifecycles and state persistence.
#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<Mutex<Inner>>,
    #[allow(dead_code)]
    db_path: PathBuf,
    // KV store for persistence (backed by NATS JetStream)
    kv: Option<KvStore>,
    #[allow(dead_code)]
    kv_bucket: String,
}

impl AgentManager {
    /// Create a new AgentManager with the given database path.
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            db_path,
            kv: None,
            kv_bucket: "agents_state".into(),
        }
    }

    /// Attach a KV store (backed by JetStream) for persistence.
    pub fn attach_kv(&mut self, kv: KvStore) {
        self.kv = Some(kv);
    }

    /// Returns a connection to the SQLite database.
    // Placeholder retained for compatibility; no file DB used when NATS enabled.
    #[allow(dead_code)]
    fn init_db(&self) -> Result<(), ()> {
        Ok(())
    }

    /// Upsert the agent's state to NATS KV if available.
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

    /// Spawn a lightweight agent task that listens for commands.
    /// Returns the agent id used in persistence.
    pub async fn spawn_agent(&self, id: String, name: String) {
        // Create command channel
        let (tx, mut rx) = mpsc::channel::<AgentCommand>(32);
        // Persist initial state
        self.persist_state(&id, &name, AgentStatus::Running);

        // Agent task body (mock)
        let handle = tokio::spawn(async move {
            // Example: handle commands until shutdown
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    AgentCommand::Shutdown => {
                        break;
                    }
                    AgentCommand::Event(_payload) => {
                        // Handle incoming event:
                        // If payload starts with "data-query:", treat remainder as a local JSON/JSON-LD path,
                        // run MDH translate, then call LLM with the URI appended.
                        if let Some(path) = _payload.strip_prefix("data-query:") {
                            let ld_path = std::path::PathBuf::from(path.trim());
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

    /// Publish an event to a specific agent if present.
    /// Attempt to load a prior state for an agent from KV (best-effort).
    pub async fn load_state(&self, id: &str) -> Option<String> {
        if let Some(kv) = &self.kv {
            let key = format!("agents/{}", id);
            if let Ok(Some(bytes)) = kv.get(&key).await {
                return Some(String::from_utf8_lossy(&bytes).to_string());
            }
        }
        None
    }

    pub async fn send_event(&self, id: &str, payload: String) {
        let tx_opt = {
            let inner = self.inner.lock().await;
            inner.agents.get(id).map(|r| r.tx.clone())
        };
        if let Some(tx) = tx_opt {
            let _ = tx.send(AgentCommand::Event(payload)).await;
        }
    }

    /// Gracefully shutdown all agents, waiting up to `grace_secs` for completion.
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
        time::sleep(Duration::from_secs(grace_secs)).await;
    }

    /// Compute a default DB path under the user's data dir.
    pub fn default_db_path() -> PathBuf {
        let mut dir = dirs::data_dir().unwrap_or_default();
        dir.push("Gestura");
        std::fs::create_dir_all(&dir).ok();
        dir.push("gestura.db");
        dir
    }
}
