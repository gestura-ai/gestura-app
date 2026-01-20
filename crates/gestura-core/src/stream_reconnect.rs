//! Stream Reconnection Logic
//!
//! Provides automatic reconnection for dropped streaming connections
//! with exponential backoff and state preservation.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;

/// Default maximum reconnection attempts
pub const DEFAULT_MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// Default initial backoff delay in milliseconds
pub const DEFAULT_INITIAL_BACKOFF_MS: u64 = 1000;

/// Default maximum backoff delay in milliseconds
pub const DEFAULT_MAX_BACKOFF_MS: u64 = 30000;

/// Default backoff multiplier
pub const DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;

/// Reconnection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconnectState {
    /// Not attempting reconnection
    Idle,
    /// Waiting before next attempt
    Waiting,
    /// Currently attempting to reconnect
    Connecting,
    /// Successfully reconnected
    Connected,
    /// All attempts exhausted
    Failed,
}

/// Reconnection event for frontend notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReconnectEvent {
    /// Starting reconnection attempt
    AttemptStarted {
        /// Current attempt number (1-indexed)
        attempt: u32,
        /// Maximum attempts
        max_attempts: u32,
    },
    /// Waiting before next attempt
    Waiting {
        /// Delay in milliseconds
        delay_ms: u64,
        /// Reason for reconnection
        reason: String,
    },
    /// Reconnection succeeded
    Connected {
        /// Total attempts made
        attempts: u32,
        /// Total time spent reconnecting in milliseconds
        total_time_ms: u64,
    },
    /// Reconnection failed
    Failed {
        /// Total attempts made
        attempts: u32,
        /// Final error message
        error: String,
    },
}

/// Configuration for reconnection behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectConfig {
    /// Maximum number of reconnection attempts
    pub max_attempts: u32,
    /// Initial backoff delay in milliseconds
    pub initial_backoff_ms: u64,
    /// Maximum backoff delay in milliseconds
    pub max_backoff_ms: u64,
    /// Backoff multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Whether to add jitter to backoff delays
    pub jitter: bool,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_RECONNECT_ATTEMPTS,
            initial_backoff_ms: DEFAULT_INITIAL_BACKOFF_MS,
            max_backoff_ms: DEFAULT_MAX_BACKOFF_MS,
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            jitter: true,
        }
    }
}

impl ReconnectConfig {
    /// Calculate backoff delay for a given attempt
    pub fn backoff_delay_ms(&self, attempt: u32) -> u64 {
        let base_delay = self.initial_backoff_ms as f64
            * self
                .backoff_multiplier
                .powi(attempt.saturating_sub(1) as i32);
        let delay = (base_delay as u64).min(self.max_backoff_ms);

        if self.jitter {
            // Add up to 25% jitter
            let jitter = (delay as f64 * 0.25 * rand_jitter()) as u64;
            delay.saturating_add(jitter)
        } else {
            delay
        }
    }
}

/// Simple pseudo-random jitter (0.0 to 1.0)
fn rand_jitter() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1000) as f64 / 1000.0
}

/// Stream state that can be preserved across reconnections
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamState {
    /// Number of chunks received before disconnect
    pub chunks_received: u64,
    /// Total bytes received before disconnect
    pub bytes_received: u64,
    /// Last successful chunk timestamp (ms since stream start)
    pub last_chunk_time_ms: u64,
    /// Whether the stream was in the middle of a tool call
    pub in_tool_call: bool,
    /// Current tool call ID if in progress
    pub current_tool_id: Option<String>,
    /// Accumulated tool arguments if in progress
    pub tool_args_buffer: String,
}

impl StreamState {
    /// Create a new empty stream state
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a chunk received
    pub fn record_chunk(&mut self, bytes: u64, time_ms: u64) {
        self.chunks_received += 1;
        self.bytes_received += bytes;
        self.last_chunk_time_ms = time_ms;
    }

    /// Start a tool call
    pub fn start_tool_call(&mut self, id: String) {
        self.in_tool_call = true;
        self.current_tool_id = Some(id);
        self.tool_args_buffer.clear();
    }

    /// Append tool arguments
    pub fn append_tool_args(&mut self, args: &str) {
        self.tool_args_buffer.push_str(args);
    }

    /// End tool call
    pub fn end_tool_call(&mut self) {
        self.in_tool_call = false;
        self.current_tool_id = None;
        self.tool_args_buffer.clear();
    }

    /// Check if stream can be resumed
    pub fn can_resume(&self) -> bool {
        // Can resume if we haven't received any chunks yet
        // or if we're not in the middle of a tool call
        self.chunks_received == 0 || !self.in_tool_call
    }
}

/// Reconnection manager for streaming connections
pub struct ReconnectManager {
    config: ReconnectConfig,
    state: ReconnectState,
    attempt_count: Arc<AtomicU32>,
    start_time: Option<Instant>,
    stream_state: StreamState,
    event_tx: Option<mpsc::Sender<ReconnectEvent>>,
}

impl ReconnectManager {
    /// Create a new reconnection manager
    pub fn new(config: ReconnectConfig) -> Self {
        Self {
            config,
            state: ReconnectState::Idle,
            attempt_count: Arc::new(AtomicU32::new(0)),
            start_time: None,
            stream_state: StreamState::new(),
            event_tx: None,
        }
    }

    /// Create with event channel for notifications
    pub fn with_events(config: ReconnectConfig, tx: mpsc::Sender<ReconnectEvent>) -> Self {
        Self {
            config,
            state: ReconnectState::Idle,
            attempt_count: Arc::new(AtomicU32::new(0)),
            start_time: None,
            stream_state: StreamState::new(),
            event_tx: Some(tx),
        }
    }

    /// Get current reconnection state
    pub fn state(&self) -> ReconnectState {
        self.state
    }

    /// Get current attempt count
    pub fn attempt_count(&self) -> u32 {
        self.attempt_count.load(Ordering::SeqCst)
    }

    /// Get stream state
    pub fn stream_state(&self) -> &StreamState {
        &self.stream_state
    }

    /// Get mutable stream state
    pub fn stream_state_mut(&mut self) -> &mut StreamState {
        &mut self.stream_state
    }

    /// Check if more attempts are available
    pub fn can_retry(&self) -> bool {
        self.attempt_count() < self.config.max_attempts
    }

    /// Start a reconnection attempt
    pub async fn start_attempt(&mut self) -> Option<u64> {
        if !self.can_retry() {
            self.state = ReconnectState::Failed;
            if let Some(ref tx) = self.event_tx {
                let _ = tx
                    .send(ReconnectEvent::Failed {
                        attempts: self.attempt_count(),
                        error: "Maximum reconnection attempts exceeded".to_string(),
                    })
                    .await;
            }
            return None;
        }

        let attempt = self.attempt_count.fetch_add(1, Ordering::SeqCst) + 1;

        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Calculate backoff delay
        let delay_ms = self.config.backoff_delay_ms(attempt);

        self.state = ReconnectState::Waiting;
        if let Some(ref tx) = self.event_tx {
            let _ = tx
                .send(ReconnectEvent::Waiting {
                    delay_ms,
                    reason: format!("Attempt {} of {}", attempt, self.config.max_attempts),
                })
                .await;
        }

        Some(delay_ms)
    }

    /// Mark as connecting
    pub async fn mark_connecting(&mut self) {
        self.state = ReconnectState::Connecting;
        if let Some(ref tx) = self.event_tx {
            let _ = tx
                .send(ReconnectEvent::AttemptStarted {
                    attempt: self.attempt_count(),
                    max_attempts: self.config.max_attempts,
                })
                .await;
        }
    }

    /// Mark as successfully connected
    pub async fn mark_connected(&mut self) {
        self.state = ReconnectState::Connected;
        let total_time_ms = self
            .start_time
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        if let Some(ref tx) = self.event_tx {
            let _ = tx
                .send(ReconnectEvent::Connected {
                    attempts: self.attempt_count(),
                    total_time_ms,
                })
                .await;
        }
    }

    /// Mark as failed with error
    pub async fn mark_failed(&mut self, error: &str) {
        self.state = ReconnectState::Failed;
        if let Some(ref tx) = self.event_tx {
            let _ = tx
                .send(ReconnectEvent::Failed {
                    attempts: self.attempt_count(),
                    error: error.to_string(),
                })
                .await;
        }
    }

    /// Reset for a new stream
    pub fn reset(&mut self) {
        self.state = ReconnectState::Idle;
        self.attempt_count.store(0, Ordering::SeqCst);
        self.start_time = None;
        self.stream_state = StreamState::new();
    }

    /// Get configuration
    pub fn config(&self) -> &ReconnectConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_config_default() {
        let config = ReconnectConfig::default();
        assert_eq!(config.max_attempts, DEFAULT_MAX_RECONNECT_ATTEMPTS);
        assert_eq!(config.initial_backoff_ms, DEFAULT_INITIAL_BACKOFF_MS);
        assert!(config.jitter);
    }

    #[test]
    fn test_backoff_delay_exponential() {
        let config = ReconnectConfig {
            initial_backoff_ms: 1000,
            backoff_multiplier: 2.0,
            max_backoff_ms: 30000,
            jitter: false,
            ..Default::default()
        };

        assert_eq!(config.backoff_delay_ms(1), 1000);
        assert_eq!(config.backoff_delay_ms(2), 2000);
        assert_eq!(config.backoff_delay_ms(3), 4000);
        assert_eq!(config.backoff_delay_ms(4), 8000);
    }

    #[test]
    fn test_backoff_delay_max_cap() {
        let config = ReconnectConfig {
            initial_backoff_ms: 1000,
            backoff_multiplier: 2.0,
            max_backoff_ms: 5000,
            jitter: false,
            ..Default::default()
        };

        assert_eq!(config.backoff_delay_ms(10), 5000);
    }

    #[test]
    fn test_stream_state_record_chunk() {
        let mut state = StreamState::new();
        state.record_chunk(100, 1000);
        assert_eq!(state.chunks_received, 1);
        assert_eq!(state.bytes_received, 100);
        assert_eq!(state.last_chunk_time_ms, 1000);
    }

    #[test]
    fn test_stream_state_tool_call() {
        let mut state = StreamState::new();
        state.start_tool_call("tool-1".to_string());
        assert!(state.in_tool_call);
        assert_eq!(state.current_tool_id, Some("tool-1".to_string()));

        state.append_tool_args("{\"arg\":\"value\"}");
        assert_eq!(state.tool_args_buffer, "{\"arg\":\"value\"}");

        state.end_tool_call();
        assert!(!state.in_tool_call);
        assert!(state.current_tool_id.is_none());
    }

    #[test]
    fn test_stream_state_can_resume() {
        let mut state = StreamState::new();
        assert!(state.can_resume()); // No chunks yet

        state.record_chunk(100, 1000);
        assert!(state.can_resume()); // Has chunks but not in tool call

        state.start_tool_call("tool-1".to_string());
        assert!(!state.can_resume()); // In tool call
    }

    #[test]
    fn test_reconnect_manager_can_retry() {
        let manager = ReconnectManager::new(ReconnectConfig {
            max_attempts: 3,
            ..Default::default()
        });
        assert!(manager.can_retry());
        assert_eq!(manager.attempt_count(), 0);
    }

    #[tokio::test]
    async fn test_reconnect_manager_start_attempt() {
        let mut manager = ReconnectManager::new(ReconnectConfig {
            max_attempts: 3,
            initial_backoff_ms: 1000,
            jitter: false,
            ..Default::default()
        });

        let delay = manager.start_attempt().await;
        assert!(delay.is_some());
        assert_eq!(delay.unwrap(), 1000);
        assert_eq!(manager.attempt_count(), 1);
        assert_eq!(manager.state(), ReconnectState::Waiting);
    }

    #[tokio::test]
    async fn test_reconnect_manager_exhausted() {
        let mut manager = ReconnectManager::new(ReconnectConfig {
            max_attempts: 1,
            ..Default::default()
        });

        let _ = manager.start_attempt().await;
        let delay = manager.start_attempt().await;
        assert!(delay.is_none());
        assert_eq!(manager.state(), ReconnectState::Failed);
    }

    #[tokio::test]
    async fn test_reconnect_manager_reset() {
        let mut manager = ReconnectManager::new(ReconnectConfig::default());
        let _ = manager.start_attempt().await;
        manager.reset();
        assert_eq!(manager.attempt_count(), 0);
        assert_eq!(manager.state(), ReconnectState::Idle);
    }
}
