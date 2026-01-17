//! Event dispatcher for routing NATS messages to appropriate handlers
//! Provides centralized event routing and processing

use crate::AppError;
use crate::agents::AgentSpawner;
use crate::nats_mq::{DispatchEvent, subjects};
use gestura_core::interaction::{InteractionContext, InteractionEvent};
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
            subjects::EVENTS_GESTURE => {
                let data = String::from_utf8_lossy(&payload).to_string();
                DispatchEvent::Gesture(data)
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
            DispatchEvent::Gesture(data) => {
                tracing::info!("Gesture event: {}", data);
                // Parse the interaction event and build context
                if let Ok(interaction_event) =
                    serde_json::from_str::<InteractionEvent>(&data)
                {
                    let ctx = InteractionContext::new("gesture-agent")
                        .with_interaction(interaction_event);
                    tracing::debug!(
                        agent_id = %ctx.agent_id,
                        tool_hints = ?ctx.tool_hints,
                        expects_voice = %ctx.expects_voice_response,
                        "Built interaction context for gesture"
                    );
                    // Forward to gesture agent with context
                    self.agent_spawner
                        .send_event("gesture-agent", format!("gesture:{}", data))
                        .await;
                } else {
                    tracing::warn!("Failed to parse gesture event: {}", data);
                }
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
    use gestura_core::interaction::GestureType;

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

    #[tokio::test]
    async fn test_gesture_event_dispatch() {
        let manager = AgentManager::new(std::env::temp_dir().join("test_gesture.db"));
        let spawner: Arc<dyn AgentSpawner> = Arc::new(manager);
        let dispatcher = EventDispatcher::new(spawner);

        // Create a gesture interaction event
        let gesture_event = InteractionEvent::gesture(GestureType::DoubleTap, "ring", 0.95);
        let payload = serde_json::to_string(&gesture_event).unwrap();

        // Test gesture event dispatch
        let result = dispatcher
            .dispatch("events.gesture", payload.as_bytes().to_vec())
            .await;
        assert!(result.is_ok());
    }
}
