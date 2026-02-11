//! Stream Health Monitoring
//!
//! Provides health monitoring, heartbeat detection, and timeout handling
//! for streaming LLM responses.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;

/// Default heartbeat interval in seconds
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Default stream timeout in seconds (no activity)
pub const DEFAULT_STREAM_TIMEOUT_SECS: u64 = 120;

/// Stream health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamHealthStatus {
    /// Stream is healthy and receiving data
    Healthy,
    /// Stream is idle but within timeout
    Idle,
    /// Stream is stalled (no activity for extended period)
    Stalled,
    /// Stream has timed out
    TimedOut,
    /// Stream has been cancelled
    Cancelled,
    /// Stream completed successfully
    Completed,
    /// Stream failed with error
    Failed,
}

/// Stream health event for frontend notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamHealthEvent {
    /// Heartbeat received
    Heartbeat {
        /// Time since last activity in milliseconds
        idle_ms: u64,
    },
    /// Stream status changed
    StatusChanged {
        /// Previous status
        from: StreamHealthStatus,
        /// New status
        to: StreamHealthStatus,
    },
    /// Stream timeout warning
    TimeoutWarning {
        /// Seconds until timeout
        seconds_remaining: u64,
    },
    /// Stream recovered from stall
    Recovered {
        /// Duration of stall in milliseconds
        stall_duration_ms: u64,
    },
}

/// Configuration for stream health monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamHealthConfig {
    /// Heartbeat interval in seconds
    pub heartbeat_interval_secs: u64,
    /// Stream timeout in seconds (no activity)
    pub timeout_secs: u64,
    /// Warning threshold before timeout (percentage)
    pub warning_threshold_percent: u8,
    /// Enable automatic recovery attempts
    pub auto_recovery: bool,
    /// Maximum recovery attempts
    pub max_recovery_attempts: u32,
}

impl Default for StreamHealthConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_secs: DEFAULT_HEARTBEAT_INTERVAL_SECS,
            timeout_secs: DEFAULT_STREAM_TIMEOUT_SECS,
            warning_threshold_percent: 80,
            auto_recovery: true,
            max_recovery_attempts: 3,
        }
    }
}

/// Stream health monitor
///
/// Tracks stream activity and provides health status updates.
pub struct StreamHealthMonitor {
    config: StreamHealthConfig,
    status: Arc<AtomicU64>,
    last_activity: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    start_time: Instant,
    event_tx: Option<mpsc::Sender<StreamHealthEvent>>,
}

impl StreamHealthMonitor {
    /// Create a new stream health monitor
    pub fn new(config: StreamHealthConfig) -> Self {
        Self {
            config,
            status: Arc::new(AtomicU64::new(StreamHealthStatus::Healthy as u64)),
            last_activity: Arc::new(AtomicU64::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            start_time: Instant::now(),
            event_tx: None,
        }
    }

    /// Create with event channel for notifications
    pub fn with_events(config: StreamHealthConfig, tx: mpsc::Sender<StreamHealthEvent>) -> Self {
        Self {
            config,
            status: Arc::new(AtomicU64::new(StreamHealthStatus::Healthy as u64)),
            last_activity: Arc::new(AtomicU64::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            start_time: Instant::now(),
            event_tx: Some(tx),
        }
    }

    /// Record activity on the stream
    pub fn record_activity(&self) {
        let now = self.start_time.elapsed().as_millis() as u64;
        self.last_activity.store(now, Ordering::SeqCst);

        // Check if we're recovering from a stall
        let current_status = self.status();
        if current_status == StreamHealthStatus::Stalled {
            self.set_status(StreamHealthStatus::Healthy);
        }
    }

    /// Get current stream status
    pub fn status(&self) -> StreamHealthStatus {
        let val = self.status.load(Ordering::SeqCst);
        match val {
            0 => StreamHealthStatus::Healthy,
            1 => StreamHealthStatus::Idle,
            2 => StreamHealthStatus::Stalled,
            3 => StreamHealthStatus::TimedOut,
            4 => StreamHealthStatus::Cancelled,
            5 => StreamHealthStatus::Completed,
            _ => StreamHealthStatus::Failed,
        }
    }

    /// Set stream status
    fn set_status(&self, status: StreamHealthStatus) {
        let old = self.status();
        self.status.store(status as u64, Ordering::SeqCst);

        if old != status
            && let Some(ref tx) = self.event_tx
        {
            let _ = tx.try_send(StreamHealthEvent::StatusChanged {
                from: old,
                to: status,
            });
        }
    }

    /// Mark stream as cancelled
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.set_status(StreamHealthStatus::Cancelled);
    }

    /// Check if stream is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Mark stream as completed
    pub fn complete(&self) {
        self.set_status(StreamHealthStatus::Completed);
    }

    /// Mark stream as failed
    pub fn fail(&self) {
        self.set_status(StreamHealthStatus::Failed);
    }

    /// Get time since last activity in milliseconds
    pub fn idle_time_ms(&self) -> u64 {
        let now = self.start_time.elapsed().as_millis() as u64;
        let last = self.last_activity.load(Ordering::SeqCst);
        now.saturating_sub(last)
    }

    /// Check stream health and update status
    ///
    /// Returns true if stream is still healthy/recoverable
    pub fn check_health(&self) -> bool {
        if self.is_cancelled() {
            return false;
        }

        let idle_ms = self.idle_time_ms();
        let timeout_ms = self.config.timeout_secs * 1000;
        let warning_ms = timeout_ms * self.config.warning_threshold_percent as u64 / 100;
        let stall_threshold_ms = self.config.heartbeat_interval_secs * 2 * 1000;

        if idle_ms >= timeout_ms {
            self.set_status(StreamHealthStatus::TimedOut);
            return false;
        }

        if idle_ms >= warning_ms
            && let Some(ref tx) = self.event_tx
        {
            let remaining = (timeout_ms - idle_ms) / 1000;
            let _ = tx.try_send(StreamHealthEvent::TimeoutWarning {
                seconds_remaining: remaining,
            });
        }

        if idle_ms >= stall_threshold_ms {
            self.set_status(StreamHealthStatus::Stalled);
        } else if idle_ms >= self.config.heartbeat_interval_secs * 1000 {
            self.set_status(StreamHealthStatus::Idle);
        } else {
            self.set_status(StreamHealthStatus::Healthy);
        }

        true
    }

    /// Get configuration
    pub fn config(&self) -> &StreamHealthConfig {
        &self.config
    }

    /// Create a handle for sharing across tasks
    pub fn handle(&self) -> StreamHealthHandle {
        StreamHealthHandle {
            status: Arc::clone(&self.status),
            last_activity: Arc::clone(&self.last_activity),
            cancelled: Arc::clone(&self.cancelled),
            start_time: self.start_time,
        }
    }
}

/// Lightweight handle for stream health monitoring
///
/// Can be cloned and shared across tasks for recording activity.
#[derive(Clone)]
pub struct StreamHealthHandle {
    status: Arc<AtomicU64>,
    last_activity: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    start_time: Instant,
}

impl StreamHealthHandle {
    /// Record activity on the stream
    pub fn record_activity(&self) {
        let now = self.start_time.elapsed().as_millis() as u64;
        self.last_activity.store(now, Ordering::SeqCst);
    }

    /// Check if stream is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Get current status
    pub fn status(&self) -> StreamHealthStatus {
        let val = self.status.load(Ordering::SeqCst);
        match val {
            0 => StreamHealthStatus::Healthy,
            1 => StreamHealthStatus::Idle,
            2 => StreamHealthStatus::Stalled,
            3 => StreamHealthStatus::TimedOut,
            4 => StreamHealthStatus::Cancelled,
            5 => StreamHealthStatus::Completed,
            _ => StreamHealthStatus::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_health_config_default() {
        let config = StreamHealthConfig::default();
        assert_eq!(
            config.heartbeat_interval_secs,
            DEFAULT_HEARTBEAT_INTERVAL_SECS
        );
        assert_eq!(config.timeout_secs, DEFAULT_STREAM_TIMEOUT_SECS);
        assert!(config.auto_recovery);
    }

    #[test]
    fn test_stream_health_monitor_initial_status() {
        let monitor = StreamHealthMonitor::new(StreamHealthConfig::default());
        assert_eq!(monitor.status(), StreamHealthStatus::Healthy);
        assert!(!monitor.is_cancelled());
    }

    #[test]
    fn test_stream_health_monitor_cancel() {
        let monitor = StreamHealthMonitor::new(StreamHealthConfig::default());
        monitor.cancel();
        assert!(monitor.is_cancelled());
        assert_eq!(monitor.status(), StreamHealthStatus::Cancelled);
    }

    #[test]
    fn test_stream_health_monitor_complete() {
        let monitor = StreamHealthMonitor::new(StreamHealthConfig::default());
        monitor.complete();
        assert_eq!(monitor.status(), StreamHealthStatus::Completed);
    }

    #[test]
    fn test_stream_health_monitor_fail() {
        let monitor = StreamHealthMonitor::new(StreamHealthConfig::default());
        monitor.fail();
        assert_eq!(monitor.status(), StreamHealthStatus::Failed);
    }

    #[test]
    fn test_stream_health_monitor_record_activity() {
        let monitor = StreamHealthMonitor::new(StreamHealthConfig::default());
        monitor.record_activity();
        // Activity was just recorded, idle time should be very small
        assert!(monitor.idle_time_ms() < 100);
    }

    #[test]
    fn test_stream_health_handle_clone() {
        let monitor = StreamHealthMonitor::new(StreamHealthConfig::default());
        let handle1 = monitor.handle();
        let handle2 = handle1.clone();

        handle1.record_activity();
        assert!(!handle2.is_cancelled());
        assert_eq!(handle2.status(), StreamHealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_stream_health_events() {
        let (tx, mut rx) = mpsc::channel(10);
        let monitor = StreamHealthMonitor::with_events(StreamHealthConfig::default(), tx);

        monitor.complete();

        if let Some(event) = rx.recv().await {
            match event {
                StreamHealthEvent::StatusChanged { from, to } => {
                    assert_eq!(from, StreamHealthStatus::Healthy);
                    assert_eq!(to, StreamHealthStatus::Completed);
                }
                _ => panic!("Expected StatusChanged event"),
            }
        }
    }
}
