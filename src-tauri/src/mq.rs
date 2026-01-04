//! MQ integration (Stage 3)
//! Provides a minimal Redis pub/sub consumer that forwards messages to agents.

use std::time::Duration;
use tokio::time;

use tokio::sync::mpsc;

/// Messages received from MQ
#[derive(Debug, Clone)]
pub struct MqMessage(pub String);

/// Minimal MQ subscriber trait so we can mock in tests.
#[async_trait::async_trait]
pub trait MqSubscriber: Send + Sync {
    async fn subscribe(&self, topics: &[&str], tx: mpsc::Sender<MqMessage>);
}

/// Mock subscriber that generates periodic messages
pub struct MockSubscriber;

#[async_trait::async_trait]
impl MqSubscriber for MockSubscriber {
    async fn subscribe(&self, _topics: &[&str], tx: mpsc::Sender<MqMessage>) {
        let mut interval = time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let _ = tx.send(MqMessage("mock-event".into())).await;
        }
    }
}

