//! Event dispatcher for routing NATS messages to appropriate handlers
//! Provides centralized event routing and processing

use crate::AppError;
use crate::agents::AgentSpawner;
use crate::nats_mq::{DispatchEvent, subjects};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Event dispatcher that routes NATS messages to appropriate handlers
#[derive(Clone)]
pub struct EventDispatcher {
    agent_spawner: Arc<dyn AgentSpawner>,
    event_tx: broadcast::Sender<DispatchEvent>,
}

impl EventDispatcher {
    /// Create a new event dispatcher
    pub fn new(agent_spawner: Arc<dyn AgentSpawner>) -> Self {
        let (event_tx, _) = broadcast::channel(1000);
        Self {
            agent_spawner,
            event_tx,
        }
    }

    /// Route a NATS message to the appropriate handler
    pub async fn dispatch(&self, subject: &str, payload: Vec<u8>) -> Result<(), AppError> {
        let event = match subject {
            subjects::EVENTS_VOICE => {
                let text = String::from_utf8_lossy(&payload).to_string();
                DispatchEvent::Voice(text)
            }
            subjects::EVENTS_HOTKEY => {
                let action = String::from_utf8_lossy(&payload).to_string();
                DispatchEvent::Hotkey(action)
            }
            subjects::EVENTS_MCP => {
                let data = String::from_utf8_lossy(&payload).to_string();
                DispatchEvent::Mcp(data)
            }
            _ if subject.starts_with("agents.") => {
                let agent_id = subject
                    .strip_prefix("agents.")
                    .unwrap_or("unknown")
                    .to_string();
                DispatchEvent::Agent(agent_id, payload)
            }
            _ => {
                tracing::warn!("Unknown subject: {}", subject);
                return Ok(());
            }
        };

        // Send to event channel for other subscribers
        let _ = self.event_tx.send(event.clone());

        // Handle the event
        self.handle_event(event).await
    }

    /// Handle a specific event type
    async fn handle_event(&self, event: DispatchEvent) -> Result<(), AppError> {
        match event {
            DispatchEvent::Voice(text) => {
                tracing::info!("Voice event: {}", text);
                // Forward to default agent for processing
                self.agent_spawner
                    .send_event("default-agent", format!("voice:{}", text))
                    .await;
            }
            DispatchEvent::Hotkey(action) => {
                tracing::info!("Hotkey event: {}", action);
                // Trigger voice recording or show window
                self.agent_spawner
                    .send_event("default-agent", format!("hotkey:{}", action))
                    .await;
            }
            DispatchEvent::Mcp(data) => {
                tracing::info!("MCP event: {}", data);
                // Forward to MCP handler agent
                self.agent_spawner
                    .send_event("mcp-agent", format!("mcp:{}", data))
                    .await;
            }
            DispatchEvent::Agent(agent_id, payload) => {
                let payload_str = String::from_utf8_lossy(&payload).to_string();
                tracing::info!("Agent event for {}: {}", agent_id, payload_str);
                self.agent_spawner.send_event(&agent_id, payload_str).await;
            }
            DispatchEvent::Health(data) => {
                tracing::debug!("Health event: {}", data);
                // Health events are for monitoring, no agent forwarding needed
            }
        }
        Ok(())
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<DispatchEvent> {
        self.event_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentManager;

    #[tokio::test]
    async fn test_event_dispatch() {
        let manager = AgentManager::new(std::env::temp_dir().join("test.db"));
        let spawner: Arc<dyn AgentSpawner> = Arc::new(manager);
        let dispatcher = EventDispatcher::new(spawner);

        // Test voice event
        let result = dispatcher
            .dispatch("events.voice", b"hello world".to_vec())
            .await;
        assert!(result.is_ok());

        // Test hotkey event
        let result = dispatcher
            .dispatch("events.hotkey", b"trigger".to_vec())
            .await;
        assert!(result.is_ok());

        // Test agent event
        let result = dispatcher.dispatch("agents.test", b"data".to_vec()).await;
        assert!(result.is_ok());
    }
}
