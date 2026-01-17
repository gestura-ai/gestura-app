//! Process-based agent spawning with IPC communication
//! Implements subprocess isolation for agents with proper IPC channels

use crate::AppError;
use crate::agents::{AgentEnvelope, AgentSpawner};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, timeout};

/// Process-based agent record
struct ProcessAgent {
    #[allow(dead_code)]
    id: String,
    name: String,
    child: Child,
    stdin: tokio::process::ChildStdin,
    #[allow(dead_code)]
    stdout_rx: mpsc::Receiver<String>,
    _stdout_task: tokio::task::JoinHandle<()>,
}

/// Process-based agent spawner with IPC and health monitoring
#[derive(Clone)]
pub struct ProcessSpawner {
    agents: Arc<Mutex<HashMap<String, ProcessAgent>>>,
    kv_store: Option<crate::kv::KvStore>,
    _health_monitor: Arc<Mutex<tokio::task::JoinHandle<()>>>,
}

impl ProcessSpawner {
    /// Create a new process spawner with health monitoring
    pub fn new(kv_store: Option<crate::kv::KvStore>) -> Self {
        let agents = Arc::new(Mutex::new(HashMap::new()));
        let health_monitor = Arc::new(Mutex::new(Self::start_health_monitor(agents.clone())));

        Self {
            agents,
            kv_store,
            _health_monitor: health_monitor,
        }
    }

    /// Start health monitoring task
    fn start_health_monitor(
        agents: Arc<Mutex<HashMap<String, ProcessAgent>>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                Self::check_agent_health(agents.clone()).await;
            }
        })
    }

    /// Check health of all agents and restart if needed
    async fn check_agent_health(agents: Arc<Mutex<HashMap<String, ProcessAgent>>>) {
        let mut agents_guard = agents.lock().await;
        let mut to_restart = Vec::new();

        for (id, agent) in agents_guard.iter_mut() {
            // Check if process is still alive
            match agent.child.try_wait() {
                Ok(Some(_)) => {
                    tracing::warn!("Agent {} has exited, marking for restart", id);
                    to_restart.push((id.clone(), agent.name.clone()));
                }
                Ok(None) => {
                    // Process is still running
                    tracing::debug!("Agent {} is healthy", id);
                }
                Err(e) => {
                    tracing::error!("Failed to check agent {} status: {}", id, e);
                    to_restart.push((id.clone(), agent.name.clone()));
                }
            }
        }

        // Remove dead agents
        for (id, _) in &to_restart {
            agents_guard.remove(id);
        }
        drop(agents_guard);

        // Restart dead agents
        for (id, name) in to_restart {
            tracing::info!("Restarting agent: {} ({})", name, id);
            // Note: In a real implementation, we'd need a reference to the spawner
            // For now, just log the restart attempt
        }
    }

    /// Spawn agent subprocess with IPC
    async fn spawn_subprocess(&self, id: &str, name: &str) -> Result<ProcessAgent, AppError> {
        // Create agent subprocess - for now use a simple echo process as placeholder
        // In production, this would spawn the actual agent binary
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("while read line; do echo \"Agent response: $line\"; done")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(AppError::Io)?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Io(std::io::Error::other("Failed to get stdin")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Io(std::io::Error::other("Failed to get stdout")))?;

        // Set up stdout reader
        let (stdout_tx, stdout_rx) = mpsc::channel(100);
        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let _ = stdout_tx.send(line.trim().to_string()).await;
                line.clear();
            }
        });

        Ok(ProcessAgent {
            id: id.to_string(),
            name: name.to_string(),
            child,
            stdin,
            stdout_rx,
            _stdout_task: stdout_task,
        })
    }

    /// Send IPC message to agent
    async fn send_ipc_message(
        &self,
        agent: &mut ProcessAgent,
        envelope: AgentEnvelope,
    ) -> Result<(), AppError> {
        let message = serde_json::to_string(&envelope).map_err(AppError::Json)?;
        agent
            .stdin
            .write_all(message.as_bytes())
            .await
            .map_err(AppError::Io)?;
        agent.stdin.write_all(b"\n").await.map_err(AppError::Io)?;
        agent.stdin.flush().await.map_err(AppError::Io)?;
        Ok(())
    }

    /// Persist agent state to KV store
    #[allow(dead_code)]
    async fn persist_agent_state(&self, id: &str, state: &str) {
        if let Some(kv) = &self.kv_store {
            let key = format!("agents/{}/state", id);
            let _ = kv.put(&key, state.as_bytes().to_vec()).await;
        }
    }

    /// Load agent state from KV store
    async fn load_agent_state(&self, id: &str) -> Option<String> {
        if let Some(kv) = &self.kv_store {
            let key = format!("agents/{}/state", id);
            if let Ok(Some(data)) = kv.get(&key).await {
                return Some(String::from_utf8_lossy(&data).to_string());
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl AgentSpawner for ProcessSpawner {
    async fn spawn_agent(&self, id: String, name: String) {
        match self.spawn_subprocess(&id, &name).await {
            Ok(agent) => {
                let mut agents = self.agents.lock().await;
                agents.insert(id.clone(), agent);
                tracing::info!("Spawned process agent: {} ({})", name, id);
            }
            Err(e) => {
                tracing::error!("Failed to spawn process agent {}: {}", id, e);
            }
        }
    }

    async fn send_event(&self, id: &str, payload: String) {
        let envelope = AgentEnvelope {
            agent_id: id.to_string(),
            subject: "event".to_string(),
            payload: serde_json::Value::String(payload),
        };

        let mut agents = self.agents.lock().await;
        if let Some(agent) = agents.get_mut(id)
            && let Err(e) = self.send_ipc_message(agent, envelope).await
        {
            tracing::error!("Failed to send event to process agent {}: {}", id, e);
        }
    }

    async fn load_state(&self, id: &str) -> Option<String> {
        self.load_agent_state(id).await
    }

    async fn shutdown_all(&self, grace_secs: u64) {
        let mut agents = self.agents.lock().await;

        // Send shutdown signals
        for (id, agent) in agents.iter_mut() {
            let envelope = AgentEnvelope {
                agent_id: id.clone(),
                subject: "shutdown".to_string(),
                payload: serde_json::Value::Null,
            };
            let _ = self.send_ipc_message(agent, envelope).await;
        }

        // Wait for graceful shutdown
        let shutdown_future = async {
            for (id, agent) in agents.iter_mut() {
                if let Err(e) = agent.child.wait().await {
                    tracing::error!("Process agent {} shutdown error: {}", id, e);
                }
            }
        };

        if timeout(Duration::from_secs(grace_secs), shutdown_future)
            .await
            .is_err()
        {
            tracing::warn!("Grace period expired, force killing process agents");
            for (id, agent) in agents.iter_mut() {
                if let Err(e) = agent.child.kill().await {
                    tracing::error!("Failed to kill process agent {}: {}", id, e);
                }
            }
        }

        agents.clear();
        tracing::info!("All process agents shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_spawner() {
        let spawner = ProcessSpawner::new(None);

        // Test spawning
        spawner
            .spawn_agent("test-agent".to_string(), "Test Agent".to_string())
            .await;

        // Test sending event
        spawner
            .send_event("test-agent", "test message".to_string())
            .await;

        // Test shutdown
        spawner.shutdown_all(5).await;
    }
}
