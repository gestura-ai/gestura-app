//! In-memory message bus fallback for offline operation
//! Provides message routing when NATS is unavailable

use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// In-memory message bus for offline operation
#[derive(Clone)]
pub struct MemoryBus {
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<Vec<u8>>>>>,
    #[allow(dead_code)]
    subscribers: Arc<RwLock<HashMap<String, Vec<broadcast::Receiver<Vec<u8>>>>>>,
    message_history: Arc<RwLock<Vec<StoredMessage>>>,
    max_history: usize,
}

/// Stored message for replay functionality
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub subject: String,
    pub payload: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl MemoryBus {
    /// Create a new in-memory message bus
    pub fn new(max_history: usize) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            message_history: Arc::new(RwLock::new(Vec::new())),
            max_history,
        }
    }

    /// Publish a message to a subject
    pub async fn publish(&self, subject: &str, payload: Vec<u8>) -> Result<(), AppError> {
        // Store message in history
        let message = StoredMessage {
            subject: subject.to_string(),
            payload: payload.clone(),
            timestamp: chrono::Utc::now(),
        };
        
        {
            let mut history = self.message_history.write().await;
            history.push(message);
            
            // Trim history if it exceeds max size
            if history.len() > self.max_history {
                history.remove(0);
            }
        }

        // Get or create channel for subject
        let sender = {
            let mut channels = self.channels.write().await;
            if let Some(sender) = channels.get(subject) {
                sender.clone()
            } else {
                let (sender, _) = broadcast::channel(1000);
                channels.insert(subject.to_string(), sender.clone());
                sender
            }
        };

        // Send to subscribers
        let _ = sender.send(payload);
        tracing::debug!("Published message to subject: {}", subject);
        Ok(())
    }

    /// Subscribe to a subject
    pub async fn subscribe(&self, subject: &str) -> Result<broadcast::Receiver<Vec<u8>>, AppError> {
        let mut channels = self.channels.write().await;
        let sender = if let Some(sender) = channels.get(subject) {
            sender.clone()
        } else {
            let (sender, _) = broadcast::channel(1000);
            channels.insert(subject.to_string(), sender.clone());
            sender
        };

        let receiver = sender.subscribe();
        tracing::debug!("Subscribed to subject: {}", subject);
        Ok(receiver)
    }

    /// Get message history for a subject
    pub async fn get_history(&self, subject: &str, limit: Option<usize>) -> Vec<StoredMessage> {
        let history = self.message_history.read().await;
        let filtered: Vec<StoredMessage> = history
            .iter()
            .filter(|msg| msg.subject == subject)
            .cloned()
            .collect();

        if let Some(limit) = limit {
            filtered.into_iter().rev().take(limit).rev().collect()
        } else {
            filtered
        }
    }

    /// Replay messages for a subject since a timestamp
    pub async fn replay_since(&self, subject: &str, since: chrono::DateTime<chrono::Utc>) -> Result<(), AppError> {
        let history = self.message_history.read().await;
        let messages_to_replay: Vec<StoredMessage> = history
            .iter()
            .filter(|msg| msg.subject == subject && msg.timestamp > since)
            .cloned()
            .collect();

        drop(history);

        let replay_count = messages_to_replay.len();
        for message in messages_to_replay {
            self.publish(&message.subject, message.payload).await?;
        }

        tracing::info!("Replayed {} messages for subject: {}", replay_count, subject);
        Ok(())
    }

    /// Clear message history
    pub async fn clear_history(&self) {
        let mut history = self.message_history.write().await;
        history.clear();
        tracing::info!("Cleared message history");
    }

    /// Get statistics about the message bus
    pub async fn get_stats(&self) -> BusStats {
        let channels = self.channels.read().await;
        let history = self.message_history.read().await;
        
        BusStats {
            active_channels: channels.len(),
            total_messages: history.len(),
            max_history: self.max_history,
        }
    }
}

/// Statistics about the message bus
#[derive(Debug, Clone, serde::Serialize)]
pub struct BusStats {
    pub active_channels: usize,
    pub total_messages: usize,
    pub max_history: usize,
}

/// Unified message bus that can use NATS or fallback to memory
#[derive(Clone)]
pub enum UnifiedBus {
    #[cfg(feature = "nats")]
    Nats(crate::NatsConn),
    Memory(MemoryBus),
}

impl UnifiedBus {
    /// Create a unified bus, preferring NATS if available
    pub async fn new() -> Self {
        #[cfg(feature = "nats")]
        {
            match crate::nats_mq::connect_nats("nats://127.0.0.1:4222").await {
                Ok(conn) => {
                    tracing::info!("Using NATS message bus");
                    UnifiedBus::Nats(conn)
                }
                Err(_) => {
                    tracing::warn!("NATS unavailable, using in-memory message bus");
                    UnifiedBus::Memory(MemoryBus::new(10000))
                }
            }
        }
        #[cfg(not(feature = "nats"))]
        {
            tracing::info!("Using in-memory message bus (NATS feature disabled)");
            UnifiedBus::Memory(MemoryBus::new(10000))
        }
    }

    /// Publish a message
    pub async fn publish(&self, subject: &str, payload: Vec<u8>) -> Result<(), AppError> {
        match self {
            #[cfg(feature = "nats")]
            UnifiedBus::Nats(conn) => {
                conn.publish(subject.to_string(), bytes::Bytes::from(payload)).await
                    .map_err(|e| AppError::Nats(e.to_string()))
            }
            UnifiedBus::Memory(bus) => bus.publish(subject, payload).await,
        }
    }

    /// Subscribe to a subject (simplified interface)
    pub async fn subscribe_simple(&self, subject: &str) -> Result<broadcast::Receiver<Vec<u8>>, AppError> {
        match self {
            #[cfg(feature = "nats")]
            UnifiedBus::Nats(_) => {
                // For NATS, we'd need to implement a bridge to broadcast channels
                // For now, return an error indicating this needs NATS-specific handling
                Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "NATS subscription requires specialized handling"
                )))
            }
            UnifiedBus::Memory(bus) => bus.subscribe(subject).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_memory_bus_publish_subscribe() {
        let bus = MemoryBus::new(100);
        
        // Subscribe to a subject
        let mut receiver = bus.subscribe("test.subject").await.unwrap();
        
        // Publish a message
        let test_data = b"hello world".to_vec();
        bus.publish("test.subject", test_data.clone()).await.unwrap();
        
        // Receive the message
        let received = receiver.recv().await.unwrap();
        assert_eq!(received, test_data);
    }

    #[tokio::test]
    async fn test_message_history() {
        let bus = MemoryBus::new(100);
        
        // Publish some messages
        bus.publish("test.history", b"message1".to_vec()).await.unwrap();
        sleep(Duration::from_millis(10)).await;
        bus.publish("test.history", b"message2".to_vec()).await.unwrap();
        
        // Get history
        let history = bus.get_history("test.history", None).await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].payload, b"message1");
        assert_eq!(history[1].payload, b"message2");
    }

    #[tokio::test]
    async fn test_history_limit() {
        let bus = MemoryBus::new(2); // Small limit for testing
        
        // Publish more messages than the limit
        for i in 0..5 {
            bus.publish("test.limit", format!("message{}", i).into_bytes()).await.unwrap();
        }
        
        let stats = bus.get_stats().await;
        assert_eq!(stats.total_messages, 2); // Should be limited to 2
    }
}
